use super::super::tests::{asked, pace, Sim, State, NS, SET};
use super::*;

async fn check_of(sim: &Sim) -> (Checked, String) {
    let mut said = Vec::new();
    let checked = check(sim, &asked(), &mut |line: &str| said.push(line.to_string())).await;
    (checked, said.join("\n"))
}

async fn planned(sim: &Sim) -> Plan {
    match check_of(sim).await {
        (Checked::Enable(plan), _) => *plan,
        (_, said) => panic!("not planned: {said}"),
    }
}

#[tokio::test]
async fn checksums_on_already_is_said_and_nothing_is_changed() {
    let sim = Sim::new(State {
        checksums: true,
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    let Checked::Nothing(text) = checked else {
        panic!("{said}");
    };
    assert!(
        text.contains("page checksums on already; nothing to do"),
        "{text}"
    );
    assert!(!sim.changed_anything(), "{:#?}", sim.asked());
}

#[tokio::test]
async fn an_external_database_is_refused_with_where_its_checksums_are_set() {
    let sim = Sim::new(State {
        brought: false,
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    assert!(matches!(checked, Checked::Refused), "{said}");
    assert!(said.contains("runs no database it brought"), "{said}");
    assert!(
        said.contains("pg_checksums --enable on the stopped server"),
        "{said}"
    );
    assert!(said.contains("Nothing was changed."), "{said}");
    assert!(!sim.changed_anything());
}

#[tokio::test]
async fn without_a_backup_configured_it_is_refused_before_anything_is_asked() {
    let sim = Sim::new(State {
        backup_configured: false,
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    assert!(matches!(checked, Checked::Refused), "{said}");
    assert!(
        said.contains("no backup configured, and a backup is taken first"),
        "{said}"
    );
    assert!(said.contains("meridian backup key"), "{said}");
    assert!(!sim.changed_anything());

    // Nor without the chart's offline CronJob, which is 3b's.
    let sim = Sim::new(State {
        cron: false,
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    assert!(matches!(checked, Checked::Refused), "{said}");
    assert!(
        said.contains(&format!("no {SET}-offline CronJob")),
        "{said}"
    );
}

#[tokio::test]
async fn the_plan_states_the_downtime_from_the_size_before_it_asks() {
    let plan = planned(&Sim::new(State::default())).await;
    assert_eq!(plan.bytes, Some(2 * 1024 * 1024 * 1024));
    let text = plan_text(&asked(), &plan);
    assert!(
        text.contains("the database  2.0 GiB on member 0's volume, 1 member"),
        "{text}"
    );
    assert!(
        text.contains("downtime      about 5 minutes, an estimate"),
        "{text}"
    );
    assert!(text.contains("1. A backup, now"), "{text}");
    assert!(text.contains("If it fails, nothing is changed."), "{text}");
    assert!(
        text.contains(&format!("in a Job from {SET}-offline")),
        "{text}"
    );
    assert!(!text.contains("standbys"), "{text}");

    let plan = planned(&Sim::new(State::default().ha())).await;
    let text = plan_text(&asked(), &plan);
    assert!(text.contains("3 members, under Patroni"), "{text}");
    assert!(text.contains("member 0 made the primary"), "{text}");
    assert!(
        text.contains(&format!(
            "The standbys' volumes deleted (data-{SET}-1, data-{SET}-2)"
        )),
        "{text}"
    );
    assert!(text.contains("downtime      about 7 minutes"), "{text}");
}

#[tokio::test]
async fn a_backup_that_fails_changes_nothing() {
    let sim = Sim::new(State {
        backup_fails: true,
        ..State::default()
    });
    let plan = planned(&sim).await;
    let mut said = Vec::new();
    let done = apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_000,
        &mut |line: &str| said.push(line.to_string()),
    )
    .await;
    let refusal = done.unwrap_err();
    assert!(
        refusal.starts_with("the backup failed, so nothing was changed"),
        "{refusal}"
    );
    assert!(!sim.changed_anything(), "{:#?}", sim.asked());
    let now = sim.now();
    assert_eq!(now.deployments, State::default().deployments);
    assert_eq!(now.database, 1);
    assert!(!now.checksums);
}

#[tokio::test]
async fn backup_stop_act_start_and_verify_in_that_order() {
    let sim = Sim::new(State::default());
    let plan = planned(&sim).await;
    let mut said = Vec::new();
    let report = apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_000,
        &mut |line: &str| said.push(line.to_string()),
    )
    .await
    .unwrap_or_else(|failed| panic!("{failed}\n{said:#?}"));
    assert!(
        report.contains("Page checksums are on in meridian's database"),
        "{report}"
    );
    assert!(
        report.contains("Backed up meridian to meridian-20261108T100000Z.meridian-backup"),
        "{report}"
    );
    let asked_for = sim.asked();
    let at = |what: &str| {
        asked_for
            .iter()
            .position(|c| c.contains(what))
            .unwrap_or_else(|| panic!("{what} not asked: {asked_for:#?}"))
    };
    let backup = at("<<Job meridian-meridian-runtime-backup-1794000000");
    let stop = at(&format!("kubectl scale deployment -n {NS} --replicas=0"));
    let database_stopped = at(&format!(
        "kubectl scale statefulset {SET} -n {NS} --replicas=0"
    ));
    let act = at("<<Job meridian-meridian-runtime-database-offline");
    let database_started = at(&format!(
        "kubectl scale statefulset {SET} -n {NS} --replicas=1"
    ));
    assert!(
        backup < stop
            && stop < database_stopped
            && database_stopped < act
            && act < database_started,
        "{asked_for:#?}"
    );
    // Every component back at the count it ran at.
    let now = sim.now();
    assert_eq!(now.deployments, State::default().deployments);
    assert_eq!(now.database, 1);
    assert!(now.checksums);
    // Alone, nothing of Patroni's is touched.
    assert!(!asked_for
        .iter()
        .any(|c| c.contains("patronictl") || c.contains("delete pvc")));
}

#[tokio::test]
async fn under_patroni_member_0_is_made_primary_and_the_standbys_are_made_again() {
    let sim = Sim::new(State::default().ha());
    let plan = planned(&sim).await;
    let mut said = Vec::new();
    apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_000,
        &mut |line: &str| said.push(line.to_string()),
    )
    .await
    .unwrap_or_else(|failed| panic!("{failed}\n{said:#?}"));
    let asked_for = sim.asked();
    let at = |what: &str| {
        asked_for
            .iter()
            .position(|c| c.contains(what))
            .unwrap_or_else(|| panic!("{what} not asked: {asked_for:#?}"))
    };
    // Synchronous off first: Patroni switches only to its synchronous
    // standby, and member 0 may be the other.
    assert!(at("edit-config") < at("switchover"));
    assert!(at("switchover") < at(&format!("statefulset {SET} -n {NS} --replicas=0")));
    assert!(at("enable-checksums") < at("delete pvc"));
    assert!(at("delete pvc") < at(&format!("statefulset {SET} -n {NS} --replicas=3")));
    let now = sim.now();
    assert_eq!(now.database, 3);
    assert!(now.checksums);
    let said = said.join("\n");
    assert!(
        said.contains(&format!("switched the primary to {SET}-0")),
        "{said}"
    );
    assert!(said.contains("Patroni makes each again from"), "{said}");
    // A checksum step never clears Patroni's record: member 0 was primary.
    assert!(!asked_for.iter().any(|c| c.contains("delete configmap")));
}

#[tokio::test]
async fn an_act_that_fails_starts_everything_again_as_it_was() {
    let sim = Sim::new(State {
        act_fails: Some("pg_checksums did not finish: no space left on device".into()),
        ..State::default()
    });
    let plan = planned(&sim).await;
    let refusal = apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_000,
        &mut |_: &str| {},
    )
    .await
    .unwrap_err();
    assert!(refusal.contains("no space left on device"), "{refusal}");
    assert!(
        refusal.contains("Everything was started again."),
        "{refusal}"
    );
    assert!(
        refusal.contains("meridian-20261108T100000Z.meridian-backup"),
        "{refusal}"
    );
    let now = sim.now();
    assert_eq!(now.deployments, State::default().deployments);
    assert_eq!(now.database, 1);
    assert!(!now.checksums);
}

fn short() -> Pace {
    Pace {
        poll: std::time::Duration::ZERO,
        timeout: std::time::Duration::from_millis(200),
    }
}

#[tokio::test]
async fn the_releases_cronjobs_are_suspended_for_the_act_and_given_back_as_they_were() {
    let sim = Sim::new(State::default().ha());
    let before = sim.now().cronjobs;
    let plan = planned(&sim).await;
    assert!(
        plan_text(&asked(), &plan).contains("2. The release's CronJobs suspended"),
        "{}",
        plan_text(&asked(), &plan)
    );
    apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_000,
        &mut |_: &str| {},
    )
    .await
    .unwrap();
    assert_eq!(sim.now().cronjobs, before);
    let asked_for = sim.asked();
    let at = |what: &str| {
        asked_for
            .iter()
            .position(|c| c.contains(what))
            .unwrap_or_else(|| panic!("{what} not asked: {asked_for:#?}"))
    };
    // After the backup, before anything stops; back once the database runs.
    let suspended = at("\"suspend\":true");
    assert!(at("<<Job meridian-meridian-runtime-backup-1794000000") < suspended);
    assert!(at("--overwrite meridian.dev/stopped-for-restore=") < suspended);
    assert!(suspended < at(&format!("kubectl scale deployment -n {NS} --replicas=0")));
    assert!(at(&format!("statefulset {SET} -n {NS} --replicas=3")) < at("\"suspend\":false"));
    let record = &asked_for[at("--overwrite meridian.dev/stopped-for-restore=")];
    assert!(record.contains("\"act\":\"enable-checksums\""), "{record}");
    // Cleared once everything runs again.
    assert_eq!(sim.now().stopped_record, None);
}

#[tokio::test]
async fn an_act_that_fails_gives_the_cronjobs_back_too() {
    let sim = Sim::new(State {
        act_fails: Some("pg_checksums did not finish: no space left on device".into()),
        ..State::default()
    });
    let plan = planned(&sim).await;
    assert!(apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_000,
        &mut |_: &str| {}
    )
    .await
    .is_err());
    let now = sim.now();
    assert_eq!(now.cronjobs, State::default().cronjobs);
    assert_eq!(now.stopped_record, None);
}

#[tokio::test]
async fn a_job_left_running_keeps_everything_stopped_and_a_run_again_takes_it_from_there() {
    let sim = Sim::new(State {
        act_hangs: true,
        ..State::default()
    });
    let plan = planned(&sim).await;
    let refusal = apply(
        &sim,
        &asked(),
        &plan,
        &short(),
        1_794_000_000,
        &mut |_: &str| {},
    )
    .await
    .unwrap_err();
    assert!(
        refusal.contains("did not finish within 0s, and is left running"),
        "{refusal}"
    );
    assert!(
        refusal.contains("What its log says last (`meridian logs database-offline -n meridian`)"),
        "{refusal}"
    );
    assert!(
        refusal.contains(
            "`meridian upgrade-deployment --enable-checksums -n meridian` takes it from there"
        ),
        "{refusal}"
    );
    // Nothing started on a volume the Job still has.
    let now = sim.now();
    assert_eq!(now.database, 0);
    assert!(now.deployments.values().all(|n| *n == 0));
    assert!(now.cronjobs["meridian-meridian-runtime-backup"]);
    assert!(now.stopped_record.is_some());

    // While it runs, refused.
    let (checked, said) = check_of(&sim).await;
    assert!(matches!(checked, Checked::Refused), "{said}");
    assert!(said.contains("is still running"), "{said}");

    // Once it has ended: no second backup, nothing stopped twice, the act
    // run again, everything started at its counts and its CronJobs given back.
    sim.state.lock().unwrap().act_hangs = false;
    let (checked, said) = check_of(&sim).await;
    let Checked::Enable(plan) = checked else {
        panic!("{said}");
    };
    assert!(
        said.contains("was stopped by `meridian upgrade-deployment --enable-checksums` at 2026-11-06T21:20:00Z, which did not finish"),
        "{said}"
    );
    let text = plan_text(&asked(), &plan);
    assert!(
        text.contains("The backup the run before took first stands"),
        "{text}"
    );
    assert!(text.contains("Everything is stopped already"), "{text}");
    let asked_before = sim.asked().len();
    let report = apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_100,
        &mut |_: &str| {},
    )
    .await
    .unwrap_or_else(|failed| panic!("{failed}"));
    assert!(report.contains("Page checksums are on"), "{report}");
    let asked_after = &sim.asked()[asked_before..];
    assert!(
        !asked_after
            .iter()
            .any(|c| c.contains("--replicas=0") || c.contains("backup-1794000100")),
        "{asked_after:#?}"
    );
    assert!(
        asked_after.iter().any(|c| c.contains("enable-checksums")),
        "{asked_after:#?}"
    );
    let now = sim.now();
    assert_eq!(now.deployments, State::default().deployments);
    assert_eq!(now.database, 1);
    assert_eq!(now.cronjobs, State::default().cronjobs);
    assert_eq!(now.stopped_record, None);
    assert!(now.checksums);
}

#[tokio::test]
async fn a_database_a_restore_left_stopped_is_left_to_the_restore() {
    let record = Stopped {
        deployments: vec![("meridian-meridian-runtime-conductor".into(), 1)],
        database: 1,
        cronjobs: Vec::new(),
        act: Some("restore-to-time".into()),
        since: Some("2026-11-08T09:00:00Z".into()),
    };
    let sim = Sim::new(State {
        database: 0,
        stopped_record: Some(record.to_json()),
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    assert!(matches!(checked, Checked::Refused), "{said}");
    assert!(
        said.contains("was stopped by `meridian restore --to-time` at 2026-11-08T09:00:00Z"),
        "{said}"
    );
    assert!(
        said.contains("`meridian restore --to-time <time> -n meridian` takes it from there"),
        "{said}"
    );
    assert!(!sim.changed_anything(), "{:#?}", sim.asked());
}

#[tokio::test]
async fn a_stopped_database_no_act_recorded_is_refused() {
    let sim = Sim::new(State {
        database: 0,
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    assert!(matches!(checked, Checked::Refused), "{said}");
    assert!(
        said.contains("is stopped, and no act recorded stopping it"),
        "{said}"
    );
    assert!(!sim.changed_anything());
}

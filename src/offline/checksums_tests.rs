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
        database: 0,
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
    let act = at("enable-checksums");
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

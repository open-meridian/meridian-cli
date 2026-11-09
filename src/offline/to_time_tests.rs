use super::super::tests::{asked, pace, Sim, State, NS, SET};
use super::*;

const NOW: i64 = 1_794_182_400;

#[test]
fn a_time_is_rfc_3339_with_its_offset_and_is_said_in_utc() {
    let at = moment("2026-11-08T09:03:07.25-05:00").unwrap();
    assert_eq!(at.for_the_act, "2026-11-08 14:03:07.25+00");
    assert_eq!(at.said, "2026-11-08T14:03:07.25Z");
    let utc = moment("2026-11-08 14:03:07Z").unwrap();
    assert_eq!(utc.for_the_act, "2026-11-08 14:03:07+00");
    assert_eq!(utc.unix_seconds, at.unix_seconds);
    // Across midnight and a year's end.
    assert_eq!(
        moment("2026-12-31T23:30:00-01:00").unwrap().for_the_act,
        "2027-01-01 00:30:00+00"
    );
    for wrong in [
        "2026-11-08",
        "2026-11-08T14:03:07",
        "yesterday",
        "2026-13-08T14:03:07Z",
        "2026-11-08T25:03:07Z",
        "2026-11-08T14:03:07+5",
    ] {
        let refused = moment(wrong).unwrap_err();
        assert!(refused.contains("RFC 3339"), "{wrong}: {refused}");
    }
}

fn to() -> Moment {
    moment("2026-11-08T10:00:00Z").unwrap()
}

async fn check_of(sim: &Sim) -> (Result<Plan, ()>, String) {
    let mut said = Vec::new();
    let checked = check(sim, &asked(), &to(), NOW, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;
    (checked, said.join("\n"))
}

#[tokio::test]
async fn without_a_wal_archive_it_is_refused_saying_how_to_turn_one_on() {
    let sim = Sim::new(State::default());
    let (checked, said) = check_of(&sim).await;
    assert!(checked.is_err());
    assert!(
        said.contains("keeps no WAL archive (backup.walArchive is off)"),
        "{said}"
    );
    assert!(
        said.contains(
            "meridian upgrade-deployment -n meridian --set backup.walArchive.enabled=true"
        ),
        "{said}"
    );
    assert!(said.contains("Nothing was changed."), "{said}");
    assert!(!sim.changed_anything());
}

#[tokio::test]
async fn a_time_after_now_is_refused_before_the_cluster_is_asked() {
    let sim = Sim::new(State {
        archive: true,
        ..State::default()
    });
    let mut said = Vec::new();
    let later = moment("2027-01-01T00:00:00Z").unwrap();
    let checked = check(&sim, &asked(), &later, NOW, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;
    assert!(checked.is_err());
    assert!(said
        .join("\n")
        .contains("2027-01-01T00:00:00Z is after now"));
    assert!(sim.asked().is_empty(), "{:#?}", sim.asked());
}

#[tokio::test]
async fn the_plan_names_the_time_the_archive_and_what_stays_stopped_if_it_fails() {
    let sim = Sim::new(State {
        archive: true,
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    let plan = checked.unwrap_or_else(|()| panic!("{said}"));
    assert_eq!(plan.last_archived.as_deref(), Some("2026-11-08T10:00:00Z"));
    let text = plan_text(&asked(), &plan);
    assert!(text.contains("to 2026-11-08T10:00:00Z"), "{text}");
    assert!(
        text.contains("archived  its last segment at 2026-11-08T10:00:00Z"),
        "{text}"
    );
    assert!(
        text.contains("everything stays stopped until a time inside it is named"),
        "{text}"
    );
    assert!(!sim.changed_anything());
}

#[tokio::test]
async fn a_restore_stops_everything_runs_the_act_with_the_time_and_starts_again() {
    let sim = Sim::new(State {
        archive: true,
        ..State::default()
    });
    let (checked, _) = check_of(&sim).await;
    let plan = checked.unwrap();
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
        report.contains("Restored meridian's database to 2026-11-08T10:00:00Z"),
        "{report}"
    );
    let now = sim.now();
    assert_eq!(now.restored_to.as_deref(), Some("2026-11-08 10:00:00+00"));
    assert_eq!(now.deployments, State::default().deployments);
    assert_eq!(now.database, 1);
    let asked_for = sim.asked();
    let at = |what: &str| {
        asked_for
            .iter()
            .position(|c| c.contains(what))
            .unwrap_or_else(|| panic!("{what} not asked: {asked_for:#?}"))
    };
    // The newest WAL archived once nothing writes, before the database stops.
    assert!(at("--replicas=0 meridian") < at("pg_switch_wal"));
    assert!(at("pg_switch_wal") < at(&format!("statefulset {SET} -n {NS} --replicas=0")));
    assert!(at("restore-to-time") < at(&format!("statefulset {SET} -n {NS} --replicas=1")));
    // No switch: the restore makes member 0's volume the archive's anyway.
    assert!(!asked_for.iter().any(|c| c.contains("patronictl")));
}

#[tokio::test]
async fn a_time_outside_the_archive_fails_in_its_own_words_and_everything_stays_stopped() {
    let sim = Sim::new(State {
        archive: true,
        act_fails: Some(
            "the database stopped before it reached 2026-11-08 10:00:00+00: recovery ended \
             before configured recovery target was reached"
                .into(),
        ),
        ..State::default()
    });
    let (checked, _) = check_of(&sim).await;
    let plan = checked.unwrap();
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
    assert!(
        refusal.contains("recovery ended before configured recovery target was reached"),
        "{refusal}"
    );
    assert!(refusal.contains("Everything stays stopped"), "{refusal}");
    assert!(refusal.contains("stays restorable"), "{refusal}");
    assert!(
        refusal.contains("meridian restore --to-time <time> -n meridian"),
        "{refusal}"
    );
    assert!(refusal.contains("name 2026-11-06T21:20:00Z"), "{refusal}");
    let now = sim.now();
    assert!(
        now.deployments.values().all(|n| *n == 0),
        "{:?}",
        now.deployments
    );
    assert_eq!(now.database, 0);
}

#[tokio::test]
async fn under_patroni_the_standbys_and_the_old_primarys_record_are_made_again() {
    let mut state = State::default().ha();
    state.archive = true;
    let sim = Sim::new(state);
    let (checked, said) = check_of(&sim).await;
    let plan = checked.unwrap_or_else(|()| panic!("{said}"));
    let text = plan_text(&asked(), &plan);
    assert!(
        text.contains(&format!(
            "The standbys' volumes deleted (data-{SET}-1, data-{SET}-2)"
        )),
        "{text}"
    );
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
        report.contains("Its 2 standbys were made again from member 0."),
        "{report}"
    );
    let now = sim.now();
    assert_eq!(now.database, 3);
    // Patroni's record of the old primary cleared; its configuration kept.
    assert_eq!(
        now.configmaps.iter().cloned().collect::<Vec<_>>(),
        [format!("{SET}-config")]
    );
    let asked_for = sim.asked();
    let at = |what: &str| asked_for.iter().position(|c| c.contains(what)).unwrap();
    assert!(at("restore-to-time") < at("delete pvc"));
    assert!(at("delete configmap") < at(&format!("statefulset {SET} -n {NS} --replicas=3")));
}

#[tokio::test]
async fn a_restore_that_left_everything_stopped_is_taken_from_there_when_run_again() {
    // The first: the act fails, everything stays stopped, and what it
    // stopped is recorded on the database's StatefulSet.
    let sim = Sim::new(State {
        archive: true,
        act_fails: Some("recovery ended before configured recovery target was reached".into()),
        ..State::default()
    });
    let (checked, _) = check_of(&sim).await;
    let plan = checked.unwrap();
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
    assert!(sim.now().stopped_record.is_some());
    assert_eq!(sim.now().database, 0);

    // Run again, with a time inside the archive: nothing stopped twice, the
    // act run, and everything started at the counts it ran at.
    sim.state.lock().unwrap().act_fails = None;
    let (checked, said) = check_of(&sim).await;
    let plan = checked.unwrap_or_else(|()| panic!("{said}"));
    assert!(
        said.contains("was stopped by `meridian restore --to-time` at 2026-11-06T21:20:00Z, which did not finish"),
        "{said}"
    );
    // While stopped, the release's CronJobs stay suspended.
    assert!(sim.now().cronjobs["meridian-meridian-runtime-backup"]);
    assert!(plan_text(&asked(), &plan).contains("1. Everything is stopped already"));
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
    .unwrap();
    assert!(report.contains("Restored meridian's database"), "{report}");
    let now = sim.now();
    assert_eq!(now.deployments, State::default().deployments);
    assert_eq!(now.database, 1);
    assert_eq!(now.stopped_record, None);
    assert_eq!(now.cronjobs, State::default().cronjobs);
    let asked_after = &sim.asked()[asked_before..];
    assert!(
        !asked_after.iter().any(|c| c.contains("--replicas=0")),
        "{asked_after:#?}"
    );

    // A stopped database no restore recorded is not taken for one.
    let sim = Sim::new(State {
        archive: true,
        database: 0,
        ..State::default()
    });
    let (checked, said) = check_of(&sim).await;
    assert!(checked.is_err());
    assert!(
        said.contains("is stopped, and no restore recorded stopping it"),
        "{said}"
    );
}

fn short() -> Pace {
    Pace {
        poll: std::time::Duration::ZERO,
        timeout: std::time::Duration::from_millis(200),
    }
}

/// The release's CronJobs as the cluster holds them now.
fn cronjobs(sim: &Sim) -> Vec<(String, bool)> {
    sim.now().cronjobs.into_iter().collect()
}

#[tokio::test]
async fn the_releases_cronjobs_are_suspended_while_stopped_and_given_back_as_they_were() {
    let mut state = State::default().ha();
    state.archive = true;
    // A backup with no schedule: the chart suspends it, and it stays so.
    state
        .cronjobs
        .insert("meridian-meridian-runtime-backup".into(), true);
    let before: Vec<(String, bool)> = state.cronjobs.clone().into_iter().collect();
    let sim = Sim::new(state);
    let (checked, said) = check_of(&sim).await;
    let plan = checked.unwrap_or_else(|()| panic!("{said}"));
    assert!(
        plan_text(&asked(), &plan).contains("1. The release's CronJobs suspended"),
        "{}",
        plan_text(&asked(), &plan)
    );
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
    // Each as it was: the replica comparison running again, the backup
    // still suspended, the offline CronJob untouched.
    assert_eq!(cronjobs(&sim), before);
    let asked_for = sim.asked();
    let at = |what: &str| {
        asked_for
            .iter()
            .position(|c| c.contains(what))
            .unwrap_or_else(|| panic!("{what} not asked: {asked_for:#?}"))
    };
    let replicas = format!("kubectl patch cronjob {SET}-replicas");
    let suspended = asked_for
        .iter()
        .position(|c| c.starts_with(&replicas) && c.contains("\"suspend\":true"))
        .unwrap();
    let resumed = asked_for
        .iter()
        .position(|c| c.starts_with(&replicas) && c.contains("\"suspend\":false"))
        .unwrap();
    // Recorded, then suspended, before anything stops; given back once the
    // database runs again.
    assert!(at("--overwrite meridian.dev/stopped-for-restore=") < suspended);
    assert!(suspended < at("--replicas=0 meridian"));
    assert!(at(&format!("statefulset {SET} -n {NS} --replicas=3")) < resumed);
    // Never touched: suspended before, and the act's own.
    assert!(!asked_for.iter().any(|c| c
        .contains("patch cronjob meridian-meridian-runtime-backup")
        || c.contains(&format!("patch cronjob {SET}-offline"))));
    // The record held each one's state, the act and when.
    let record = asked_for
        .iter()
        .find(|c| c.contains("--overwrite meridian.dev/stopped-for-restore="))
        .unwrap();
    assert!(
        record.contains(&format!(
            "\"cronjobs\":[[\"meridian-meridian-runtime-backup\",true],[\"{SET}-replicas\",false]]"
        )),
        "{record}"
    );
    assert!(record.contains("\"act\":\"restore-to-time\""), "{record}");
    assert!(
        record.contains("\"since\":\"2026-11-06T21:20:00Z\""),
        "{record}"
    );
    let said = said.join("\n");
    assert!(
        said.contains(&format!(
            "suspended 1 CronJob(s) while it is stopped: {SET}-replicas"
        )),
        "{said}"
    );
    assert!(
        said.contains(&format!("resumed 1 CronJob(s): {SET}-replicas")),
        "{said}"
    );
    assert_eq!(sim.now().stopped_record, None);
}

#[tokio::test]
async fn a_job_that_failed_for_another_cause_shows_its_log_and_does_not_ask_for_another_time() {
    let sim = Sim::new(State {
        archive: true,
        act_log: Some(
            "database-offline: Postgres's log, its last lines:\n\
             database-offline: | LOG:  starting point-in-time recovery\n\
             database-offline: | FATAL:  could not open file \"base/1/2\": Permission denied\n"
                .into(),
        ),
        act_fails: Some(
            "the database stopped before it reached 2026-11-08 10:00:00+00: could not open file"
                .into(),
        ),
        ..State::default()
    });
    let (checked, _) = check_of(&sim).await;
    let plan = checked.unwrap();
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
    assert!(
        refusal.contains("failed: the database stopped before it reached"),
        "{refusal}"
    );
    assert!(
        refusal.contains("What its log says (`meridian logs database-offline -n meridian`)"),
        "{refusal}"
    );
    assert!(
        refusal.contains("database-offline: | FATAL:  could not open file \"base/1/2\""),
        "{refusal}"
    );
    assert!(
        !refusal.contains("Name a time inside the archive"),
        "{refusal}"
    );
    assert!(
        refusal.contains(
            "Once the cause above is dealt with, `meridian restore --to-time <time> -n meridian`"
        ),
        "{refusal}"
    );
    // Everything stays stopped, the CronJobs with it, and the record kept.
    let now = sim.now();
    assert_eq!(now.database, 0);
    assert!(now.cronjobs["meridian-meridian-runtime-backup"]);
    assert!(now.stopped_record.is_some());
}

#[tokio::test]
async fn a_job_that_did_not_finish_says_so_with_its_log_and_a_run_again_waits_for_it() {
    let sim = Sim::new(State {
        archive: true,
        act_hangs: true,
        ..State::default()
    });
    let (checked, _) = check_of(&sim).await;
    let plan = checked.unwrap();
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
        refusal.contains("database-offline: the act restore-to-time"),
        "{refusal}"
    );
    assert!(
        !refusal.contains("Name a time inside the archive"),
        "{refusal}"
    );
    assert!(!refusal.contains("had not finished"), "{refusal}");
    assert!(refusal.contains("Once the Job has ended"), "{refusal}");

    // Run again while it runs: refused, nothing changed.
    let asked_before = sim.asked().len();
    let (checked, said) = check_of(&sim).await;
    assert!(checked.is_err());
    assert!(said.contains("is still running"), "{said}");
    assert!(!sim.asked()[asked_before..]
        .iter()
        .any(|c| c.starts_with("kubectl scale")
            || c.starts_with("kubectl patch")
            || c.starts_with("kubectl create")));

    // Once it has ended: taken from there, CronJobs given back.
    sim.state.lock().unwrap().act_hangs = false;
    let (checked, said) = check_of(&sim).await;
    let plan = checked.unwrap_or_else(|()| panic!("{said}"));
    apply(
        &sim,
        &asked(),
        &plan,
        &pace(),
        1_794_000_100,
        &mut |_: &str| {},
    )
    .await
    .unwrap();
    let now = sim.now();
    assert_eq!(now.deployments, State::default().deployments);
    assert_eq!(now.cronjobs, State::default().cronjobs);
    assert_eq!(now.stopped_record, None);
}

#[tokio::test]
async fn an_act_that_stopped_part_way_with_the_database_running_is_taken_from_its_record() {
    // Interrupted after it stopped the components and suspended the
    // CronJobs, before it stopped the database.
    let mut state = State {
        archive: true,
        ..State::default()
    };
    let record = Stopped {
        deployments: state.deployments.clone().into_iter().collect(),
        database: 1,
        cronjobs: vec![("meridian-meridian-runtime-backup".into(), false)],
        act: Some("restore-to-time".into()),
        since: Some("2026-11-08T09:00:00Z".into()),
    };
    state.stopped_record = Some(record.to_json());
    for count in state.deployments.values_mut() {
        *count = 0;
    }
    state
        .cronjobs
        .insert("meridian-meridian-runtime-backup".into(), true);
    let sim = Sim::new(state);
    let (checked, said) = check_of(&sim).await;
    let plan = checked.unwrap_or_else(|()| panic!("{said}"));
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
    let now = sim.now();
    assert_eq!(now.deployments, State::default().deployments);
    assert!(!now.cronjobs["meridian-meridian-runtime-backup"]);
}

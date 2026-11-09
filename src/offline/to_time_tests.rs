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
        said.contains("stopped by a restore that did not finish"),
        "{said}"
    );
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

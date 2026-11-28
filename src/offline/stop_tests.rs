//! `meridian stop` and `meridian start` against a cluster that acts as one
//! (the acts' own, `super::super::tests::Sim`): what each changes, in what
//! order, what it refuses and changes nothing for, and how a session that
//! did not finish is finished by running it again.

use serde_json::json;

use super::super::tests::{pace, Sim, State, NS, SET};
use super::*;

fn release() -> Release {
    Release {
        release: "meridian".into(),
        namespace: NS.into(),
    }
}

/// Three components, a backup that runs, a scrub suspended already, and
/// the database's offline CronJob, which is always suspended.
fn running() -> State {
    let mut state = State::default();
    state
        .cronjobs
        .insert("meridian-meridian-runtime-scrub".into(), true);
    state
}

async fn planned(sim: &Sim) -> Plan {
    match check(sim, &release()).await {
        Ok(Checked::Stop(plan)) => *plan,
        other => panic!("not planned: {other:?}"),
    }
}

async fn stopped(sim: &Sim) -> String {
    let plan = planned(sim).await;
    let mut said = Vec::new();
    let done = stop(sim, &release(), &plan, &pace(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;
    done.unwrap_or_else(|failed| panic!("{failed}\n{}", said.join("\n")))
}

async fn started(sim: &Sim) -> Result<String, String> {
    let Starting::Start(set, record) = check_start(sim, &release()).await? else {
        return Err("nothing to start".into());
    };
    start(sim, &release(), &set, &record, &pace(), &mut |_: &str| {}).await
}

fn index_of(asked: &[String], starts: &str) -> usize {
    asked
        .iter()
        .position(|c| c.starts_with(starts))
        .unwrap_or_else(|| panic!("never asked `{starts}`: {asked:#?}"))
}

// ── The round trip ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_stop_then_a_start_gives_back_every_count_and_every_schedule_exactly() {
    let before = running();
    let sim = Sim::new(before.clone());

    let done = stopped(&sim).await;
    assert!(done.contains("meridian in meridian is stopped"), "{done}");
    let now = sim.now();
    assert!(now.deployments.values().all(|n| *n == 0), "{now:?}");
    assert_eq!(now.database, 0);
    assert!(now.cronjobs["meridian-meridian-runtime-backup"]);
    assert!(now.cronjobs["meridian-meridian-runtime-scrub"]);
    assert!(now.cronjobs[&format!("{SET}-offline")]);
    let record = Stopped::from_json(now.stop_record.as_deref().expect("recorded")).unwrap();
    assert_eq!(record.act.as_deref(), Some(ACT));
    assert_eq!(record.since.as_deref(), Some("2026-11-06T21:20:00Z"));
    assert_eq!(record.database, 1);
    // A restore's record is never written by a stop.
    assert!(now.stopped_record.is_none());

    // Recorded before anything stops.
    let asked = sim.asked();
    let recorded = index_of(&asked, "kubectl annotate statefulset");
    assert!(recorded < index_of(&asked, "kubectl patch cronjob"));
    assert!(recorded < index_of(&asked, "kubectl scale deployment"));
    assert!(
        index_of(&asked, "kubectl scale deployment")
            < index_of(&asked, "kubectl scale statefulset")
    );
    // The offline CronJob, and one suspended already, are never touched;
    // nothing is deleted and no helm operation runs.
    assert!(!asked
        .iter()
        .any(|c| c.contains("patch cronjob") && c.contains("-offline")));
    assert!(!asked
        .iter()
        .any(|c| c.contains("runtime-scrub") && c.contains("patch")));
    assert!(!asked.iter().any(|c| c.starts_with("kubectl delete")));
    assert!(!asked
        .iter()
        .any(|c| c.starts_with("helm ") && !c.starts_with("helm list")));

    let done = started(&sim).await.unwrap();
    assert!(done.contains("runs again"), "{done}");
    let after = sim.now();
    assert_eq!(after.deployments, before.deployments);
    assert_eq!(after.database, before.database);
    assert_eq!(after.cronjobs, before.cronjobs);
    assert!(after.stop_record.is_none());
}

#[tokio::test]
async fn under_patroni_member_0_is_made_the_primary_before_the_database_stops() {
    let before = running().ha();
    let sim = Sim::new(before.clone());
    let plan = planned(&sim).await;
    assert!(plan_text(&release(), &plan).contains("Member 0 made Patroni's primary"));
    stopped(&sim).await;
    let asked = sim.asked();
    assert!(
        index_of(&asked, "kubectl scale deployment") < index_of(&asked, "kubectl exec"),
        "components stop before the switch: {asked:#?}"
    );
    let switched = asked
        .iter()
        .position(|c| c.contains("switchover"))
        .expect("switched");
    assert!(switched < index_of(&asked, "kubectl scale statefulset"));
    assert_eq!(sim.now().primary, 0);
    assert_eq!(sim.now().database, 0);
    // Patroni's records and the standbys' volumes are kept: nothing is
    // deleted by a stop.
    assert_eq!(sim.now().claims, before.claims);
    assert_eq!(sim.now().configmaps, before.configmaps);

    started(&sim).await.unwrap();
    let after = sim.now();
    assert_eq!(after.database, 3);
    assert_eq!(after.deployments, before.deployments);
    assert_eq!(after.cronjobs, before.cronjobs);
}

// ── Each refusal, nothing changed ────────────────────────────────────────────

async fn refused(state: State) -> (String, Sim) {
    let sim = Sim::new(state);
    let why = match check(&sim, &release()).await {
        Err(why) => why,
        other => panic!("not refused: {other:?}"),
    };
    assert!(!sim.changed_anything(), "{:#?}", sim.asked());
    (why, sim)
}

#[tokio::test]
async fn a_restores_record_is_refused_naming_what_finishes_it() {
    let record = Stopped {
        deployments: vec![("meridian-meridian-runtime-conductor".into(), 1)],
        database: 1,
        act: Some("restore-to-archive-end".into()),
        since: Some("2026-11-09T03:39:15Z".into()),
        ..Stopped::default()
    };
    let (why, _) = refused(State {
        stopped_record: Some(record.to_json()),
        ..running()
    })
    .await;
    assert!(
        why.contains("stopped by `meridian restore --to-archive-end` at 2026-11-09T03:39:15Z"),
        "{why}"
    );
    assert!(
        why.contains("meridian restore --to-archive-end -n meridian"),
        "{why}"
    );
}

#[tokio::test]
async fn a_release_helm_holds_as_pending_is_refused_naming_its_state() {
    let (why, _) = refused(State {
        helm_status: "pending-upgrade".into(),
        ..running()
    })
    .await;
    assert!(why.contains("Helm holds it as pending-upgrade"), "{why}");
}

fn job(name: &str, labels: serde_json::Value, active: u64) -> serde_json::Value {
    json!({ "metadata": { "name": name, "labels": labels }, "status": { "active": active } })
}

#[tokio::test]
async fn a_running_migration_restore_rollback_or_act_is_refused_naming_its_job() {
    for (job, said) in [
        (
            job(
                "meridian-meridian-runtime-migrate-9",
                json!({ "meridian.dev/component": "migrate" }),
                1,
            ),
            "job/meridian-meridian-runtime-migrate-9, a migration, is running",
        ),
        (
            job(
                "meridian-restore-1794",
                json!({ "meridian.dev/component": "restore" }),
                1,
            ),
            "job/meridian-restore-1794, a restore, is running",
        ),
        (
            job("meridian-meridian-runtime-rollback-9", json!({}), 1),
            "job/meridian-meridian-runtime-rollback-9, a rollback, is running",
        ),
        (
            job(
                &format!("{SET}-offline-1794"),
                json!({ "meridian.dev/database-act": "restore-to-time" }),
                1,
            ),
            "an act on the database's volume, restore-to-time, is running",
        ),
    ] {
        let (why, _) = refused(State {
            release_jobs: vec![job],
            ..running()
        })
        .await;
        assert!(why.contains(said), "{why}");
    }
    // One that has ended, or a Job of another kind, stops nothing.
    let sim = Sim::new(State {
        release_jobs: vec![
            job(
                "meridian-meridian-runtime-migrate-8",
                json!({ "meridian.dev/component": "migrate" }),
                0,
            ),
            job(
                "meridian-meridian-runtime-backup-1",
                json!({ "meridian.dev/component": "backup" }),
                1,
            ),
        ],
        ..running()
    });
    planned(&sim).await;
}

#[tokio::test]
async fn a_database_somebody_else_runs_or_several_to_a_store_is_refused() {
    let (why, _) = refused(State {
        brought: false,
        ..running()
    })
    .await;
    assert!(why.contains("runs no database it brought"), "{why}");
    let (why, _) = refused(State {
        store_databases: [("bor".to_string(), 2)].into(),
        ..running()
    })
    .await;
    assert!(why.contains("a stop"), "{why}");
}

#[tokio::test]
async fn a_database_at_none_that_nothing_recorded_is_refused() {
    let (why, _) = refused(State {
        database: 0,
        ..running()
    })
    .await;
    assert!(why.contains("nothing recorded stopping it"), "{why}");
}

#[tokio::test]
async fn stopped_already_is_said_and_changes_nothing() {
    let sim = Sim::new(running());
    stopped(&sim).await;
    let again = Sim::new(sim.now());
    match check(&again, &release()).await {
        Ok(Checked::Already(said)) => {
            assert!(
                said.contains("is stopped already, since 2026-11-06T21:20:00Z"),
                "{said}"
            );
            assert!(said.contains("`meridian start` starts it"), "{said}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!again.changed_anything(), "{:#?}", again.asked());
}

// ── Run again after a session that did not finish ────────────────────────────

#[tokio::test]
async fn a_stop_that_did_not_finish_is_finished_by_stop_again_and_starts_as_it_was() {
    let before = running();
    let sim = Sim::new(State {
        patch_refused: ["meridian-meridian-runtime-backup".to_string()].into(),
        ..before.clone()
    });
    let plan = planned(&sim).await;
    let failed = stop(&sim, &release(), &plan, &pace(), &mut |_: &str| {})
        .await
        .expect_err("the CronJob's patch is refused");
    assert!(
        failed.contains("`meridian stop -n meridian` again finishes"),
        "{failed}"
    );
    assert!(sim.now().stop_record.is_some(), "recorded before it failed");

    // Something stopped by hand since, and one component scaled down, as a
    // session killed between its steps leaves it.
    let mut left = sim.now();
    left.patch_refused.clear();
    left.deployments
        .insert("meridian-meridian-runtime-dashboard".into(), 0);
    let sim = Sim::new(left);
    stopped(&sim).await;
    let record = Stopped::from_json(sim.now().stop_record.as_deref().unwrap()).unwrap();
    // The first record's counts stand, the dashboard's 2 among them.
    assert!(record
        .deployments
        .contains(&("meridian-meridian-runtime-dashboard".to_string(), 2)));
    assert_eq!(record.since.as_deref(), Some("2026-11-06T21:20:00Z"));

    started(&sim).await.unwrap();
    assert_eq!(sim.now().deployments, before.deployments);
    assert_eq!(sim.now().cronjobs, before.cronjobs);
}

#[tokio::test]
async fn a_start_that_did_not_finish_is_finished_by_start_again() {
    let before = running();
    let sim = Sim::new(before.clone());
    stopped(&sim).await;
    // Killed after the database came back and one component with it: the
    // record is still there.
    let mut left = sim.now();
    left.database = 1;
    left.deployments
        .insert("meridian-meridian-runtime-conductor".into(), 1);
    let sim = Sim::new(left);
    started(&sim).await.unwrap();
    let after = sim.now();
    assert_eq!(after.deployments, before.deployments);
    assert_eq!(after.cronjobs, before.cronjobs);
    assert!(after.stop_record.is_none());
    // And a stop run after it instead: the record's counts stand.
    let sim = Sim::new(before.clone());
    stopped(&sim).await;
    let mut left = sim.now();
    left.database = 1;
    left.deployments
        .insert("meridian-meridian-runtime-dashboard".into(), 1);
    let sim = Sim::new(left);
    stopped(&sim).await;
    started(&sim).await.unwrap();
    assert_eq!(sim.now().deployments, before.deployments);
}

#[tokio::test]
async fn start_with_nothing_stopped_says_so_and_a_restores_stop_is_left_to_it() {
    let sim = Sim::new(running());
    match check_start(&sim, &release()).await {
        Ok(Starting::Nothing(said)) => assert!(said.contains("is not stopped"), "{said}"),
        other => panic!("{other:?}"),
    }
    let record = Stopped {
        database: 1,
        act: Some("enable-checksums".into()),
        ..Stopped::default()
    };
    let sim = Sim::new(State {
        stopped_record: Some(record.to_json()),
        ..running()
    });
    let refused = check_start(&sim, &release()).await.expect_err("refused");
    assert!(
        refused.contains("meridian upgrade-deployment --enable-checksums -n meridian"),
        "{refused}"
    );
    assert!(!sim.changed_anything());
}

// ── While it is stopped ──────────────────────────────────────────────────────

#[tokio::test]
async fn while_stopped_the_commands_that_would_start_it_are_refused_pointing_to_start() {
    let sim = Sim::new(running());
    assert!(refused_while_stopped(&sim, &release(), "meridian up")
        .await
        .is_none());
    stopped(&sim).await;
    for command in [
        "meridian up",
        "meridian upgrade-deployment",
        "meridian rollback-deployment",
        "meridian restore --to-time",
    ] {
        let said = refused_while_stopped(&sim, &release(), command)
            .await
            .expect("refused");
        assert!(
            said.contains(&format!("`{command}` would start parts of it again")),
            "{said}"
        );
        assert!(
            said.contains("`meridian start` starts it as it was"),
            "{said}"
        );
        assert!(said.contains("since 2026-11-06T21:20:00Z"), "{said}");
    }
}

#[tokio::test]
async fn status_says_one_line_in_place_of_what_is_not_ready_and_json_its_since() {
    let sim = Sim::new(running());
    stopped(&sim).await;
    let record = stopped_in(&Sim::workloads(&sim.now(), true)).expect("read back");
    let mut status = crate::status::tests::healthy();
    status.stopped = Some(record);
    let said = crate::status::said(&status, NS);
    assert!(
        said.contains("Stopped since 2026-11-06T21:20:00Z; run `meridian start`"),
        "{said}"
    );
    assert!(!said.contains("Components:"), "{said}");
    assert!(!said.contains("Plugins:"), "{said}");
    let wide = crate::status::said_wide(&status, 0, NS);
    assert!(
        wide.contains("Stopped since 2026-11-06T21:20:00Z"),
        "{wide}"
    );
    let json = crate::status::json_of(&status);
    assert_eq!(json["stopped"]["since"], "2026-11-06T21:20:00Z");
    // Not stopped: the field is there, and null.
    let json = crate::status::json_of(&crate::status::tests::healthy());
    assert!(json["stopped"].is_null() && json.get("stopped").is_some());
    // Another namespace or release is named in the command.
    assert_eq!(
        status_line(&Stopped::default(), "--release r -n x"),
        "Stopped since a time not recorded; run `meridian start --release r -n x`"
    );
}

#[tokio::test]
async fn doctor_says_a_stopped_deployment_and_does_not_fail_on_its_zero_replicas() {
    let sim = Sim::new(running());
    stopped(&sim).await;
    let mut findings = crate::doctor::checks::stopped(&sim, NS).await;
    assert_eq!(findings.len(), 1, "{findings:?}");
    let said = format!("{}", findings[0]);
    assert!(said.contains("meridian in meridian is stopped"), "{said}");
    assert!(said.contains("meridian start"), "{said}");
    // The database's check finds no conductor, and says nothing.
    findings.extend(crate::doctor::checks::database(&sim, NS).await);
    let (verdict, code) = crate::doctor::verdict(&findings);
    assert_eq!(code, 0, "{verdict}");
    // Started, it says nothing of it.
    started(&sim).await.unwrap();
    assert!(crate::doctor::checks::stopped(&sim, NS).await.is_empty());
}

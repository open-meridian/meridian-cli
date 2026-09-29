use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

use super::run::{apply, check, Checked, Pace};
use super::*;
use crate::doctor::{Answered, Failure, Machine};

/// A cluster and a Helm that answer from a script, each command in turn, the
/// last answer repeating. Every command asked is kept: what an upgrade that
/// was refused did not run is as much of the test as what it said.
#[derive(Default)]
struct Stand {
    answers: Mutex<HashMap<String, VecDeque<Result<String, Failure>>>>,
    asked: Mutex<Vec<String>>,
}

impl Stand {
    fn answering(self, command: &str, answer: Result<&str, &str>) -> Self {
        self.answers
            .lock()
            .expect("a test's own lock")
            .entry(command.to_string())
            .or_default()
            .push_back(
                answer
                    .map(String::from)
                    .map_err(|said| Failure::Said(said.to_string())),
            );
        self
    }

    /// One answer in place of whatever was scripted for it.
    fn instead(self, command: &str, answer: Result<&str, &str>) -> Self {
        self.answers
            .lock()
            .expect("a test's own lock")
            .remove(command);
        self.answering(command, answer)
    }

    fn json(self, command: &str, answer: serde_json::Value) -> Self {
        self.answering(command, Ok(&answer.to_string()))
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("a test's own lock").clone()
    }

    fn changed_anything(&self) -> bool {
        self.asked().iter().any(|command| {
            command.starts_with("helm upgrade")
                || command.starts_with("helm rollback")
                || command.starts_with("kubectl delete")
        })
    }
}

#[async_trait::async_trait]
impl Machine for Stand {
    async fn run(&self, program: &str, arguments: &[&str]) -> Result<String, Failure> {
        let command = format!("{program} {}", arguments.join(" "));
        self.asked
            .lock()
            .expect("a test's own lock")
            .push(command.clone());
        let mut answers = self.answers.lock().expect("a test's own lock");
        let Some(queue) = answers.get_mut(&command) else {
            return Err(Failure::Said(format!("nothing scripted for `{command}`")));
        };
        if queue.len() > 1 {
            return queue.pop_front().expect("more than one");
        }
        queue
            .front()
            .cloned()
            .unwrap_or_else(|| Err(Failure::Said(format!("nothing left for `{command}`"))))
    }

    async fn fetch(&self, url: &str, _: &[(&str, &str)]) -> Result<Answered, String> {
        Err(format!("an upgrade fetches nothing, and asked for {url}"))
    }

    fn now_s(&self) -> u64 {
        0
    }
}

const CHART: &str = "oci://ghcr.io/open-meridian/charts/meridian-runtime";
const REPO: &str = "ghcr.io/open-meridian/meridian-runtime";
const LIST: &str = "helm list --namespace meridian --filter ^meridian$ --deployed --failed --pending --uninstalling -o json";
const SHOW_LATEST: &str = "helm show chart oci://ghcr.io/open-meridian/charts/meridian-runtime";
const SHOW_VALUES: &str =
    "helm show values oci://ghcr.io/open-meridian/charts/meridian-runtime --version 0.1.182";
const OWN_VALUES: &str = "helm get values meridian --namespace meridian -o json";
const WORKLOADS: &str = "kubectl get deployments,statefulsets -n meridian -l app.kubernetes.io/instance=meridian -o json";
const PODS: &str = "kubectl get pods -n meridian -l app.kubernetes.io/instance=meridian -o json";
const JOBS: &str = "kubectl get jobs -n meridian -l app.kubernetes.io/instance=meridian -o json";
const UPGRADE: &str = "helm upgrade meridian oci://ghcr.io/open-meridian/charts/meridian-runtime --version 0.1.182 --namespace meridian --reset-then-reuse-values --timeout 10m";

fn asked() -> Asked {
    Asked {
        release: "meridian".into(),
        namespace: "meridian".into(),
        chart: CHART.into(),
        chart_version: None,
        timeout: "10m".into(),
    }
}

fn listed(revision: u64, status: &str, version: &str, app: &str) -> serde_json::Value {
    json!([{
        "name": "meridian", "namespace": "meridian", "revision": revision.to_string(),
        "status": status, "chart": format!("meridian-runtime-{version}"), "app_version": app
    }])
}

/// Helm 4 prints its pull on stdout, before the document.
fn shown(version: &str, app: &str) -> String {
    format!(
        "Pulled: ghcr.io/open-meridian/charts/meridian-runtime:{version}\n\
         Digest: sha256:60c78b\n\
         apiVersion: v2\nappVersion: {app}\nname: meridian-runtime\nversion: {version}\n"
    )
}

fn deployment(component: &str, tag: &str, ready: u64, updated: u64) -> serde_json::Value {
    json!({
        "kind": "Deployment",
        "metadata": { "name": format!("meridian-meridian-runtime-{component}"), "generation": 2,
                      "labels": { "meridian.dev/component": component } },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app.kubernetes.io/instance": "meridian",
                                           "meridian.dev/component": component } },
            "template": { "spec": { "containers": [ { "name": component, "image": format!("{REPO}:{tag}") } ] } }
        },
        "status": { "observedGeneration": 2, "replicas": 1, "updatedReplicas": updated,
                    "readyReplicas": ready, "availableReplicas": ready }
    })
}

fn database() -> serde_json::Value {
    json!({
        "kind": "StatefulSet",
        "metadata": { "name": "meridian-meridian-runtime-database", "generation": 1,
                      "labels": { "meridian.dev/component": "database" } },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app.kubernetes.io/instance": "meridian",
                                           "meridian.dev/component": "database" } },
            "template": { "spec": { "containers": [ { "name": "database", "image": "postgres:16-alpine" } ] } }
        },
        "status": { "observedGeneration": 1, "replicas": 1, "updatedReplicas": 1, "readyReplicas": 1,
                    "currentRevision": "db-1", "updateRevision": "db-1" }
    })
}

fn items(items: Vec<serde_json::Value>) -> serde_json::Value {
    json!({ "items": items })
}

fn pod(name: &str, component: &str, image: &str, restarts: u64) -> serde_json::Value {
    json!({
        "metadata": { "name": name, "labels": { "app.kubernetes.io/instance": "meridian",
                                                "meridian.dev/component": component } },
        "spec": { "containers": [ { "name": component, "image": image } ] },
        "status": { "containerStatuses": [ {
            "name": component, "restartCount": restarts, "state": { "running": {} },
            "lastState": if restarts > 0 {
                json!({ "terminated": { "reason": "Error", "exitCode": 1 } })
            } else { json!({}) }
        } ] }
    })
}

fn job(name: &str, state: Option<&str>) -> serde_json::Value {
    let conditions = match state {
        Some(kind) => json!([{ "type": kind, "status": "True" }]),
        None => json!([]),
    };
    json!({ "metadata": { "name": name }, "status": { "conditions": conditions } })
}

fn job_pod(job: &str, waiting: &str) -> serde_json::Value {
    json!({
        "metadata": { "name": format!("{job}-x7k"), "labels": { "app.kubernetes.io/instance": "meridian" },
                      "ownerReferences": [ { "kind": "Job", "name": job } ] },
        "spec": { "containers": [ { "name": "instrument", "image": format!("{REPO}:9c5d480") } ] },
        "status": { "initContainerStatuses": [ { "name": "conductor", "restartCount": 0,
                                                 "state": { "waiting": { "reason": waiting } } } ] }
    })
}

/// A healthy deployment of 0.1.180, answering every check.
fn healthy() -> Stand {
    Stand::default()
        .answering("helm version --short", Ok("v3.16.2+g13654a5"))
        .answering("kubectl cluster-info", Ok("Kubernetes control plane is running"))
        .answering("kubectl auth can-i patch deployments -n meridian", Ok("yes"))
        .answering("kubectl auth can-i create jobs -n meridian", Ok("yes"))
        .answering("kubectl auth can-i delete jobs -n meridian", Ok("yes"))
        .answering(SHOW_LATEST, Ok(&shown("0.1.182", "9c5d480")))
        .answering(
            SHOW_VALUES,
            Ok("# Everything a deployment needs\nimage:\n  repository: ghcr.io/open-meridian/meridian-runtime\n  tag: \"9c5d480\"\n"),
        )
        .json(
            OWN_VALUES,
            json!({ "deployment": { "id": "DEP-X", "enrolmentCode": "ENROL-DO-NOT-PRINT" },
                    "ingress": { "enabled": true, "host": "meridian.localhost" } }),
        )
}

// ── Checks ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_failed_release_is_refused_with_how_to_recover_and_nothing_is_changed() {
    let stand = healthy().json(LIST, listed(7, "failed", "0.1.180", "845bd06"));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    let said = said.join("\n");
    assert!(said.contains("stops"), "{said}");
    assert!(
        said.contains("helm rollback meridian -n meridian"),
        "{said}"
    );
    assert!(said.contains("Nothing was changed"), "{said}");
    assert!(!stand.changed_anything(), "{:?}", stand.asked());
}

#[tokio::test]
async fn a_pending_release_says_another_operation_holds_it() {
    let stand = healthy().json(LIST, listed(8, "pending-upgrade", "0.1.180", "845bd06"));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    assert!(said.join("\n").contains("pending-upgrade"), "{said:?}");
    assert!(!stand.changed_anything());
}

#[tokio::test]
async fn no_release_names_the_command_that_installs_one() {
    let stand = healthy().json(LIST, json!([]));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    assert!(said.join("\n").contains("meridian up"), "{said:?}");
}

#[tokio::test]
async fn a_helm_without_reset_then_reuse_values_is_stopped_before_anything_is_read() {
    let stand = healthy()
        .instead("helm version --short", Ok("v3.12.3+g3a31588"))
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    assert!(said.join("\n").contains("3.14"), "{said:?}");
    assert!(!stand.asked().contains(&LIST.to_string()));
}

#[tokio::test]
async fn an_unreachable_cluster_is_one_finding() {
    let stand = healthy().instead("kubectl cluster-info", Err("connection refused"));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    let said = said.join("\n");
    assert!(said.contains("KUBECONFIG"), "{said}");
    assert!(
        !stand
            .asked()
            .iter()
            .any(|command| command.contains("can-i")),
        "{:?}",
        stand.asked()
    );
}

#[tokio::test]
async fn a_right_the_cluster_refuses_stops_it() {
    // kubectl answers "no" and exits 1.
    let stand = healthy()
        .instead("kubectl auth can-i delete jobs -n meridian", Err("no"))
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    assert!(said.join("\n").contains("may not delete jobs"), "{said:?}");
}

#[tokio::test]
async fn a_version_that_was_never_published_is_stopped() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .answering(
            &format!("{SHOW_LATEST} --version 0.1.999"),
            Err("Error: failed to perform \"FetchReference\": not found"),
        );
    let mut asked = asked();
    asked.chart_version = Some("0.1.999".into());
    let mut said = Vec::new();

    let checked = check(&stand, &asked, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    assert!(
        said.join("\n").contains("0.1.999 could not be found"),
        "{said:?}"
    );
    assert!(!stand.changed_anything());
}

#[tokio::test]
async fn a_downgrade_is_refused() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.182", "9c5d480"))
        .answering(
            &format!("{SHOW_LATEST} --version 0.1.180"),
            Ok(&shown("0.1.180", "845bd06")),
        );
    let mut asked = asked();
    asked.chart_version = Some("0.1.180".into());
    let mut said = Vec::new();

    let checked = check(&stand, &asked, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    assert!(
        said.join("\n").contains("older than the installed"),
        "{said:?}"
    );
    assert!(!stand.changed_anything());
}

#[tokio::test]
async fn the_version_it_is_at_already_is_nothing_to_do() {
    let stand = healthy().json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    let Checked::Current(done) = checked else {
        panic!("not current: {said:?}");
    };
    assert!(done.contains("nothing to do"), "{done}");
    assert!(!stand.changed_anything());
}

#[tokio::test]
async fn another_chart_is_not_an_upgrade() {
    let stand = healthy().json(LIST, listed(7, "deployed", "0.1.180", "845bd06"));
    let mut asked = asked();
    asked.chart = "oci://example.com/charts/something-else".into();
    let stand = stand.answering(
        "helm show chart oci://example.com/charts/something-else",
        Ok("name: something-else\nversion: 9.0.0\n"),
    );
    let mut said = Vec::new();

    assert!(matches!(
        check(&stand, &asked, &mut |line: &str| said
            .push(line.to_string()))
        .await,
        Checked::Refused
    ));
    assert!(said.join("\n").contains("runs the chart meridian-runtime"));
}

#[tokio::test]
async fn the_unruled_checks_say_they_did_not_check_rather_than_passing() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(
            WORKLOADS,
            items(vec![deployment("street", "845bd06", 1, 1)]),
        );
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Upgrade(_)));
    let said = said.join("\n");
    assert!(said.contains("unknown  whether 0.1.180 to 0.1.182 is within the skip policy"));
    assert!(said.contains("unknown  whether every plugin runs on 0.1.182"));
}

#[tokio::test]
async fn the_plan_names_both_versions_and_both_images_and_never_the_values() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(
            WORKLOADS,
            items(vec![deployment("street", "845bd06", 1, 1), database()]),
        );
    let mut said = Vec::new();

    let Checked::Upgrade(plan) = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    else {
        panic!("refused: {said:?}");
    };
    let text = plan_text(&asked(), &plan);

    assert!(
        text.contains("from  meridian-runtime 0.1.180 (revision 7)"),
        "{text}"
    );
    assert!(text.contains(&format!("running {REPO}:845bd06")), "{text}");
    assert!(text.contains(&format!(
        "to    meridian-runtime 0.1.182, running {REPO}:9c5d480"
    )));
    // The database's own image is not the runtime's.
    assert!(!text.contains("postgres"), "{text}");
    assert!(text.contains("--reset-then-reuse-values"), "{text}");
    assert!(!text.contains("--reuse-values "), "{text}");
    assert!(!text.contains("--wait"), "{text}");
    let everything = format!("{}\n{text}", said.join("\n"));
    assert!(!everything.contains("ENROL-DO-NOT-PRINT"), "{everything}");
}

#[tokio::test]
async fn a_tag_the_deployment_pins_is_kept_and_said_before_anything_is_applied() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .instead(
            OWN_VALUES,
            Ok(
                &json!({ "deployment": { "enrolmentCode": "ENROL-DO-NOT-PRINT" },
                        "image": { "repository": REPO, "tag": "845bd06" } })
                .to_string(),
            ),
        )
        .json(WORKLOADS, items(vec![]));
    let mut said = Vec::new();

    let Checked::Upgrade(plan) = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    else {
        panic!("refused: {said:?}");
    };

    assert_eq!(plan.to_image, format!("{REPO}:845bd06"));
    let said = said.join("\n");
    assert!(said.contains("worth    this deployment's own values pin image.tag to 845bd06"));
    assert!(!said.contains("ENROL-DO-NOT-PRINT"));
}

// ── Applying and waiting ─────────────────────────────────────────────────

fn pace() -> Pace {
    Pace {
        poll: Duration::ZERO,
        timeout: Duration::from_secs(60),
    }
}

async fn planned(stand: &Stand) -> Plan {
    let mut said = Vec::new();
    match check(stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    {
        Checked::Upgrade(plan) => plan,
        _ => panic!("refused: {said:?}"),
    }
}

/// 0.1.180 to 0.1.182 as it went on 2026-09-28: the conductor and the
/// launcher each restarted once, and two finished Jobs from earlier
/// revisions were left behind.
fn upgrading() -> Stand {
    let old = |component| deployment(component, "845bd06", 1, 1);
    let new = |component| deployment(component, "9c5d480", 1, 1);
    healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"))
        // Read for the plan, then while waiting: mid-rollout, then done.
        .json(WORKLOADS, items(vec![old("conductor"), old("launcher"), database()]))
        .json(
            WORKLOADS,
            items(vec![
                deployment("conductor", "9c5d480", 0, 1),
                new("launcher"),
                database(),
            ]),
        )
        .json(WORKLOADS, items(vec![new("conductor"), new("launcher"), database()]))
        .json(
            PODS,
            items(vec![
                pod("conductor-old", "conductor", &format!("{REPO}:845bd06"), 0),
                pod("launcher-old", "launcher", &format!("{REPO}:845bd06"), 0),
                pod("database-0", "database", "postgres:16-alpine", 2),
            ]),
        )
        .json(
            PODS,
            items(vec![
                pod("conductor-new", "conductor", &format!("{REPO}:9c5d480"), 0),
                pod("launcher-new", "launcher", &format!("{REPO}:9c5d480"), 1),
                pod("database-0", "database", "postgres:16-alpine", 2),
                job_pod("meridian-meridian-runtime-migrate-8", "PodInitializing"),
            ]),
        )
        .json(
            PODS,
            items(vec![
                pod("conductor-new", "conductor", &format!("{REPO}:9c5d480"), 1),
                pod("launcher-new", "launcher", &format!("{REPO}:9c5d480"), 1),
                pod("database-0", "database", "postgres:16-alpine", 2),
            ]),
        )
        .json(
            JOBS,
            items(vec![
                job("meridian-meridian-runtime-key", Some("Complete")),
                job("meridian-meridian-runtime-migrate-6", Some("Complete")),
                job("meridian-meridian-runtime-migrate-7", Some("Complete")),
                job("meridian-meridian-runtime-first-run-6", Some("Failed")),
                job("meridian-meridian-runtime-migrate-8", None),
            ]),
        )
        .json(
            JOBS,
            items(vec![
                job("meridian-meridian-runtime-key", Some("Complete")),
                job("meridian-meridian-runtime-migrate-6", Some("Complete")),
                job("meridian-meridian-runtime-migrate-7", Some("Complete")),
                job("meridian-meridian-runtime-first-run-6", Some("Failed")),
                job("meridian-meridian-runtime-migrate-8", Some("Complete")),
            ]),
        )
        .answering(UPGRADE, Ok("Release \"meridian\" has been upgraded. Happy Helming!"))
        .answering(
            "kubectl delete job -n meridian --ignore-not-found meridian-meridian-runtime-migrate-6 meridian-meridian-runtime-migrate-7 meridian-meridian-runtime-first-run-6",
            Ok("job.batch \"meridian-meridian-runtime-migrate-6\" deleted"),
        )
}

#[tokio::test]
async fn an_upgrade_waits_for_its_migration_and_every_component_then_cleans_up() {
    let stand = upgrading();
    let plan = planned(&stand).await;
    let mut said = Vec::new();

    let report = apply(&stand, &asked(), &plan, &pace(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    .unwrap_or_else(|failed| panic!("{failed}\n{said:?}"));

    let asked_for = stand.asked();
    assert!(asked_for.contains(&UPGRADE.to_string()), "{asked_for:#?}");
    assert!(
        !asked_for.iter().any(|command| command.contains("--wait")),
        "{asked_for:#?}"
    );
    // It waited: the migration and the conductor, then nothing.
    let said = said.join("\n");
    assert!(said.contains("waiting: the migration, job/meridian-meridian-runtime-migrate-8: conductor PodInitializing"), "{said}");
    assert!(
        said.contains("waiting: deployment/meridian-meridian-runtime-conductor: 0 of 1 ready"),
        "{said}"
    );

    assert_eq!(report.revision, 8);
    assert_eq!(
        report.migrated.as_deref(),
        Some("meridian-meridian-runtime-migrate-8")
    );
    // Earlier revisions' finished Jobs only: not the key Job, which is kept
    // for its log, and not this revision's migration.
    assert_eq!(
        report.cleaned,
        [
            "meridian-meridian-runtime-migrate-6",
            "meridian-meridian-runtime-migrate-7",
            "meridian-meridian-runtime-first-run-6"
        ]
    );
    // Restarts during the upgrade, and not the database's from before it.
    assert_eq!(report.restarted.len(), 2, "{:?}", report.restarted);
    assert!(report.restarted[0]
        .contains("pod/conductor-new conductor: restarted 1 time(s), last Error, exit 1"));
    assert!(report
        .restarted
        .iter()
        .all(|each| !each.contains("database")));

    let text = report_text(&asked(), &report);
    assert!(
        text.contains("meridian-runtime 0.1.180 -> meridian-runtime 0.1.182, now revision 8"),
        "{text}"
    );
    assert!(
        text.contains(&format!("conductor  1/1  {REPO}:9c5d480")),
        "{text}"
    );
    assert!(text.contains("worth knowing, not a failure"), "{text}");
    assert!(text.contains("--previous"), "{text}");
    assert!(!text.contains("ENROL-DO-NOT-PRINT"), "{text}");
}

#[tokio::test]
async fn a_failed_migration_says_where_its_log_is_and_cleans_up_nothing() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"))
        .json(
            WORKLOADS,
            items(vec![deployment("street", "845bd06", 1, 1)]),
        )
        .json(PODS, items(vec![]))
        .json(
            JOBS,
            items(vec![
                job("meridian-meridian-runtime-migrate-7", Some("Complete")),
                job("meridian-meridian-runtime-migrate-8", Some("Failed")),
            ]),
        )
        .answering(UPGRADE, Ok("upgraded"));
    let plan = planned(&stand).await;
    let mut said = Vec::new();

    let failed = apply(&stand, &asked(), &plan, &pace(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    .expect_err("a failed migration is a failed upgrade");

    assert!(
        failed.contains(
            "kubectl logs -n meridian job/meridian-meridian-runtime-migrate-8 --all-containers"
        ),
        "{failed}"
    );
    assert!(!stand
        .asked()
        .iter()
        .any(|command| command.starts_with("kubectl delete")));
}

#[tokio::test]
async fn a_timeout_says_what_is_still_waited_for() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"))
        .json(WORKLOADS, items(vec![deployment("street", "845bd06", 1, 1)]))
        .json(WORKLOADS, items(vec![deployment("street", "9c5d480", 0, 1)]))
        .json(
            PODS,
            items(vec![json!({
                "metadata": { "name": "street-new", "labels": { "app.kubernetes.io/instance": "meridian",
                                                                "meridian.dev/component": "street" } },
                "spec": { "containers": [ { "name": "street", "image": format!("{REPO}:9c5d480") } ] },
                "status": { "containerStatuses": [ { "name": "street", "restartCount": 4,
                    "state": { "waiting": { "reason": "CrashLoopBackOff" } } } ] }
            })]),
        )
        .json(JOBS, items(vec![job("meridian-meridian-runtime-migrate-8", Some("Complete"))]))
        .answering(UPGRADE, Ok("upgraded"));
    let plan = planned(&stand).await;
    let pace = Pace {
        poll: Duration::ZERO,
        timeout: Duration::ZERO,
    };
    let mut said = Vec::new();

    let failed = apply(&stand, &asked(), &plan, &pace, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    .expect_err("it never became ready");

    assert!(failed.contains("after 10m, still waiting for"), "{failed}");
    assert!(
        failed.contains("deployment/meridian-meridian-runtime-street: 0 of 1 ready; pod/street-new street CrashLoopBackOff"),
        "{failed}"
    );
    assert!(failed.contains("nothing was rolled back"), "{failed}");
}

#[tokio::test]
async fn helm_refusing_says_how_to_get_back() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(WORKLOADS, items(vec![]))
        .json(PODS, items(vec![]))
        .answering(UPGRADE, Err("Error: UPGRADE FAILED: cannot patch"));
    let plan = planned(&stand).await;
    let mut said = Vec::new();

    let failed = apply(&stand, &asked(), &plan, &pace(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    .expect_err("helm refused");

    assert!(
        failed.contains("helm rollback meridian -n meridian"),
        "{failed}"
    );
    assert!(failed.contains("revision 7"), "{failed}");
}

// ── What is read ─────────────────────────────────────────────────────────

#[test]
fn a_timeout_is_a_duration_as_helm_writes_one() {
    assert_eq!(duration("10m"), Some(Duration::from_secs(600)));
    assert_eq!(duration("90s"), Some(Duration::from_secs(90)));
    assert_eq!(duration("1h30m"), Some(Duration::from_secs(5400)));
    for refused in ["600", "", "10x", "m", "0s"] {
        assert_eq!(duration(refused), None, "{refused}");
    }
}

#[test]
fn helm_3_and_helm_4_state_a_revision_differently_and_both_are_read() {
    let three = r#"[{"name":"meridian","revision":"7","status":"deployed","chart":"meridian-runtime-0.1.180","app_version":"845bd06"}]"#;
    let four = r#"[{"name":"meridian","revision":7,"status":"deployed","chart":"meridian-runtime-0.1.180-rc.1","app_version":"845bd06"}]"#;

    let three = installed(three, "meridian").unwrap().unwrap();
    let four = installed(four, "meridian").unwrap().unwrap();

    assert_eq!(
        (three.revision, three.chart.as_str(), three.version.as_str()),
        (7, "meridian-runtime", "0.1.180")
    );
    assert_eq!((four.revision, four.version.as_str()), (7, "0.1.180-rc.1"));
    assert_eq!(installed("[]", "meridian"), Ok(None));
    assert_eq!(
        installed(r#"[{"name":"meridian-2","revision":1}]"#, "meridian"),
        Ok(None)
    );
}

#[test]
fn a_tag_yaml_reads_as_a_number_is_a_tag_all_the_same() {
    let chart = chart_of("name: meridian-runtime\nversion: 0.1.99\nappVersion: 1896148\n").unwrap();
    assert_eq!(chart.app_version, "1896148");
    assert_eq!(
        image_in("image:\n  repository: r\n  tag: 1896148\n")
            .tag
            .as_deref(),
        Some("1896148")
    );
}

#[test]
fn only_the_image_is_kept_from_the_deployments_own_values() {
    let own = image_in(r#"{"deployment":{"enrolmentCode":"ENROL-DO-NOT-PRINT"}}"#);
    assert_eq!(own, Image::default());
    assert!(!format!("{own:?}").contains("ENROL"));
}

#[test]
fn only_finished_jobs_of_earlier_revisions_are_left_over() {
    let held = jobs(
        &items(vec![
            job("m-key", Some("Complete")),
            job("m-migrate-7", Some("Complete")),
            job("m-first-run-7", None),
            job("m-migrate-8", Some("Failed")),
            job("m-settings-key-6", Some("Failed")),
        ])
        .to_string(),
    );

    let (finished, running) = leftovers(&held, 8);

    assert_eq!(finished, ["m-migrate-7", "m-settings-key-6"]);
    assert_eq!(running, ["m-first-run-7"]);
}

#[test]
fn a_statefulset_mid_update_is_not_rolled_out() {
    let mut set = database();
    set["status"]["updateRevision"] = json!("db-2");
    let held = workloads(&items(vec![set]).to_string());
    assert_eq!(
        held[0].unfinished.as_deref(),
        Some("its update has not finished")
    );
    assert_eq!(
        held[0].name,
        "statefulset/meridian-meridian-runtime-database"
    );
}

#[test]
fn a_pod_on_the_old_image_is_waited_for_after_its_rollout_says_done() {
    let held = workloads(&items(vec![deployment("street", "9c5d480", 1, 1)]).to_string());
    let running = pods(
        &items(vec![
            pod("street-new", "street", &format!("{REPO}:9c5d480"), 0),
            pod("street-old", "street", &format!("{REPO}:845bd06"), 0),
        ])
        .to_string(),
    );

    let now = progress(8, &[], &held, &running);

    assert_eq!(
        now.waiting,
        [format!(
            "deployment/meridian-meridian-runtime-street: pod/street-old still runs {REPO}:845bd06"
        )]
    );
}

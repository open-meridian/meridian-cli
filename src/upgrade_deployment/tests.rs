use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;

use super::run::{apply, check, Checked, Pace};
use super::watch::{render, Board, Log, Mark, Step};
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

    /// Kept as the command and what kind of document it was given, never
    /// the document: a Secret's key is in it.
    async fn run_with_input(
        &self,
        program: &str,
        arguments: &[&str],
        input: &str,
    ) -> Result<String, Failure> {
        let kind = serde_json::from_str::<serde_json::Value>(input)
            .ok()
            .and_then(|document| document["kind"].as_str().map(String::from))
            .unwrap_or_default();
        self.asked
            .lock()
            .expect("a test's own lock")
            .push(format!("{program} {} <<{kind}", arguments.join(" ")));
        Ok(String::new())
    }
}

const CHART: &str = "oci://ghcr.io/open-meridian/charts/meridian-runtime";
const REPO: &str = "ghcr.io/open-meridian/meridian-runtime";
const LIST: &str = "helm list --namespace meridian --filter ^meridian$ --deployed --failed --pending --uninstalling -o json";
const SHOW_LATEST: &str = "helm show chart oci://ghcr.io/open-meridian/charts/meridian-runtime";
const SHOW_VALUES: &str =
    "helm show values oci://ghcr.io/open-meridian/charts/meridian-runtime --version 0.1.182";
const OWN_VALUES: &str = "helm get values meridian --namespace meridian -o json";
const WORKLOADS: &str = "kubectl get deployments,statefulsets,replicasets -n meridian -l app.kubernetes.io/instance=meridian -o json";
const PODS: &str = "kubectl get pods -n meridian -l app.kubernetes.io/instance=meridian -o json";
const JOBS: &str = "kubectl get jobs -n meridian -l app.kubernetes.io/instance=meridian -o json";
const NODES: &str = "kubectl get nodes -o json";
const SUMMARY: &str = "kubectl get --raw /api/v1/nodes/node-1/proxy/stats/summary";
const GIB: u64 = 1 << 30;
const UPGRADE: &str = "helm upgrade meridian oci://ghcr.io/open-meridian/charts/meridian-runtime --version 0.1.182 --namespace meridian --reset-then-reuse-values --timeout 10m";

fn asked() -> Asked {
    Asked {
        release: "meridian".into(),
        namespace: "meridian".into(),
        chart: CHART.into(),
        chart_version: None,
        timeout: "10m".into(),
        https: false,
        authority: None,
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

/// One node, under disk pressure or not.
fn nodes(pressure: bool) -> serde_json::Value {
    json!({ "items": [ {
        "metadata": { "name": "node-1" },
        "spec": if pressure {
            json!({ "taints": [ { "key": "node.kubernetes.io/disk-pressure", "effect": "NoSchedule" } ] })
        } else { json!({}) },
        "status": { "conditions": [
            { "type": "DiskPressure", "status": if pressure { "True" } else { "False" } }
        ] }
    } ] })
}

/// Its kubelet's summary: one disk, shared by the node and its images.
fn disk(free_gib: u64, capacity_gib: u64) -> serde_json::Value {
    let fs = json!({ "availableBytes": free_gib * GIB, "capacityBytes": capacity_gib * GIB });
    json!({ "node": { "fs": fs, "runtime": { "imageFs": fs } }, "pods": [] })
}

/// A healthy deployment of 0.1.180, answering every check.
fn healthy() -> Stand {
    Stand::default()
        .answering("helm version --short", Ok("v3.16.2+g13654a5"))
        .answering("kubectl cluster-info", Ok("Kubernetes control plane is running"))
        .answering("kubectl auth can-i patch deployments -n meridian", Ok("yes"))
        .answering("kubectl auth can-i create jobs -n meridian", Ok("yes"))
        .answering("kubectl auth can-i delete jobs -n meridian", Ok("yes"))
        .json(NODES, nodes(false))
        .json(SUMMARY, disk(62, 98))
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
async fn a_node_under_disk_pressure_refuses_it_before_the_release_is_read() {
    // Pulling the new images onto a node that is already evicting its pods
    // makes it worse, and the new pods would be scheduled nowhere.
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .instead(NODES, Ok(&nodes(true).to_string()));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Refused));
    let said = said.join("\n");
    assert!(
        said.contains("stops    node node-1 is under disk pressure"),
        "{said}"
    );
    assert!(said.contains("pulls the new version's images"), "{said}");
    assert!(said.contains("taint to lift"), "{said}");
    assert!(said.contains("Nothing was changed"), "{said}");
    assert!(
        !stand.asked().contains(&LIST.to_string()),
        "{:?}",
        stand.asked()
    );
    assert!(
        !stand.asked().contains(&SUMMARY.to_string()),
        "{:?}",
        stand.asked()
    );
    assert!(!stand.changed_anything(), "{:?}", stand.asked());
}

#[tokio::test]
async fn low_free_disk_is_said_and_does_not_refuse_it() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .instead(SUMMARY, Ok(&disk(15, 98).to_string()));
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Upgrade(_)), "{said:?}");
    let said = said.join("\n");
    assert!(
        said.contains("worth    node node-1 has 15.0 GiB of 98.0 GiB disk free (15%)"),
        "{said}"
    );
    assert!(said.contains("pulls the new version's images"), "{said}");
}

#[tokio::test]
async fn free_disk_it_may_not_read_is_unknown_and_does_not_refuse_it() {
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .instead(
            SUMMARY,
            Err("Error from server (Forbidden): nodes \"node-1\" is forbidden"),
        );
    let mut said = Vec::new();

    let checked = check(&stand, &asked(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;

    assert!(matches!(checked, Checked::Upgrade(_)), "{said:?}");
    let said = said.join("\n");
    assert!(said.contains("unknown  free disk on node node-1"), "{said}");
    assert!(said.contains("nodes/proxy"), "{said}");
    assert!(
        said.contains("ok       node node-1 is not under disk pressure"),
        "{said}"
    );
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

#[tokio::test]
async fn the_archive_the_deployment_was_given_is_kept_and_said_and_never_set_again() {
    // `meridian up --archive /mnt/nas/meridian-archive` wrote pluginArchive.path
    // into the release's own values; --reset-then-reuse-values reapplies them
    // over the new chart's defaults, so the upgrade sets nothing of its own.
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .instead(
            OWN_VALUES,
            Ok(
                &json!({ "deployment": { "enrolmentCode": "ENROL-DO-NOT-PRINT" },
                        "pluginArchive": { "path": "/mnt/nas/meridian-archive" } })
                .to_string(),
            ),
        )
        .json(WORKLOADS, items(vec![]));
    let plan = planned(&stand).await;
    assert_eq!(
        plan.archive.as_deref(),
        Some("/mnt/nas/meridian-archive on the cluster's node")
    );
    let text = plan_text(&asked(), &plan);
    assert!(
        text.contains(
            "Its archive, /mnt/nas/meridian-archive on the cluster's node, is kept: the \
             deployment's own values carry it."
        ),
        "{text}"
    );
    let arguments = helm_arguments(&asked(), &plan);
    assert!(arguments.contains(&"--reset-then-reuse-values".to_string()));
    assert!(
        !arguments.iter().any(|each| each.contains("pluginArchive")),
        "{arguments:?}"
    );
    assert!(!text.contains("ENROL-DO-NOT-PRINT"), "{text}");

    // None given, none said.
    let plan = planned(
        &healthy()
            .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
            .json(WORKLOADS, items(vec![])),
    )
    .await;
    assert_eq!(plan.archive, None);
    assert!(!plan_text(&asked(), &plan).contains("archive"));
}

#[test]
fn an_archive_is_a_path_a_claim_or_a_bucket_and_an_empty_one_is_none() {
    assert_eq!(archive_in("{}"), None);
    assert_eq!(
        archive_in(r#"{"pluginArchive": {"path": "", "size": "100Gi"}}"#),
        None
    );
    assert_eq!(
        archive_in(r#"{"pluginArchive": {"existingClaim": "nas"}}"#).as_deref(),
        Some("the claim nas")
    );
    assert_eq!(
        archive_in(r#"{"pluginArchive": {"bucket": "s3://firm-archive"}}"#).as_deref(),
        Some("s3://firm-archive")
    );
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
        Checked::Upgrade(plan) => *plan,
        _ => panic!("refused: {said:?}"),
    }
}

/// 0.1.180 to 0.1.182 as it went on 2026-09-28: the conductor and the
/// launcher each restarted once, and two finished Jobs from earlier
/// revisions were left behind.
fn upgrading() -> Stand {
    let old = |component| deployment(component, "845bd06", 1, 1);
    let new = |component| deployment(component, "9c5d480", 1, 1);
    let before = || {
        items(vec![
            pod("conductor-old", "conductor", &format!("{REPO}:845bd06"), 0),
            pod("launcher-old", "launcher", &format!("{REPO}:845bd06"), 0),
            pod("database-0", "database", "postgres:16-alpine", 2),
        ])
    };
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
        // Read for the plan, then before the upgrade, then while waiting.
        .json(PODS, before())
        .json(PODS, before())
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

// ── Pods left over from a restart ────────────────────────────────────────

/// A Deployment at a template revision, as Kubernetes annotates it.
fn at_revision(mut workload: serde_json::Value, revision: &str) -> serde_json::Value {
    workload["metadata"]["annotations"] = json!({ "deployment.kubernetes.io/revision": revision });
    workload
}

/// One of a Deployment's ReplicaSets, `<deployment>-<hash>`, at a revision.
fn replica_set(deployment: &str, hash: &str, revision: &str) -> serde_json::Value {
    json!({
        "kind": "ReplicaSet",
        "metadata": { "name": format!("{deployment}-{hash}"),
                      "annotations": { "deployment.kubernetes.io/revision": revision },
                      "ownerReferences": [ { "kind": "Deployment", "name": deployment } ] }
    })
}

/// A pod made by a ReplicaSet, in a phase.
fn made_by(mut pod: serde_json::Value, set: &str, phase: &str) -> serde_json::Value {
    pod["metadata"]["ownerReferences"] = json!([{ "kind": "ReplicaSet", "name": set }]);
    pod["status"]["phase"] = json!(phase);
    pod
}

const STREET: &str = "meridian-meridian-runtime-street";
const PLUGIN: &str = "meridian-meridian-runtime-plugin-ref";

/// A plugin the launcher launched: its Deployment carries the release's
/// labels, as the chart's plugin template gives every one.
fn plugin(tag: &str) -> serde_json::Value {
    let labels = json!({ "app.kubernetes.io/instance": "meridian",
                         "meridian.dev/component": "sidecar", "meridian.dev/instance": "ref",
                         "meridian.dev/launched": "true" });
    json!({
        "kind": "Deployment",
        "metadata": { "name": PLUGIN, "generation": 1, "labels": labels,
                      "annotations": { "deployment.kubernetes.io/revision": "2" } },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app.kubernetes.io/instance": "meridian",
                                           "meridian.dev/instance": "ref" } },
            "template": { "spec": { "containers": [
                { "name": "sidecar", "image": format!("{REPO}:{tag}") },
                { "name": "plugin", "image": "ghcr.io/example/ref-plugin:0.1.0" } ] } }
        },
        "status": { "observedGeneration": 1, "replicas": 1, "updatedReplicas": 1,
                    "readyReplicas": 1, "availableReplicas": 1 }
    })
}

fn plugin_pod(name: &str, tag: &str) -> serde_json::Value {
    json!({
        "metadata": { "name": name, "labels": { "app.kubernetes.io/instance": "meridian",
                                                "meridian.dev/component": "sidecar",
                                                "meridian.dev/instance": "ref" } },
        "spec": { "containers": [ { "name": "sidecar", "image": format!("{REPO}:{tag}") },
                                  { "name": "plugin", "image": "ghcr.io/example/ref-plugin:0.1.0" } ] },
        "status": { "containerStatuses": [] }
    })
}

/// 0.1.180 to 0.1.182 on a cluster whose node restarted before it: the
/// street and a launched plugin each left a pod `Failed` or `Succeeded`, on
/// an image two releases old, from a ReplicaSet since replaced.
fn restarted_before_the_upgrade() -> Stand {
    let street = |hash: &str, suffix: &str, tag: &str, phase: &str| {
        made_by(
            pod(
                &format!("{STREET}-{hash}-{suffix}"),
                "street",
                &format!("{REPO}:{tag}"),
                0,
            ),
            &format!("{STREET}-{hash}"),
            phase,
        )
    };
    let dead_plugin = made_by(
        plugin_pod(&format!("{PLUGIN}-7a1-dead"), "461c0a3"),
        &format!("{PLUGIN}-7a1"),
        "Succeeded",
    );
    let plugin_now = made_by(
        plugin_pod(&format!("{PLUGIN}-8b2-live"), "845bd06"),
        &format!("{PLUGIN}-8b2"),
        "Running",
    );
    let before = items(vec![
        street("6c7", "live", "845bd06", "Running"),
        street("5f6", "dead", "461c0a3", "Failed"),
        dead_plugin.clone(),
        plugin_now.clone(),
    ]);
    let sets = vec![
        replica_set(STREET, "5f6", "1"),
        replica_set(STREET, "6c7", "2"),
        replica_set(PLUGIN, "7a1", "1"),
        replica_set(PLUGIN, "8b2", "2"),
    ];
    let listed_before = [
        vec![
            at_revision(deployment("street", "845bd06", 1, 1), "2"),
            plugin("845bd06"),
        ],
        sets.clone(),
    ]
    .concat();
    let listed_after = [
        vec![
            at_revision(deployment("street", "9c5d480", 1, 1), "3"),
            plugin("845bd06"),
            replica_set(STREET, "9d8", "3"),
        ],
        sets,
    ]
    .concat();
    healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"))
        .json(WORKLOADS, items(listed_before))
        .json(WORKLOADS, items(listed_after))
        .json(PODS, before.clone())
        .json(PODS, before)
        .json(
            PODS,
            items(vec![
                street("9d8", "live", "9c5d480", "Running"),
                // Of the current ReplicaSet, on the new image: stopped, and
                // kept for what it says, since it is not left over.
                street("9d8", "evicted", "9c5d480", "Failed"),
                street("5f6", "dead", "461c0a3", "Failed"),
                dead_plugin,
                plugin_now,
            ]),
        )
        .json(JOBS, items(vec![job("meridian-meridian-runtime-migrate-8", Some("Complete"))]))
        .answering(UPGRADE, Ok("upgraded"))
        .answering(
            &format!(
                "kubectl delete pod -n meridian --ignore-not-found {STREET}-5f6-dead {PLUGIN}-7a1-dead"
            ),
            Ok("pod deleted"),
        )
}

#[tokio::test]
async fn pods_left_over_from_a_restart_are_not_waited_for_and_are_removed() {
    let stand = restarted_before_the_upgrade();
    let plan = planned(&stand).await;
    let plan_said = plan_text(&asked(), &plan);
    let mut said = Vec::new();

    // Before, the wait counted them, and they never change: it waited out
    // its timeout. A pace with none shows it no longer waits at all.
    let report = apply(
        &stand,
        &asked(),
        &plan,
        &Pace {
            poll: Duration::ZERO,
            timeout: Duration::ZERO,
        },
        &mut |line: &str| said.push(line.to_string()),
    )
    .await
    .unwrap_or_else(|failed| panic!("{failed}\n{said:?}"));

    let dead = [format!("{STREET}-5f6-dead"), format!("{PLUGIN}-7a1-dead")];
    // In the plan the person approves, so the approval covers removing them.
    assert_eq!(plan.left_over, dead);
    assert!(
        plan_said.contains("Then it will remove 2 pods left over from a restart"),
        "{plan_said}"
    );
    assert!(
        plan_said.contains(&format!("    pod/{STREET}-5f6-dead\n")),
        "{plan_said}"
    );
    let said = said.join("\n");
    assert!(
        said.contains(&format!(
            "not waited for: pod/{STREET}-5f6-dead, left over from a restart"
        )),
        "{said}"
    );
    assert!(!said.contains("still runs"), "{said}");
    assert_eq!(report.removed, dead);
    assert!(stand.asked().contains(&format!(
        "kubectl delete pod -n meridian --ignore-not-found {STREET}-5f6-dead {PLUGIN}-7a1-dead"
    )));
    let text = report_text(&asked(), &report);
    assert!(
        text.contains(&format!(
            "Removed 2 pods left over from a restart: pod/{STREET}-5f6-dead, pod/{PLUGIN}-7a1-dead"
        )),
        "{text}"
    );
}

#[test]
fn a_pending_or_terminating_old_pod_is_still_waited_for() {
    let held = workloads(
        &items(vec![
            at_revision(deployment("street", "9c5d480", 1, 1), "3"),
            replica_set(STREET, "6c7", "2"),
            replica_set(STREET, "9d8", "3"),
        ])
        .to_string(),
    );
    let old = format!("{REPO}:845bd06");
    let mut terminating = made_by(
        pod("street-terminating", "street", &old, 0),
        &format!("{STREET}-6c7"),
        "Running",
    );
    terminating["metadata"]["deletionTimestamp"] = json!("2026-09-29T10:00:00Z");
    let running = pods(
        &items(vec![
            made_by(
                pod("street-new", "street", &format!("{REPO}:9c5d480"), 0),
                &format!("{STREET}-9d8"),
                "Running",
            ),
            terminating,
            made_by(
                pod("street-pending", "street", &old, 0),
                &format!("{STREET}-6c7"),
                "Pending",
            ),
            made_by(
                pod("street-dead", "street", &old, 0),
                &format!("{STREET}-6c7"),
                "Failed",
            ),
        ])
        .to_string(),
    );

    let now = progress(8, &[], &held, &running);

    assert_eq!(
        now.waiting,
        [
            format!("deployment/{STREET}: pod/street-terminating still runs {old}"),
            format!("deployment/{STREET}: pod/street-pending still runs {old}"),
        ]
    );
    assert_eq!(now.left_over, ["street-dead"]);
    assert_eq!(now.ready, (0, 1));
    assert_eq!(
        now.now.as_deref(),
        Some("street: pod/street-terminating still on meridian-runtime:845bd06, terminating")
    );
}

#[test]
fn without_its_replica_sets_no_pod_is_taken_for_left_over() {
    // Where the listing holds no ReplicaSet, which one is current is not
    // known, and a pod is waited for rather than removed on a guess.
    let held = workloads(&items(vec![deployment("street", "9c5d480", 1, 1)]).to_string());
    let running = pods(
        &items(vec![made_by(
            pod("street-dead", "street", &format!("{REPO}:845bd06"), 0),
            &format!("{STREET}-6c7"),
            "Failed",
        )])
        .to_string(),
    );

    assert!(left_by_a_restart(&held, &running).is_empty());
}

// ── What is watched ──────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn off_a_terminal_it_says_what_it_waits_for_at_its_interval() {
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
                "status": { "containerStatuses": [ { "name": "street", "restartCount": 0,
                    "state": { "waiting": { "reason": "ContainerCreating" } } } ] }
            })]),
        )
        .json(JOBS, items(vec![job("meridian-meridian-runtime-migrate-8", Some("Complete"))]))
        .answering(UPGRADE, Ok("upgraded"));
    let plan = planned(&stand).await;
    let mut said = Vec::new();
    let mut log = Log::new(Duration::from_secs(20), |line: &str| {
        said.push(line.to_string())
    });

    // Time is paused, and moves only as the wait sleeps: seventy seconds of
    // polls every two, in no time at all.
    let failed = apply(
        &stand,
        &asked(),
        &plan,
        &Pace {
            poll: Duration::from_secs(2),
            timeout: Duration::from_secs(70),
        },
        &mut log,
    )
    .await
    .expect_err("it never became ready");
    drop(log);

    assert!(failed.contains("still waiting for"), "{failed}");
    let waited: Vec<&String> = said
        .iter()
        .filter(|line| line.contains("still waiting"))
        .collect();
    let now =
        "0 of 1 components ready: street: 0 of 1 ready; pod/street-new: street ContainerCreating";
    assert_eq!(
        waited,
        [
            &format!("[20s] still waiting, {now}"),
            &format!("[40s] still waiting, {now}"),
            &format!("[1m 00s] still waiting, {now}"),
        ],
        "{said:#?}"
    );
    for step in [
        "[0s] apply: started",
        "[0s] apply: done in 0s",
        "[0s] migration: done in 0s",
        "[1m 10s] components: failed after 1m 10s",
    ] {
        assert!(said.iter().any(|line| line == step), "{step}: {said:#?}");
    }
}

#[test]
fn a_terminal_is_shown_each_step_the_components_ready_and_what_is_waited_on() {
    let seconds = Duration::from_secs;
    let board = Board {
        title: "Upgrading meridian in meridian".into(),
        elapsed: seconds(64),
        steps: vec![
            (Step::Checks, Mark::Done(seconds(3))),
            (Step::Confirmation, Mark::Done(seconds(12))),
            (Step::Apply, Mark::Done(seconds(8))),
            (Step::Migration, Mark::Running(seconds(41))),
            (Step::Components, Mark::Running(seconds(41))),
            (Step::Cleanup, Mark::Pending),
        ],
        ready: Some((4, 7)),
        now: Some("conductor: 0 of 1 ready, waiting for its migration".into()),
    };

    assert_eq!(
        render(&board, 2, false),
        [
            "Upgrading meridian in meridian  1m 04s",
            "  ✓ checks                     3s",
            "  ✓ plan and confirmation     12s",
            "  ✓ apply                      8s",
            "  ⠹ migration                 41s",
            "  ⠹ components                41s  ready 4 of 7  ███████████░░░░░░░░░",
            "  · cleanup",
            "  waiting on conductor: 0 of 1 ready, waiting for its migration",
        ]
    );
    // The spinner turns with the frame; a failed step is marked as one.
    let mut failed = board.clone();
    failed.steps[4].1 = Mark::Failed(seconds(600));
    failed.now = None;
    let lines = render(&failed, 3, false);
    assert_eq!(lines[4], "  ⠸ migration                 41s");
    assert!(
        lines[5].starts_with("  ✗ components            10m 00s  ready 4 of 7"),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 7, "{lines:?}");
}

#[test]
fn colour_is_escape_codes_this_writes_and_no_color_turns_it_off() {
    let board = Board {
        title: "Upgrading meridian in meridian".into(),
        elapsed: Duration::from_secs(3),
        steps: vec![(Step::Checks, Mark::Done(Duration::from_secs(3)))],
        ready: None,
        now: None,
    };

    let coloured = render(&board, 0, true).join("\n");
    assert!(coloured.contains("\x1b[32m✓\x1b[0m"), "{coloured:?}");
    assert!(!render(&board, 0, false).join("\n").contains('\x1b'));

    assert!(watch::colour(None));
    assert!(watch::colour(Some("".into())));
    assert!(!watch::colour(Some("1".into())));
}

// ── Launched plugins after an upgrade ────────────────────────────────────

/// A launched plugin's pod, as the chart's plugin template labels it and the
/// conductor names its image, on a sidecar tag, made at a time.
fn launched_pod(name: &str, tag: &str, created: &str) -> serde_json::Value {
    json!({
        "metadata": { "name": name, "creationTimestamp": created,
                      "labels": { "app.kubernetes.io/instance": "meridian",
                                  "meridian.dev/component": "sidecar",
                                  "meridian.dev/instance": "snaptrade",
                                  "meridian.dev/launched": "true" } },
        "spec": { "containers": [
            { "name": "sidecar", "image": format!("{REPO}:{tag}") },
            { "name": "plugin", "image": "localhost:5000/plugins/snaptrade@sha256:5663243f580d" } ] },
        "status": { "phase": "Running", "containerStatuses": [] }
    })
}

/// When the new launcher became ready.
const LAUNCHER_READY: &str = "2026-09-30T05:53:40Z";

/// 0.1.180 to 0.1.182 with SnapTrade launched: `before` is its pod before
/// the upgrade, if it had one, and `after` its pod once every component is
/// on the new image.
fn upgrading_with_a_plugin(before: Option<serde_json::Value>, after: serde_json::Value) -> Stand {
    let at = |component, tag| deployment(component, tag, 1, 1);
    let mut launcher = pod("launcher-new", "launcher", &format!("{REPO}:9c5d480"), 0);
    launcher["status"]["conditions"] =
        json!([{ "type": "Ready", "status": "True", "lastTransitionTime": LAUNCHER_READY }]);
    let before = items(
        [
            pod("conductor-old", "conductor", &format!("{REPO}:845bd06"), 0),
            pod("launcher-old", "launcher", &format!("{REPO}:845bd06"), 0),
        ]
        .into_iter()
        .chain(before)
        .collect(),
    );
    healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"))
        .json(
            WORKLOADS,
            items(vec![at("conductor", "845bd06"), at("launcher", "845bd06")]),
        )
        .json(
            WORKLOADS,
            items(vec![at("conductor", "9c5d480"), at("launcher", "9c5d480")]),
        )
        .json(PODS, before.clone())
        .json(PODS, before)
        .json(
            PODS,
            items(vec![
                pod("conductor-new", "conductor", &format!("{REPO}:9c5d480"), 0),
                launcher,
                after,
            ]),
        )
        .json(
            JOBS,
            items(vec![job(
                "meridian-meridian-runtime-migrate-8",
                Some("Complete"),
            )]),
        )
        .answering(UPGRADE, Ok("upgraded"))
}

async fn upgraded(stand: &Stand) -> Report {
    let plan = planned(stand).await;
    let mut said = Vec::new();
    apply(stand, &asked(), &plan, &pace(), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    .unwrap_or_else(|failed| panic!("{failed}\n{said:?}"))
}

#[tokio::test]
async fn a_plugin_on_the_deployments_sidecar_is_not_said() {
    // Launched after the upgrade's new launcher was ready, on its sidecar.
    let stand = upgrading_with_a_plugin(
        None,
        launched_pod("snaptrade-b", "9c5d480", "2026-09-30T05:54:10Z"),
    );

    let report = upgraded(&stand).await;

    assert!(report.relaunch.is_empty(), "{:?}", report.relaunch);
    let text = report_text(&asked(), &report);
    assert!(!text.contains("Relaunch"), "{text}");
    assert!(!text.contains("meridian plugin"), "{text}");
    // Nothing is relaunched either way.
    assert!(!stand
        .asked()
        .iter()
        .any(|command| command.contains("plugin")));
}

#[tokio::test]
async fn a_plugin_left_on_the_old_sidecar_is_named_with_the_commands_that_move_it() {
    let launched = launched_pod("snaptrade-a", "845bd06", "2026-09-30T04:55:57Z");
    let stand = upgrading_with_a_plugin(Some(launched.clone()), launched);

    let report = upgraded(&stand).await;

    assert_eq!(report.relaunch.len(), 1, "{:?}", report.relaunch);
    assert_eq!(report.relaunch[0].instance, "snaptrade");
    assert!(!report.relaunch[0].during_rollout);
    let text = report_text(&asked(), &report);
    assert!(
        text.contains(
            "Relaunch this plugin to move it to this version's sidecar. Nothing was relaunched"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "  snaptrade  sidecar meridian-runtime:845bd06, and the deployment runs \
             meridian-runtime:9c5d480\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("keeps the sidecar it was launched with until it is relaunched"),
        "{text}"
    );
    assert!(
        text.contains(
            "  meridian plugin stop snaptrade\n  \
             meridian plugin launch snaptrade <version> --instance snaptrade\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("`meridian plugin list` names the version"),
        "{text}"
    );
    assert!(!text.contains("launched during the upgrade"), "{text}");
    // Said, and never done.
    let asked_for = stand.asked();
    assert!(
        !asked_for.iter().any(|command| command.contains("plugin")
            || command.starts_with("kubectl delete")
            || command.starts_with("kubectl rollout")),
        "{asked_for:#?}"
    );
}

#[tokio::test]
async fn a_plugin_launched_during_the_rollout_is_said_to_be() {
    // Stopped and launched again mid-upgrade, before the new launcher was
    // ready, as on 2026-09-30: the old launcher made it, on the old sidecar.
    let stand = upgrading_with_a_plugin(
        Some(launched_pod(
            "snaptrade-a",
            "845bd06",
            "2026-09-30T04:55:57Z",
        )),
        launched_pod("snaptrade-b", "845bd06", "2026-09-30T05:53:31Z"),
    );

    let report = upgraded(&stand).await;

    assert_eq!(report.relaunch.len(), 1, "{:?}", report.relaunch);
    assert!(report.relaunch[0].during_rollout);
    let text = report_text(&asked(), &report);
    assert!(
        text.contains(
            "launched during the upgrade: pod/snaptrade-b started before the new launcher was ready"
        ),
        "{text}"
    );
    assert!(
        text.contains("can be launched by the old launcher"),
        "{text}"
    );
    assert!(
        text.contains("meridian plugin launch snaptrade <version> --instance snaptrade"),
        "{text}"
    );
}

#[test]
fn a_live_plugin_is_moved_by_plugin_dev_and_an_unnamed_one_by_placeholders() {
    let mut live = launched_pod("snaptrade-a", "845bd06", "2026-09-30T04:55:57Z");
    live["metadata"]["labels"]["meridian.dev/live"] = json!("true");
    let mut unnamed = launched_pod("other-a", "845bd06", "2026-09-30T04:55:57Z");
    unnamed["metadata"]["labels"]["meridian.dev/instance"] = json!("other");
    unnamed["spec"]["containers"][1]["image"] = json!("ghcr.io/example/other:0.1.0");
    let held = pods(
        &items(vec![
            pod("launcher-new", "launcher", &format!("{REPO}:9c5d480"), 0),
            live,
            unnamed,
        ])
        .to_string(),
    );

    let relaunch = to_relaunch(&held, None);

    assert_eq!(relaunch.len(), 2, "{relaunch:?}");
    assert_eq!(
        relaunch[0].commands()[1],
        "meridian plugin dev --instance snaptrade"
    );
    assert_eq!(
        relaunch[1].commands()[1],
        "meridian plugin launch <name> <version> --instance other"
    );
    let text = relaunch_text(&relaunch);
    assert!(text.contains("these 2 plugins"), "{text}");
    assert!(
        text.contains("  meridian plugin dev --instance snaptrade   (in the plugin's directory)\n"),
        "{text}"
    );
    assert!(text.contains("names the plugin and version"), "{text}");
}

// ── HTTPS for a local deployment ─────────────────────────────────────────

const SECRET: &str =
    "kubectl get secret meridian-tls -n meridian --ignore-not-found -o jsonpath={.data.tls\\.crt}";
const WRITE_SECRET: &str =
    "kubectl apply --server-side --force-conflicts --field-manager meridian -f - <<Secret";
const SETS: &str =
    " --set ingress.tls.secretName=meridian-tls --set dashboard.url=https://meridian.localhost";

fn https(authority: &std::sync::Arc<crate::authority::Authority>) -> Asked {
    Asked {
        https: true,
        authority: Some(authority.clone()),
        ..asked()
    }
}

fn this_machines() -> std::sync::Arc<crate::authority::Authority> {
    std::sync::Arc::new(crate::authority::Authority::make("m", 0).unwrap())
}

#[tokio::test]
async fn https_writes_the_certificate_first_then_one_upgrade_moves_the_address() {
    let authority = this_machines();
    let asked = https(&authority);
    let stand = upgrading()
        .answering(SECRET, Ok(""))
        .answering(&format!("{UPGRADE}{SETS}"), Ok("upgraded"));
    let mut said = Vec::new();
    let Checked::Upgrade(plan) = check(&stand, &asked, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    else {
        panic!("refused: {said:?}");
    };
    assert_eq!(plan.https.as_deref(), Some("meridian.localhost"));
    assert!(
        !stand.asked().contains(&WRITE_SECRET.to_string()),
        "nothing written by a check"
    );
    let text = plan_text(&asked, &plan);
    assert!(
        text.contains(
            "write a certificate for meridian.localhost and *.plugins.meridian.localhost"
        ),
        "{text}"
    );
    assert!(
        text.contains("becomes https://meridian.localhost"),
        "{text}"
    );
    assert!(text.contains(SETS.trim()), "{text}");

    let report = apply(&stand, &asked, &plan, &pace(), &mut |_: &str| {})
        .await
        .unwrap();
    let commands = stand.asked();
    let written = commands
        .iter()
        .position(|c| c == WRITE_SECRET)
        .expect("written");
    let upgraded = commands
        .iter()
        .position(|c| c == &format!("{UPGRADE}{SETS}"))
        .expect("upgraded with the address");
    assert!(written < upgraded, "{commands:#?}");
    assert!(report_text(&asked, &report).contains("Issued a certificate for meridian.localhost"));
}

#[tokio::test]
async fn https_at_the_version_it_is_at_already_is_still_one_upgrade() {
    let authority = this_machines();
    let stand = healthy()
        .json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"))
        .answering(SECRET, Ok(""));
    let mut said = Vec::new();
    let checked = check(&stand, &https(&authority), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;
    let Checked::Upgrade(plan) = checked else {
        panic!("not an upgrade: {said:?}");
    };
    assert_eq!(plan.to.version, "0.1.182");
    assert!(said.join("\n").contains("moves its address to https"));
}

#[tokio::test]
async fn https_for_a_firms_own_name_is_refused_and_nothing_is_changed() {
    let authority = this_machines();
    let stand = healthy()
        .json(LIST, listed(7, "deployed", "0.1.180", "845bd06"))
        .instead(
            OWN_VALUES,
            Ok(r#"{"ingress":{"enabled":true,"host":"meridian.firm.example"}}"#),
        );
    let mut said = Vec::new();
    let checked = check(&stand, &https(&authority), &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;
    assert!(matches!(checked, Checked::Refused));
    assert!(said.join("\n").contains("meridian.firm.example"));
    assert!(!stand.changed_anything());
    assert!(!stand.asked().iter().any(|c| c.contains("<<")));
}

#[tokio::test]
async fn a_certificate_this_machine_wrote_is_renewed_when_due_without_being_asked() {
    let authority = this_machines();
    let elsewhere = crate::authority::Authority::make("m", 0).unwrap();
    let theirs = elsewhere.issue("meridian.localhost", 0).unwrap();
    let encoded = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&theirs.cert_pem)
    };
    let stand = healthy()
        .json(LIST, listed(8, "deployed", "0.1.182", "9c5d480"))
        .instead(
            OWN_VALUES,
            Ok(r#"{"ingress":{"enabled":true,"host":"meridian.localhost","tls":{"secretName":"meridian-tls"}}}"#),
        )
        .answering(SECRET, Ok(&encoded));
    let asked = Asked {
        authority: Some(authority),
        ..asked()
    };
    let mut said = Vec::new();
    let Checked::Upgrade(plan) = check(&stand, &asked, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    else {
        panic!("not renewed: {said:?}");
    };
    let step = plan.certificate.as_ref().expect("a certificate step");
    assert!(step
        .new
        .as_ref()
        .is_some_and(|(_, why)| why.contains("another authority")));
    // Renewing sets no values: the address is where it was.
    assert!(plan.https.is_none());
    assert!(!helm_arguments(&asked, &plan).iter().any(|a| a == "--set"));
}

#[test]
fn where_it_is_reached_is_read_and_nothing_else() {
    let reached = reached_in(
        r#"{"deployment":{"enrolmentCode":"ENROL-DO-NOT-PRINT"},
            "ingress":{"enabled":true,"host":"meridian.localhost","tls":{"secretName":"meridian-tls"}}}"#,
    )
    .unwrap();
    assert_eq!(reached.host, "meridian.localhost");
    assert_eq!(reached.secret.as_deref(), Some("meridian-tls"));
    assert!(!format!("{reached:?}").contains("ENROL"));
    assert_eq!(
        reached_in(r#"{"ingress":{"enabled":false,"host":"x.localhost"}}"#),
        None
    );
}

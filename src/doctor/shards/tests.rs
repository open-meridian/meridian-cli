use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};

use super::*;
use crate::doctor::Answered;

/// A machine answering from a script; anything unscripted fails as a
/// command that ran and said so.
#[derive(Default)]
struct Fake {
    commands: HashMap<String, Result<String, Failure>>,
}

impl Fake {
    fn running(mut self, command: &str, answer: Result<&str, &str>) -> Self {
        self.commands.insert(
            command.to_string(),
            answer
                .map(String::from)
                .map_err(|said| Failure::Said(said.to_string())),
        );
        self
    }
}

#[async_trait::async_trait]
impl Machine for Fake {
    async fn run(&self, program: &str, arguments: &[&str]) -> Result<String, Failure> {
        let asked = format!("{program} {}", arguments.join(" "));
        self.commands
            .get(&asked)
            .cloned()
            .unwrap_or_else(|| Err(Failure::Said(format!("nothing scripted for `{asked}`"))))
    }

    async fn fetch(&self, url: &str, _: &[(&str, &str)]) -> Result<Answered, String> {
        Err(format!("nothing scripted for {url}"))
    }

    fn now_s(&self) -> u64 {
        0
    }
}

/// History pages in order, and the cursors they were asked with.
struct Pages {
    pages: Vec<Value>,
    asked: Mutex<Vec<(u32, String)>>,
}

impl Pages {
    fn of(pages: Vec<Value>) -> Pages {
        Pages {
            pages,
            asked: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl History for Pages {
    async fn page(&self, page_size: u32, cursor: &str) -> Result<Value, String> {
        let mut asked = self.asked.lock().unwrap();
        asked.push((page_size, cursor.to_string()));
        self.pages
            .get(asked.len() - 1)
            .cloned()
            .ok_or_else(|| "read past the last page".to_string())
    }
}

/// A plugin's Deployment as the launcher makes it: its sidecar's shard count
/// where it has one, its storage claim and its archive bucket where given.
fn plugin(
    instance: &str,
    shards: Option<u64>,
    replicas: u64,
    storage: Option<&str>,
    bucket: Option<&str>,
) -> Value {
    let mut sidecar = vec![];
    if let Some(shards) = shards {
        sidecar.push(json!({"name": "MERIDIAN_PLUGIN_SHARDS", "value": shards.to_string()}));
    }
    let mut plugin_env = vec![json!({"name": "STORAGE_DIR", "value": "/var/lib/meridian/storage"})];
    if let Some(bucket) = bucket {
        plugin_env.push(json!({"name": "MERIDIAN_ARCHIVE_BUCKET", "value": bucket}));
    }
    let volumes: Vec<Value> = storage
        .map(|claim| {
            vec![json!({"name": "storage", "persistentVolumeClaim": {"claimName": claim}})]
        })
        .unwrap_or_default();
    let several = shards.is_some_and(|n| n > 1) || replicas > shards.unwrap_or(1);
    json!({
        "metadata": {"name": format!("meridian-meridian-runtime-plugin-{instance}"),
                     "labels": {"meridian.dev/instance": instance, "meridian.dev/launched": "true"}},
        "spec": {"replicas": replicas, "template": {"spec": {
            "serviceAccountName": if several {
                format!("meridian-meridian-runtime-shards-{instance}")
            } else {
                "default".to_string()
            },
            "containers": [
                {"name": "plugin", "env": plugin_env},
                {"name": "sidecar", "env": sidecar},
            ],
            "volumes": volumes,
        }}},
    })
}

fn list(items: Vec<Value>) -> String {
    json!({"items": items}).to_string()
}

fn pods_on(nodes: &[&str]) -> String {
    let items: Vec<Value> = nodes
        .iter()
        .map(|node| json!({"spec": {"nodeName": node}, "status": {"phase": "Running"}}))
        .collect();
    list(items)
}

const DEPLOYMENTS: &str = "kubectl get deployments -n meridian -o json";
const PODS: &str = "kubectl get pods -n meridian -o json";
const RELEASES: &str = "helm list -n meridian -o json";
const VALUES: &str = "helm get values meridian -n meridian -o json";
const ONE_RELEASE: &str = r#"[{"name":"meridian","chart":"meridian-runtime-0.1.540"}]"#;

fn said(findings: &[Finding]) -> String {
    findings.iter().map(|f| format!("{f}\n")).collect()
}

// ── Item 30: storage that cannot take a standby ────────────────────────────

#[tokio::test]
async fn storage_on_several_nodes_with_no_rwx_class_is_listed_naming_the_value() {
    let machine = Fake::default()
        .running(
            DEPLOYMENTS,
            Ok(&list(vec![
                plugin(
                    "feed",
                    None,
                    1,
                    Some("meridian-meridian-runtime-storage-feed"),
                    None,
                ),
                plugin(
                    "ledger",
                    None,
                    1,
                    Some("meridian-meridian-runtime-storage-ledger"),
                    None,
                ),
                // Keeps none: not listed.
                plugin("signal", None, 1, None, None),
            ])),
        )
        .running(
            PODS,
            Ok(&pods_on(&["node-a", "node-b", "node-c", "node-a"])),
        )
        .running(RELEASES, Ok(ONE_RELEASE))
        .running(
            VALUES,
            Ok(r#"{"pluginStorage":{"shared":{"storageClassName":""}}}"#),
        );
    let findings = shards(&machine, "meridian", &NoHistory("unused".into())).await;
    assert_eq!(findings.len(), 1, "{}", said(&findings));
    let finding = format!("{}", findings[0]);
    assert!(matches!(findings[0], Finding::Worth { .. }), "{finding}");
    for words in [
        "plugins feed and ledger keep storage and cannot take a standby",
        "runs on 3 nodes",
        "pluginStorage.shared.storageClassName",
        "--set pluginStorage.shared.storageClassName=<class>",
    ] {
        assert!(finding.contains(words), "{words}: {finding}");
    }
    assert!(!finding.contains("signal"), "{finding}");
}

#[test]
fn storage_can_take_a_standby_on_one_node_or_with_a_class() {
    let held = instances(
        &serde_json::from_str(&list(vec![plugin(
            "feed",
            None,
            1,
            Some("meridian-meridian-runtime-storage-feed"),
            None,
        )]))
        .unwrap(),
    );
    // One node: a ReadWriteOnce claim reaches every pod.
    let one = standby_storage(&held, Some(""), Some(1), "").unwrap();
    assert!(matches!(one, Finding::Fine(_)), "{one}");
    // A class named: fine, naming it.
    let classed = standby_storage(&held, Some("nfs-rwx"), Some(3), "").unwrap();
    assert!(
        matches!(classed, Finding::Fine(_)) && format!("{classed}").contains("nfs-rwx"),
        "{classed}"
    );
    // Nodes that could not be counted count as several, as the conductor
    // counts them.
    let uncounted = standby_storage(&held, Some(""), None, "").unwrap();
    assert!(
        matches!(uncounted, Finding::Worth { .. })
            && format!("{uncounted}").contains("could not be counted"),
        "{uncounted}"
    );
    // The values unread on several nodes: not known, never fine.
    let unread = standby_storage(&held, None, Some(2), "helm is not installed here").unwrap();
    assert!(
        matches!(unread, Finding::Unknown { .. })
            && format!("{unread}").contains("helm is not installed here"),
        "{unread}"
    );
    // No instance keeps storage: nothing to say.
    assert_eq!(standby_storage(&[], Some(""), Some(3), ""), None);
}

#[test]
fn the_shared_class_and_the_release_are_read_and_nothing_else() {
    assert_eq!(
        shared_class(
            &json!({"pluginStorage": {"shared": {"storageClassName": "efs-sc"}},
                             "deployment": {"enrolmentCode": "never read"}})
        ),
        "efs-sc"
    );
    assert_eq!(shared_class(&json!({})), "");
    assert_eq!(
        release_in(&serde_json::from_str(ONE_RELEASE).unwrap()),
        Some("meridian".into())
    );
    assert_eq!(release_in(&json!([])), None);
    assert_eq!(
        release_in(&json!([{"name": "a", "chart": "meridian-runtime-0.1.1"},
                           {"name": "b", "chart": "meridian-runtime-0.1.1"}])),
        None
    );
    assert_eq!(nodes(&json!({"items": []})), Some(0));
    assert_eq!(nodes(&json!("not a list")), None);
    // A finished pod's node is not where the deployment runs.
    assert_eq!(
        nodes(&json!({"items": [
            {"spec": {"nodeName": "a"}, "status": {"phase": "Running"}},
            {"spec": {"nodeName": "b"}, "status": {"phase": "Succeeded"}},
            {"spec": {}, "status": {"phase": "Pending"}},
        ]})),
        Some(1)
    );
}

// ── Items 60 and 63: the account a bucket archive must trust ──────────────

#[test]
fn a_sharded_instance_with_a_bucket_names_its_account_and_the_clouds_step() {
    let workloads = serde_json::from_str(&list(vec![
        plugin("feed", Some(2), 3, None, Some("s3://firm-archive/feed")),
        plugin("gfeed", Some(1), 2, None, Some("gs://firm-archive/gfeed")),
        plugin(
            "afeed",
            Some(2),
            2,
            None,
            Some("https://firm.blob.core.windows.net/archive/afeed"),
        ),
        // One pod: its own account is the chart's, nothing to trust anew.
        plugin("solo", None, 1, None, Some("s3://firm-archive/solo")),
        // Several pods, no archive: nothing.
        plugin("bare", Some(2), 2, None, None),
    ]))
    .unwrap();
    let findings = archive_accounts(&instances(&workloads), "meridian");
    let all = said(&findings);
    assert_eq!(findings.len(), 3, "{all}");
    assert!(
        findings.iter().all(|f| matches!(f, Finding::Worth { .. })),
        "{all}"
    );
    // EKS: the wildcard on the shards accounts, and Pod Identity's per-account
    // association.
    let eks = format!("{}", findings[1]);
    assert!(
        eks.contains("plugin feed runs 2 shards and a standby pod")
            && eks.contains("meridian/meridian-meridian-runtime-shards-feed")
            && eks.contains("system:serviceaccount:meridian:meridian-meridian-runtime-shards-*")
            && eks.contains("EKS Pod Identity"),
        "{eks}"
    );
    // GKE: one binding per shards account, never the namespace (item 63).
    let gke = format!("{}", findings[2]);
    assert!(
        gke.contains("svc.id.goog[meridian/meridian-meridian-runtime-shards-gfeed]")
            && gke.contains("never the namespace as a whole")
            && !gke.contains("principalSet"),
        "{gke}"
    );
    // Azure: a federated credential for the account.
    let azure = format!("{}", findings[0]);
    assert!(
        azure.contains(
            "federated identity credential for meridian/meridian-meridian-runtime-shards-afeed"
        ),
        "{azure}"
    );
    assert!(!all.contains("solo") && !all.contains("bare"), "{all}");
}

// ── Item 32: each shard's epoch and last takeover ──────────────────────────

fn take(instance: &str, shard: u64, epoch: u64, at_ns: u64, holder: &str) -> Value {
    json!({
        "at_ns": at_ns.to_string(),
        "kind": "shard_taken_over",
        "component": instance,
        "partition": format!("shard {shard}"),
        "words": format!("{instance} shard {shard}: {holder} took it, epoch {epoch}: its lease lapsed"),
    })
}

#[tokio::test]
async fn each_shards_epoch_and_last_takeover_come_from_the_newest_take() {
    let held = instances(
        &serde_json::from_str(&list(vec![
            plugin("feed", Some(2), 3, None, None),
            plugin("solo", None, 1, None, None),
        ]))
        .unwrap(),
    );
    let pages = Pages::of(vec![
        json!({"history": [
            take("feed", 1, 7, 1_794_916_800_000_000_000, "feed-b"),
            {"at_ns": "1", "kind": "takeover", "component": "lake", "words": "lake: x took over, epoch 99: its lease lapsed"},
            take("feed", 1, 6, 1_794_900_000_000_000_000, "feed-a"),
        ], "next_cursor": "c1"}),
        json!({"history": [
            take("feed", 0, 3, 1_794_800_000_000_000_000, "feed-a"),
        ], "next_cursor": "c2"}),
    ]);
    let findings = fences(&held, &pages).await;
    let all = said(&findings);
    assert_eq!(findings.len(), 2, "{all}");
    assert!(
        all.contains(
            "plugin feed shard 0: epoch 3, last taken over 2026-11-16T03:33:20Z: feed shard 0: \
             feed-a took it, epoch 3: its lease lapsed"
        ),
        "{all}"
    );
    // The newest of shard 1's takes, not an older one or another store's.
    assert!(
        all.contains("plugin feed shard 1: epoch 7, last taken over 2026-11-17T12:00:00Z"),
        "{all}"
    );
    assert!(
        !all.contains("epoch 6") && !all.contains("epoch 99"),
        "{all}"
    );
    // Read until every shard was found, and no further.
    assert_eq!(
        *pages.asked.lock().unwrap(),
        vec![(200, String::new()), (200, "c1".to_string())]
    );
}

#[tokio::test]
async fn a_shard_with_no_take_in_the_history_read_is_not_known() {
    let held = instances(
        &serde_json::from_str(&list(vec![plugin("feed", Some(1), 2, None, None)])).unwrap(),
    );
    let pages = Pages::of(vec![json!({"history": [
        take("other", 0, 2, 5, "other-a"),
    ], "next_cursor": ""})]);
    let findings = fences(&held, &pages).await;
    assert_eq!(findings.len(), 1);
    let finding = format!("{}", findings[0]);
    assert!(
        matches!(findings[0], Finding::Unknown { .. })
            && finding.contains("plugin feed shard 0's epoch")
            && finding.contains("newest 1 items"),
        "{finding}"
    );
}

#[tokio::test]
async fn the_history_is_asked_only_where_a_plugin_runs_several_pods_and_its_refusal_said() {
    let one_pod =
        instances(&serde_json::from_str(&list(vec![plugin("solo", None, 1, None, None)])).unwrap());
    let none = Pages::of(vec![]);
    assert!(fences(&one_pod, &none).await.is_empty());
    assert!(none.asked.lock().unwrap().is_empty());

    let several = instances(
        &serde_json::from_str(&list(vec![plugin("feed", Some(2), 2, None, None)])).unwrap(),
    );
    let findings = fences(
        &several,
        &NoHistory("not connected to a deployment: `meridian connect` first".into()),
    )
    .await;
    assert_eq!(findings.len(), 1);
    let finding = format!("{}", findings[0]);
    assert!(
        matches!(findings[0], Finding::Unknown { .. })
            && finding.contains("each plugin shard's epoch and last takeover")
            && finding.contains("meridian connect"),
        "{finding}"
    );
}

#[test]
fn a_takes_epoch_is_read_from_its_words() {
    assert_eq!(
        epoch_in("feed shard 0: feed-b took it from feed-a, epoch 12: its lease lapsed"),
        Some(12)
    );
    assert_eq!(epoch_in("feed shard 0: feed-b took it"), None);
}

#[tokio::test]
async fn nothing_is_said_where_no_plugin_runs() {
    let machine = Fake::default().running(DEPLOYMENTS, Ok(&list(vec![])));
    assert!(shards(&machine, "meridian", &NoHistory(String::new()))
        .await
        .is_empty());
    // A namespace it cannot read: the plugins check says so already.
    let unread = Fake::default();
    assert!(shards(&unread, "meridian", &NoHistory(String::new()))
        .await
        .is_empty());
}

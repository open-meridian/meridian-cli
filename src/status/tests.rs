use std::sync::Mutex;

use super::*;

/// A cluster answering kubectl and helm as the test says, each call kept.
struct Fake {
    metrics: bool,
    /// What each node's cAdvisor answers, by node; a node not here refuses,
    /// as a cluster does a person without the nodes/proxy read.
    cadvisor: BTreeMap<String, String>,
    history: Result<String, String>,
    asked: Mutex<Vec<Vec<String>>>,
    empty: bool,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            metrics: true,
            cadvisor: BTreeMap::new(),
            history: Ok(HISTORY.into()),
            asked: Mutex::new(Vec::new()),
            empty: false,
        }
    }
}

const HISTORY: &str = r#"[{"revision": 12, "updated": "2026-10-17T17:03:12Z", "status": "deployed",
    "chart": "meridian-runtime-0.1.307", "app_version": "15f6b22", "description": "Upgrade complete"}]"#;

fn sidecar(instance: &str, image: &str) -> serde_json::Value {
    serde_json::json!({"kind": "Deployment",
     "metadata": {"name": format!("meridian-plugin-{instance}"), "labels": {
         "meridian.dev/component": "sidecar", "meridian.dev/instance": instance,
         "meridian.dev/launched": "true"}},
     "spec": {"replicas": 1,
       "selector": {"matchLabels": {"app.kubernetes.io/instance": "meridian",
                                    "meridian.dev/instance": instance}},
       "template": {"spec": {"containers": [
         {"name": "sidecar", "image": "ghcr.io/open-meridian/meridian-runtime@sha256:6eb6",
          "resources": {"limits": {"memory": "64Mi"}}},
         {"name": "plugin", "image": image, "resources": {"limits": {"memory": "192Mi"}}}]}}},
     "status": {"readyReplicas": 1}})
}

fn sidecar_pod(instance: &str) -> serde_json::Value {
    serde_json::json!({"metadata": {"name": format!("meridian-plugin-{instance}-5d9"), "labels": {
        "app.kubernetes.io/instance": "meridian", "meridian.dev/component": "sidecar",
        "meridian.dev/instance": instance}},
     "spec": {"nodeName": "node-b"},
     "status": {"phase": "Running", "containerStatuses": [
         {"name": "sidecar", "restartCount": 0}, {"name": "plugin", "restartCount": 0}]}})
}

fn workloads() -> serde_json::Value {
    serde_json::json!({"items": [
        {"kind": "Deployment",
         "metadata": {"name": "meridian-bor", "labels": {"meridian.dev/component": "bor"}},
         "spec": {"replicas": 1, "template": {"spec": {"containers": [
             {"name": "bor", "image": "ghcr.io/open-meridian/meridian-runtime@sha256:6eb6",
              "resources": {"limits": {"memory": "512Mi"}}}]}}},
         "status": {"readyReplicas": 1}},
        {"kind": "Deployment",
         "metadata": {"name": "meridian-dashboard", "labels": {"meridian.dev/component": "dashboard"}},
         "spec": {"replicas": 2, "template": {"spec": {"containers": [
             {"name": "dashboard", "image": "ghcr.io/open-meridian/meridian-runtime@sha256:6eb6",
              "resources": {"limits": {"memory": "256Mi"}}}]}}},
         "status": {"readyReplicas": 1}},
        // As the chart makes it: the component in its pods' selector alone.
        {"kind": "StatefulSet",
         "metadata": {"name": "meridian-database", "labels": {"app.kubernetes.io/instance": "meridian"}},
         "spec": {"replicas": 1,
           "selector": {"matchLabels": {"app.kubernetes.io/instance": "meridian", "meridian.dev/component": "database"}},
           "template": {"spec": {"containers": [
             {"name": "postgres", "image": "postgres:17", "resources": {"limits": {"memory": "1Gi"}}}]}}},
         "status": {"readyReplicas": 1}},
        sidecar("snaptrade", "localhost:5000/plugins/open-meridian/snaptrade@sha256:915f"),
        sidecar("tiingo", "localhost:5000/plugins/tiingo@sha256:aa01"),
    ]})
}

fn pods() -> serde_json::Value {
    serde_json::json!({"items": [
        {"metadata": {"name": "meridian-bor-7c9", "labels": {"meridian.dev/component": "bor"}},
         "spec": {"nodeName": "node-a"},
         "status": {"phase": "Running", "containerStatuses": [
             {"name": "bor", "restartCount": 3, "lastState": {"terminated":
                 {"reason": "OOMKilled", "finishedAt": "2026-10-15T09:12:00Z"}}}]}},
        {"metadata": {"name": "meridian-dashboard-1", "labels": {"meridian.dev/component": "dashboard"}},
         "spec": {"nodeName": "node-a"},
         "status": {"phase": "Running", "containerStatuses": [{"name": "dashboard", "restartCount": 0}]}},
        {"metadata": {"name": "meridian-dashboard-2", "labels": {"meridian.dev/component": "dashboard"}},
         "spec": {"nodeName": "node-b"},
         "status": {"phase": "Running", "containerStatuses": [{"name": "dashboard", "restartCount": 0}]}},
        {"metadata": {"name": "meridian-database-0", "labels": {"app.kubernetes.io/instance": "meridian", "meridian.dev/component": "database"}},
         "status": {"phase": "Running", "containerStatuses": [{"name": "postgres", "restartCount": 0}]}},
        sidecar_pod("snaptrade"),
        sidecar_pod("tiingo"),
    ]})
}

#[async_trait::async_trait]
impl Cluster for Fake {
    async fn kubectl(&self, arguments: &[&str]) -> Result<String, String> {
        self.asked
            .lock()
            .unwrap()
            .push(arguments.iter().map(|a| a.to_string()).collect());
        let joined = arguments.join(" ");
        if joined.starts_with("get deployments,statefulsets") {
            if self.empty {
                return Ok(r#"{"items": []}"#.into());
            }
            return Ok(workloads().to_string());
        }
        if joined.starts_with("get pods") {
            return Ok(pods().to_string());
        }
        if joined.starts_with("get jobs") {
            return Ok(serde_json::json!({"items": [
                {"metadata": {"name": "meridian-migrate-11", "creationTimestamp": "2026-10-16T10:00:00Z"}},
                {"metadata": {"name": "meridian-migrate-12", "creationTimestamp": "2026-10-17T17:03:20Z"}},
                {"metadata": {"name": "meridian-key", "creationTimestamp": "2026-09-01T10:00:00Z"}},
                {"metadata": {"name": "meridian-first-run", "creationTimestamp": "2026-09-01T10:00:00Z",
                              "labels": {"meridian.dev/component": "first-run"}}},
            ]})
            .to_string());
        }
        if let Some(node) = joined
            .strip_prefix("get --raw /api/v1/nodes/")
            .and_then(|rest| rest.strip_suffix("/proxy/metrics/cadvisor"))
        {
            return self.cadvisor.get(node).cloned().ok_or_else(|| {
                format!(
                    "Error from server (Forbidden): nodes \"{node}\" is forbidden: User \"admin\" \
                     cannot get resource \"nodes/proxy\" in API group \"\" at the cluster scope"
                )
            });
        }
        if joined.starts_with("get --raw /apis/metrics.k8s.io/") {
            return if self.metrics {
                Ok(serde_json::json!({"items": [
                    {"metadata": {"name": "meridian-bor-7c9"},
                     "containers": [{"name": "bor", "usage": {"memory": "419430Ki"}}]},
                    {"metadata": {"name": "meridian-dashboard-1"},
                     "containers": [{"name": "dashboard", "usage": {"memory": "120Mi"}}]},
                    {"metadata": {"name": "meridian-dashboard-2"},
                     "containers": [{"name": "dashboard", "usage": {"memory": "98Mi"}}]},
                    {"metadata": {"name": "meridian-plugin-snaptrade-5d9"},
                     "containers": [{"name": "sidecar", "usage": {"memory": "5Mi"}},
                                    {"name": "plugin", "usage": {"memory": "4Mi"}}]},
                    {"metadata": {"name": "meridian-plugin-tiingo-5d9"},
                     "containers": [{"name": "sidecar", "usage": {"memory": "4Mi"}},
                                    {"name": "plugin", "usage": {"memory": "3Mi"}}]},
                ]})
                .to_string())
            } else {
                Err("the server could not find the requested resource".into())
            };
        }
        if joined.starts_with("logs") {
            if joined.contains("--previous") && joined.contains("component=database") {
                return Err("previous terminated container \"postgres\" in pod \"meridian-database-0\" not found".into());
            }
            return Ok("[pod/meridian-bor-7c9/bor] replayed ACC-01SNAP\n".into());
        }
        Err(format!("not stood in for: {joined}"))
    }

    async fn helm(&self, arguments: &[&str]) -> Result<String, String> {
        self.asked
            .lock()
            .unwrap()
            .push(arguments.iter().map(|a| a.to_string()).collect());
        let joined = arguments.join(" ");
        if joined.starts_with("history") {
            return self.history.clone();
        }
        if joined.starts_with("list") {
            return Ok(r#"[{"name": "staging"}, {"name": "meridian-old"}]"#.into());
        }
        Err(format!("not stood in for: helm {joined}"))
    }
}

fn asked() -> Asked {
    Asked {
        release: "meridian".into(),
        namespace: "meridian".into(),
    }
}

#[test]
fn memory_quantities_read_as_bytes() {
    assert_eq!(bytes_of("512Mi"), Some(536_870_912));
    assert_eq!(bytes_of("1Gi"), Some(1_073_741_824));
    assert_eq!(bytes_of("419430Ki"), Some(429_496_320));
    assert_eq!(bytes_of("500M"), Some(500_000_000));
    assert_eq!(bytes_of("1048576"), Some(1_048_576));
    assert_eq!(bytes_of("lots"), None);
    assert_eq!(said_bytes(429_496_320), "410 MiB");
    assert_eq!(said_bytes(1_610_612_736), "1.5 GiB");
}

#[tokio::test]
async fn without_metrics_it_says_so_and_book_health_not_read_says_why() {
    let fake = Fake {
        metrics: false,
        ..Fake::new()
    };
    let mut held = status(&fake, &asked()).await.unwrap();
    assert!(held.components.iter().all(|c| c.memory.is_none()));
    held.accounts = Err("not connected to a deployment: `meridian connect` first".into());
    let said = said_wide(&held, 0, "meridian");
    assert!(
        said.contains("memory in use is not known: the cluster serves no metrics"),
        "{said}"
    );
    assert!(said.contains("bor        1/1 ready  3 restart(s), the last OOMKilled at 2026-10-15T09:12:00Z  limit 512 MiB"), "{said}");
    assert!(
        said.contains(
            "Book health:\n  not read: not connected to a deployment: `meridian connect` first"
        ),
        "{said}"
    );
}

#[tokio::test]
async fn each_plugin_is_its_own_line_by_its_instance_with_its_own_pods_memory() {
    // 0.1.46 said 762 MiB on every sidecar line: every plugin pod's, summed.
    let fake = Fake::new();
    let held = status(&fake, &asked()).await.unwrap();
    let names: Vec<&str> = held.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["bor", "dashboard", "database"],
        "plugins are not components"
    );
    let instances: Vec<&str> = held
        .plugins
        .iter()
        .filter_map(|c| c.instance.as_deref())
        .collect();
    assert_eq!(instances, ["snaptrade", "tiingo"]);
    // The plugin container's own, its sidecar's apart.
    assert_eq!(held.plugins[0].memory, Some(4 * 1024 * 1024));
    assert_eq!(held.plugins[1].memory, Some(3 * 1024 * 1024));
    assert_eq!(held.plugins[0].limit, Some(192 * 1024 * 1024));
    let said = said_wide(&held, 0, "meridian");
    assert!(said.contains("Plugins:\n  snaptrade  1/1 ready  no restarts  memory 4 MiB of 192 MiB (2%)  localhost:5000/plugins/open-meridian/snaptrade@sha256:915f\n             sidecar  memory 5 MiB of 64 MiB (7%)\n"), "{said}");
    assert!(said.contains("  tiingo     1/1 ready  no restarts  memory 3 MiB of 192 MiB (1%)  localhost:5000/plugins/tiingo@sha256:aa01\n             sidecar  memory 4 MiB of 64 MiB (6%)\n"), "{said}");
    // Two replicas: each pod's own, against the limit each has.
    assert!(
        said.contains("dashboard  1/2 ready  no restarts  memory 120 MiB, 98 MiB of 256 MiB each"),
        "{said}"
    );
    let json = json_of(&held);
    assert_eq!(json["plugins"][0]["instance_id"], "snaptrade");
    assert_eq!(json["plugins"][0]["memory_bytes"], 4 * 1024 * 1024);
    assert_eq!(
        json["plugins"][0]["sidecar"]["memory_bytes"],
        5 * 1024 * 1024
    );
    assert_eq!(
        json["components"][1]["pods"][1]["pod"],
        "meridian-dashboard-2"
    );
}

#[tokio::test]
async fn the_release_is_said_and_helms_words_where_it_is_not_deployed() {
    let held = status(&Fake::new(), &asked()).await.unwrap();
    let said = said_wide(&held, 0, "meridian");
    assert!(said.starts_with("Release: meridian revision 12, deployed, chart meridian-runtime-0.1.307 (app 15f6b22), updated 2026-10-17T17:03:12Z\n"), "{said}");
    assert!(!said.contains("Helm says"));
    assert_eq!(json_of(&held)["release"]["revision"], 12);

    let failed = Fake {
        history: Ok(r#"[{"revision": 13, "status": "failed", "chart": "meridian-runtime-0.1.308",
            "app_version": "77aa", "updated": "", "description": "Upgrade \"meridian\" failed: context deadline exceeded"}]"#.into()),
        ..Fake::new()
    };
    let said = super::said_wide(&status(&failed, &asked()).await.unwrap(), 0, "meridian");
    assert!(
        said.contains(
            "Release: meridian revision 13, failed, chart meridian-runtime-0.1.308 (app 77aa)\n"
        ),
        "{said}"
    );
    assert!(
        said.contains("  Helm says: Upgrade \"meridian\" failed: context deadline exceeded\n"),
        "{said}"
    );
    assert!(
        said.contains("`meridian rollback-deployment -n meridian` returns it"),
        "{said}"
    );

    let unread = Fake {
        history: Err("helm is not installed here".into()),
        ..Fake::new()
    };
    let said = super::said_wide(&status(&unread, &asked()).await.unwrap(), 0, "meridian");
    assert!(
        said.starts_with("Release: not read: helm is not installed here\n"),
        "{said}"
    );
}

#[tokio::test]
async fn no_deployment_there_names_the_releases_that_are() {
    let fake = Fake {
        empty: true,
        ..Fake::new()
    };
    let refused = status(&fake, &asked()).await.err().unwrap();
    assert_eq!(
        refused,
        "no deployment `meridian` in meridian (its releases: staging, meridian-old): `meridian up` \
         installs one, and -n and --release name another"
    );
}

#[tokio::test]
async fn logs_are_a_known_components_and_since_takes_a_span() {
    let fake = Fake::new();
    let printed = logs(
        &fake,
        &asked(),
        "bor",
        &Read {
            since: Some("10m".into()),
            previous: false,
        },
    )
    .await
    .unwrap();
    assert!(printed.contains("[pod/meridian-bor-7c9/bor]"));
    let call = fake.asked.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        call,
        [
            "logs",
            "-n",
            "meridian",
            "-l",
            "app.kubernetes.io/instance=meridian,meridian.dev/component=bor",
            "--all-containers",
            "--prefix",
            "--tail=-1",
            "--max-log-requests=20",
            "--since=10m",
        ]
    );
    // A day, which kubectl refuses as `1d`, reaches it in hours.
    logs(
        &fake,
        &asked(),
        "bor",
        &Read {
            since: Some("1d".into()),
            previous: false,
        },
    )
    .await
    .unwrap();
    let call = fake.asked.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        call.last().map(String::as_str),
        Some("--since=24h"),
        "{call:?}"
    );
    let refused = logs(&fake, &asked(), "kernel", &Read::default())
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        "`kernel` is not a component of meridian in meridian: bor, dashboard, database, \
         first-run, key, migrate; its plugins: snaptrade, tiingo"
    );
    assert!(logs(
        &fake,
        &asked(),
        "bor",
        &Read {
            since: Some("yesterday".into()),
            previous: false
        }
    )
    .await
    .unwrap_err()
    .contains("--since takes a span"));
    assert!(is_since("30s") && is_since("2h") && is_since("1d"));
    // kubectl's --since stops at hours: a day is said as 24 of them.
    assert_eq!(kubectl_since("1d"), "24h");
    assert_eq!(kubectl_since("7d"), "168h");
    assert_eq!(kubectl_since("10m"), "10m");
    assert_eq!(kubectl_since("2h"), "2h");
    assert!(!is_since("h") && !is_since("10") && !is_since("1h30m"));
}

#[tokio::test]
async fn logs_read_a_restarted_container_a_plugin_and_the_releases_jobs() {
    let fake = Fake::new();
    let previous = Read {
        since: None,
        previous: true,
    };
    logs(&fake, &asked(), "bor", &previous).await.unwrap();
    let call = fake.asked.lock().unwrap().last().cloned().unwrap();
    assert!(call.contains(&"--previous".to_string()), "{call:?}");
    // Nothing restarted: said, in the CLI's words.
    let refused = logs(&fake, &asked(), "database", &previous)
        .await
        .unwrap_err();
    assert!(
        refused.contains("no container of database has restarted"),
        "{refused}"
    );

    logs(&fake, &asked(), "snaptrade", &Read::default())
        .await
        .unwrap();
    let call = fake.asked.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        call[4],
        "app.kubernetes.io/instance=meridian,meridian.dev/instance=snaptrade"
    );
    // The migration's newest run, found by its name past the release's prefix.
    logs(&fake, &asked(), "migrate", &Read::default())
        .await
        .unwrap();
    let call = fake.asked.lock().unwrap().last().cloned().unwrap();
    assert_eq!(call[4], "job-name=meridian-migrate-12");
    logs(&fake, &asked(), "first-run", &Read::default())
        .await
        .unwrap();
    let call = fake.asked.lock().unwrap().last().cloned().unwrap();
    assert_eq!(call[4], "job-name=meridian-first-run");
    // Read-only: get, logs and helm's reads, nothing else.
    for call in fake.asked.lock().unwrap().iter() {
        assert!(
            matches!(call[0].as_str(), "get" | "logs" | "history" | "list"),
            "{call:?}"
        );
    }
}

#[tokio::test]
async fn status_says_each_component_and_the_books_health() {
    let fake = Fake::new();
    let mut held = status(&fake, &asked()).await.unwrap();
    let (accounts, next) = accounts_from(&serde_json::json!({"accounts": [
        {"account_id": "ACC-01SNAP", "journal_entries": 23114, "entries_since_snapshot": 1200,
         "last_snapshot_at_ns": "1792108800000000000"},
        {"account_id": "ACC-02FRESH", "journal_entries": 12, "entries_since_snapshot": 12},
    ]}));
    assert!(next.is_empty());
    held.accounts = Ok(accounts);
    let bor = &held.components[0];
    assert_eq!((bor.ready, bor.wanted, bor.restarts), (1, 1, 3));
    assert_eq!(bor.last_reason, "OOMKilled");
    assert_eq!(bor.memory, Some(429_496_320));
    assert_eq!(bor.limit, Some(536_870_912));
    let said = said_wide(&held, 1_792_195_200_000_000_000, "meridian");
    assert!(said.contains("bor        1/1 ready  3 restart(s), the last OOMKilled at 2026-10-15T09:12:00Z  memory 410 MiB of 512 MiB (79%)  ghcr.io/open-meridian/meridian-runtime@sha256:6eb6"), "{said}");
    assert!(said.contains("ACC-01SNAP   replays 1200 of its 23114 entries, since its snapshot of 2026-10-16T00:00:00Z (24h ago)"), "{said}");
    assert!(
        said.contains(
            "ACC-02FRESH  replays 12 of its 12 entries, no snapshot yet: its whole journal"
        ),
        "{said}"
    );
    let json = json_of(&held);
    assert_eq!(json["components"][0]["last_restart_reason"], "OOMKilled");
    assert_eq!(json["book_health"][0]["entries_since_snapshot"], 1200);
    assert_eq!(json["book_health"][0]["journal_entries"], 23114);
    // Read-only: nothing but get, get --raw, helm history, a log, and each
    // store member's probe, which changes nothing.
    for call in fake.asked.lock().unwrap().iter() {
        let probe = call[0] == "exec" && call.last().map(String::as_str) == Some("probe");
        assert!(
            matches!(call[0].as_str(), "get" | "history" | "logs") || probe,
            "{call:?}"
        );
    }
}

/// design's fixture for the row, fixtures/config/read-book-health-from-terminal.yaml
/// (DESIGN25 amendment 2, 081f95f): its reply, as the deployment answers it.
const FIXTURE_REPLY: &str = r#"{"accounts": [{"account_id": "ACC-01J9N6E3B70000000000001", "journal_entries": 23114, "entries_since_snapshot": 114, "last_snapshot_at_ns": "1792195200000000000"}]}"#;

#[tokio::test]
async fn book_health_is_the_deployments_answer_page_by_page() {
    use crate::fake_http::{serve, Asked, Reply};
    let (address, kept) = serve(|asked: &Asked| {
        let (path, query) = asked
            .path
            .split_once('?')
            .unwrap_or((asked.path.as_str(), ""));
        if path != BOOK_HEALTH_PATH {
            return Reply::json(404, serde_json::json!({"error": "no such route"}));
        }
        if query.contains("cursor=page-2") {
            return Reply::json(200, serde_json::from_str(FIXTURE_REPLY).unwrap());
        }
        Reply::json(
            200,
            serde_json::json!({"accounts": [
                {"account_id": "ACC-00FIRST", "journal_entries": 5, "entries_since_snapshot": 5},
            ], "next_cursor": "page-2"}),
        )
    })
    .await;
    let session = crate::credential::Credential::terminal_session(&address, "a-session");
    let accounts = book_health(&address, &session).await.unwrap();
    assert_eq!(
        accounts,
        [
            Account {
                account_id: "ACC-00FIRST".into(),
                journal_entries: 5,
                replay: 5,
                snapshot_at_ns: None,
            },
            Account {
                account_id: "ACC-01J9N6E3B70000000000001".into(),
                journal_entries: 23_114,
                replay: 114,
                snapshot_at_ns: Some(1_792_195_200_000_000_000),
            },
        ]
    );
    let asked: Vec<Asked> = kept.lock().unwrap().clone();
    assert_eq!(asked.len(), 2);
    assert_eq!(
        asked[0].path,
        format!("{BOOK_HEALTH_PATH}?page_size={BOOK_HEALTH_PAGE}")
    );
    assert_eq!(
        asked[1].path,
        format!("{BOOK_HEALTH_PATH}?page_size={BOOK_HEALTH_PAGE}&cursor=page-2")
    );
    assert_eq!(asked[0].header("authorization"), Some("Bearer a-session"));

    // The fixture's cases: not a deployment admin (403); the book
    // unreachable (503), said as not read, never as healthy; an older core.
    for (status, said) in [
        (403, "takes a deployment admin"),
        (503, "cannot reach the book now"),
        (404, "does not answer ReadBookHealthFromTerminal"),
    ] {
        let (refusing, _) =
            serve(move |_: &Asked| Reply::json(status, serde_json::json!({}))).await;
        let session = crate::credential::Credential::terminal_session(&refusing, "a-session");
        let refused = book_health(&refusing, &session).await.unwrap_err();
        assert!(refused.contains(said), "{status}: {refused}");
    }
}

/// What a node's cAdvisor printed on 2026-10-31 (rancher-desktop, k3s
/// v1.36.5 on the Docker runtime) for a pod that had read a 300 MiB file:
/// its cgroup's usage (304 MiB), nearly all of it inactive file cache, and
/// its working set (3.6 MiB). The metrics API, asked of the same pod that
/// day, gave its usage less a little: the usage, as that runtime gives it.
const MEASURED: &str = r#"# HELP container_memory_working_set_bytes Current working set in bytes.
# TYPE container_memory_working_set_bytes gauge
container_memory_total_inactive_file_bytes{container="",id="/kubepods/burstable/pod00c2dc31-a95d-46d4-9651-6133cb261463",image="",name="",namespace="cli0154-scratch",pod="cache"} 3.14925056e+08 1793465853249
container_memory_usage_bytes{container="",id="/kubepods/burstable/pod00c2dc31-a95d-46d4-9651-6133cb261463",image="",name="",namespace="cli0154-scratch",pod="cache"} 3.18697472e+08 1793465853249
container_memory_working_set_bytes{container="",id="/kubepods/burstable/pod00c2dc31-a95d-46d4-9651-6133cb261463",image="",name="",namespace="cli0154-scratch",pod="cache"} 3.772416e+06 1793465853249
container_memory_working_set_bytes{container="",id="/kubepods/burstable/pod11",image="",name="",namespace="kube-system",pod="cache"} 9.9e+07 1793465853249
"#;

#[test]
fn a_pods_working_set_is_cadvisors_not_its_usage_with_the_file_cache() {
    let found = working_sets(MEASURED, "cli0154-scratch");
    assert_eq!(
        found.len(),
        1,
        "another namespace's pod of the same name is not this one's"
    );
    assert_eq!(found["cache"], 3_772_416, "3.6 MiB, not the 304 MiB in use");
    assert_eq!(said_bytes(found["cache"]), "4 MiB");
}

#[test]
fn without_a_pods_own_line_its_containers_are_summed_never_the_sandbox() {
    let text = "\
container_memory_working_set_bytes{container=\"plugin\",namespace=\"meridian\",pod=\"p-1\"} 4194304\n\
container_memory_working_set_bytes{container=\"sidecar\",namespace=\"meridian\",pod=\"p-1\"} 5.24288e+06 1793465853249\n\
container_memory_working_set_bytes{container=\"POD\",namespace=\"meridian\",pod=\"p-1\"} 999999\n\
container_memory_working_set_bytes{container=\"x\",id=\"/a,b=\\\"c\\\"}\",namespace=\"meridian\",pod=\"p-2\"} 1024\n\
container_memory_working_set_bytes{container=\"x\",namespace=\"meridian\",pod=\"p-3\"} NaN\n\
container_memory_usage_bytes{container=\"plugin\",namespace=\"meridian\",pod=\"p-1\"} 1e+09\n";
    let found = working_sets(text, "meridian");
    assert_eq!(found["p-1"], 9 * 1024 * 1024);
    assert_eq!(
        found["p-2"], 1024,
        "a label value holding a comma, a brace and quotes"
    );
    assert!(
        !found.contains_key("p-3"),
        "a sample that is not a number is not memory"
    );
}

/// node-a: bor and one dashboard, each with its pod's own line; node-b: the
/// other dashboard and the plugins, their containers' alone.
fn cadvisor() -> BTreeMap<String, String> {
    let line = |pod: &str, container: &str, bytes: u64| {
        format!(
            "container_memory_working_set_bytes{{container=\"{container}\",id=\"/kubepods/{pod}\",\
             image=\"\",name=\"\",namespace=\"meridian\",pod=\"{pod}\"}} {bytes} 1793465853249\n"
        )
    };
    let mib = 1024 * 1024;
    BTreeMap::from([
        (
            "node-a".to_string(),
            line("meridian-bor-7c9", "", 300 * mib) + &line("meridian-dashboard-1", "", 60 * mib),
        ),
        (
            "node-b".to_string(),
            line("meridian-dashboard-2", "dashboard", 50 * mib)
                + &line("meridian-plugin-snaptrade-5d9", "sidecar", 2 * mib)
                + &line("meridian-plugin-snaptrade-5d9", "plugin", 3 * mib)
                + &line("meridian-plugin-tiingo-5d9", "sidecar", 2 * mib)
                + &line("meridian-plugin-tiingo-5d9", "plugin", 2 * mib),
        ),
    ])
}

#[tokio::test]
async fn memory_is_each_pods_working_set_from_its_nodes_cadvisor() {
    let fake = Fake {
        cadvisor: cadvisor(),
        ..Fake::new()
    };
    let held = status(&fake, &asked()).await.unwrap();
    assert_eq!(held.memory_read, Some(MemoryRead::Cadvisor));
    assert_eq!(held.memory_note, None);
    assert_eq!(held.components[0].memory, Some(300 * 1024 * 1024));
    assert_eq!(held.plugins[0].memory, Some(3 * 1024 * 1024));
    assert_eq!(
        held.plugins[0].sidecar.as_ref().and_then(|s| s.memory),
        Some(2 * 1024 * 1024)
    );
    let said = said_wide(&held, 0, "meridian");
    assert!(said.contains("memory 300 MiB of 512 MiB (58%)"), "{said}");
    assert!(
        said.contains("memory 60 MiB, 50 MiB of 256 MiB each"),
        "{said}"
    );
    assert!(!said.contains("metrics API"), "{said}");
    assert_eq!(json_of(&held)["memory_read"], "cadvisor");
    let calls = fake.asked.lock().unwrap();
    let raw: Vec<String> = calls
        .iter()
        .filter(|call| call.get(1).map(String::as_str) == Some("--raw"))
        .map(|call| call[2].clone())
        .collect();
    assert_eq!(
        raw,
        [
            "/api/v1/nodes/node-a/proxy/metrics/cadvisor",
            "/api/v1/nodes/node-b/proxy/metrics/cadvisor"
        ],
        "each node the release runs on, once, and the metrics API not asked"
    );
}

#[tokio::test]
async fn where_a_nodes_cadvisor_is_refused_the_metrics_apis_figure_is_said_as_that() {
    let mut nodes = cadvisor();
    nodes.remove("node-b");
    let fake = Fake {
        cadvisor: nodes,
        ..Fake::new()
    };
    let held = status(&fake, &asked()).await.unwrap();
    assert_eq!(held.memory_read, Some(MemoryRead::MetricsApi));
    assert_eq!(
        held.components[0].memory,
        Some(429_496_320),
        "the metrics API's, every pod's"
    );
    let said = said_wide(&held, 0, "meridian");
    assert!(
        said.contains(
            "(memory is the metrics API's figure, which some container runtimes give with the \
             file cache in: the nodes' own working set was not read (Error from server \
             (Forbidden): nodes \"node-b\" is forbidden"
        ),
        "{said}"
    );
    assert_eq!(json_of(&held)["memory_read"], "metrics-api");
    assert_eq!(json_of(&held)["metrics"], true);
}

#[tokio::test]
async fn a_sharded_plugin_wants_its_shards_ready_and_its_other_pods_stand_by() {
    // Step 3d: two shards and a standby pod are three replicas, all ready.
    let mut sharded = sidecar("tiingo", "localhost:5000/plugins/tiingo@sha256:aa01");
    sharded["spec"]["replicas"] = serde_json::json!(3);
    sharded["status"]["readyReplicas"] = serde_json::json!(3);
    sharded["spec"]["template"]["spec"]["containers"][0]["env"] =
        serde_json::json!([{"name": "MERIDIAN_PLUGIN_SHARDS", "value": "2"}]);
    // One that says its count and has no pod past it, and one from before.
    let mut single = sidecar("oanda", "localhost:5000/plugins/oanda@sha256:bb02");
    single["spec"]["template"]["spec"]["containers"][0]["env"] =
        serde_json::json!([{"name": "MERIDIAN_PLUGIN_SHARDS", "value": "1"}]);
    let older = sidecar("snaptrade", "localhost:5000/plugins/snaptrade@sha256:915f");
    let found = components(
        &serde_json::json!({ "items": [sharded, single, older] }),
        &serde_json::json!({ "items": [] }),
        &Usage::default(),
    );
    let shape: Vec<(u64, u64, u64)> = found
        .iter()
        .map(|c| (c.ready, c.wanted, c.standbys))
        .collect();
    assert_eq!(shape, [(1, 1, 0), (1, 1, 0), (2, 2, 1)]);

    let mut held = status(&Fake::new(), &asked()).await.unwrap();
    held.plugins = found;
    let said = said_wide(&held, 0, "meridian");
    assert!(
        said.contains("  tiingo     2/2 ready, 1 standby  no restarts"),
        "{said}"
    );
    assert!(
        said.contains("  oanda      1/1 ready  no restarts"),
        "{said}"
    );
    let json = json_of(&held);
    assert_eq!(json["plugins"][2]["wanted"], 2);
    assert_eq!(json["plugins"][2]["standbys"], 1);
    assert_eq!(json["plugins"][0]["standbys"], 0);
}

/// A healthy deployment as `up` leaves one: twelve components, the database
/// it brought, six store members, no plugin, not connected.
pub(crate) fn healthy() -> Status {
    let component = |name: &str| Component {
        name: name.into(),
        kind: "Deployment".into(),
        images: vec![format!("ghcr.io/open-meridian/meridian-runtime:4859c49")],
        ready: 1,
        wanted: 1,
        standbys: 0,
        restarts: 0,
        last_reason: String::new(),
        last_at: String::new(),
        memory: Some(16 * 1024 * 1024),
        limit: Some(256 * 1024 * 1024),
        pods: vec![(
            format!("meridian-{name}-7796799698-ljr6d"),
            Some(16 * 1024 * 1024),
        )],
        instance: None,
        plugin_image: None,
        launched: false,
        sidecar: None,
    };
    let names = [
        "analytics-coordinator",
        "analytics-worker",
        "bor",
        "broker",
        "conductor",
        "dashboard",
        "database",
        "instrument",
        "lake",
        "launcher",
        "registry",
        "street",
    ];
    let path = serde_json::json!({
        "ok": true, "data_checksums": "on", "full_page_writes": "on",
        "integrity": {"findings": [], "checked": 0},
    });
    let probe = |component: &str| resilience::Probed {
        component: component.into(),
        pod: format!("meridian-meridian-runtime-{component}-7796799698-ljr6d"),
        member: serde_json::json!({"mode": "writer", "ready": true, "lag_ms": 0,
            "partitions": ["control"]}),
        database: Some(path.clone()),
        unread: None,
    };
    Status {
        release: Ok(Release {
            name: "meridian".into(),
            revision: 1,
            status: "deployed".into(),
            chart: "meridian-runtime-0.1.655".into(),
            app_version: "4859c49".into(),
            updated: "2026-11-22T23:52:57.448486-05:00".into(),
            description: String::new(),
        }),
        components: names.iter().map(|n| component(n)).collect(),
        plugins: vec![],
        memory_read: Some(MemoryRead::Cadvisor),
        memory_note: None,
        accounts: Err(NOT_CONNECTED.into()),
        members: Err(NOT_CONNECTED.into()),
        resilience: resilience::Resilience {
            database: Some(resilience::Database {
                set: crate::upgrade_deployment::DatabaseSet {
                    name: "meridian-meridian-runtime-database".into(),
                    replicas: 1,
                    patroni: false,
                    archive: false,
                    tls: false,
                },
                members: vec![crate::offline::Member {
                    name: "meridian-meridian-runtime-database-0".into(),
                    role: None,
                    ready: true,
                    node: None,
                    terminating: false,
                }],
                behind: BTreeMap::new(),
                waiting_for_node: BTreeMap::new(),
            }),
            probes: [
                "bor",
                "conductor",
                "instrument",
                "lake",
                "launcher",
                "street",
            ]
            .iter()
            .map(|c| probe(c))
            .collect(),
            replicas: None,
            internal_tls: false,
            stopped: None,
            scope: String::new(),
        },
    }
}

#[test]
fn a_healthy_status_is_a_line_a_part() {
    let said = said(&healthy(), "meridian");
    assert_eq!(
        said,
        "Release: meridian revision 1, deployed, chart meridian-runtime-0.1.655 (app 4859c49)\n\
         Components: 12 of 12 ready, no restarts\n\
         Database: 1 of 1 member(s) ready, page checksums on, WAL archive off, internal TLS off\n\
         Members: 6 of 6 ready, lag at most 0 ms, no integrity findings\n\
         Plugins: none running; `meridian plugin list` shows the catalogue\n\
         Book health: not read; run `meridian connect` first (with its address if on another \
         machine)\n\
         Images, pods and each member's probe: `meridian status --wide`\n"
    );
    assert!(said.lines().count() <= 20);
    assert!(
        said.lines().all(|line| line.chars().count() <= WIDTH),
        "{said}"
    );
    // The wide view still says every image, pod and probe.
    let wide = said_wide(&healthy(), 0, "meridian");
    assert!(
        wide.contains("ghcr.io/open-meridian/meridian-runtime:4859c49"),
        "{wide}"
    );
    assert!(
        wide.contains("meridian-meridian-runtime-bor-7796799698-ljr6d"),
        "{wide}"
    );
}

#[test]
fn whatever_is_unhealthy_expands_with_the_command_that_reads_why() {
    let mut held = healthy();
    held.components[2].ready = 0;
    held.components[3].restarts = 2;
    held.components[3].last_reason = "OOMKilled".into();
    held.components[3].last_at = "2026-11-22T23:59:00Z".into();
    held.resilience.probes[1].member["ready"] = serde_json::json!(false);
    held.resilience.scope = "-n firm".into();
    let said = said(&held, "firm");
    assert!(
        said.contains("Components: 11 of 12 ready, 2 restart(s)\n"),
        "{said}"
    );
    assert!(said.contains("  bor  0/1 ready, no restarts, memory 16 MiB of 256 MiB (6%): `meridian logs bor -n firm`\n"), "{said}");
    assert!(
        said.contains("`meridian logs broker --previous -n firm`"),
        "{said}"
    );
    assert!(said.contains("Members: 5 of 6 ready"), "{said}");
    assert!(
        said.contains("  conductor: not ready: `meridian logs conductor -n firm`\n"),
        "{said}"
    );
    assert!(
        said.lines().all(|line| line.chars().count() <= WIDTH),
        "{said}"
    );
}

/// A plugin's pod as the launcher makes it: the plugin and its sidecar, each
/// with a limit of its own (256 MiB each), and cAdvisor's line for the pod as
/// a whole beside each container's.
fn plugin_of_two() -> (Vec<Component>, Usage) {
    let mut workload = sidecar("oanda", "localhost:5000/plugins/oanda@sha256:bb02");
    for container in 0..2 {
        workload["spec"]["template"]["spec"]["containers"][container]["resources"]["limits"]
            ["memory"] = serde_json::json!("256Mi");
    }
    let mut pod = sidecar_pod("oanda");
    pod["status"]["containerStatuses"][1]["restartCount"] = serde_json::json!(1);
    let line = |container: &str, mib: u64| {
        format!(
            "container_memory_working_set_bytes{{container=\"{container}\",namespace=\"meridian\",\
             pod=\"meridian-plugin-oanda-5d9\"}} {}\n",
            mib * 1024 * 1024
        )
    };
    let usage = cadvisor_usage(
        &(line("", 150) + &line("plugin", 100) + &line("sidecar", 50)),
        "meridian",
    );
    let found = components(
        &serde_json::json!({ "items": [workload] }),
        &serde_json::json!({ "items": [pod] }),
        &usage,
    );
    (found, usage)
}

#[test]
fn a_plugins_memory_is_its_own_container_and_its_sidecar_is_said_apart() {
    // The pod's limit, its containers' summed, said 512 MiB where the plugin
    // container has 256 MiB.
    let mib = 1024 * 1024;
    let (found, usage) = plugin_of_two();
    assert_eq!(usage.pods["meridian-plugin-oanda-5d9"], 150 * mib);
    let plugin = &found[0];
    assert_eq!(plugin.limit, Some(256 * mib), "the plugin container's own");
    assert_eq!(plugin.memory, Some(100 * mib), "the plugin container's own");
    let sidecar = plugin.sidecar.as_ref().expect("the sidecar, apart");
    assert_eq!(sidecar.limit, Some(256 * mib));
    assert_eq!(sidecar.memory, Some(50 * mib));

    let mut held = healthy();
    held.plugins = found;
    let wide = said_wide(&held, 0, "meridian");
    assert!(
        wide.contains(
            "  oanda  1/1 ready  1 restart(s)  memory 100 MiB of 256 MiB (39%)  \
             localhost:5000/plugins/oanda@sha256:bb02\n         sidecar  memory 50 MiB of \
             256 MiB (19%)\n"
        ),
        "{wide}"
    );
    assert!(!wide.contains("512 MiB"), "{wide}");

    // The compact status: one line for it, the plugin's own memory.
    let compact = said(&held, "meridian");
    let lines: Vec<&str> = compact
        .lines()
        .filter(|line| line.contains("oanda"))
        .collect();
    assert_eq!(
        lines,
        [
            "  oanda  1/1 ready, 1 restart(s), memory 100 MiB of 256 MiB (39%): `meridian logs \
          oanda --previous`"
        ],
        "{compact}"
    );

    let json = json_of(&held);
    assert_eq!(json["plugins"][0]["memory_bytes"], 100 * mib);
    assert_eq!(json["plugins"][0]["memory_limit_bytes"], 256 * mib);
    assert_eq!(json["plugins"][0]["sidecar"]["memory_bytes"], 50 * mib);
    assert_eq!(
        json["plugins"][0]["sidecar"]["memory_limit_bytes"],
        256 * mib
    );
    assert_eq!(
        json["plugins"][0]["sidecar"]["pods"][0]["memory_bytes"],
        50 * mib
    );
    assert!(json["components"][0].get("sidecar").is_none());
}

#[test]
fn a_sidecar_near_its_own_limit_is_said_on_its_plugins_one_line() {
    let mib = 1024 * 1024;
    let (mut found, _) = plugin_of_two();
    found[0].restarts = 0;
    let sidecar = found[0].sidecar.as_mut().unwrap();
    sidecar.memory = Some(240 * mib);
    sidecar.pods[0].1 = Some(240 * mib);
    let mut held = healthy();
    held.plugins = found;
    // Carried on to a second line past the width, as every long line is.
    let compact = said(&held, "meridian").replace("\n      ", " ");
    assert!(
        compact.contains(
            "  oanda  1/1 ready, no restarts, memory 100 MiB of 256 MiB (39%), sidecar memory \
             240 MiB of 256 MiB (93%): `meridian logs oanda`\n"
        ),
        "{compact}"
    );
}

/// A lake member as ReadResilience answers it, with its health and the
/// words it said it with.
fn lake_member(member: &str, health: &str, detail: &str) -> serde_json::Value {
    serde_json::json!({"component": "lake", "member": member, "mode": "writer", "zone": "",
        "lag_ms": 0, "partitions": ["control"], "restarts": 0, "last_restart_reason": "",
        "health": health, "detail": detail})
}

/// The lake's writer's words while its write-ahead stream is stuck full,
/// and while the broker refuses its writes, as the lake says them.
const STUCK: &str = "the lake's write-ahead stream is stuck full for 30s, since \
    2026-11-24T10:00:00Z: holds 256.0 MiB of its 256.0 MiB bound, the oldest write the lake has \
    not recorded 40s old of its 3600s bound, 12 write(s) past the lake's place (the lake at 99, \
    the stream at 111); a write it has no room for is refused for load, none it holds is dropped";
const REFUSED: &str = "the lake's write-ahead writes are refused or not answered by the broker \
    for 35s, since 2026-11-24T10:05:00Z: every look the lake's guard took at the stream was \
    refused or not answered; the last: timed out";

/// The dashboard's notice for a stuck write-ahead stream sends a person to
/// `meridian status`: each member whose stream is stuck full or refused is
/// a line with its state, since when and what it holds against its bound,
/// in both views; nothing more is said while every member is serving, and
/// `--json` lists every member's health and words as the deployment said
/// them.
#[test]
fn a_stuck_write_ahead_stream_is_a_line_a_member_with_since_when_and_what_it_holds() {
    let mut held = healthy();
    held.members = Ok(vec![lake_member("om-lake-0", "serving", "")]);
    assert_eq!(said(&held, "meridian"), said(&healthy(), "meridian"));
    assert_eq!(
        said_wide(&held, 0, "meridian"),
        said_wide(&healthy(), 0, "meridian")
    );

    held.members = Ok(vec![
        lake_member("om-lake-0", "degraded", STUCK),
        lake_member("om-lake-1", "degraded", REFUSED),
        lake_member("om-lake-2", "degraded", &format!("{STUCK}; {REFUSED}")),
        lake_member("om-lake-3", "serving", ""),
    ]);
    let stuck =
        "  lake/om-lake-0: write-ahead stream stuck full since 2026-11-24T10:00:00Z, holds \
                 256.0 MiB of its 256.0 MiB bound\n";
    let refused = "  lake/om-lake-1: write-ahead writes refused or not answered by the broker \
                   since 2026-11-24T10:05:00Z\n";
    let both = "  lake/om-lake-2: write-ahead stream stuck full since 2026-11-24T10:00:00Z, holds \
                256.0 MiB of its 256.0 MiB bound; write-ahead writes refused or not answered by \
                the broker since 2026-11-24T10:05:00Z\n";
    let wide = said_wide(&held, 0, "meridian");
    for line in [stuck, refused, both] {
        assert_eq!(wide.matches(line).count(), 1, "{wide}");
    }
    assert!(!wide.contains("om-lake-3"), "{wide}");
    let compact = said(&held, "meridian");
    assert!(
        compact.contains(&format!(
            "Members: 6 of 6 ready, lag at most 0 ms, no integrity findings\n{}",
            wrapped(stuck)
        )),
        "{compact}"
    );
    assert!(compact.contains(&wrapped(refused)), "{compact}");
    assert!(compact.contains(&wrapped(both)), "{compact}");
    assert!(!compact.contains("om-lake-3"), "{compact}");
    assert!(
        compact.lines().all(|line| line.chars().count() <= WIDTH),
        "{compact}"
    );

    // Words with no time or figures that read say the state alone.
    held.members = Ok(vec![lake_member(
        "om-lake-0",
        "degraded",
        "the lake's write-ahead stream is stuck full for 30s, since soon",
    )]);
    assert!(said_wide(&held, 0, "meridian")
        .contains("  lake/om-lake-0: write-ahead stream stuck full\n"));

    let json = json_of(&held);
    assert_eq!(json["members"][0]["health"], "degraded");
    held.members = Err("not read".into());
    assert_eq!(json_of(&held)["members"]["not_read"], "not read");
    assert_eq!(said(&held, "meridian"), said(&healthy(), "meridian"));
}

use std::sync::Mutex;

use super::*;

/// A cluster answering kubectl and helm as the test says, each call kept.
struct Fake {
    metrics: bool,
    history: Result<String, String>,
    asked: Mutex<Vec<Vec<String>>>,
    empty: bool,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            metrics: true,
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
         "status": {"phase": "Running", "containerStatuses": [
             {"name": "bor", "restartCount": 3, "lastState": {"terminated":
                 {"reason": "OOMKilled", "finishedAt": "2026-10-15T09:12:00Z"}}}]}},
        {"metadata": {"name": "meridian-dashboard-1", "labels": {"meridian.dev/component": "dashboard"}},
         "status": {"phase": "Running", "containerStatuses": [{"name": "dashboard", "restartCount": 0}]}},
        {"metadata": {"name": "meridian-dashboard-2", "labels": {"meridian.dev/component": "dashboard"}},
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
        if joined.starts_with("get --raw") {
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
    let said = said(&held, 0, "meridian");
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
    assert_eq!(held.plugins[0].memory, Some(9 * 1024 * 1024));
    assert_eq!(held.plugins[1].memory, Some(7 * 1024 * 1024));
    assert_eq!(held.plugins[0].limit, Some(256 * 1024 * 1024));
    let said = said(&held, 0, "meridian");
    assert!(said.contains("Plugins:\n  snaptrade  1/1 ready  no restarts  memory 9 MiB of 256 MiB (3%)  localhost:5000/plugins/open-meridian/snaptrade@sha256:915f\n"), "{said}");
    assert!(said.contains("  tiingo     1/1 ready  no restarts  memory 7 MiB of 256 MiB (2%)  localhost:5000/plugins/tiingo@sha256:aa01\n"), "{said}");
    // Two replicas: each pod's own, against the limit each has.
    assert!(
        said.contains("dashboard  1/2 ready  no restarts  memory 120 MiB, 98 MiB of 256 MiB each"),
        "{said}"
    );
    let json = json_of(&held);
    assert_eq!(json["plugins"][0]["instance_id"], "snaptrade");
    assert_eq!(json["plugins"][0]["memory_bytes"], 9 * 1024 * 1024);
    assert_eq!(
        json["components"][1]["pods"][1]["pod"],
        "meridian-dashboard-2"
    );
}

#[tokio::test]
async fn the_release_is_said_and_helms_words_where_it_is_not_deployed() {
    let held = status(&Fake::new(), &asked()).await.unwrap();
    let said = said(&held, 0, "meridian");
    assert!(said.starts_with("Release: meridian revision 12, deployed, chart meridian-runtime-0.1.307 (app 15f6b22), updated 2026-10-17T17:03:12Z\n"), "{said}");
    assert!(!said.contains("Helm says"));
    assert_eq!(json_of(&held)["release"]["revision"], 12);

    let failed = Fake {
        history: Ok(r#"[{"revision": 13, "status": "failed", "chart": "meridian-runtime-0.1.308",
            "app_version": "77aa", "updated": "", "description": "Upgrade \"meridian\" failed: context deadline exceeded"}]"#.into()),
        ..Fake::new()
    };
    let said = super::said(&status(&failed, &asked()).await.unwrap(), 0, "meridian");
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
    let said = super::said(&status(&unread, &asked()).await.unwrap(), 0, "meridian");
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
    let said = said(&held, 1_792_195_200_000_000_000, "meridian");
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
    // Read-only: nothing but get, get --raw and helm history.
    for call in fake.asked.lock().unwrap().iter() {
        assert!(matches!(call[0].as_str(), "get" | "history"), "{call:?}");
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

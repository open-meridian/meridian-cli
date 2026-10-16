use std::sync::Mutex;

use super::*;

/// A cluster answering kubectl as the test says, each call kept.
struct Fake {
    metrics: bool,
    asked: Mutex<Vec<Vec<String>>>,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            metrics: true,
            asked: Mutex::new(Vec::new()),
        }
    }
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
             {"name": "dashboard", "image": "ghcr.io/open-meridian/meridian-runtime@sha256:6eb6"}]}}},
         "status": {"readyReplicas": 1}},
        {"kind": "StatefulSet",
         "metadata": {"name": "meridian-database", "labels": {"meridian.dev/component": "database"}},
         "spec": {"replicas": 1, "template": {"spec": {"containers": [
             {"name": "postgres", "image": "postgres:17", "resources": {"limits": {"memory": "1Gi"}}}]}}},
         "status": {"readyReplicas": 1}},
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
        {"metadata": {"name": "meridian-database-0", "labels": {"meridian.dev/component": "database"}},
         "status": {"phase": "Running", "containerStatuses": [{"name": "postgres", "restartCount": 0}]}},
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
            return Ok(workloads().to_string());
        }
        if joined.starts_with("get pods") {
            return Ok(pods().to_string());
        }
        if joined.starts_with("get --raw") {
            return if self.metrics {
                Ok(serde_json::json!({"items": [
                    {"metadata": {"name": "meridian-bor-7c9"},
                     "containers": [{"name": "bor", "usage": {"memory": "419430Ki"}}]},
                ]})
                .to_string())
            } else {
                Err("the server could not find the requested resource".into())
            };
        }
        if joined.starts_with("logs") {
            return Ok("[pod/meridian-bor-7c9/bor] replayed ACC-01SNAP\n".into());
        }
        Err(format!("not stood in for: {joined}"))
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
async fn status_says_each_component_and_the_books_health() {
    let fake = Fake::new();
    let mut held = status(&fake, &asked()).await.unwrap();
    held.accounts = Ok(accounts_from(&serde_json::json!({"accounts": [
        {"account_id": "ACC-01SNAP", "replay_length": 1200, "snapshot_at_ns": "1792108800000000000"},
        {"account_id": "ACC-02FRESH", "replay_length": 12},
    ]})));
    let names: Vec<&str> = held.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["bor", "dashboard", "database"]);
    let bor = &held.components[0];
    assert_eq!((bor.ready, bor.wanted, bor.restarts), (1, 1, 3));
    assert_eq!(bor.last_reason, "OOMKilled");
    assert_eq!(bor.memory, Some(429_496_320));
    assert_eq!(bor.limit, Some(536_870_912));
    let said = said(&held, 1_792_195_200_000_000_000);
    assert!(said.contains("bor        1/1 ready  3 restart(s), the last OOMKilled at 2026-10-15T09:12:00Z  memory 410 MiB of 512 MiB (79%)  ghcr.io/open-meridian/meridian-runtime@sha256:6eb6"), "{said}");
    assert!(
        said.contains("dashboard  1/2 ready  no restarts  no limit"),
        "{said}"
    );
    assert!(said.contains("ACC-01SNAP   replays 1200 entries, since its snapshot of 2026-10-16T00:00:00Z (24h ago)"), "{said}");
    assert!(
        said.contains("ACC-02FRESH  replays 12 entries, no snapshot yet: its whole journal"),
        "{said}"
    );
    let json = json_of(&held);
    assert_eq!(json["components"][0]["last_restart_reason"], "OOMKilled");
    assert_eq!(json["book_health"][0]["replay_entries"], 1200);
    // Read-only: nothing but get and get --raw.
    for call in fake.asked.lock().unwrap().iter() {
        assert_eq!(call[0], "get", "{call:?}");
    }
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
    let said = said(&held, 0);
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
async fn book_health_is_the_deployments_answer_to_read_book_health() {
    use crate::fake_http::{serve, Asked, Reply};
    let (address, kept) = serve(|asked: &Asked| {
        if asked.path == BOOK_HEALTH_PATH {
            Reply::json(
                200,
                serde_json::json!({"accounts": [
                    {"account_id": "ACC-01", "replay_length": 40, "snapshot_at_ns": 1792108800000000000u64},
                ]}),
            )
        } else {
            Reply::json(404, serde_json::json!({"error": "no such route"}))
        }
    })
    .await;
    let session = crate::credential::Credential::terminal_session(&address, "a-session");
    let accounts = book_health(&address, &session).await.unwrap();
    assert_eq!(
        accounts,
        [Account {
            account_id: "ACC-01".into(),
            replay: 40,
            snapshot_at_ns: Some(1_792_108_800_000_000_000)
        }]
    );
    assert_eq!(
        kept.lock().unwrap()[0].header("authorization"),
        Some("Bearer a-session")
    );

    let (older, _) = serve(|_: &Asked| Reply::json(404, serde_json::json!({}))).await;
    let session = crate::credential::Credential::terminal_session(&older, "a-session");
    assert!(book_health(&older, &session)
        .await
        .unwrap_err()
        .contains("does not answer ReadBookHealth yet"));
}

#[tokio::test]
async fn logs_are_a_known_components_and_since_takes_a_span() {
    let fake = Fake::new();
    let printed = logs(&fake, &asked(), "bor", Some("10m")).await.unwrap();
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
    assert_eq!(
        logs(&fake, &asked(), "kernel", None).await.unwrap_err(),
        "`kernel` is not a component of meridian in meridian: bor, dashboard, database"
    );
    assert!(logs(&fake, &asked(), "bor", Some("yesterday"))
        .await
        .unwrap_err()
        .contains("--since takes a span"));
    assert!(is_since("30s") && is_since("2h") && is_since("1d"));
    assert!(!is_since("h") && !is_since("10") && !is_since("1h30m"));
}

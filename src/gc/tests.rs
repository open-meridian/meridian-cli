use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use crate::doctor::Answered;

/// kubectl answering from a script, every command kept.
#[derive(Default)]
struct Stand {
    answers: HashMap<String, Result<String, String>>,
    asked: Mutex<Vec<String>>,
}

impl Stand {
    fn answering(mut self, command: &str, answer: Result<&str, &str>) -> Self {
        self.answers.insert(
            command.to_string(),
            answer.map(String::from).map_err(String::from),
        );
        self
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Machine for Stand {
    async fn run(&self, program: &str, arguments: &[&str]) -> Result<String, Failure> {
        let command = format!("{program} {}", arguments.join(" "));
        self.asked.lock().unwrap().push(command.clone());
        self.answers
            .get(&command)
            .cloned()
            .unwrap_or_else(|| Err(format!("nothing scripted for `{command}`")))
            .map_err(Failure::Said)
    }
    async fn fetch(&self, url: &str, _: &[(&str, &str)]) -> Result<Answered, String> {
        Err(format!("gc fetches nothing, and asked for {url}"))
    }
    fn now_s(&self) -> u64 {
        0
    }
}

const LIST: &str =
    "kubectl get statefulsets -n meridian -l app.kubernetes.io/instance=meridian -o json";
const SET: &str = r#"kubectl set env statefulset/meridian-meridian-runtime-registry -n meridian -c registry REGISTRY_STORAGE_MAINTENANCE_READONLY={"enabled":true}"#;
const BACK: &str = "kubectl set env statefulset/meridian-meridian-runtime-registry -n meridian -c registry REGISTRY_STORAGE_MAINTENANCE_READONLY-";
const ROLLOUT: &str =
    "kubectl rollout status statefulset/meridian-meridian-runtime-registry -n meridian --timeout=5m";
const COLLECT: &str = "kubectl exec -n meridian meridian-meridian-runtime-registry-0 -c registry -- registry garbage-collect /etc/docker/registry/config.yml";

fn registry(read_only: bool) -> String {
    let env = if read_only {
        serde_json::json!([{"name": "REGISTRY_HTTP_ADDR", "value": ":5000"},
                           {"name": READ_ONLY, "value": "{\"enabled\":true}"}])
    } else {
        serde_json::json!([{"name": "REGISTRY_HTTP_ADDR", "value": ":5000"}])
    };
    // As the chart makes them: the component in the selector alone.
    serde_json::json!({"items": [
        {"metadata": {"name": "meridian-meridian-runtime-database"},
         "spec": {"selector": {"matchLabels": {"meridian.dev/component": "database"}},
                  "template": {"spec": {"containers": [{"name": "postgres"}]}}}},
        {"metadata": {"name": "meridian-meridian-runtime-registry"},
         "spec": {"selector": {"matchLabels": {"app.kubernetes.io/instance": "meridian",
                                               "meridian.dev/component": "registry"}},
                  "template": {"spec": {"containers": [{"name": "registry", "image": "registry:2.8.3", "env": env}]}}}}]})
    .to_string()
}

fn stand(read_only: bool) -> Stand {
    Stand::default()
        .answering(LIST, Ok(&registry(read_only)))
        .answering(
            "kubectl auth can-i patch statefulsets -n meridian",
            Ok("yes"),
        )
        .answering("kubectl auth can-i create pods/exec -n meridian", Ok("yes"))
        .answering(
            SET,
            Ok("statefulset.apps/meridian-meridian-runtime-registry env updated"),
        )
        .answering(
            BACK,
            Ok("statefulset.apps/meridian-meridian-runtime-registry env updated"),
        )
        .answering(ROLLOUT, Ok("partitioned roll out complete"))
}

fn asked() -> Asked {
    Asked {
        release: "meridian".into(),
        namespace: "meridian".into(),
        timeout: "5m".into(),
    }
}

const SAID: &str = "plugins/acme/venue-desk\nplugins/acme/venue-desk: marking manifest sha256:aa\n\n12 blobs marked, 7 blobs and 0 manifests eligible for deletion\nblob eligible for deletion: sha256:bb\n";

#[tokio::test]
async fn read_only_then_the_collection_then_back() {
    let stand = stand(false).answering(COLLECT, Ok(SAID));
    let plan = check(&stand, &asked()).await.unwrap();
    assert!(!plan.read_only_already);
    let text = plan_text(&asked(), &plan);
    assert!(
        text.contains("pulls go on") && text.contains("no manifest is"),
        "{text}"
    );
    // The checks changed nothing.
    assert!(!stand
        .asked()
        .iter()
        .any(|c| c.starts_with("kubectl set env") || c.starts_with("kubectl exec")));
    let mut said = Vec::new();
    let done = apply(&stand, &asked(), &plan, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    .unwrap();
    assert!(
        done.contains("12 blobs marked, 7 blobs and 0 manifests eligible for deletion"),
        "{done}"
    );
    let changes: Vec<String> = stand
        .asked()
        .into_iter()
        .filter(|c| {
            c.starts_with("kubectl set env")
                || c.starts_with("kubectl exec")
                || c.contains("rollout")
        })
        .collect();
    assert_eq!(changes, [SET, ROLLOUT, COLLECT, BACK, ROLLOUT]);
}

#[tokio::test]
async fn a_run_left_read_only_is_finished_and_a_failed_collection_sets_it_back() {
    let left = stand(true).answering(COLLECT, Ok(SAID));
    let plan = check(&left, &asked()).await.unwrap();
    assert!(plan.read_only_already);
    assert!(plan_text(&asked(), &plan).contains("read-only already"));
    apply(&left, &asked(), &plan, &mut |_: &str| {})
        .await
        .unwrap();
    assert!(!left.asked().contains(&SET.to_string()), "not set again");

    let failing = stand(false).answering(COLLECT, Err("error: unable to upgrade connection"));
    let plan = check(&failing, &asked()).await.unwrap();
    let refused = apply(&failing, &asked(), &plan, &mut |_: &str| {})
        .await
        .unwrap_err();
    assert!(
        refused.contains("the collection failed") && refused.contains("set back"),
        "{refused}"
    );
    assert!(failing.asked().contains(&BACK.to_string()));

    // An empty registry: nothing to collect, and set back.
    let empty = stand(false).answering(
        COLLECT,
        Err("failed to garbage collect: failed to mark: filesystem: Path not found: /docker/registry/v2/repositories"),
    );
    let plan = check(&empty, &asked()).await.unwrap();
    let done = apply(&empty, &asked(), &plan, &mut |_: &str| {})
        .await
        .unwrap();
    assert!(done.contains("holds no image yet"), "{done}");
    assert!(empty.asked().contains(&BACK.to_string()));
}

#[tokio::test]
async fn no_registry_or_no_right_is_refused_before_anything_changes() {
    let none = Stand::default().answering(
        LIST,
        Ok(r#"{"items": [{"metadata": {"name": "db"}, "spec": {"selector": {"matchLabels": {"meridian.dev/component": "database"}}}}]}"#),
    );
    let refused = check(&none, &asked()).await.unwrap_err();
    assert!(refused.contains("brings no plugin registry"), "{refused}");
    let unpermitted =
        stand(false).answering("kubectl auth can-i create pods/exec -n meridian", Err("no"));
    let refused = check(&unpermitted, &asked()).await.unwrap_err();
    assert!(refused.contains("create pods/exec"), "{refused}");
    assert!(!unpermitted.asked().iter().any(|c| c.contains("set env")));
}

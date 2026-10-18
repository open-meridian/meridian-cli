use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use crate::doctor::Answered;

/// helm and kubectl answering from a script, every command kept.
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
        match self.answers.get(&command) {
            Some(answer) => answer.clone().map_err(Failure::Said),
            None if program == "kubectl" && arguments[0] == "get" => Ok(r#"{"items": []}"#.into()),
            None => Err(Failure::Said(format!("nothing scripted for `{command}`"))),
        }
    }
    async fn fetch(&self, url: &str, _: &[(&str, &str)]) -> Result<Answered, String> {
        Err(format!("a rollback fetches nothing, and asked for {url}"))
    }
    fn now_s(&self) -> u64 {
        0
    }
}

const HISTORY: &str = "helm history meridian --namespace meridian --max 256 -o json";
const REVISIONS: &str = r#"[
  {"revision": 11, "status": "superseded", "chart": "meridian-runtime-0.1.306", "app_version": "abf028f", "description": "Upgrade complete"},
  {"revision": 12, "status": "superseded", "chart": "meridian-runtime-0.1.307", "app_version": "15f6b22", "description": "Upgrade complete"},
  {"revision": 13, "status": "failed", "chart": "meridian-runtime-0.1.308", "app_version": "77aa01c", "description": "Upgrade \"meridian\" failed: timed out waiting for the condition"}
]"#;

fn stand() -> Stand {
    Stand::default()
        .answering("helm version --short", Ok("v3.16.2+g13654a5"))
        .answering(
            "kubectl cluster-info",
            Ok("Kubernetes control plane is running"),
        )
        .answering(HISTORY, Ok(REVISIONS))
}

fn asked(to: Option<u64>) -> Asked {
    Asked {
        release: "meridian".into(),
        namespace: "meridian".into(),
        to_revision: to,
        timeout: "10m".into(),
    }
}

#[tokio::test]
async fn it_returns_to_the_last_deployed_revision_and_says_the_database_stays() {
    let stand = stand();
    let plan = check(&stand, &asked(None)).await.unwrap();
    assert_eq!((plan.from.revision, plan.to.revision), (13, 12));
    let said = plan_text(&asked(None), &plan);
    assert!(
        said.contains("from revision 13, failed, chart meridian-runtime-0.1.308"),
        "{said}"
    );
    assert!(
        said.contains("to   revision 12, chart meridian-runtime-0.1.307 (app 15f6b22)"),
        "{said}"
    );
    assert!(
        said.contains("Helm says of 13: Upgrade \"meridian\" failed"),
        "{said}"
    );
    assert!(
        said.contains("The database is not rolled back with it"),
        "{said}"
    );
    // The checks changed nothing.
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));
    // A revision named.
    let plan = check(&stand, &asked(Some(11))).await.unwrap();
    assert_eq!(plan.to.revision, 11);
}

#[tokio::test]
async fn what_cannot_be_rolled_back_to_is_refused_before_anything_changes() {
    let stand = stand();
    assert!(check(&stand, &asked(Some(9)))
        .await
        .unwrap_err()
        .contains("has no revision 9; Helm keeps 11, 12, 13"));
    assert!(check(&stand, &asked(Some(13)))
        .await
        .unwrap_err()
        .contains("is the newest already"));
    let alone = Stand::default()
        .answering("helm version --short", Ok("v3.16.2"))
        .answering("kubectl cluster-info", Ok(""))
        .answering(
            HISTORY,
            Ok(r#"[{"revision": 1, "status": "failed", "chart": "meridian-runtime-0.1.300"}]"#),
        );
    assert!(check(&alone, &asked(None))
        .await
        .unwrap_err()
        .contains("no earlier revision that was deployed"));
    let none = Stand::default()
        .answering("helm version --short", Ok("v3.16.2"))
        .answering("kubectl cluster-info", Ok(""))
        .answering(HISTORY, Err("Error: release: not found"));
    let refused = check(&none, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains("`meridian status -n meridian`"),
        "{refused}"
    );
    for stand in [stand, alone, none] {
        assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));
    }
}

#[tokio::test]
async fn it_rolls_back_and_waits_for_the_components() {
    let stand = stand()
        .answering(
            "helm rollback meridian 12 --namespace meridian --timeout 10m",
            Ok("Rollback was a success! Happy Helming!"),
        )
        .answering(
            "helm history meridian --namespace meridian --max 1 -o json",
            Ok(r#"[{"revision": 14, "status": "deployed", "chart": "meridian-runtime-0.1.307"}]"#),
        );
    let plan = check(&stand, &asked(None)).await.unwrap();
    let pace = Pace {
        poll: Duration::from_millis(1),
        timeout: Duration::from_secs(5),
    };
    let mut said = Vec::new();
    let done = apply(&stand, &asked(None), &plan, &pace, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await
    .unwrap();
    assert!(
        done.contains("meridian is at revision 14: revision 12's chart, meridian-runtime-0.1.307"),
        "{done}"
    );
    assert!(said[0].contains("as revision 14"));
    assert!(stand
        .asked()
        .contains(&"helm rollback meridian 12 --namespace meridian --timeout 10m".to_string()));

    // The older revision's migration failing is said, with what to read.
    let failing = stand.answering(
        "kubectl get jobs -n meridian -l app.kubernetes.io/instance=meridian -o json",
        Ok(r#"{"items": [{"metadata": {"name": "meridian-migrate-12"},
                "status": {"conditions": [{"type": "Failed", "status": "True"}]}}]}"#),
    );
    let refused = apply(&failing, &asked(None), &plan, &pace, &mut |_: &str| {})
        .await
        .unwrap_err();
    assert!(
        refused.contains("meridian logs migrate -n meridian"),
        "{refused}"
    );
}

use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use crate::doctor::Answered;

/// helm and kubectl answering from a script, every command kept.
#[derive(Default)]
struct Stand {
    answers: HashMap<String, Result<String, String>>,
    /// What a command answers once it has been asked that many times.
    later: HashMap<String, (usize, Result<String, String>)>,
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

    /// `answer` from the asking after the first `times`: what changed in
    /// the cluster meanwhile.
    fn answering_after(mut self, times: usize, command: &str, answer: Result<&str, &str>) -> Self {
        self.later.insert(
            command.to_string(),
            (times, answer.map(String::from).map_err(String::from)),
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
        let before = {
            let mut asked = self.asked.lock().unwrap();
            let before = asked.iter().filter(|c| **c == command).count();
            asked.push(command.clone());
            before
        };
        let later = self
            .later
            .get(&command)
            .filter(|(times, _)| before >= *times)
            .map(|(_, answer)| answer);
        match later.or(self.answers.get(&command)) {
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
        // A revision whose manifest states no launched plugin's credential.
        .answering(
            "helm get manifest meridian --revision 12 --namespace meridian",
            Ok(""),
        )
        .answering(
            "helm get manifest meridian --revision 11 --namespace meridian",
            Ok(""),
        )
        // The newest revision's: what Helm merges a rollback against.
        .answering(MANIFEST_13, Ok(""))
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

const LIVE: &str =
    "kubectl get deployments -n meridian -l app.kubernetes.io/instance=meridian -o json";
const MANIFEST_12: &str = "helm get manifest meridian --revision 12 --namespace meridian";
const MANIFEST_13: &str = "helm get manifest meridian --revision 13 --namespace meridian";
const VALUES_12: &str = "helm get values meridian --revision 12 --namespace meridian --all -o json";

/// A store's group-0 writer Deployment, as `kubectl get -o json` lists it.
fn deployment(component: &str, env: &[(&str, &str)]) -> String {
    let env: Vec<String> = env
        .iter()
        .map(|(name, value)| format!(r#"{{"name": "{name}", "value": "{value}"}}"#))
        .collect();
    format!(
        r#"{{"kind": "Deployment", "metadata": {{"name": "meridian-{component}",
            "labels": {{"meridian.dev/component": "{component}"}}}},
            "spec": {{"template": {{"spec": {{"containers": [{{"name": "{component}",
            "env": [{}]}}]}}}}}}}}"#,
        env.join(", ")
    )
}

fn live(deployments: &[String]) -> String {
    format!(r#"{{"items": [{}]}}"#, deployments.join(", "))
}

/// The same Deployment as `helm get manifest` prints it.
fn manifested(component: &str, env: &[(&str, &str)]) -> String {
    let mut out = format!(
        "---\n# Source: meridian-runtime/templates/{component}.yaml\napiVersion: apps/v1\n\
         kind: Deployment\nmetadata:\n  name: meridian-{component}\n  labels:\n    \
         meridian.dev/component: {component}\nspec:\n  template:\n    spec:\n      \
         containers:\n        - name: {component}\n          env:\n            \
         - name: MERIDIAN_INSTANCE_ID\n              value: meridian-{component}\n"
    );
    for (name, value) in env {
        out.push_str(&format!(
            "            - name: {name}\n              value: \"{value}\"\n"
        ));
    }
    out
}

fn with_groups(live_lake: &[(&str, &str)], live_bor: &[(&str, &str)], target: &str) -> Stand {
    stand()
        .answering(
            LIVE,
            Ok(&live(&[
                deployment("lake", live_lake),
                deployment("bor", live_bor),
            ])),
        )
        .answering(MANIFEST_12, Ok(target))
}

#[tokio::test]
async fn a_rollback_that_lowers_writer_groups_in_ha_is_refused_before_helm_rollback() {
    // Raised 2 -> 3 in ha; revision 12 runs 2.
    let target = format!(
        "{}{}",
        manifested("lake", &[("MERIDIAN_WRITER_GROUPS", "2")]),
        manifested("bor", &[("MERIDIAN_WRITER_GROUPS", "2")])
    );
    let three = [("MERIDIAN_WRITER_GROUPS", "3")];
    let stand = with_groups(&three, &three, &target)
        .answering(VALUES_12, Ok(r#"{"resilience": {"profile": "ha"}}"#));
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains(
            "lake.writerGroups is 2 at revision 12, but this deployment's lake runs 3 writer \
             groups. In the ha profile, this release can raise the lake's writer groups but \
             can't lower them yet"
        ),
        "{refused}"
    );
    assert!(
        refused.contains("Keep lake.writerGroups at 3 or more"),
        "{refused}"
    );
    assert!(
        refused.contains(
            "bor.writerGroups is 2 at revision 12, but this deployment's book runs 3 writer \
             groups. In the ha profile, this release can raise the book's writer groups but \
             can't lower them yet. Keep bor.writerGroups at 3 or more"
        ),
        "{refused}"
    );
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));
}

#[tokio::test]
async fn a_stream_or_a_lease_that_runs_refuses_a_lowering_whatever_the_revision_says() {
    let target = format!(
        "{}{}",
        manifested("lake", &[("MERIDIAN_WRITER_GROUPS", "2")]),
        manifested("bor", &[("MERIDIAN_WRITER_GROUPS", "2")])
    );
    // The running lake keeps its writes in a stream; revision 12 is
    // tolerant. Its values are never needed to refuse.
    let stand = with_groups(
        &[
            ("MERIDIAN_WRITER_GROUPS", "3"),
            ("MERIDIAN_LAKE_WRITE_AHEAD", "meridian-lake"),
        ],
        &[],
        &target,
    );
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.starts_with("lake.writerGroups is 2 at revision 12"),
        "{refused}"
    );
    assert!(!refused.contains("bor.writerGroups"), "{refused}");
    // Revision 12 keeps it in a stream, and the running lake does not.
    let streamed = format!(
        "{}{}",
        manifested(
            "lake",
            &[
                ("MERIDIAN_WRITER_GROUPS", "2"),
                ("MERIDIAN_LAKE_WRITE_AHEAD", "meridian-lake")
            ]
        ),
        manifested("bor", &[])
    );
    let stand = with_groups(&[("MERIDIAN_WRITER_GROUPS", "3")], &[], &streamed);
    assert!(check(&stand, &asked(None))
        .await
        .unwrap_err()
        .starts_with("lake.writerGroups is 2 at revision 12"));
    // The running book keeps a lease (a standby beside its writer).
    let stand = with_groups(
        &[],
        &[
            ("MERIDIAN_WRITER_GROUPS", "3"),
            ("MERIDIAN_LEASE_NAME", "meridian-bor"),
        ],
        &target,
    );
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.starts_with(
            "bor.writerGroups is 2 at revision 12, but this deployment's book runs 3 writer groups"
        ),
        "{refused}"
    );
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));
    assert!(!stand.asked().iter().any(|c| c == VALUES_12));
}

#[tokio::test]
async fn a_raise_an_unchanged_count_and_tolerant_roll_back_as_before() {
    let two = format!(
        "{}{}",
        manifested("lake", &[("MERIDIAN_WRITER_GROUPS", "2")]),
        manifested("bor", &[("MERIDIAN_WRITER_GROUPS", "2")])
    );
    // Lowered from 3 in tolerant: no stream, no lease, revision 12 tolerant.
    let three = [("MERIDIAN_WRITER_GROUPS", "3")];
    let tolerant = with_groups(&three, &three, &two)
        .answering(VALUES_12, Ok(r#"{"resilience": {"profile": "tolerant"}}"#));
    assert_eq!(
        check(&tolerant, &asked(None)).await.unwrap().to.revision,
        12
    );
    // No profile said is the chart's default, tolerant.
    let unsaid = with_groups(&three, &three, &two).answering(VALUES_12, Ok("{}"));
    assert!(check(&unsaid, &asked(None)).await.is_ok());
    // Unchanged, and raised (2 run now, revision 12 runs 3): the values are
    // never read.
    let two_groups = [("MERIDIAN_WRITER_GROUPS", "2")];
    let unchanged = with_groups(&two_groups, &two_groups, &two);
    assert!(check(&unchanged, &asked(None)).await.is_ok());
    assert!(!unchanged.asked().iter().any(|c| c == VALUES_12));
    let raised = with_groups(
        &two_groups,
        &two_groups,
        &format!(
            "{}{}",
            manifested("lake", &three),
            manifested("bor", &three)
        ),
    );
    assert!(check(&raised, &asked(None)).await.is_ok());
    assert!(!raised.asked().iter().any(|c| c == VALUES_12));
    // One group runs: nothing can be lowered, and the revision is read once,
    // for the launched plugins' credentials alone.
    let one = stand().answering(
        LIVE,
        Ok(&live(&[deployment("lake", &[]), deployment("bor", &[])])),
    );
    assert!(check(&one, &asked(None)).await.is_ok());
    assert_eq!(one.asked().iter().filter(|c| *c == MANIFEST_12).count(), 1);
    assert!(!one.asked().iter().any(|c| c == VALUES_12));
}

#[tokio::test]
async fn a_revision_that_cannot_be_read_is_refused_not_passed() {
    let three = [("MERIDIAN_WRITER_GROUPS", "3")];
    let stand = stand()
        .answering(LIVE, Ok(&live(&[deployment("lake", &three)])))
        .answering(MANIFEST_12, Err("Error: release: not found"));
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains(
            "Can't read revision 12's manifest (Error: release: not found), so can't tell \
             whether returning to it would lower a store's writer groups"
        ),
        "{refused}"
    );
    let stand = stand
        .answering(MANIFEST_12, Ok(&manifested("lake", &[])))
        .answering(VALUES_12, Err("Error: forbidden"));
    assert!(check(&stand, &asked(None))
        .await
        .unwrap_err()
        .contains("Can't read revision 12's values (Error: forbidden), so can't tell"));
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));
}

const LAUNCHED: &str =
    "kubectl get secret meridian-meridian-runtime-broker-launched -n meridian --ignore-not-found -o json";

/// Revision 12's manifest with the launched plugins' Secret as an older
/// chart rendered it: each credential as it was that day.
fn manifest_with_credentials(data: &[(&str, &str)]) -> String {
    let data: String = data
        .iter()
        .map(|(key, value)| format!("  {key}: {value}\n"))
        .collect();
    format!(
        "---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: meridian-meridian-runtime-broker\n\
         data:\n  dashboard: b25l\n\
         ---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: meridian-meridian-runtime-broker-launched\n\
         type: Opaque\ndata:\n{data}"
    )
}

fn held(data: &[(&str, &str)]) -> String {
    let data: serde_json::Map<String, Value> = data
        .iter()
        .map(|(key, value)| (key.to_string(), Value::String(value.to_string())))
        .collect();
    serde_json::json!({"kind": "Secret", "data": data}).to_string()
}

#[tokio::test]
async fn a_revision_holding_a_credential_made_again_since_is_refused_before_helm_rollback() {
    // tiingo and tradier were stopped and launched again since revision 12;
    // alpaca's credential is as it was; kalshi was launched after revision
    // 13 was made, so no revision states its credential and Helm leaves it.
    let stand = stand()
        .answering(
            MANIFEST_12,
            Ok(&manifest_with_credentials(&[
                ("alpaca.password", "YWxwYWNhLXRoZW4="),
                ("alpaca.url", "bmF0czovL2FscGFjYS10aGVu"),
                ("tiingo.password", "dGlpbmdvLXRoZW4="),
                ("tiingo.url", "bmF0czovL3RpaW5nby10aGVu"),
                ("tradier.password", "dHJhZGllci10aGVu"),
                ("tradier.url", "bmF0czovL3RyYWRpZXItdGhlbg=="),
            ])),
        )
        .answering(
            LAUNCHED,
            Ok(&held(&[
                ("alpaca.password", "YWxwYWNhLXRoZW4="),
                ("alpaca.url", "bmF0czovL2FscGFjYS10aGVu"),
                ("kalshi.password", "a2Fsc2hpLW5vdw=="),
                ("kalshi.url", "bmF0czovL2thbHNoaS1ub3c="),
                ("tiingo.password", "dGlpbmdvLW5vdw=="),
                ("tiingo.url", "bmF0czovL3RpaW5nby1ub3c="),
                ("tradier.password", "dHJhZGllci1ub3c="),
                ("tradier.url", "bmF0czovL3RyYWRpZXItbm93"),
            ])),
        );
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains("some have been made again since: tiingo, tradier."),
        "{refused}"
    );
    assert!(
        refused.ends_with("  meridian plugin stop tiingo\n  meridian plugin stop tradier"),
        "{refused}"
    );
    // The instances, never a credential, then or now.
    for value in ["dGlpbmdv", "dHJhZGllci", "bmF0czovL", "alpaca", "kalshi"] {
        assert!(!refused.contains(value), "{value} in: {refused}");
    }
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));
}

#[tokio::test]
async fn a_credential_as_it_was_or_of_a_plugin_stopped_since_rolls_back_as_before() {
    let then = manifest_with_credentials(&[
        ("alpaca.password", "YWxwYWNhLXRoZW4="),
        ("alpaca.url", "bmF0czovL2FscGFjYS10aGVu"),
        ("tiingo.password", "dGlpbmdvLXRoZW4="),
        ("tiingo.url", "bmF0czovL3RpaW5nby10aGVu"),
    ]);
    // alpaca's as it was, and tiingo stopped: nothing of a running plugin's
    // would change.
    let stand = stand().answering(MANIFEST_12, Ok(&then)).answering(
        LAUNCHED,
        Ok(&held(&[
            ("alpaca.password", "YWxwYWNhLXRoZW4="),
            ("alpaca.url", "bmF0czovL2FscGFjYS10aGVu"),
        ])),
    );
    assert_eq!(check(&stand, &asked(None)).await.unwrap().to.revision, 12);
    // The Secret gone altogether, as `--ignore-not-found` answers: nothing.
    let stand = stand.answering(LAUNCHED, Ok(""));
    assert_eq!(check(&stand, &asked(None)).await.unwrap().to.revision, 12);
    // A revision whose manifest states no key asks nothing of the cluster.
    let stand = self::stand().answering(MANIFEST_12, Ok(&manifest_with_credentials(&[])));
    assert_eq!(check(&stand, &asked(None)).await.unwrap().to.revision, 12);
    assert!(!stand
        .asked()
        .iter()
        .any(|c| c.starts_with("kubectl get secret")));
    // And one that cannot be read is refused, not passed.
    let stand = self::stand()
        .answering(MANIFEST_12, Ok(&then))
        .answering(LAUNCHED, Err("Error: forbidden"));
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains(
            "Can't read which launched plugins hold a credential now (Error: forbidden), so \
             can't tell whether returning to revision 12 would put an old one back"
        ),
        "{refused}"
    );
    // The newest revision's manifest is read too, and refused the same way.
    let stand = self::stand()
        .answering(MANIFEST_12, Ok(&then))
        .answering(MANIFEST_13, Err("Error: release: not found"));
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains(
            "Can't read revision 13's manifest (Error: release: not found), so can't tell \
             whether returning to revision 12 would put back a launched plugin's old credential"
        ),
        "{refused}"
    );
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));
}

#[tokio::test]
async fn a_plugin_launched_since_the_revision_is_refused_before_helm_rollback() {
    // kalshi was launched between revision 12 and revision 13: revision 13's
    // manifest states its credential, revision 12's states none, and the
    // Secret holds it. Helm takes out what the newest manifest states and
    // the one it returns to does not. alpaca's is in both, as it was.
    let alpaca = [
        ("alpaca.password", "YWxwYWNhLXRoZW4="),
        ("alpaca.url", "bmF0czovL2FscGFjYS10aGVu"),
    ];
    let with_kalshi = [
        alpaca[0],
        alpaca[1],
        ("kalshi.password", "a2Fsc2hpLW5vdw=="),
        ("kalshi.url", "bmF0czovL2thbHNoaS1ub3c="),
    ];
    let stand = stand()
        .answering(MANIFEST_12, Ok(&manifest_with_credentials(&alpaca)))
        .answering(MANIFEST_13, Ok(&manifest_with_credentials(&with_kalshi)))
        .answering(LAUNCHED, Ok(&held(&with_kalshi)));
    let refused = check(&stand, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains(
            "revision 12 keeps no credential for what was launched after it was made, and \
             revision 13 does: kalshi. Returning to revision 12 would take the credential away \
             under a plugin that runs on it. Stop it first, roll back, and launch it again"
        ),
        "{refused}"
    );
    assert!(
        refused.ends_with("  meridian plugin stop kalshi"),
        "{refused}"
    );
    for value in ["a2Fsc2hp", "bmF0czovL", "alpaca", "made again"] {
        assert!(!refused.contains(value), "{value} in: {refused}");
    }
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));

    // Stopped since: the Secret holds no credential of kalshi's to lose.
    let stopped = self::stand()
        .answering(MANIFEST_12, Ok(&manifest_with_credentials(&alpaca)))
        .answering(MANIFEST_13, Ok(&manifest_with_credentials(&with_kalshi)))
        .answering(LAUNCHED, Ok(&held(&alpaca)));
    assert_eq!(check(&stopped, &asked(None)).await.unwrap().to.revision, 12);

    // Revision 12 states no key at all (its chart stated none, or nothing
    // was launched then): the newest revision's alone is enough to refuse.
    let none_then = self::stand()
        .answering(MANIFEST_13, Ok(&manifest_with_credentials(&with_kalshi)))
        .answering(LAUNCHED, Ok(&held(&with_kalshi)));
    let refused = check(&none_then, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains("revision 13 does: alpaca, kalshi."),
        "{refused}"
    );
    assert!(
        refused.ends_with("  meridian plugin stop alpaca\n  meridian plugin stop kalshi"),
        "{refused}"
    );

    // Both at once, each said, every instance to stop named once.
    let both = self::stand()
        .answering(MANIFEST_12, Ok(&manifest_with_credentials(&alpaca)))
        .answering(MANIFEST_13, Ok(&manifest_with_credentials(&with_kalshi)))
        .answering(
            LAUNCHED,
            Ok(&held(&[
                ("alpaca.password", "YWxwYWNhLW5vdw=="),
                ("alpaca.url", "bmF0czovL2FscGFjYS1ub3c="),
                with_kalshi[2],
                with_kalshi[3],
            ])),
        );
    let refused = check(&both, &asked(None)).await.unwrap_err();
    assert!(
        refused.contains("one has been made again since: alpaca."),
        "{refused}"
    );
    assert!(refused.contains("revision 13 does: kalshi."), "{refused}");
    assert!(
        refused.ends_with(
            "Stop them first, roll back, and launch them again (a stopped plugin has no \
             credential to lose):\n  meridian plugin stop alpaca\n  meridian plugin stop kalshi"
        ),
        "{refused}"
    );
}

#[tokio::test]
async fn a_credential_that_changes_after_the_checks_is_refused_before_helm_rollback() {
    let then = [
        ("tiingo.password", "dGlpbmdvLXRoZW4="),
        ("tiingo.url", "bmF0czovL3RpaW5nby10aGVu"),
    ];
    // As it was at the checks; made again while the plan waited for its
    // approval (the Secret is asked for once by the checks).
    let stand = stand()
        .answering(MANIFEST_12, Ok(&manifest_with_credentials(&then)))
        .answering(LAUNCHED, Ok(&held(&then)))
        .answering_after(
            1,
            LAUNCHED,
            Ok(&held(&[
                ("tiingo.password", "dGlpbmdvLW5vdw=="),
                ("tiingo.url", "bmF0czovL3RpaW5nby1ub3c="),
            ])),
        )
        .answering(
            "helm rollback meridian 12 --namespace meridian --timeout 10m",
            Ok("Rollback was a success! Happy Helming!"),
        );
    let plan = check(&stand, &asked(None)).await.unwrap();
    let pace = Pace {
        poll: Duration::from_millis(1),
        timeout: Duration::from_secs(5),
    };
    let refused = apply(&stand, &asked(None), &plan, &pace, &mut |_: &str| {})
        .await
        .unwrap_err();
    assert!(
        refused.starts_with("this changed after the first check, and nothing was rolled back:"),
        "{refused}"
    );
    assert!(
        refused.contains("one has been made again since: tiingo."),
        "{refused}"
    );
    assert!(!stand.asked().iter().any(|c| c.starts_with("helm rollback")));

    // Unchanged since the checks: it rolls back.
    let stand = self::stand()
        .answering(MANIFEST_12, Ok(&manifest_with_credentials(&then)))
        .answering(LAUNCHED, Ok(&held(&then)))
        .answering(
            "helm rollback meridian 12 --namespace meridian --timeout 10m",
            Ok("Rollback was a success! Happy Helming!"),
        );
    let plan = check(&stand, &asked(None)).await.unwrap();
    assert!(apply(&stand, &asked(None), &plan, &pace, &mut |_: &str| {})
        .await
        .is_ok());
    assert_eq!(stand.asked().iter().filter(|c| *c == LAUNCHED).count(), 2);
}

use std::collections::HashMap;
use std::sync::Mutex;

use super::*;

/// A machine that answers from a script. What it is not asked is as much of
/// the test as what it is: a check that shells out to something unexpected
/// fails here rather than on somebody's laptop.
/// One fetch, as it was made.
type Asked = (String, Vec<(String, String)>);

#[derive(Default)]
struct Fake {
    commands: HashMap<String, Result<String, Failure>>,
    fetches: HashMap<String, Result<Answered, String>>,
    now_s: u64,
    /// What was sent with each fetch, so a test can pin how a registry is
    /// asked rather than only what it answered.
    sent: Mutex<Vec<Asked>>,
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

    /// A program this machine does not have, which is a different fix from a
    /// program that ran and failed.
    fn without(mut self, command: &str, program: &str) -> Self {
        self.commands
            .insert(command.to_string(), Err(Failure::Missing(program.into())));
        self
    }

    fn fetching(mut self, url: &str, answered: Answered) -> Self {
        self.fetches.insert(url.to_string(), Ok(answered));
        self
    }

    fn at(mut self, now_s: u64) -> Self {
        self.now_s = now_s;
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

    async fn fetch(&self, url: &str, headers: &[(&str, &str)]) -> Result<Answered, String> {
        self.sent.lock().expect("a test's own lock").push((
            url.to_string(),
            headers
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        ));
        self.fetches
            .get(url)
            .cloned()
            .unwrap_or_else(|| Err(format!("nothing scripted for {url}")))
    }

    fn now_s(&self) -> u64 {
        self.now_s
    }
}

fn answered(status: u16, body: &str) -> Answered {
    Answered {
        status,
        body: body.to_string(),
        date_s: None,
    }
}

const NOW: u64 = 1_790_000_000;

#[tokio::test]
async fn an_old_helm_is_stopped_with_the_version_to_install() {
    let machine = Fake::default().running("helm version --short", Ok("v3.9.4+g1234"));

    let finding = checks::helm(&machine).await;

    assert!(finding.stops(), "{finding:?}");
    assert!(format!("{finding}").contains("3.12"), "{finding}");
}

#[tokio::test]
async fn a_recent_helm_passes() {
    let machine = Fake::default().running("helm version --short", Ok("v3.16.2+gabc"));
    assert!(!checks::helm(&machine).await.stops());
}

#[tokio::test]
async fn no_helm_at_all_says_what_to_install() {
    let machine = Fake::default().without("helm version --short", "helm");

    let finding = checks::helm(&machine).await;

    assert!(finding.stops());
    assert!(format!("{finding}").contains("Install Helm 3"), "{finding}");
}

#[tokio::test]
async fn rights_are_asked_of_the_cluster_rather_than_assumed() {
    let machine = Fake::default()
        .running(
            "kubectl cluster-info",
            Ok("Kubernetes control plane is running"),
        )
        .running(
            "kubectl auth can-i create deployments -n meridian",
            Ok("yes"),
        )
        .running("kubectl auth can-i create secrets -n meridian", Ok("no"))
        .running("kubectl auth can-i create jobs -n meridian", Ok("yes"));

    let findings = checks::cluster(&machine, "meridian").await;

    let stopping: Vec<String> = findings
        .iter()
        .filter(|finding| finding.stops())
        .map(|finding| format!("{finding}"))
        .collect();
    assert_eq!(stopping.len(), 1, "{findings:?}");
    assert!(stopping[0].contains("secrets"), "{stopping:?}");
}

#[tokio::test]
async fn no_cluster_is_one_finding_and_not_four() {
    // A person with no cluster does not need to be told three more times.
    let machine = Fake::default().running("kubectl cluster-info", Err("connection refused"));

    let findings = checks::cluster(&machine, "meridian").await;

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].stops());
    assert!(
        format!("{}", findings[0]).contains("KUBECONFIG"),
        "{findings:?}"
    );
}

#[tokio::test]
async fn a_missing_kubectl_is_not_told_to_check_its_configuration() {
    // The fix for a machine with no kubectl is to install kubectl, and this
    // said "point KUBECONFIG at the cluster" for an afternoon.
    let machine = Fake::default().without("kubectl cluster-info", "kubectl");

    let findings = checks::cluster(&machine, "meridian").await;

    let said = format!("{}", findings[0]);
    assert!(said.contains("Install kubectl"), "{said}");
    assert!(!said.contains("KUBECONFIG"), "{said}");
}

#[tokio::test]
async fn a_cluster_with_nowhere_to_put_the_key_is_stopped() {
    let machine = Fake::default().running("kubectl get storageclass -o name", Ok("\n"));

    let finding = checks::storage_class(&machine).await;

    assert!(finding.stops());
    assert!(format!("{finding}").contains("key"), "{finding}");
}

#[tokio::test]
async fn an_image_that_does_not_exist_is_stopped_before_the_install() {
    let machine = Fake::default()
        .fetching(
            "https://ghcr.io/token?scope=repository:open-meridian/meridian-runtime:pull&service=ghcr.io",
            answered(200, r#"{"token": "t"}"#),
        )
        .fetching(
            "https://ghcr.io/v2/open-meridian/meridian-runtime/manifests/nope",
            answered(404, ""),
        );

    let finding = checks::image(&machine, "ghcr.io/open-meridian/meridian-runtime:nope").await;

    assert!(finding.stops(), "{finding:?}");
}

#[tokio::test]
async fn the_registrys_token_is_presented_the_way_it_asks_for_it() {
    // As a query parameter it is answered 401, which reads as "you may not
    // pull this" and means "you did not ask properly".
    let machine = Fake::default()
        .fetching(
            "https://ghcr.io/token?scope=repository:open-meridian/meridian-runtime:pull&service=ghcr.io",
            answered(200, r#"{"token": "t"}"#),
        )
        .fetching(
            "https://ghcr.io/v2/open-meridian/meridian-runtime/manifests/1",
            answered(200, ""),
        );

    let finding = checks::image(&machine, "ghcr.io/open-meridian/meridian-runtime:1").await;

    assert!(!finding.stops(), "{finding:?}");
    let sent = machine.sent.lock().unwrap();
    let (url, headers) = sent.last().expect("the manifest was asked for");
    assert!(!url.contains("token="), "{url}");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "Bearer t"),
        "{headers:?}"
    );
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "Accept" && value.contains("index.v1+json")),
        "a registry may refuse the only manifest it has: {headers:?}"
    );
}

#[tokio::test]
async fn an_image_with_no_tag_is_refused() {
    let finding = checks::image(&Fake::default(), "ghcr.io/open-meridian/meridian-runtime").await;
    assert!(finding.stops());
    assert!(format!("{finding}").contains("tag"), "{finding}");
}

#[tokio::test]
async fn a_registry_this_cannot_ask_is_unknown_rather_than_fine() {
    // Saying "ok" about something it did not check is the failure that makes
    // a check worse than none.
    let finding = checks::image(&Fake::default(), "registry.firm.internal/meridian:1").await;
    assert!(matches!(finding, Finding::Unknown { .. }), "{finding:?}");
}

#[tokio::test]
async fn a_skewed_clock_is_stopped_and_says_what_it_would_look_like() {
    let machine = Fake::default().at(NOW).fetching(
        "https://open-meridian.com",
        Answered {
            status: 200,
            body: String::new(),
            date_s: Some(NOW + 300),
        },
    );

    let findings = checks::platform(&machine, "https://open-meridian.com").await;

    let stopping = findings
        .iter()
        .find(|finding| finding.stops())
        .expect("stopped");
    let said = format!("{stopping}");
    assert!(said.contains("expired"), "{said}");
    assert!(said.contains("time synchronisation"), "{said}");
}

#[tokio::test]
async fn a_clock_with_little_room_left_is_worth_saying() {
    let machine = Fake::default().at(NOW).fetching(
        "https://open-meridian.com",
        Answered {
            status: 200,
            body: String::new(),
            date_s: Some(NOW + 45),
        },
    );

    let findings = checks::platform(&machine, "https://open-meridian.com").await;

    assert!(!findings.iter().any(|finding| finding.stops()));
    assert!(
        findings
            .iter()
            .any(|finding| matches!(finding, Finding::Worth { .. })),
        "{findings:?}"
    );
}

#[tokio::test]
async fn an_unreachable_platform_stops_the_install() {
    let machine = Fake::default().at(NOW);
    let findings = checks::platform(&machine, "https://open-meridian.com").await;
    assert!(findings[0].stops());
}

#[test]
fn the_verdict_exits_non_zero_only_when_something_stops_it() {
    let (said, code) = verdict(&[
        Finding::Fine("a cluster is reachable".into()),
        Finding::Unknown {
            what: "an image".into(),
            why: "not asked".into(),
        },
    ]);
    assert_eq!(code, 0, "{said}");

    let (said, code) = verdict(&[Finding::Stops {
        what: "no helm".into(),
        fix: "install it".into(),
    }]);
    assert_eq!(code, 1);
    assert!(said.contains("1 thing(s) would stop an install"), "{said}");
}

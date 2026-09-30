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
async fn a_refusal_kubectl_exits_with_is_a_stop_naming_the_right_not_unknown() {
    // kubectl auth can-i answers "no" on stdout and exits 1, so the refusal
    // arrives as a failed command. It is an answer, and the answer is no.
    let machine = Fake::default()
        .running(
            "kubectl cluster-info",
            Ok("Kubernetes control plane is running"),
        )
        .running(
            "kubectl auth can-i create deployments -n meridian",
            Ok("yes"),
        )
        .running("kubectl auth can-i create secrets -n meridian", Ok("yes"))
        .running("kubectl auth can-i create jobs -n meridian", Err("no"));

    let findings = checks::cluster(&machine, "meridian").await;

    let stopping: Vec<String> = findings
        .iter()
        .filter(|finding| finding.stops())
        .map(|finding| format!("{finding}"))
        .collect();
    assert_eq!(stopping.len(), 1, "{findings:?}");
    assert!(
        stopping[0].contains("may not create jobs in meridian"),
        "{stopping:?}"
    );
    assert!(
        stopping[0].contains("the right to create jobs"),
        "{stopping:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|finding| matches!(finding, Finding::Unknown { .. })),
        "{findings:?}"
    );
}

#[tokio::test]
async fn a_can_i_that_could_not_ask_is_unknown_rather_than_a_refusal() {
    let machine = Fake::default()
        .running(
            "kubectl cluster-info",
            Ok("Kubernetes control plane is running"),
        )
        .running(
            "kubectl auth can-i create deployments -n meridian",
            Err("error: the server doesn't have a resource type"),
        )
        .running("kubectl auth can-i create secrets -n meridian", Ok("yes"))
        .running("kubectl auth can-i create jobs -n meridian", Ok("yes"));

    let findings = checks::cluster(&machine, "meridian").await;

    assert!(!findings.iter().any(Finding::stops), "{findings:?}");
    assert!(
        findings
            .iter()
            .any(|finding| matches!(finding, Finding::Unknown { .. })),
        "{findings:?}"
    );
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

// ── The nodes' disk ─────────────────────────────────────────────────────────

const NODES: &str = "kubectl get nodes -o json";
const SUMMARY: &str = "kubectl get --raw /api/v1/nodes/lima-rancher-desktop/proxy/stats/summary";
const GIB: u64 = 1 << 30;

/// A node as `kubectl get nodes -o json` lists it, with the parts the check
/// reads: its conditions and its taints.
fn node(name: &str, pressure: bool, tainted: bool) -> serde_json::Value {
    let taints = if tainted {
        serde_json::json!([{ "key": "node.kubernetes.io/disk-pressure", "effect": "NoSchedule" }])
    } else {
        serde_json::Value::Null
    };
    serde_json::json!({
        "metadata": { "name": name },
        "spec": { "taints": taints },
        "status": {
            "capacity": { "ephemeral-storage": "102625208Ki" },
            "allocatable": { "ephemeral-storage": "99833802265" },
            "conditions": [
                { "type": "MemoryPressure", "status": "False" },
                { "type": "DiskPressure", "status": if pressure { "True" } else { "False" } },
                { "type": "Ready", "status": "True" }
            ]
        }
    })
}

fn listing(nodes: Vec<serde_json::Value>) -> String {
    serde_json::json!({ "kind": "List", "items": nodes }).to_string()
}

/// The kubelet's summary, as far as the check reads it: the node's disk and
/// its image disk, in bytes. The pods it also lists are left out.
fn summary(free: u64, capacity: u64, image_free: u64, image_capacity: u64) -> String {
    serde_json::json!({
        "node": {
            "nodeName": "lima-rancher-desktop",
            "fs": { "availableBytes": free, "capacityBytes": capacity, "usedBytes": capacity - free },
            "runtime": { "imageFs": { "availableBytes": image_free, "capacityBytes": image_capacity } }
        },
        "pods": []
    })
    .to_string()
}

/// One disk shared by the node and its images, as on a local VM.
fn one_disk(free: u64, capacity: u64) -> String {
    summary(free, capacity, free, capacity)
}

fn one_node(pressure: bool, tainted: bool) -> Fake {
    Fake::default().running(
        NODES,
        Ok(&listing(vec![node(
            "lima-rancher-desktop",
            pressure,
            tainted,
        )])),
    )
}

#[tokio::test]
async fn a_node_under_disk_pressure_stops_and_says_what_it_does_and_what_to_free() {
    // 2026-09-30: the VM was 86% full, the kubelet set DiskPressure, and every
    // pod of the deployment was evicted with nothing said beforehand.
    let machine = one_node(true, true);

    let findings = checks::disk(&machine).await;

    let stopping: Vec<String> = findings
        .iter()
        .filter(|finding| finding.stops())
        .map(|finding| format!("{finding}"))
        .collect();
    assert_eq!(stopping.len(), 1, "{findings:?}");
    let said = &stopping[0];
    assert!(
        said.contains("node lima-rancher-desktop is under disk pressure"),
        "{said}"
    );
    assert!(said.contains("DiskPressure=True"), "{said}");
    assert!(said.contains("evicts"), "{said}");
    assert!(said.contains("schedules none"), "{said}");
    assert!(said.contains("build cache"), "{said}");
    assert!(said.contains("taint to lift"), "{said}");
    // Its free disk is not asked for: the stop says all there is to say.
    assert!(
        !findings
            .iter()
            .any(|finding| matches!(finding, Finding::Unknown { .. })),
        "{findings:?}"
    );
}

#[tokio::test]
async fn the_taint_alone_is_disk_pressure_too() {
    let machine = one_node(false, true);

    let findings = checks::disk(&machine).await;

    let stopping = findings
        .iter()
        .find(|finding| finding.stops())
        .expect("stopped");
    assert!(
        format!("{stopping}").contains("tainted node.kubernetes.io/disk-pressure"),
        "{stopping}"
    );
}

#[tokio::test]
async fn low_free_disk_is_worth_saying_before_kubernetes_acts_on_it() {
    // 81% full: under the 20% line, and above the kubelet's 15% for images.
    let machine = one_node(false, false).running(SUMMARY, Ok(&one_disk(19 * GIB, 100 * GIB)));

    let findings = checks::disk(&machine).await;

    assert!(!findings.iter().any(Finding::stops), "{findings:?}");
    let worth: Vec<String> = findings
        .iter()
        .filter(|finding| matches!(finding, Finding::Worth { .. }))
        .map(|finding| format!("{finding}"))
        .collect();
    // One disk shared by the node and its images is said once.
    assert_eq!(worth.len(), 1, "{findings:?}");
    assert!(
        worth[0].contains("19.0 GiB of 100.0 GiB disk free (19%)"),
        "{worth:?}"
    );
    assert!(worth[0].contains("below 20.0 GiB"), "{worth:?}");
    assert!(worth[0].contains("docker builder prune"), "{worth:?}");
}

#[tokio::test]
async fn a_small_disk_is_warned_at_ten_gibibytes_rather_than_a_fifth() {
    // 30% of 20 GiB is 6 GiB, which is less than one pull of room.
    let machine = one_node(false, false).running(SUMMARY, Ok(&one_disk(6 * GIB, 20 * GIB)));

    let findings = checks::disk(&machine).await;

    assert!(
        findings
            .iter()
            .any(|finding| matches!(finding, Finding::Worth { .. })),
        "{findings:?}"
    );
    assert_eq!(checks::disk_line(20 * GIB), 10 * GIB);
    assert_eq!(checks::disk_line(100 * GIB), 20 * GIB);
}

#[tokio::test]
async fn an_image_disk_of_its_own_is_read_on_its_own() {
    // The node's disk has room and the images' does not.
    let machine = one_node(false, false).running(
        SUMMARY,
        Ok(&summary(80 * GIB, 100 * GIB, 30 * GIB, 200 * GIB)),
    );

    let findings = checks::disk(&machine).await;

    let worth: Vec<String> = findings
        .iter()
        .filter(|finding| matches!(finding, Finding::Worth { .. }))
        .map(|finding| format!("{finding}"))
        .collect();
    assert_eq!(worth.len(), 1, "{findings:?}");
    assert!(worth[0].contains("image disk free (15%)"), "{worth:?}");
}

#[tokio::test]
async fn plenty_of_disk_is_ok() {
    let machine = one_node(false, false).running(SUMMARY, Ok(&one_disk(62 * GIB, 98 * GIB)));

    let findings = checks::disk(&machine).await;

    assert!(
        findings
            .iter()
            .all(|finding| matches!(finding, Finding::Fine(_))),
        "{findings:?}"
    );
    let said: Vec<String> = findings
        .iter()
        .map(|finding| format!("{finding}"))
        .collect();
    assert!(
        said.iter()
            .any(|line| line.contains("not under disk pressure")),
        "{said:?}"
    );
    assert!(
        said.iter()
            .any(|line| line.contains("62.0 GiB of 98.0 GiB disk free (63%)")),
        "{said:?}"
    );
}

#[tokio::test]
async fn free_disk_the_account_may_not_read_is_unknown_with_the_right_it_needs() {
    let machine = one_node(false, false).running(
        SUMMARY,
        Err("Error from server (Forbidden): nodes \"lima-rancher-desktop\" is forbidden: \
             User \"dev\" cannot get resource \"nodes/proxy\" in API group \"\" at the cluster scope"),
    );

    let findings = checks::disk(&machine).await;

    assert!(!findings.iter().any(Finding::stops), "{findings:?}");
    let unknown = findings
        .iter()
        .find(|finding| matches!(finding, Finding::Unknown { .. }))
        .expect("unknown");
    let said = format!("{unknown}");
    assert!(
        said.contains("free disk on node lima-rancher-desktop"),
        "{said}"
    );
    assert!(said.contains("may not get nodes/proxy"), "{said}");
    // What could be read still was.
    assert!(
        findings
            .iter()
            .any(|finding| format!("{finding}").contains("not under disk pressure")),
        "{findings:?}"
    );
}

#[tokio::test]
async fn nodes_the_account_may_not_list_are_one_unknown() {
    let machine = Fake::default().running(
        NODES,
        Err(
            "Error from server (Forbidden): nodes is forbidden: User \"dev\" cannot list \
             resource \"nodes\" in API group \"\" at the cluster scope",
        ),
    );

    let findings = checks::disk(&machine).await;

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(
        matches!(&findings[0], Finding::Unknown { why, .. } if why.contains("may not list nodes")),
        "{findings:?}"
    );
}

#[tokio::test]
async fn a_summary_with_no_disk_in_it_is_unknown_rather_than_fine() {
    let machine = one_node(false, false).running(SUMMARY, Ok(r#"{"node": {}}"#));

    let findings = checks::disk(&machine).await;

    assert!(
        findings
            .iter()
            .any(|finding| matches!(finding, Finding::Unknown { .. })),
        "{findings:?}"
    );
}

#[tokio::test]
async fn several_nodes_with_room_are_one_line_naming_the_least() {
    let machine = Fake::default()
        .running(
            NODES,
            Ok(&listing(vec![
                node("a", false, false),
                node("b", false, false),
            ])),
        )
        .running(
            "kubectl get --raw /api/v1/nodes/a/proxy/stats/summary",
            Ok(&one_disk(70 * GIB, 100 * GIB)),
        )
        .running(
            "kubectl get --raw /api/v1/nodes/b/proxy/stats/summary",
            Ok(&one_disk(40 * GIB, 100 * GIB)),
        );

    let findings = checks::disk(&machine).await;

    let said: Vec<String> = findings
        .iter()
        .map(|finding| format!("{finding}"))
        .collect();
    assert_eq!(findings.len(), 2, "{said:?}");
    assert!(said[0].contains("none of the 2 nodes"), "{said:?}");
    assert!(
        said[1].contains("2 nodes have room; the least is node b"),
        "{said:?}"
    );
}

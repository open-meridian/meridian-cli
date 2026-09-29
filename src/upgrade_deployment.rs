//! `meridian upgrade-deployment`: a running deployment moved to a newer chart,
//! in place (task kernel/upgrading-a-deployment-in-place, ruled 2026-09-28).
//!
//! Not `meridian upgrade`, which replaces this binary. This one changes a
//! deployment, with the person's own cluster rights: nothing in the
//! deployment holds a right to change the cluster (decisions/016), and an
//! upgrade is the widest change there is.
//!
//! The order is what this adds over typing `helm upgrade` by hand: check
//! first and change nothing if a check fails; apply the new chart's defaults
//! with the deployment's own values; wait for the migration and for every
//! component on the new image; clean up after earlier revisions; and say
//! what it did. It waits itself rather than with `helm upgrade --wait`,
//! which published charts up to 0.1.182 fail on their own hook.
//!
//! `--reset-then-reuse-values`, never `--reuse-values`: that reuses the
//! previous release's computed values, the old chart's image tag among them,
//! so pods restart under the new templates on the old image. It is what
//! happened on 2026-09-28, and nothing said so.
//!
//! It never prints the deployment's values: they hold its enrolment code.
//! The one thing read from them is `image`.

pub mod run;
pub mod watch;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::doctor::Finding;
use crate::release;

/// The oldest Helm with `--reset-then-reuse-values` (3.14.0).
pub const HELM_MINIMUM: (u32, u32) = (3, 14);

/// What an upgrade is told. Everything else is the deployment's own.
#[derive(Debug, Clone)]
pub struct Asked {
    pub release: String,
    pub namespace: String,
    pub chart: String,
    /// None is the latest published, resolved once and then named, so what
    /// was checked is what is applied.
    pub chart_version: Option<String>,
    /// As given, for Helm; and as a duration, for this command's own wait.
    pub timeout: String,
}

impl Asked {
    /// Every workload, pod and Job of this release carries this label, and
    /// nothing of another release in the namespace can.
    pub fn selector(&self) -> String {
        format!("app.kubernetes.io/instance={}", self.release)
    }
}

/// `10m`, `90s`, `1h30m`: the durations Helm takes, which is what `--timeout`
/// is given as. A bare number is refused, since Helm would refuse it too.
pub fn duration(said: &str) -> Option<Duration> {
    let mut total = 0u64;
    let mut digits = String::new();
    for c in said.trim().chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c {
            'h' => 3600,
            'm' => 60,
            's' => 1,
            _ => return None,
        };
        total += digits.parse::<u64>().ok()? * unit;
        digits.clear();
    }
    (digits.is_empty() && total > 0).then(|| Duration::from_secs(total))
}

// ── What Helm says ──────────────────────────────────────────────────────────

/// The release as `helm list -o json` states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub revision: u64,
    pub status: String,
    pub chart: String,
    pub version: String,
    pub app_version: String,
}

/// The one release named, from `helm list -o json`. Helm 3 states a revision
/// as a string and Helm 4 as a number; both are read.
pub fn installed(listed: &str, release: &str) -> Result<Option<Installed>, String> {
    let listed: serde_json::Value = serde_json::from_str(listed.trim())
        .map_err(|failed| format!("`helm list` said something that is not JSON: {failed}"))?;
    let Some(found) = listed
        .as_array()
        .into_iter()
        .flatten()
        .find(|each| each["name"] == release)
    else {
        return Ok(None);
    };
    let revision = match &found["revision"] {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(said) => said.parse().ok(),
        _ => None,
    }
    .ok_or("`helm list` stated no revision for it")?;
    let (chart, version) = chart_and_version(found["chart"].as_str().unwrap_or_default());
    Ok(Some(Installed {
        revision,
        status: found["status"].as_str().unwrap_or_default().to_string(),
        chart,
        version,
        app_version: found["app_version"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    }))
}

/// `meridian-runtime-0.1.180` as its name and its version: split before the
/// first hyphen a digit follows, since a version may hold hyphens of its own.
fn chart_and_version(said: &str) -> (String, String) {
    let at = said
        .char_indices()
        .find(|(at, c)| *c == '-' && said[at + 1..].starts_with(|next: char| next.is_ascii_digit()))
        .map(|(at, _)| at);
    match at {
        Some(at) => (said[..at].to_string(), said[at + 1..].to_string()),
        None => (said.to_string(), String::new()),
    }
}

/// A published chart, from `helm show chart`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chart {
    pub name: String,
    pub version: String,
    /// The runtime's commit, which a published chart's image tag is too.
    pub app_version: String,
}

/// What Helm prints about an OCI pull before the document itself. Helm 4 puts
/// it on stdout, where a YAML reader would take it for two more keys.
fn document(said: &str) -> String {
    said.lines()
        .filter(|line| !line.starts_with("Pulled: ") && !line.starts_with("Digest: "))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn chart_of(shown: &str) -> Option<Chart> {
    let chart: serde_json::Value = serde_yaml::from_str(&document(shown)).ok()?;
    Some(Chart {
        name: scalar(&chart["name"])?,
        version: scalar(&chart["version"])?,
        app_version: scalar(&chart["appVersion"]).unwrap_or_default(),
    })
}

/// A string, or a number YAML read as one: a short commit that happens to be
/// all digits is a tag all the same.
fn scalar(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(said) if !said.is_empty() => Some(said.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `image.repository` and `image.tag`, where a values document states them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Image {
    pub repository: Option<String>,
    pub tag: Option<String>,
}

/// The image a values document names: the chart's defaults, from `helm show
/// values`, or the deployment's own, from `helm get values -o json`. Nothing
/// else in the document is kept, because the deployment's own values hold its
/// enrolment code.
pub fn image_in(values: &str) -> Image {
    let values: serde_json::Value = serde_yaml::from_str(&document(values)).unwrap_or_default();
    Image {
        repository: scalar(&values["image"]["repository"]),
        tag: scalar(&values["image"]["tag"]),
    }
}

// ── What the cluster says ───────────────────────────────────────────────────

/// A Deployment or StatefulSet of the release, and how far its rollout is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workload {
    /// `deployment/<name>` or `statefulset/<name>`, as kubectl names it.
    pub name: String,
    pub component: String,
    pub selector: BTreeMap<String, String>,
    /// Each container's image, by container name, as the template says now.
    pub images: BTreeMap<String, String>,
    pub wanted: u64,
    pub ready: u64,
    /// Why its rollout is not finished, or None when it is.
    pub unfinished: Option<String>,
    /// A Deployment's ReplicaSet for its current template: the one whose
    /// revision is the Deployment's. None where the listing holds none.
    pub current: Option<String>,
}

impl Workload {
    /// Whether a pod is one of its own: its selector matches, and no Job runs
    /// it.
    fn owns(&self, pod: &Pod) -> bool {
        pod.job.is_none()
            && !self.selector.is_empty()
            && self
                .selector
                .iter()
                .all(|(name, value)| pod.labels.get(name) == Some(value))
    }
}

fn number(value: &serde_json::Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

fn strings(value: &serde_json::Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_string())))
        .collect()
}

fn images(containers: &serde_json::Value) -> BTreeMap<String, String> {
    containers
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|container| {
            Some((
                container["name"].as_str()?.to_string(),
                container["image"].as_str()?.to_string(),
            ))
        })
        .collect()
}

/// What Kubernetes numbers a Deployment's templates by, on the Deployment and
/// on each ReplicaSet it made.
const REVISION: &str = "deployment.kubernetes.io/revision";

/// The name of the first owner of one kind that a resource names.
fn owner<'a>(item: &'a serde_json::Value, kind: &str) -> Option<&'a str> {
    item["metadata"]["ownerReferences"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|owner| owner["kind"] == kind)
        .and_then(|owner| owner["name"].as_str())
}

/// From `kubectl get deployments,statefulsets,replicasets -o json`. The
/// ReplicaSets are read only for which is each Deployment's current one.
pub fn workloads(listed: &str) -> Vec<Workload> {
    let listed: serde_json::Value = serde_json::from_str(listed).unwrap_or_default();
    let items = listed["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let sets: Vec<(&str, &str, &str)> = items
        .iter()
        .filter(|item| item["kind"] == "ReplicaSet")
        .filter_map(|set| {
            Some((
                owner(set, "Deployment")?,
                set["metadata"]["annotations"][REVISION].as_str()?,
                set["metadata"]["name"].as_str()?,
            ))
        })
        .collect();
    items
        .iter()
        .filter(|item| item["kind"] == "Deployment" || item["kind"] == "StatefulSet")
        .map(|item| {
            let kind = item["kind"].as_str().unwrap_or_default().to_lowercase();
            let name = item["metadata"]["name"].as_str().unwrap_or_default();
            let revision = item["metadata"]["annotations"][REVISION].as_str();
            let current = sets
                .iter()
                .find(|(owner, set, _)| *owner == name && Some(*set) == revision)
                .map(|(_, _, set)| set.to_string());
            let (spec, status) = (&item["spec"], &item["status"]);
            let wanted = spec["replicas"].as_u64().unwrap_or(1);
            let ready = number(&status["readyReplicas"]);
            let updated = number(&status["updatedReplicas"]);
            let unfinished = if number(&item["metadata"]["generation"])
                > number(&status["observedGeneration"])
            {
                Some("its new template is not yet seen by its controller".to_string())
            } else if updated < wanted {
                Some(format!("{updated} of {wanted} on the new template"))
            } else if ready < wanted {
                Some(format!("{ready} of {wanted} ready"))
            } else if kind == "deployment"
                && (number(&status["replicas"]) > wanted
                    || number(&status["availableReplicas"]) < wanted)
            {
                Some("old pods are still being replaced".to_string())
            } else if kind == "statefulset"
                && spec["updateStrategy"]["type"] != "OnDelete"
                && status["currentRevision"] != status["updateRevision"]
            {
                Some("its update has not finished".to_string())
            } else {
                None
            };
            Workload {
                name: format!("{kind}/{name}"),
                component: item["metadata"]["labels"]["meridian.dev/component"]
                    .as_str()
                    .unwrap_or(name)
                    .to_string(),
                selector: strings(&spec["selector"]["matchLabels"]),
                images: images(&spec["template"]["spec"]["containers"]),
                wanted,
                ready,
                unfinished,
                current,
            }
        })
        .collect()
}

/// One container's state in a pod, as much of it as an upgrade reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub name: String,
    pub restarts: u64,
    /// Why it last stopped, as Kubernetes gives it: `Error`, `OOMKilled`.
    pub last_reason: Option<String>,
    pub last_exit: Option<i64>,
    /// Why it is not running now: `CrashLoopBackOff`, `ImagePullBackOff`.
    pub waiting: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pod {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    /// `Pending`, `Running`, `Succeeded`, `Failed` or `Unknown`.
    pub phase: String,
    /// Being deleted: still there, and serving until it is gone.
    pub terminating: bool,
    /// The Job that runs it, for a Job's pod.
    pub job: Option<String>,
    /// The ReplicaSet that made it, for a Deployment's pod.
    pub replica_set: Option<String>,
    pub images: BTreeMap<String, String>,
    pub init: Vec<Container>,
    pub containers: Vec<Container>,
}

fn containers(statuses: &serde_json::Value) -> Vec<Container> {
    statuses
        .as_array()
        .into_iter()
        .flatten()
        .map(|status| {
            let last = &status["lastState"]["terminated"];
            let waiting = &status["state"]["waiting"]["reason"];
            let stopped = &status["state"]["terminated"];
            Container {
                name: status["name"].as_str().unwrap_or_default().to_string(),
                restarts: number(&status["restartCount"]),
                last_reason: last["reason"].as_str().map(String::from),
                last_exit: last["exitCode"].as_i64(),
                waiting: waiting.as_str().map(String::from).or_else(|| {
                    // An init container that stopped and did not succeed.
                    match stopped["exitCode"].as_i64() {
                        Some(code) if code != 0 => Some(format!("exited {code}")),
                        _ => None,
                    }
                }),
            }
        })
        .collect()
}

/// From `kubectl get pods -o json`.
pub fn pods(listed: &str) -> Vec<Pod> {
    let listed: serde_json::Value = serde_json::from_str(listed).unwrap_or_default();
    listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| Pod {
            name: item["metadata"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            labels: strings(&item["metadata"]["labels"]),
            phase: item["status"]["phase"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            terminating: !item["metadata"]["deletionTimestamp"].is_null(),
            job: owner(item, "Job").map(String::from),
            replica_set: owner(item, "ReplicaSet").map(String::from),
            images: images(&item["spec"]["containers"]),
            init: containers(&item["status"]["initContainerStatuses"]),
            containers: containers(&item["status"]["containerStatuses"]),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub name: String,
    /// The release revision that made it, where its name ends with one, as
    /// every Job the chart runs per revision does. The key Job, run once at
    /// install and kept for its log, ends with none and is never touched.
    pub revision: Option<u64>,
    pub state: JobState,
}

/// From `kubectl get jobs -o json`.
pub fn jobs(listed: &str) -> Vec<Job> {
    let listed: serde_json::Value = serde_json::from_str(listed).unwrap_or_default();
    listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            let name = item["metadata"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let holds = |kind: &str| {
                item["status"]["conditions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|condition| condition["type"] == kind && condition["status"] == "True")
            };
            let state = if holds("Complete") {
                JobState::Complete
            } else if holds("Failed") {
                JobState::Failed
            } else {
                JobState::Running
            };
            Job {
                revision: name.rsplit_once('-').and_then(|(_, n)| n.parse().ok()),
                name,
                state,
            }
        })
        .collect()
}

// ── Deciding ────────────────────────────────────────────────────────────────

/// What `helm list` says about the release, as a finding with its recovery.
pub fn release_finding(asked: &Asked, installed: Option<&Installed>) -> Finding {
    let (release, namespace) = (&asked.release, &asked.namespace);
    let Some(installed) = installed else {
        return Finding::Stops {
            what: format!("there is no release {release} in {namespace}"),
            fix: format!(
                "`helm list -n {namespace}` lists what is there; --release and -n name another. \
                 `meridian up` installs one."
            ),
        };
    };
    let at = format!(
        "{release} is at revision {}, chart {} {}",
        installed.revision, installed.chart, installed.version
    );
    match installed.status.as_str() {
        "deployed" => Finding::Fine(format!("{at}, deployed")),
        "failed" => Finding::Stops {
            what: format!("{at}, and that revision failed"),
            fix: format!(
                "`helm history {release} -n {namespace}` says why. Fix the cause and \
                 `helm rollback {release} -n {namespace}` returns it to its last deployed \
                 revision; then upgrade again."
            ),
        },
        pending if pending.starts_with("pending") => Finding::Stops {
            what: format!("{at}, and Helm holds it as {pending}"),
            fix: format!(
                "Another Helm operation is running on it, or one was interrupted. Let a running \
                 one finish. If none is running, `helm rollback {release} -n {namespace}` \
                 returns it to its last deployed revision; then upgrade again."
            ),
        },
        other => Finding::Stops {
            what: format!("{at}, and Helm holds it as {other}"),
            fix: format!("`helm history {release} -n {namespace}` says what happened to it."),
        },
    }
}

/// Where the upgrade goes, and whether it goes at all.
pub enum Direction {
    Up,
    /// Already there: nothing to do, and that is success.
    Current,
    Down(Finding),
}

pub fn direction(installed: &Installed, target: &Chart) -> Direction {
    use std::cmp::Ordering;
    match release::compare(&target.version, &installed.version) {
        Ordering::Greater => Direction::Up,
        Ordering::Equal => Direction::Current,
        Ordering::Less => Direction::Down(Finding::Stops {
            what: format!(
                "{} {} is older than the installed {}",
                target.name, target.version, installed.version
            ),
            fix: "An upgrade never goes back: a migration already applied may not be served by \
                  an older runtime. Name a version at or after the installed one."
                .into(),
        }),
    }
}

/// How many versions one upgrade may skip, and the oldest still supported.
///
/// NOT BUILT: the skip policy's numbers and the release channels are the
/// product owner's to rule (task kernel/upgrading-a-deployment-in-place,
/// "Release channels and a skip policy"), and are not ruled yet. When they
/// are, this refuses an upgrade outside them with the version to go through
/// first. Until then it says it did not check, which is not passing.
pub fn skip_policy(installed: &Installed, target: &Chart) -> Finding {
    Finding::Unknown {
        what: format!(
            "whether {} to {} is within the skip policy",
            installed.version, target.version
        ),
        why: "the skip policy is not ruled yet, so no upgrade is refused for skipping versions"
            .into(),
    }
}

/// Whether every installed plugin's runtime floor is met by the target.
///
/// NOT BUILT: nothing in a deployment declares a plugin's runtime floor yet,
/// so there is nothing to read. When the plugin manifest carries one, this
/// reads each launched version's and refuses an upgrade that leaves one below
/// it, naming the plugin.
pub fn plugin_floors(target: &Chart) -> Finding {
    Finding::Unknown {
        what: format!("whether every plugin runs on {}", target.version),
        why: "no plugin declares a runtime floor yet, so there is none to check".into(),
    }
}

/// The image the upgraded components run: the deployment's own, where its
/// values name one, and the new chart's otherwise. A tag of its own is kept
/// by the upgrade, which is worth saying before it is applied.
pub fn target_image(chart: &Image, own: &Image) -> (String, Option<Finding>) {
    let repository = own
        .repository
        .clone()
        .or_else(|| chart.repository.clone())
        .unwrap_or_default();
    let charts = chart.tag.clone().unwrap_or_default();
    match &own.tag {
        Some(pinned) if *pinned != charts => (
            format!("{repository}:{pinned}"),
            Some(Finding::Worth {
                what: format!("this deployment's own values pin image.tag to {pinned}"),
                why: format!(
                    "and an upgrade keeps them, so it runs the new chart on {pinned} rather than \
                     the chart's {charts}. If that is not meant, remove image.tag from its values."
                ),
            }),
        ),
        _ => (format!("{repository}:{charts}"), None),
    }
}

/// The images of the runtime the components run now: those from the target's
/// repository, so the broker's and the database's own are not counted.
pub fn running_images(workloads: &[Workload], target: &str) -> Vec<String> {
    let repository = target.rsplit_once(':').map_or(target, |(repo, _)| repo);
    workloads
        .iter()
        .flat_map(|workload| workload.images.values())
        .filter(|image| image.rsplit_once(':').map(|(repo, _)| repo) == Some(repository))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// What will be done, once every check has passed.
#[derive(Debug, Clone)]
pub struct Plan {
    pub from: Installed,
    pub from_images: Vec<String>,
    pub to: Chart,
    pub to_image: String,
    /// Pods left over from a restart, which the cleanup removes.
    pub left_over: Vec<String>,
}

/// The same command, as somebody would type it.
pub fn helm_arguments(asked: &Asked, plan: &Plan) -> Vec<String> {
    [
        "upgrade",
        &asked.release,
        &asked.chart,
        "--version",
        &plan.to.version,
        "--namespace",
        &asked.namespace,
        // The new chart's defaults, then the deployment's own values over
        // them. Not --reuse-values, which keeps the old chart's image tag.
        "--reset-then-reuse-values",
        // What Helm waits for of its own accord: a hook, where a chart has
        // one. No --wait: this command waits itself, below.
        "--timeout",
        &asked.timeout,
    ]
    .iter()
    .map(|each| each.to_string())
    .collect()
}

/// `1 pod`, `7 pods`.
fn counted(count: usize, what: &str) -> String {
    match count {
        1 => format!("1 {what}"),
        _ => format!("{count} {what}s"),
    }
}

pub fn plan_text(asked: &Asked, plan: &Plan) -> String {
    let from_images = match plan.from_images.as_slice() {
        [] => "no runtime image found running".to_string(),
        images => images.join(", "),
    };
    // Asked for with the rest, so the one answer covers removing them.
    let left_over = match plan.left_over.as_slice() {
        [] => String::new(),
        pods => format!(
            "\n\x20 Then it will remove {} left over from a restart, which serve nothing:\n{}",
            counted(pods.len(), "pod"),
            pods.iter()
                .map(|pod| format!("    pod/{pod}\n"))
                .collect::<String>()
        ),
    };
    format!(
        "Upgrading {release} in {namespace}:\n\
         \x20 from  {chart} {from} (revision {revision}), running {from_images}\n\
         \x20 to    {chart} {to}, running {to_image}\n\n\
         \x20 helm {command}\n\
         {left_over}",
        release = asked.release,
        namespace = asked.namespace,
        chart = plan.to.name,
        from = plan.from.version,
        revision = plan.from.revision,
        to = plan.to.version,
        to_image = plan.to_image,
        command = helm_arguments(asked, plan).join(" "),
    )
}

// ── Waiting ─────────────────────────────────────────────────────────────────

/// The migration Job of one revision, as the chart names it.
pub fn migration(jobs: &[Job], revision: u64) -> Option<&Job> {
    let suffix = format!("-migrate-{revision}");
    jobs.iter().find(|job| job.name.ends_with(&suffix))
}

/// Why a pod is not yet running, in a few words: its first container that is
/// waiting, and on what.
fn stuck(pod: &Pod) -> Option<String> {
    pod.init
        .iter()
        .chain(pod.containers.iter())
        .find_map(|container| {
            let waiting = container.waiting.as_ref()?;
            Some(format!("{} {waiting}", container.name))
        })
}

/// Pods left over from a restart: stopped for good, `Succeeded` or `Failed`,
/// and made by a ReplicaSet their Deployment has since replaced. A node that
/// restarts leaves one for each pod it ran (Rancher Desktop left seven, on
/// images two releases old, under an upgrade from 0.1.184 to 0.1.189), and
/// nothing restarts or removes them. They serve nothing, so they are not
/// waited for, and the cleanup removes them. A pod still Pending, Running or
/// terminating is not one: it is waited for.
///
/// A launched plugin's Deployment carries the release's labels as the chart's
/// own do, so its pods are found the same way.
pub fn left_by_a_restart(workloads: &[Workload], pods: &[Pod]) -> Vec<String> {
    pods.iter()
        .filter(|pod| matches!(pod.phase.as_str(), "Succeeded" | "Failed"))
        .filter(|pod| {
            let Some(made_by) = &pod.replica_set else {
                return false;
            };
            workloads.iter().any(|workload| {
                workload.owns(pod)
                    && workload
                        .current
                        .as_ref()
                        .is_some_and(|current| current != made_by)
            })
        })
        .map(|pod| pod.name.clone())
        .collect()
}

/// `ghcr.io/open-meridian/meridian-runtime:9c5d480` as `meridian-runtime:9c5d480`.
fn short(image: &str) -> &str {
    image.rsplit_once('/').map_or(image, |(_, last)| last)
}

/// Where the wait stands: what is still waited for, each with why, or that
/// the migration failed. Empty `waiting` is done.
#[derive(Debug, Default)]
pub struct Progress {
    pub waiting: Vec<String>,
    /// The failed migration Job's name.
    pub failed: Option<String>,
    /// This revision's migration has not finished.
    pub migrating: bool,
    /// Of every workload, how many have nothing left to wait for.
    pub ready: (usize, usize),
    /// What is waited on first, and why, in a few words: the one line a
    /// person watching reads.
    pub now: Option<String>,
    /// Pods left over from a restart, which are not waited for.
    pub left_over: Vec<String>,
}

pub fn progress(revision: u64, jobs: &[Job], workloads: &[Workload], pods: &[Pod]) -> Progress {
    let mut progress = Progress::default();
    let mut the_migration = None;

    // Absent once Helm has returned is a chart that ran it as a hook, which
    // Helm waited for and removes when it succeeds; or a chart with none.
    if let Some(job) = migration(jobs, revision) {
        match job.state {
            JobState::Complete => {}
            JobState::Failed => progress.failed = Some(job.name.clone()),
            JobState::Running => {
                let why = pods
                    .iter()
                    .filter(|pod| pod.job.as_deref() == Some(job.name.as_str()))
                    .find_map(stuck);
                progress.waiting.push(format!(
                    "the migration, job/{}{}",
                    job.name,
                    why.as_ref()
                        .map(|why| format!(": {why}"))
                        .unwrap_or_default()
                ));
                progress.migrating = true;
                the_migration = Some(format!(
                    "the migration, job/{}: {}",
                    job.name,
                    why.as_deref().unwrap_or("running")
                ));
            }
        }
    }

    progress.left_over = left_by_a_restart(workloads, pods);
    let mut settled = 0;
    for workload in workloads {
        let theirs: Vec<&Pod> = pods
            .iter()
            .filter(|pod| workload.owns(pod) && !progress.left_over.contains(&pod.name))
            .collect();
        if let Some(why) = &workload.unfinished {
            let stuck = theirs
                .iter()
                .find_map(|pod| stuck(pod).map(|why| (&pod.name, why)));
            progress.waiting.push(format!(
                "{}: {why}{}",
                workload.name,
                stuck
                    .as_ref()
                    .map(|(pod, why)| format!("; pod/{pod} {why}"))
                    .unwrap_or_default()
            ));
            // A component that starts before its migration exits and is
            // restarted, so while that runs, the migration is the reason.
            if progress.now.is_none() {
                let component = &workload.component;
                progress.now = Some(match &stuck {
                    _ if progress.migrating => {
                        format!("{component}: {why}, waiting for its migration")
                    }
                    Some((pod, stuck)) => format!("{component}: {why}; pod/{pod}: {stuck}"),
                    None => format!("{component}: {why}"),
                });
            }
            continue;
        }
        // Rolled out is not yet every pod on the new image: an old one may
        // still be terminating, and it is still serving until it is gone.
        let before = progress.waiting.len();
        for pod in theirs {
            let old: Vec<&String> = pod
                .images
                .iter()
                .filter(|(container, image)| workload.images.get(*container) != Some(image))
                .map(|(_, image)| image)
                .collect();
            if !old.is_empty() {
                progress.waiting.push(format!(
                    "{}: pod/{} still runs {}",
                    workload.name,
                    pod.name,
                    old.iter()
                        .map(|image| image.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                if progress.now.is_none() {
                    progress.now = Some(format!(
                        "{}: pod/{} still on {}{}",
                        workload.component,
                        pod.name,
                        old.iter()
                            .map(|image| short(image))
                            .collect::<Vec<_>>()
                            .join(", "),
                        if pod.terminating { ", terminating" } else { "" }
                    ));
                }
            }
        }
        if progress.waiting.len() == before {
            settled += 1;
        }
    }
    progress.ready = (settled, workloads.len());
    progress.now = progress.now.or(the_migration);
    progress
}

// ── Reporting ───────────────────────────────────────────────────────────────

/// Each container's restarts, by pod and container, before the upgrade.
pub fn restarts(pods: &[Pod]) -> BTreeMap<(String, String), u64> {
    pods.iter()
        .flat_map(|pod| {
            pod.containers.iter().map(|container| {
                (
                    (pod.name.clone(), container.name.clone()),
                    container.restarts,
                )
            })
        })
        .collect()
}

/// Containers that restarted while the upgrade ran, each with the reason
/// Kubernetes gives and where its log is. Worth knowing, and not a failure:
/// a component that starts before its migration, or before its broker, exits
/// and is restarted, and the chart is what should stop that.
pub fn restarted(
    before: &BTreeMap<(String, String), u64>,
    after: &[Pod],
    namespace: &str,
) -> Vec<String> {
    let mut said = Vec::new();
    for pod in after.iter().filter(|pod| pod.job.is_none()) {
        for container in &pod.containers {
            let earlier = before
                .get(&(pod.name.clone(), container.name.clone()))
                .copied()
                .unwrap_or(0);
            let since = container.restarts.saturating_sub(earlier);
            if since == 0 {
                continue;
            }
            let reason = match (&container.last_reason, container.last_exit) {
                (Some(reason), Some(code)) => format!("{reason}, exit {code}"),
                (Some(reason), None) => reason.clone(),
                (None, Some(code)) => format!("exit {code}"),
                (None, None) => "no reason given".to_string(),
            };
            said.push(format!(
                "pod/{} {}: restarted {since} time(s), last {reason}. \
                 `kubectl logs -n {namespace} pod/{} -c {} --previous` says why",
                pod.name, container.name, pod.name, container.name
            ));
        }
    }
    said
}

/// Finished Jobs this release ran at earlier revisions: what is cleaned up.
/// Only those whose name ends with an earlier revision, and only finished
/// ones. A Job still running is left, and so is anything of this revision.
pub fn leftovers(jobs: &[Job], revision: u64) -> (Vec<String>, Vec<String>) {
    let mut finished = Vec::new();
    let mut running = Vec::new();
    for job in jobs {
        match job.revision {
            Some(made) if made < revision => match job.state {
                JobState::Running => running.push(job.name.clone()),
                _ => finished.push(job.name.clone()),
            },
            _ => {}
        }
    }
    (finished, running)
}

/// What was done, to say at the end.
#[derive(Debug, Default)]
pub struct Report {
    pub from: String,
    pub to: String,
    pub revision: u64,
    pub migrated: Option<String>,
    /// Each workload: its component, its images, and ready of wanted.
    pub components: Vec<(String, String, u64, u64)>,
    pub restarted: Vec<String>,
    pub cleaned: Vec<String>,
    pub left: Vec<String>,
    /// Pods left over from a restart, removed.
    pub removed: Vec<String>,
    /// Each removal that failed, and how to make it.
    pub not_cleaned: Vec<String>,
}

pub fn report_text(asked: &Asked, report: &Report) -> String {
    let mut said = format!(
        "\nUpgraded {} in {}: {} -> {}, now revision {}.\n",
        asked.release, asked.namespace, report.from, report.to, report.revision
    );
    match &report.migrated {
        Some(job) => said.push_str(&format!("Migrated: job/{job} completed.\n")),
        None => said.push_str(&format!(
            "No migration Job of revision {} was left to wait for: a chart that runs it as a \
             Helm hook removes it once it succeeds.\n",
            report.revision
        )),
    }
    let width = report
        .components
        .iter()
        .map(|(component, ..)| component.len())
        .max()
        .unwrap_or(0);
    said.push_str("\nEvery component on its new template, and ready:\n");
    for (component, images, ready, wanted) in &report.components {
        said.push_str(&format!(
            "  {component:<width$}  {ready}/{wanted}  {images}\n"
        ));
    }
    if !report.restarted.is_empty() {
        said.push_str("\nRestarted during the upgrade (worth knowing, not a failure):\n");
        for each in &report.restarted {
            said.push_str(&format!("  {each}\n"));
        }
    }
    said.push('\n');
    match report.cleaned.as_slice() {
        [] => said.push_str("No finished Job of an earlier revision to clean up.\n"),
        cleaned => said.push_str(&format!(
            "Cleaned up {} finished Job(s) of earlier revisions: {}\n",
            cleaned.len(),
            cleaned
                .iter()
                .map(|job| format!("job/{job}"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
    if !report.removed.is_empty() {
        said.push_str(&format!(
            "Removed {} left over from a restart: {}\n",
            counted(report.removed.len(), "pod"),
            report
                .removed
                .iter()
                .map(|pod| format!("pod/{pod}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for why in &report.not_cleaned {
        said.push_str(&format!("Not cleaned up: {why}\n"));
    }
    for job in &report.left {
        said.push_str(&format!(
            "Left job/{job}, of an earlier revision: it has not finished.\n"
        ));
    }
    said.push_str("Old ReplicaSets are left to the chart's revisionHistoryLimit.\n");
    said
}

#[cfg(test)]
#[path = "upgrade_deployment/tests.rs"]
mod tests;

//! `meridian status` and `meridian logs` (the v25 plan, step 7, ruled
//! 2026-10-16, item 122): what a deployment admin needs to see of a running
//! deployment without learning kubectl or helm (the product owner, 2026-10-16:
//! "meridian CLI should be comprehensive to manage their cluster").
//!
//! **`meridian status`** says, for each of the deployment's core components
//! -- the workloads its chart runs, each named by its `meridian.dev/component`
//! label -- the image it runs, how many of its pods are ready, how often they
//! restarted and why the last time (`OOMKilled`, `Error`), and the memory each
//! uses against its limit, where the cluster serves its metrics. Then **book
//! health**, a section of it (item 122): per account, the replay length since
//! the account's last snapshot -- the journal's entries the book replays to
//! answer it -- and when that snapshot was taken (W9.10), read from the row
//! ReadBookHealthFromTerminal (item 155) on the session `meridian connect`
//! keeps.
//! Read-only: it changes nothing.
//!
//! From CLI 0.1.47 it also says the **release** -- its revision, its state,
//! its chart and, where Helm holds it as anything but deployed, Helm's own
//! words for why -- and lists the **plugins**: each launched instance, by its
//! name, with the plugin's image, readiness, restarts and memory, as core's
//! components are said. Memory is each pod's own, against the limit each pod
//! has (0.1.46 summed every pod of a component, so every plugin's line showed
//! the memory of all of them).
//!
//! From CLI 0.1.54 (ruling 948) a pod's memory is its **working set**, as
//! Kubernetes counts it: the pod's cgroup usage less its inactive file cache,
//! which the kernel takes back before the pod runs short. It is read from each
//! node's cAdvisor (`/api/v1/nodes/<node>/proxy/metrics/cadvisor`,
//! `container_memory_working_set_bytes`), which computes it from the cgroup
//! itself. The metrics API (metrics-server) names the same figure, but takes it
//! from the container runtime, and some runtimes give the whole usage, file
//! cache and all: on a Docker-runtime k3s, a pod holding 27 MiB of inactive
//! file cache showed 30 MiB there and a working set of 4 MiB (2026-10-31). Where the
//! nodes' cAdvisor is refused (it takes the nodes/proxy read), the metrics
//! API's figure is said, and said to be that.
//!
//! A plugin's pod holds two containers, the plugin and its sidecar, each with
//! a limit of its own. A plugin's memory is its own container's, against that
//! container's limit, and its sidecar's is said apart: on a line of its own
//! under `--wide`, on the plugin's one line when it is at 90% of its limit or
//! more, and as `sidecar` in `--json` (a plugin's limit was once its pod's
//! containers' summed). Every other component's pod is said as one.
//!
//! **`meridian logs <component> [--since <duration>] [--previous]`** prints
//! what a component's pods printed, each line under its pod's name; with
//! `--previous`, what each container printed before its last restart (as
//! kubectl's own flag). A plugin's instance is named as `status` lists it,
//! and the release's Jobs -- `migrate`, `key`, `first-run`, `settings-key`,
//! `dashboard-key`, `backup` -- by their component, the newest run of each.
//!
//! The components and logs are read through the administrator's own kubectl,
//! as `doctor`, `up` and `backup` do: the chart says what runs, and this reads
//! it. Book health is the deployment's own answer.

use std::collections::BTreeMap;

pub mod profile;
pub mod resilience;
pub use resilience::Resilience;

/// The cluster, as these commands read it.
#[async_trait::async_trait]
pub trait Cluster: Send + Sync {
    /// kubectl, with `arguments`: what it printed, or why it failed.
    async fn kubectl(&self, arguments: &[&str]) -> Result<String, String>;
    /// helm, read-only here: what it printed, or why it failed.
    async fn helm(&self, arguments: &[&str]) -> Result<String, String>;
    /// kubectl, with what it printed on stdout even where it exited
    /// non-zero: a member's probe prints its answer and exits 1 when a path
    /// did not answer.
    async fn kubectl_printed(&self, arguments: &[&str]) -> Result<String, String> {
        self.kubectl(arguments).await
    }
}

pub struct ThisCluster;

async fn ran(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new(program)
        .args(arguments)
        .output()
        .await
        .map_err(|failed| match failed.kind() {
            std::io::ErrorKind::NotFound => format!(
                "{program} is not installed here; `meridian doctor` says what a deployment's \
                 machine needs"
            ),
            _ => format!("{program} could not be run: {failed}"),
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "`{program} {}`: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[async_trait::async_trait]
impl Cluster for ThisCluster {
    async fn kubectl(&self, arguments: &[&str]) -> Result<String, String> {
        ran("kubectl", arguments).await
    }
    async fn kubectl_printed(&self, arguments: &[&str]) -> Result<String, String> {
        let output = tokio::process::Command::new("kubectl")
            .args(arguments)
            .output()
            .await
            .map_err(|failed| format!("kubectl could not be run: {failed}"))?;
        let printed = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() || !printed.trim().is_empty() {
            Ok(printed)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }
    async fn helm(&self, arguments: &[&str]) -> Result<String, String> {
        ran("helm", arguments).await
    }
}

/// What a status is asked for: the release and its namespace.
pub struct Asked {
    pub release: String,
    pub namespace: String,
}

/// The flags naming a release in a `meridian` command: `-n <namespace>`, and
/// `--release <name>` where it is not the default.
pub fn scope(release: &str, namespace: &str) -> String {
    if release == "meridian" {
        format!("-n {namespace}")
    } else {
        format!("--release {release} -n {namespace}")
    }
}

/// The release's workloads: every one carries its instance label. Which
/// component each is, `component_of` reads: the chart labels some on their
/// own metadata and the database's and the registry's only in their pods'
/// selector.
fn selector(release: &str) -> String {
    format!("app.kubernetes.io/instance={release}")
}

/// A workload's component: its own label, or its pods' selector's.
pub fn component_of(workload: &serde_json::Value) -> Option<&str> {
    workload["metadata"]["labels"]["meridian.dev/component"]
        .as_str()
        .or_else(|| workload["spec"]["selector"]["matchLabels"]["meridian.dev/component"].as_str())
}

async fn json(cluster: &dyn Cluster, arguments: &[&str]) -> Result<serde_json::Value, String> {
    let printed = cluster.kubectl(arguments).await?;
    serde_json::from_str(&printed)
        .map_err(|failed| format!("kubectl's answer could not be read: {failed}"))
}

/// A Kubernetes quantity of memory, in bytes: `512Mi`, `1Gi`, `123456Ki`,
/// `1G`, `500M`, `1048576`.
pub fn bytes_of(quantity: &str) -> Option<u64> {
    let quantity = quantity.trim();
    let split = quantity
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(quantity.len());
    let (number, unit) = quantity.split_at(split);
    let number: f64 = number.parse().ok()?;
    let scale: f64 = match unit {
        "" => 1.0,
        "Ki" => 1024.0,
        "Mi" => 1024.0 * 1024.0,
        "Gi" => 1024.0 * 1024.0 * 1024.0,
        "Ti" => 1024.0_f64.powi(4),
        "k" | "K" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "m" => 1e-3,
        _ => return None,
    };
    Some((number * scale).round() as u64)
}

/// Bytes as a person reads them: `412 MiB`, `1.5 GiB`.
pub fn said_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let mib = bytes as f64 / MIB;
    if mib >= 1024.0 {
        format!("{:.1} GiB", mib / 1024.0)
    } else {
        format!("{:.0} MiB", mib)
    }
}

/// One component, as `status` says it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub kind: String,
    pub images: Vec<String>,
    pub ready: u64,
    pub wanted: u64,
    /// A plugin's standby pods (step 3d): its replicas past its shards,
    /// which wait and are never counted as wanted or missing.
    pub standbys: u64,
    pub restarts: u64,
    /// The last restart's reason and when it ended, where one is known.
    pub last_reason: String,
    pub last_at: String,
    /// The working set of its busiest pod, where the cluster serves metrics,
    /// and the limit each pod has.
    pub memory: Option<u64>,
    pub limit: Option<u64>,
    /// Each of its pods, by name, with its working set where known.
    pub pods: Vec<(String, Option<u64>)>,
    /// A plugin's: its instance, the plugin's own image (the container
    /// that is not its sidecar) and whether the launcher launched it.
    pub instance: Option<String>,
    pub plugin_image: Option<String>,
    pub launched: bool,
    /// A plugin's sidecar, where the pod holds the plugin beside it: its
    /// memory and limit, apart from the plugin's (`memory`, `limit` and
    /// `pods` are then the plugin container's own).
    pub sidecar: Option<Container>,
}

/// One container of each of a component's pods: its working set in its
/// busiest pod, the limit it has in each, and each pod's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Container {
    pub memory: Option<u64>,
    pub limit: Option<u64>,
    pub pods: Vec<(String, Option<u64>)>,
}

/// The memory read of the release's pods: each pod's working set, and each
/// of its containers'.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    /// By pod.
    pub pods: BTreeMap<String, u64>,
    /// By pod and container.
    pub containers: BTreeMap<(String, String), u64>,
}

/// The component plugins run as: one Deployment each, a sidecar beside the
/// plugin, named by its instance.
pub const SIDECAR: &str = "sidecar";

/// Whether a pod is one of a workload's: every label of the workload's
/// selector on the pod; or, where it names none, the component's label.
fn selects(workload: &serde_json::Value, pod: &serde_json::Value, component: &str) -> bool {
    let labels = &pod["metadata"]["labels"];
    match workload["spec"]["selector"]["matchLabels"].as_object() {
        Some(wanted) if !wanted.is_empty() => wanted
            .iter()
            .all(|(key, value)| labels[key.as_str()] == *value),
        _ => labels["meridian.dev/component"].as_str() == Some(component),
    }
}

/// Where the memory `status` says was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRead {
    /// Each node's cAdvisor: the working set, from the cgroup itself.
    Cadvisor,
    /// The metrics API, where the nodes' cAdvisor was refused: the working
    /// set as the container runtime gives it, on some runtimes the whole
    /// usage.
    MetricsApi,
}

impl MemoryRead {
    /// As `--json` says it.
    pub fn word(self) -> &'static str {
        match self {
            MemoryRead::Cadvisor => "cadvisor",
            MemoryRead::MetricsApi => "metrics-api",
        }
    }
}

/// One sample of a Prometheus text exposition, where it is `metric`'s:
/// its labels and its value.
fn sample(line: &str, metric: &str) -> Option<(BTreeMap<String, String>, f64)> {
    let mut rest = line.strip_prefix(metric)?.strip_prefix('{')?;
    let mut labels = BTreeMap::new();
    loop {
        rest = rest.trim_start_matches([',', ' ']);
        if let Some(after) = rest.strip_prefix('}') {
            rest = after;
            break;
        }
        let (key, after) = rest.split_once("=\"")?;
        let mut value = String::new();
        let mut chars = after.char_indices();
        let end = loop {
            match chars.next()? {
                (_, '\\') => match chars.next()?.1 {
                    'n' => value.push('\n'),
                    other => value.push(other),
                },
                (at, '"') => break at,
                (_, other) => value.push(other),
            }
        };
        labels.insert(key.trim().to_string(), value);
        rest = &after[end + 1..];
    }
    let value: f64 = rest.split_whitespace().next()?.parse().ok()?;
    (value.is_finite() && value >= 0.0).then_some((labels, value))
}

/// Each pod of `namespace`'s working set, in bytes, from a node's cAdvisor
/// (`container_memory_working_set_bytes`, its cgroup usage less its inactive
/// file cache): the pod's own cgroup's (`container=""`) where cAdvisor gives
/// it, which is every container of the pod; else its containers' summed,
/// never the sandbox's (`POD`).
pub fn working_sets(cadvisor: &str, namespace: &str) -> BTreeMap<String, u64> {
    let mut whole: BTreeMap<String, u64> = BTreeMap::new();
    let mut summed: BTreeMap<String, u64> = BTreeMap::new();
    for line in cadvisor.lines() {
        let Some((labels, value)) = sample(line, "container_memory_working_set_bytes") else {
            continue;
        };
        let at = |key: &str| labels.get(key).map(String::as_str).unwrap_or_default();
        if at("namespace") != namespace || at("pod").is_empty() {
            continue;
        }
        let bytes = value.round() as u64;
        match at("container") {
            "" => {
                whole.insert(at("pod").to_string(), bytes);
            }
            "POD" => {}
            _ => *summed.entry(at("pod").to_string()).or_default() += bytes,
        }
    }
    summed.extend(whole);
    summed
}

/// Each container's working set, by pod and container, from a node's
/// cAdvisor: never the pod's own cgroup's or the sandbox's.
fn container_working_sets(cadvisor: &str, namespace: &str) -> BTreeMap<(String, String), u64> {
    let mut found = BTreeMap::new();
    for line in cadvisor.lines() {
        let Some((labels, value)) = sample(line, "container_memory_working_set_bytes") else {
            continue;
        };
        let at = |key: &str| labels.get(key).map(String::as_str).unwrap_or_default();
        if at("namespace") != namespace || at("pod").is_empty() {
            continue;
        }
        if !matches!(at("container"), "" | "POD") {
            found.insert(
                (at("pod").to_string(), at("container").to_string()),
                value.round() as u64,
            );
        }
    }
    found
}

/// A node's cAdvisor, read: each pod's working set and each container's.
pub fn cadvisor_usage(cadvisor: &str, namespace: &str) -> Usage {
    Usage {
        pods: working_sets(cadvisor, namespace),
        containers: container_working_sets(cadvisor, namespace),
    }
}

/// Each pod's memory as the metrics API gives it: its containers' summed,
/// and each container's.
pub fn metrics_api_usage(metrics: &serde_json::Value) -> Usage {
    let mut usage = Usage::default();
    for pod in metrics["items"].as_array().into_iter().flatten() {
        let name = pod["metadata"]["name"].as_str().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let mut used = Vec::new();
        for container in pod["containers"].as_array().into_iter().flatten() {
            let Some(bytes) = container["usage"]["memory"].as_str().and_then(bytes_of) else {
                continue;
            };
            used.push(bytes);
            if let Some(container) = container["name"].as_str() {
                usage
                    .containers
                    .insert((name.to_string(), container.to_string()), bytes);
            }
        }
        if !used.is_empty() {
            usage.pods.insert(name.to_string(), used.iter().sum());
        }
    }
    usage
}

/// The limit of the containers named, summed; none where none has one.
fn limit_of(containers: &[&serde_json::Value]) -> Option<u64> {
    let limit: u64 = containers
        .iter()
        .filter_map(|c| {
            c["resources"]["limits"]["memory"]
                .as_str()
                .and_then(bytes_of)
        })
        .sum();
    (limit > 0).then_some(limit)
}

/// The working set of the containers named in `pod`, summed, where any of
/// theirs was read.
fn used_by(usage: &Usage, pod: &str, containers: &[&serde_json::Value]) -> Option<u64> {
    let used: Vec<u64> = containers
        .iter()
        .filter_map(|c| c["name"].as_str())
        .filter_map(|name| {
            usage
                .containers
                .get(&(pod.to_string(), name.to_string()))
                .copied()
        })
        .collect();
    (!used.is_empty()).then(|| used.iter().sum())
}

/// The components among the workloads and pods kubectl listed, with each
/// pod's memory where it was read.
pub fn components(
    workloads: &serde_json::Value,
    pods: &serde_json::Value,
    usage: &Usage,
) -> Vec<Component> {
    let mut found = Vec::new();
    for workload in workloads["items"].as_array().into_iter().flatten() {
        let labels = &workload["metadata"]["labels"];
        let Some(name) = component_of(workload) else {
            continue;
        };
        let spec = &workload["spec"]["template"]["spec"];
        let containers: Vec<&serde_json::Value> = spec["containers"]
            .as_array()
            .into_iter()
            .flatten()
            .collect();
        let mut component = Component {
            name: name.to_string(),
            kind: workload["kind"].as_str().unwrap_or_default().to_string(),
            images: containers
                .iter()
                .filter_map(|c| c["image"].as_str().map(String::from))
                .collect(),
            ready: workload["status"]["readyReplicas"].as_u64().unwrap_or(0),
            wanted: workload["spec"]["replicas"].as_u64().unwrap_or(1),
            ..Component::default()
        };
        // A plugin's pod: the plugin, said as the component, and its sidecar
        // apart, each against its own limit.
        let (sidecars, own): (Vec<&serde_json::Value>, Vec<&serde_json::Value>) = containers
            .iter()
            .partition(|c| c["name"].as_str() == Some(SIDECAR));
        let apart = name == SIDECAR && !sidecars.is_empty() && !own.is_empty();
        if apart {
            component.limit = limit_of(&own);
            component.sidecar = Some(Container {
                limit: limit_of(&sidecars),
                ..Container::default()
            });
        } else {
            component.limit = limit_of(&containers);
        }
        let mut last: Option<(String, String)> = None;
        for pod in pods["items"].as_array().into_iter().flatten() {
            if !selects(workload, pod, name) {
                continue;
            }
            let pod_name = pod["metadata"]["name"].as_str().unwrap_or_default();
            for status in pod["status"]["containerStatuses"]
                .as_array()
                .into_iter()
                .flatten()
            {
                component.restarts += status["restartCount"].as_u64().unwrap_or(0);
                let ended = &status["lastState"]["terminated"];
                if let Some(reason) = ended["reason"].as_str() {
                    let at = ended["finishedAt"].as_str().unwrap_or_default().to_string();
                    if last.as_ref().is_none_or(|(_, held)| at > *held) {
                        last = Some((reason.to_string(), at));
                    }
                }
            }
            if let Some(sidecar) = component.sidecar.as_mut() {
                sidecar
                    .pods
                    .push((pod_name.to_string(), used_by(usage, pod_name, &sidecars)));
                component
                    .pods
                    .push((pod_name.to_string(), used_by(usage, pod_name, &own)));
            } else {
                component
                    .pods
                    .push((pod_name.to_string(), usage.pods.get(pod_name).copied()));
            }
        }
        component.pods.sort();
        component.memory = component.pods.iter().filter_map(|(_, used)| *used).max();
        if let Some(sidecar) = component.sidecar.as_mut() {
            sidecar.pods.sort();
            sidecar.memory = sidecar.pods.iter().filter_map(|(_, used)| *used).max();
        }
        if let Some((reason, at)) = last {
            component.last_reason = reason;
            component.last_at = at;
        }
        // A plugin's shards are what it wants ready; its replicas past them
        // stand by (step 3d).
        if let Some((shards, standbys)) = crate::upgrade_deployment::shards::shape(workload) {
            component.wanted = shards;
            component.ready = component.ready.min(shards);
            component.standbys = standbys;
        }
        if name == SIDECAR {
            component.instance = labels["meridian.dev/instance"].as_str().map(String::from);
            component.launched = labels["meridian.dev/launched"] == "true";
            component.plugin_image = containers
                .iter()
                .find(|c| c["name"].as_str() != Some(SIDECAR))
                .and_then(|c| c["image"].as_str())
                .map(String::from);
        }
        found.push(component);
    }
    found.sort_by(|a, b| (&a.name, &a.instance).cmp(&(&b.name, &b.instance)));
    found
}

/// One account's book health (AccountBookHealth): its journal's entries,
/// the entries since its last snapshot -- what the book replays to answer it
/// -- and that snapshot's time where it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub account_id: String,
    pub journal_entries: u64,
    pub replay: u64,
    pub snapshot_at_ns: Option<u64>,
}

/// Where `meridian status` reads book health: the row
/// `ReadBookHealthFromTerminal` (DESIGN25 amendment 2, 081f95f; item 155),
/// W9.10's query through the dashboard's terminal surface, on the delegation
/// `meridian connect` keeps, as the CLI reads the catalogue.
pub const BOOK_HEALTH_PATH: &str = "/terminal/book-health";

/// How many accounts a page of the answer asks for, and the most pages read.
pub const BOOK_HEALTH_PAGE: u32 = 500;
const BOOK_HEALTH_PAGES: usize = 1_000;

/// A count or nanoseconds as JSON carries it: a number, or a decimal string
/// (a 64-bit integer's JSON).
fn count(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

/// A page of the answer (ReadBookHealthReply): its accounts, and the cursor
/// of the next page, empty on the last.
pub fn accounts_from(answer: &serde_json::Value) -> (Vec<Account>, String) {
    let accounts = answer["accounts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|held| {
            let at = count(&held["last_snapshot_at_ns"]).unwrap_or(0);
            Some(Account {
                account_id: held["account_id"].as_str()?.to_string(),
                journal_entries: count(&held["journal_entries"]).unwrap_or(0),
                replay: count(&held["entries_since_snapshot"])?,
                snapshot_at_ns: (at > 0).then_some(at),
            })
        })
        .collect();
    let next = answer["next_cursor"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    (accounts, next)
}

/// Book health from the deployment, every page of it. A deployment that does
/// not answer the row, or a book it cannot reach (503), is said as not read,
/// never as healthy.
pub async fn book_health(
    address: &str,
    session: &crate::credential::Credential,
) -> Result<Vec<Account>, String> {
    let http = crate::catalogue::http().map_err(|failed| failed.said().to_string())?;
    let mut found = Vec::new();
    let mut cursor = String::new();
    for _ in 0..BOOK_HEALTH_PAGES {
        let mut url = reqwest::Url::parse(&format!("{address}{BOOK_HEALTH_PATH}"))
            .map_err(|failed| failed.to_string())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("page_size", &BOOK_HEALTH_PAGE.to_string());
            if !cursor.is_empty() {
                query.append_pair("cursor", &cursor);
            }
        }
        let (status, body, _) = session
            .send(|bearer| http.get(url.clone()).bearer_auth(bearer))
            .await
            .map_err(|failed| failed.said().to_string())?;
        match status.as_u16() {
            404 => {
                return Err(format!(
                    "{address} does not answer ReadBookHealthFromTerminal: its core is older than \
                     this CLI"
                ))
            }
            403 => return Err("reading it takes a deployment admin".into()),
            503 => return Err(format!("{address} cannot reach the book now")),
            _ if !status.is_success() => {
                return Err(format!("{address} refused ({status}): {}", body.trim()))
            }
            _ => {}
        }
        let answer: serde_json::Value = serde_json::from_str(&body)
            .map_err(|failed| format!("{address} answered with something unreadable: {failed}"))?;
        let (accounts, next) = accounts_from(&answer);
        found.extend(accounts);
        if next.is_empty() || next == cursor {
            return Ok(found);
        }
        cursor = next;
    }
    Err(format!(
        "{address} answered more than {BOOK_HEALTH_PAGES} pages of accounts; book health is not \
         said in part"
    ))
}

/// The release, as `helm history --max 1` says its newest revision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    pub name: String,
    pub revision: u64,
    pub status: String,
    pub chart: String,
    pub app_version: String,
    pub updated: String,
    pub description: String,
}

/// The newest revision `helm history -o json` lists.
pub fn release_from(name: &str, history: &str) -> Option<Release> {
    let listed: serde_json::Value = serde_json::from_str(history).ok()?;
    let newest = listed
        .as_array()?
        .iter()
        .max_by_key(|revision| revision["revision"].as_u64().unwrap_or(0))?;
    Some(Release {
        name: name.to_string(),
        revision: newest["revision"].as_u64()?,
        status: newest["status"].as_str().unwrap_or_default().to_string(),
        chart: newest["chart"].as_str().unwrap_or_default().to_string(),
        app_version: newest["app_version"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        updated: newest["updated"].as_str().unwrap_or_default().to_string(),
        description: newest["description"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    })
}

/// The release's revisions, every one `helm history -o json` lists, oldest
/// first: what `rollback-deployment` returns to.
pub fn revisions_from(name: &str, history: &str) -> Vec<Release> {
    let listed: serde_json::Value = serde_json::from_str(history).unwrap_or_default();
    let mut all: Vec<Release> = listed
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|revision| {
            let one = serde_json::Value::Array(vec![revision.clone()]).to_string();
            release_from(name, &one)
        })
        .collect();
    all.sort_by_key(|release| release.revision);
    all
}

/// The releases helm lists in a namespace, by name.
async fn releases_in(cluster: &dyn Cluster, namespace: &str) -> Option<Vec<String>> {
    let listed = cluster // S9: helm list
        .helm(&["list", "--namespace", namespace, "--all", "-o", "json"])
        .await
        .ok()?;
    let listed: serde_json::Value = serde_json::from_str(&listed).ok()?;
    Some(
        listed
            .as_array()?
            .iter()
            .filter_map(|release| release["name"].as_str().map(String::from))
            .collect(),
    )
}

/// Everything `status` reads.
pub struct Status {
    pub release: Result<Release, String>,
    pub components: Vec<Component>,
    /// Each plugin instance's Deployment, named by its instance.
    pub plugins: Vec<Component>,
    /// Where memory was read, or None where it was not.
    pub memory_read: Option<MemoryRead>,
    /// What a person is told of that: why memory is not said, or that it is
    /// the metrics API's figure; None where it is the nodes' own.
    pub memory_note: Option<String>,
    pub accounts: Result<Vec<Account>, String>,
    /// The database, the members' probes and integrity at rest (step 3b).
    pub resilience: Resilience,
    /// What `meridian stop` stopped, while it is stopped.
    pub stopped: Option<crate::offline::Stopped>,
}

pub async fn status(cluster: &dyn Cluster, asked: &Asked) -> Result<Status, String> {
    let ns = asked.namespace.as_str();
    let selector = selector(&asked.release);
    let workloads = json(
        cluster, // S9: kubectl get deployments; kubectl get statefulsets
        &[
            "get",
            "deployments,statefulsets",
            "-n",
            ns,
            "-l",
            &selector,
            "-o",
            "json",
        ],
    )
    .await?;
    let pods = json(
        cluster, // S9: kubectl get pods
        &["get", "pods", "-n", ns, "-l", &selector, "-o", "json"],
    )
    .await?;
    let (usage, memory_read, memory_note) = memory(cluster, ns, &pods).await;
    let (plugins, components): (Vec<Component>, Vec<Component>) =
        components(&workloads, &pods, &usage)
            .into_iter()
            .partition(|component| component.name == SIDECAR);
    if components.is_empty() && plugins.is_empty() {
        let there = match releases_in(cluster, ns).await {
            Some(names) if names.is_empty() => " (it holds no release)".to_string(),
            Some(names) => format!(" (its releases: {})", names.join(", ")),
            None => String::new(),
        };
        return Err(format!(
            "no deployment `{}` in {ns}{there}: `meridian up` installs one, and -n and --release \
             name another",
            asked.release
        ));
    }
    let release = match cluster // S9: helm history
        .helm(&[
            "history",
            &asked.release,
            "--namespace",
            ns,
            "--max",
            "1",
            "-o",
            "json",
        ])
        .await
    {
        Ok(history) => release_from(&asked.release, &history)
            .ok_or_else(|| "Helm listed no revision of it".to_string()),
        Err(failed) => Err(failed),
    };
    let resilience = resilience::read(cluster, asked, &workloads, &pods).await;
    let stopped = crate::offline::stop::stopped_in(&workloads);
    Ok(Status {
        stopped,
        release,
        components,
        plugins,
        memory_read,
        memory_note,
        resilience,
        // Read from the deployment by the caller (`book_health`), which
        // holds the session; the cluster has no part in it.
        accounts: Err("not read".into()),
    })
}

/// The memory of the release's pods (ruling 948): each one's working set
/// from the cAdvisor of every node they run on; where one node's is refused,
/// the metrics API's figure for every pod, and said to be that; where neither
/// answers, nothing, and why.
async fn memory(
    cluster: &dyn Cluster,
    ns: &str,
    pods: &serde_json::Value,
) -> (Usage, Option<MemoryRead>, Option<String>) {
    let nodes: std::collections::BTreeSet<&str> = pods["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pod| pod["spec"]["nodeName"].as_str())
        .filter(|node| !node.is_empty())
        .collect();
    let mut usage = Usage::default();
    let mut refused = None;
    for node in &nodes {
        match cluster // S9: kubectl get --raw
            .kubectl(&[
                "get",
                "--raw",
                &format!("/api/v1/nodes/{node}/proxy/metrics/cadvisor"),
            ])
            .await
        {
            Ok(text) => {
                let read = cadvisor_usage(&text, ns);
                usage.pods.extend(read.pods);
                usage.containers.extend(read.containers);
            }
            Err(why) => {
                refused = Some(why.lines().next().unwrap_or_default().trim().to_string());
                break;
            }
        }
    }
    if !nodes.is_empty() && refused.is_none() {
        return (usage, Some(MemoryRead::Cadvisor), None);
    }
    let why = refused.unwrap_or_else(|| "no pod is on a node yet".into());
    match json(
        cluster, // S9: kubectl get --raw
        &[
            "get",
            "--raw",
            &format!("/apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods"),
        ],
    )
    .await
    {
        Ok(metrics) => (
            metrics_api_usage(&metrics),
            Some(MemoryRead::MetricsApi),
            Some(format!(
                "memory is the metrics API's figure, which some container runtimes give with \
                 the file cache in: the nodes' own working set was not read ({why})"
            )),
        ),
        Err(_) => (
            Usage::default(),
            None,
            Some(format!(
                "memory in use is not known: the cluster serves no metrics (metrics-server), and \
                 the nodes' own working set was not read ({why})"
            )),
        ),
    }
}

/// A component's memory as a person reads it: each pod's working set against
/// the limit each pod has.
fn memory_said(c: &Component) -> String {
    memory_of(&c.pods, c.limit)
}

/// Memory as a person reads it: each pod's working set against the limit
/// each pod has.
fn memory_of(pods: &[(String, Option<u64>)], limit: Option<u64>) -> String {
    let used: Vec<u64> = pods.iter().filter_map(|(_, used)| *used).collect();
    let each = if used.len() > 1 { " each" } else { "" };
    let listed = used
        .iter()
        .map(|bytes| said_bytes(*bytes))
        .collect::<Vec<_>>()
        .join(", ");
    match (used.is_empty(), limit) {
        (false, Some(limit)) if used.len() == 1 => format!(
            "memory {listed} of {} ({}%)",
            said_bytes(limit),
            used[0] * 100 / limit.max(1)
        ),
        (false, Some(limit)) => format!("memory {listed} of {}{each}", said_bytes(limit)),
        (false, None) => format!("memory {listed} (no limit)"),
        (true, Some(limit)) => format!("limit {}", said_bytes(limit)),
        (true, None) => "no limit".into(),
    }
}

fn restarts_said(c: &Component) -> String {
    if c.restarts == 0 {
        "no restarts".to_string()
    } else if c.last_reason.is_empty() {
        format!("{} restart(s)", c.restarts)
    } else {
        format!(
            "{} restart(s), the last {} at {}",
            c.restarts, c.last_reason, c.last_at
        )
    }
}

/// The release, as a line, and Helm's words where it is not deployed; when
/// it was updated only when wide.
fn release_said(release: &Result<Release, String>, namespace: &str, wide: bool) -> String {
    match release {
        Err(why) => format!("Release: not read: {why}\n"),
        Ok(r) => {
            let mut out = format!(
                "Release: {} revision {}, {}, chart {}{}{}\n",
                r.name,
                r.revision,
                r.status,
                r.chart,
                if r.app_version.is_empty() {
                    String::new()
                } else {
                    format!(" (app {})", r.app_version)
                },
                if r.updated.is_empty() || !wide {
                    String::new()
                } else {
                    format!(", updated {}", r.updated)
                },
            );
            if r.status != "deployed" {
                if !r.description.is_empty() {
                    out.push_str(&format!("  Helm says: {}\n", r.description));
                }
                out.push_str(&format!(
                    "  `meridian rollback-deployment -n {namespace}` returns it to its last \
                     deployed revision{}\n",
                    if r.status.starts_with("pending") {
                        ", once no other Helm operation is running on it"
                    } else {
                        ""
                    }
                ));
            }
            out
        }
    }
}

/// Why book health was not read, where no deployment is connected.
pub const NOT_CONNECTED: &str =
    "run `meridian connect` first (with its address if on another machine)";

/// The widest line the compact status prints.
pub const WIDTH: usize = 100;

/// A component worth a line of its own: not all ready, restarted, or its
/// busiest pod at 90% of its limit or more.
fn unwell(c: &Component) -> bool {
    let sidecar_near = c
        .sidecar
        .as_ref()
        .is_some_and(|s| near_limit(s.memory, s.limit));
    c.ready < c.wanted || c.restarts > 0 || near_limit(c.memory, c.limit) || sidecar_near
}

/// At 90% of its limit or more.
fn near_limit(memory: Option<u64>, limit: Option<u64>) -> bool {
    match (memory, limit) {
        (Some(used), Some(limit)) if limit > 0 => used * 10 >= limit * 9,
        _ => false,
    }
}

/// A plugin's sidecar, on its plugin's compact line, where it is near its
/// own limit.
fn sidecar_near_said(c: &Component) -> String {
    match &c.sidecar {
        Some(s) if near_limit(s.memory, s.limit) => {
            format!(", sidecar {}", memory_of(&s.pods, s.limit))
        }
        _ => String::new(),
    }
}

/// The command that reads why a component is unwell.
fn logs_for(name: &str, c: &Component, scope: &str) -> String {
    format!(
        "`meridian logs {name}{}{}`",
        if c.restarts > 0 { " --previous" } else { "" },
        match scope.is_empty() {
            true => String::new(),
            false => format!(" {scope}"),
        }
    )
}

/// A line wider than [`WIDTH`] carried on to the next, indented.
pub fn wrapped(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let mut rest = line.to_string();
        while rest.chars().count() > WIDTH {
            let cut = rest
                .char_indices()
                .take_while(|(i, _)| *i <= WIDTH)
                .filter(|(i, c)| *c == ' ' && *i > indent + 4)
                .map(|(i, _)| i)
                .last();
            let Some(cut) = cut else { break };
            out.push_str(&rest[..cut]);
            out.push('\n');
            rest = format!("{}{}", " ".repeat(indent + 4), rest[cut..].trim_start());
        }
        out.push_str(&rest);
        out.push('\n');
    }
    out
}

/// The status, for a person to read: a line a part when healthy, and each
/// thing that is not with the command that reads why. `--wide` says it all.
pub fn said(status: &Status, namespace: &str) -> String {
    let mut out = release_said(&status.release, namespace, false);
    out.push_str(&resilience::stopped_said(&status.resilience));
    let scope = status.resilience.scope.as_str();
    // Stopped by `meridian stop`: one line in place of what is not ready.
    if let Some(stopped) = &status.stopped {
        out.push_str(&crate::offline::stop::status_line(stopped, scope));
        out.push_str("\nImages, pods and each member's probe: `meridian status --wide`\n");
        return wrapped(&out);
    }
    let ready = status
        .components
        .iter()
        .filter(|c| c.ready >= c.wanted)
        .count();
    let restarts: u64 = status.components.iter().map(|c| c.restarts).sum();
    out.push_str(&format!(
        "Components: {ready} of {} ready, {}\n",
        status.components.len(),
        match restarts {
            0 => "no restarts".to_string(),
            n => format!("{n} restart(s)"),
        }
    ));
    for c in status.components.iter().filter(|c| unwell(c)) {
        out.push_str(&format!(
            "  {}  {}/{} ready, {}, {}: {}\n",
            c.name,
            c.ready,
            c.wanted,
            restarts_said(c),
            memory_said(c),
            logs_for(&c.name, c, scope)
        ));
    }
    out.push_str(&resilience::compact(&status.resilience, namespace));
    let named = |c: &Component| c.instance.clone().unwrap_or_else(|| "(unnamed)".into());
    match status.plugins.len() {
        0 => out.push_str("Plugins: none running; `meridian plugin list` shows the catalogue\n"),
        n => out.push_str(&format!(
            "Plugins: {n} running, {} ready\n",
            status
                .plugins
                .iter()
                .filter(|c| c.ready >= c.wanted)
                .count()
        )),
    }
    for c in status.plugins.iter().filter(|c| unwell(c)) {
        let name = named(c);
        out.push_str(&format!(
            "  {name}  {}/{} ready, {}, {}{}: {}\n",
            c.ready,
            c.wanted,
            restarts_said(c),
            memory_said(c),
            sidecar_near_said(c),
            logs_for(&name, c, scope)
        ));
    }
    match &status.accounts {
        Err(why) => out.push_str(&format!("Book health: not read; {why}\n")),
        Ok(accounts) if accounts.is_empty() => {
            out.push_str("Book health: no account has a journal yet\n")
        }
        Ok(accounts) => {
            let longest = accounts.iter().max_by_key(|a| a.replay);
            out.push_str(&format!(
                "Book health: {} account(s){}\n",
                accounts.len(),
                longest
                    .map(|a| format!(
                        ", the longest replay {} entries ({})",
                        a.replay, a.account_id
                    ))
                    .unwrap_or_default()
            ));
        }
    }
    out.push_str("Images, pods and each member's probe: `meridian status --wide`\n");
    wrapped(&out)
}

/// The status, all of it: each component's image and memory, each pod and
/// each store member's probe.
pub fn said_wide(status: &Status, now_ns: u64, namespace: &str) -> String {
    let mut out = release_said(&status.release, namespace, true);
    out.push_str(&resilience::stopped_said(&status.resilience));
    if let Some(stopped) = &status.stopped {
        let scope = status.resilience.scope.as_str();
        out.push_str(&crate::offline::stop::status_line(stopped, scope));
        out.push('\n');
    }
    let width = status
        .components
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(0);
    out.push_str("\nComponents:\n");
    for c in &status.components {
        out.push_str(&format!(
            "  {:width$}  {}/{} ready  {}  {}  {}\n",
            c.name,
            c.ready,
            c.wanted,
            restarts_said(c),
            memory_said(c),
            c.images.join(", "),
        ));
    }
    if let Some(why) = &status.memory_note {
        out.push_str(&format!("  ({why})\n"));
    }
    out.push_str(&resilience::said(&status.resilience, now_ns, namespace));
    out.push_str("\nPlugins:\n");
    if status.plugins.is_empty() {
        out.push_str(
            "  none running: `meridian plugin list` shows the catalogue and each launch\n",
        );
    }
    let named = |c: &Component| c.instance.clone().unwrap_or_else(|| "(unnamed)".into());
    let width = status
        .plugins
        .iter()
        .map(|c| named(c).len())
        .max()
        .unwrap_or(0);
    for c in &status.plugins {
        out.push_str(&format!(
            "  {:width$}  {}/{} ready{}  {}  {}  {}{}\n",
            named(c),
            c.ready,
            c.wanted,
            match c.standbys {
                0 => String::new(),
                n => format!(", {n} standby"),
            },
            restarts_said(c),
            memory_said(c),
            c.plugin_image
                .clone()
                .unwrap_or_else(|| "a sidecar alone".into()),
            if c.launched { "" } else { "  (the chart's)" },
        ));
        if let Some(sidecar) = &c.sidecar {
            out.push_str(&format!(
                "  {:width$}  sidecar  {}\n",
                "",
                memory_of(&sidecar.pods, sidecar.limit)
            ));
        }
    }
    out.push_str("\nBook health:\n");
    match &status.accounts {
        Err(why) => out.push_str(&format!("  not read: {why}\n")),
        Ok(accounts) if accounts.is_empty() => {
            out.push_str("  no account has a journal yet\n");
        }
        Ok(accounts) => {
            let width = accounts
                .iter()
                .map(|a| a.account_id.len())
                .max()
                .unwrap_or(0);
            for account in accounts {
                let snapshot = match account.snapshot_at_ns {
                    Some(at) => format!(
                        "since its snapshot of {} ({} ago)",
                        crate::registry::moment(at),
                        ago(now_ns.saturating_sub(at))
                    ),
                    None => "no snapshot yet: its whole journal".into(),
                };
                out.push_str(&format!(
                    "  {:width$}  replays {} of its {} entries, {snapshot}\n",
                    account.account_id, account.replay, account.journal_entries
                ));
            }
        }
    }
    out
}

/// A span of nanoseconds, roughly, as a person says it.
pub(crate) fn ago(ns: u64) -> String {
    let seconds = ns / 1_000_000_000;
    match seconds {
        0..=119 => format!("{seconds}s"),
        120..=7_199 => format!("{}m", seconds / 60),
        7_200..=172_799 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn component_json(c: &Component) -> serde_json::Value {
    let mut said = serde_json::json!({
        "component": c.name, "kind": c.kind, "images": c.images,
        "ready": c.ready, "wanted": c.wanted, "restarts": c.restarts,
        "last_restart_reason": c.last_reason, "last_restart_at": c.last_at,
        "memory_bytes": c.memory, "memory_limit_bytes": c.limit,
        "pods": c.pods.iter().map(|(pod, used)| serde_json::json!({
            "pod": pod, "memory_bytes": used,
        })).collect::<Vec<_>>(),
    });
    if let Some(sidecar) = &c.sidecar {
        said["sidecar"] = serde_json::json!({
            "memory_bytes": sidecar.memory, "memory_limit_bytes": sidecar.limit,
            "pods": sidecar.pods.iter().map(|(pod, used)| serde_json::json!({
                "pod": pod, "memory_bytes": used,
            })).collect::<Vec<_>>(),
        });
    }
    if let Some(instance) = &c.instance {
        said["instance_id"] = instance.clone().into();
        said["plugin_image"] = c.plugin_image.clone().into();
        said["launched"] = c.launched.into();
        said["standbys"] = c.standbys.into();
    }
    said
}

/// The status as one JSON object, for a script or an agent. `memory_bytes`
/// is the busiest pod's working set, against `memory_limit_bytes`, each
/// pod's limit; `pods` holds each pod's own; `memory_read` says where it was
/// read (`cadvisor`, or `metrics-api` where the nodes' was refused). A
/// plugin's are its own container's, and its `sidecar` holds the sidecar's
/// alike.
pub fn json_of(status: &Status) -> serde_json::Value {
    let mut said = serde_json::json!({
        "release": match &status.release {
            Ok(r) => serde_json::json!({
                "release": r.name, "revision": r.revision, "status": r.status,
                "chart": r.chart, "app_version": r.app_version, "updated": r.updated,
                "description": r.description,
            }),
            Err(why) => serde_json::json!({ "not_read": why }),
        },
        "components": status.components.iter().map(component_json).collect::<Vec<_>>(),
        "plugins": status.plugins.iter().map(component_json).collect::<Vec<_>>(),
        "metrics": status.memory_read.is_some(),
        "stopped": status.stopped.as_ref().map(|s| serde_json::json!({ "since": s.since })),
        "memory_read": status.memory_read.map(MemoryRead::word),
        "book_health": match &status.accounts {
            Ok(accounts) => serde_json::json!(accounts.iter().map(|a| serde_json::json!({
                "account_id": a.account_id, "journal_entries": a.journal_entries,
                "entries_since_snapshot": a.replay, "last_snapshot_at_ns": a.snapshot_at_ns,
            })).collect::<Vec<_>>()),
            Err(why) => serde_json::json!({ "not_read": why }),
        },
    });
    // Step 3b's, beside the rest: additive, so a reader of the older shape
    // reads what it read.
    if let (Some(all), serde_json::Value::Object(more)) = (
        said.as_object_mut(),
        resilience::json_of(&status.resilience),
    ) {
        all.extend(more);
    }
    said
}

/// `10m`, `1h`, `30s`, `2d`: a span kubectl's `--since` takes.
pub fn is_since(given: &str) -> bool {
    let digits = given.trim_end_matches(['s', 'm', 'h', 'd']);
    given.len() > digits.len()
        && given.len() == digits.len() + 1
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
}

/// A span as kubectl's `--since` takes it: Go's durations, which stop at
/// hours, so days (`1d`, which the help offers) are said in hours (`24h`).
/// Asked only of a span `is_since` passed.
pub fn kubectl_since(given: &str) -> String {
    match given.strip_suffix('d').map(str::parse::<u64>) {
        Some(Ok(days)) => format!("{}h", days.saturating_mul(24)),
        _ => given.to_string(),
    }
}

/// What `meridian logs` was asked beside the component.
#[derive(Debug, Clone, Default)]
pub struct Read {
    pub since: Option<String>,
    pub previous: bool,
}

/// A Job's component: its label where it has one, else its name past the
/// release's own prefix, a revision's number dropped (`<prefix>-migrate-12`
/// is `migrate`).
fn job_component(job: &serde_json::Value, prefix: Option<&str>) -> Option<String> {
    if let Some(labelled) = job["metadata"]["labels"]["meridian.dev/component"].as_str() {
        return Some(labelled.to_string());
    }
    let name = job["metadata"]["name"].as_str()?;
    let past = name.strip_prefix(&format!("{}-", prefix?))?;
    let named = match past.rsplit_once('-') {
        Some((head, revision)) if revision.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => past,
    };
    (!named.is_empty()).then(|| named.to_string())
}

/// `meridian logs <component>`: what the component's pods printed, each line
/// under its pod's name; a plugin's, by its instance; a Job's, its newest
/// run's. Anything else is refused, naming what is the deployment's.
pub async fn logs(
    cluster: &dyn Cluster,
    asked: &Asked,
    component: &str,
    read: &Read,
) -> Result<String, String> {
    let ns = asked.namespace.as_str();
    if let Some(since) = &read.since {
        if !is_since(since) {
            return Err(format!(
                "--since takes a span, as 30s, 10m, 2h or 1d, not `{since}`"
            ));
        }
    }
    let workloads = json(
        cluster, // S9: kubectl get deployments; kubectl get statefulsets
        &[
            "get",
            "deployments,statefulsets",
            "-n",
            ns,
            "-l",
            &selector(&asked.release),
            "-o",
            "json",
        ],
    )
    .await?;
    let items: Vec<&serde_json::Value> = workloads["items"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let label =
        |w: &serde_json::Value, key: &str| w["metadata"]["labels"][key].as_str().map(String::from);
    let mut known: Vec<String> = items
        .iter()
        .filter_map(|w| component_of(w).map(String::from))
        .filter(|name| name != SIDECAR)
        .collect();
    let instances: Vec<String> = items
        .iter()
        .filter(|w| component_of(w) == Some(SIDECAR))
        .filter_map(|w| label(w, "meridian.dev/instance"))
        .collect();
    // The release's own prefix, from a component's workload name.
    let prefix = items.iter().find_map(|w| {
        let name = w["metadata"]["name"].as_str()?;
        let component = component_of(w)?;
        name.strip_suffix(&format!("-{component}"))
            .map(String::from)
    });
    let chosen = if known.iter().any(|name| name == component) {
        format!(
            "app.kubernetes.io/instance={},meridian.dev/component={component}",
            asked.release
        )
    } else if instances.iter().any(|name| name == component) {
        format!(
            "app.kubernetes.io/instance={},meridian.dev/instance={component}",
            asked.release
        )
    } else {
        // The release's Jobs, by component, the newest run of each.
        let jobs = json(
            cluster, // S9: kubectl get jobs
            &[
                "get",
                "jobs",
                "-n",
                ns,
                "-l",
                &format!("app.kubernetes.io/instance={}", asked.release),
                "-o",
                "json",
            ],
        )
        .await
        .unwrap_or_default();
        let mut runs: Vec<(String, String, String)> = jobs["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|job| {
                Some((
                    job_component(job, prefix.as_deref())?,
                    job["metadata"]["creationTimestamp"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    job["metadata"]["name"].as_str()?.to_string(),
                ))
            })
            .collect();
        runs.sort();
        known.extend(runs.iter().map(|(name, _, _)| name.clone()));
        match runs.iter().rev().find(|(name, _, _)| name == component) {
            Some((_, _, job)) => format!("job-name={job}"),
            None => {
                known.sort();
                known.dedup();
                let mut said = format!(
                    "`{component}` is not a component of {} in {ns}: {}",
                    asked.release,
                    known.join(", ")
                );
                if !instances.is_empty() {
                    let mut instances = instances;
                    instances.sort();
                    said.push_str(&format!("; its plugins: {}", instances.join(", ")));
                }
                return Err(said);
            }
        }
    };
    let since_flag = read
        .since
        .as_ref()
        .map(|s| format!("--since={}", kubectl_since(s)));
    let mut arguments = vec![
        "logs",
        "-n",
        ns,
        "-l",
        chosen.as_str(),
        "--all-containers",
        "--prefix",
        "--tail=-1",
        "--max-log-requests=20",
    ];
    if read.previous {
        arguments.push("--previous");
    }
    if let Some(flag) = &since_flag {
        arguments.push(flag);
    }
    // S9: kubectl logs
    cluster.kubectl(&arguments).await.map_err(|failed| {
        if read.previous && failed.contains("not found") {
            format!(
                "no container of {component} has restarted, so there is nothing from before a \
                 restart to print: `meridian logs {component}` prints what it says now"
            )
        } else {
            failed
        }
    })
}

#[cfg(test)]
pub(crate) mod tests;

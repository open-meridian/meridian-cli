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

/// The cluster, as these commands read it.
#[async_trait::async_trait]
pub trait Cluster: Send + Sync {
    /// kubectl, with `arguments`: what it printed, or why it failed.
    async fn kubectl(&self, arguments: &[&str]) -> Result<String, String>;
    /// helm, read-only here: what it printed, or why it failed.
    async fn helm(&self, arguments: &[&str]) -> Result<String, String>;
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
    pub restarts: u64,
    /// The last restart's reason and when it ended, where one is known.
    pub last_reason: String,
    pub last_at: String,
    /// Bytes in use by its busiest pod, where the cluster serves metrics,
    /// and the limit each pod has.
    pub memory: Option<u64>,
    pub limit: Option<u64>,
    /// Each of its pods, by name, with the bytes it uses where known.
    pub pods: Vec<(String, Option<u64>)>,
    /// A plugin's: its instance, the plugin's own image (the container
    /// that is not its sidecar) and whether the launcher launched it.
    pub instance: Option<String>,
    pub plugin_image: Option<String>,
    pub launched: bool,
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

/// The components among the workloads and pods kubectl listed, with the
/// pods' metrics where the cluster gave them.
pub fn components(
    workloads: &serde_json::Value,
    pods: &serde_json::Value,
    metrics: Option<&serde_json::Value>,
) -> Vec<Component> {
    let mut usage: BTreeMap<(String, String), u64> = BTreeMap::new();
    for pod in metrics
        .and_then(|m| m["items"].as_array())
        .into_iter()
        .flatten()
    {
        let name = pod["metadata"]["name"].as_str().unwrap_or_default();
        for container in pod["containers"].as_array().into_iter().flatten() {
            if let Some(bytes) = container["usage"]["memory"].as_str().and_then(bytes_of) {
                usage.insert(
                    (
                        name.to_string(),
                        container["name"].as_str().unwrap_or_default().to_string(),
                    ),
                    bytes,
                );
            }
        }
    }
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
        let limit: u64 = containers
            .iter()
            .filter_map(|c| {
                c["resources"]["limits"]["memory"]
                    .as_str()
                    .and_then(bytes_of)
            })
            .sum();
        component.limit = (limit > 0).then_some(limit);
        let mut last: Option<(String, String)> = None;
        for pod in pods["items"].as_array().into_iter().flatten() {
            if !selects(workload, pod, name) {
                continue;
            }
            let pod_name = pod["metadata"]["name"].as_str().unwrap_or_default();
            let mut used: Option<u64> = None;
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
                let container = status["name"].as_str().unwrap_or_default().to_string();
                if let Some(bytes) = usage.get(&(pod_name.to_string(), container)) {
                    used = Some(used.unwrap_or(0) + bytes);
                }
            }
            component.pods.push((pod_name.to_string(), used));
        }
        component.pods.sort();
        component.memory = component.pods.iter().filter_map(|(_, used)| *used).max();
        if let Some((reason, at)) = last {
            component.last_reason = reason;
            component.last_at = at;
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
    let listed = cluster
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
    /// Why memory in use is not said, where the cluster serves no metrics.
    pub no_metrics: Option<String>,
    pub accounts: Result<Vec<Account>, String>,
}

pub async fn status(cluster: &dyn Cluster, asked: &Asked) -> Result<Status, String> {
    let ns = asked.namespace.as_str();
    let selector = selector(&asked.release);
    let workloads = json(
        cluster,
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
        cluster,
        &["get", "pods", "-n", ns, "-l", &selector, "-o", "json"],
    )
    .await?;
    let (metrics, no_metrics) = match json(
        cluster,
        &[
            "get",
            "--raw",
            &format!("/apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods"),
        ],
    )
    .await
    {
        Ok(metrics) => (Some(metrics), None),
        Err(_) => (
            None,
            Some(
                "memory in use is not known: the cluster serves no metrics (metrics-server)"
                    .to_string(),
            ),
        ),
    };
    let (plugins, components): (Vec<Component>, Vec<Component>) =
        components(&workloads, &pods, metrics.as_ref())
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
    let release = match cluster
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
    Ok(Status {
        release,
        components,
        plugins,
        no_metrics,
        // Read from the deployment by the caller (`book_health`), which
        // holds the session; the cluster has no part in it.
        accounts: Err("not read".into()),
    })
}

/// A component's memory as a person reads it: each pod's in use against
/// the limit each pod has.
fn memory_said(c: &Component) -> String {
    let used: Vec<u64> = c.pods.iter().filter_map(|(_, used)| *used).collect();
    let each = if used.len() > 1 { " each" } else { "" };
    let listed = used
        .iter()
        .map(|bytes| said_bytes(*bytes))
        .collect::<Vec<_>>()
        .join(", ");
    match (used.is_empty(), c.limit) {
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

/// The release, as a line, and Helm's words where it is not deployed.
fn release_said(release: &Result<Release, String>, namespace: &str) -> String {
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
                if r.updated.is_empty() {
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

/// The status, for a person to read.
pub fn said(status: &Status, now_ns: u64, namespace: &str) -> String {
    let mut out = release_said(&status.release, namespace);
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
    if let Some(why) = &status.no_metrics {
        out.push_str(&format!("  ({why})\n"));
    }
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
            "  {:width$}  {}/{} ready  {}  {}  {}{}\n",
            named(c),
            c.ready,
            c.wanted,
            restarts_said(c),
            memory_said(c),
            c.plugin_image
                .clone()
                .unwrap_or_else(|| "a sidecar alone".into()),
            if c.launched { "" } else { "  (the chart's)" },
        ));
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
fn ago(ns: u64) -> String {
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
    if let Some(instance) = &c.instance {
        said["instance_id"] = instance.clone().into();
        said["plugin_image"] = c.plugin_image.clone().into();
        said["launched"] = c.launched.into();
    }
    said
}

/// The status as one JSON object, for a script or an agent. `memory_bytes`
/// is the busiest pod's, against `memory_limit_bytes`, each pod's limit;
/// `pods` holds each pod's own.
pub fn json_of(status: &Status) -> serde_json::Value {
    serde_json::json!({
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
        "metrics": status.no_metrics.is_none(),
        "book_health": match &status.accounts {
            Ok(accounts) => serde_json::json!(accounts.iter().map(|a| serde_json::json!({
                "account_id": a.account_id, "journal_entries": a.journal_entries,
                "entries_since_snapshot": a.replay, "last_snapshot_at_ns": a.snapshot_at_ns,
            })).collect::<Vec<_>>()),
            Err(why) => serde_json::json!({ "not_read": why }),
        },
    })
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
        cluster,
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
            cluster,
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
mod tests;

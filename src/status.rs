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
//! **`meridian logs <component> [--since <duration>]`** prints what a
//! component's pods printed, each line under its pod's name.
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
}

pub struct ThisCluster;

#[async_trait::async_trait]
impl Cluster for ThisCluster {
    async fn kubectl(&self, arguments: &[&str]) -> Result<String, String> {
        let output = tokio::process::Command::new("kubectl")
            .args(arguments)
            .output()
            .await
            .map_err(|failed| match failed.kind() {
                std::io::ErrorKind::NotFound => {
                    "kubectl is not installed here; `meridian doctor` says what a deployment's \
                     machine needs"
                        .to_string()
                }
                _ => format!("kubectl could not be run: {failed}"),
            })?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(format!(
                "`kubectl {}`: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

/// What a status is asked for: the release and its namespace.
pub struct Asked {
    pub release: String,
    pub namespace: String,
}

fn selector(release: &str) -> String {
    format!("app.kubernetes.io/instance={release},meridian.dev/component")
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
    /// Bytes in use, where the cluster serves metrics, and the limit.
    pub memory: Option<u64>,
    pub limit: Option<u64>,
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
        let Some(name) = labels["meridian.dev/component"].as_str() else {
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
        let mut used = None;
        let mut last: Option<(String, String)> = None;
        for pod in pods["items"].as_array().into_iter().flatten() {
            if pod["metadata"]["labels"]["meridian.dev/component"].as_str() != Some(name) {
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
                let container = status["name"].as_str().unwrap_or_default().to_string();
                if let Some(bytes) = usage.get(&(pod_name.to_string(), container)) {
                    used = Some(used.unwrap_or(0) + bytes);
                }
            }
        }
        component.memory = used;
        if let Some((reason, at)) = last {
            component.last_reason = reason;
            component.last_at = at;
        }
        found.push(component);
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
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

/// Everything `status` reads.
pub struct Status {
    pub components: Vec<Component>,
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
    let components = components(&workloads, &pods, metrics.as_ref());
    if components.is_empty() {
        return Err(format!(
            "no deployment `{}` in {ns}: `meridian up` installs one, and -n and --release name \
             another",
            asked.release
        ));
    }
    Ok(Status {
        components,
        no_metrics,
        // Read from the deployment by the caller (`book_health`), which
        // holds the session; the cluster has no part in it.
        accounts: Err("not read".into()),
    })
}

/// The status, for a person to read.
pub fn said(status: &Status, now_ns: u64) -> String {
    let width = status
        .components
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(0);
    let mut out = String::from("Components:\n");
    for c in &status.components {
        let memory = match (c.memory, c.limit) {
            (Some(used), Some(limit)) => format!(
                "memory {} of {} ({}%)",
                said_bytes(used),
                said_bytes(limit),
                used * 100 / limit.max(1)
            ),
            (Some(used), None) => format!("memory {} (no limit)", said_bytes(used)),
            (None, Some(limit)) => format!("limit {}", said_bytes(limit)),
            (None, None) => "no limit".into(),
        };
        let restarts = if c.restarts == 0 {
            "no restarts".to_string()
        } else if c.last_reason.is_empty() {
            format!("{} restart(s)", c.restarts)
        } else {
            format!(
                "{} restart(s), the last {} at {}",
                c.restarts, c.last_reason, c.last_at
            )
        };
        out.push_str(&format!(
            "  {:width$}  {}/{} ready  {}  {}  {}\n",
            c.name,
            c.ready,
            c.wanted,
            restarts,
            memory,
            c.images.join(", "),
        ));
    }
    if let Some(why) = &status.no_metrics {
        out.push_str(&format!("  ({why})\n"));
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

/// The status as one JSON object, for a script or an agent.
pub fn json_of(status: &Status) -> serde_json::Value {
    serde_json::json!({
        "components": status.components.iter().map(|c| serde_json::json!({
            "component": c.name, "kind": c.kind, "images": c.images,
            "ready": c.ready, "wanted": c.wanted, "restarts": c.restarts,
            "last_restart_reason": c.last_reason, "last_restart_at": c.last_at,
            "memory_bytes": c.memory, "memory_limit_bytes": c.limit,
        })).collect::<Vec<_>>(),
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

/// `meridian logs <component>`: what the component's pods printed, each line
/// under its pod's name. A component that is not the deployment's is
/// refused, naming those that are.
pub async fn logs(
    cluster: &dyn Cluster,
    asked: &Asked,
    component: &str,
    since: Option<&str>,
) -> Result<String, String> {
    let ns = asked.namespace.as_str();
    if let Some(since) = since {
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
    let known: Vec<String> = workloads["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            w["metadata"]["labels"]["meridian.dev/component"]
                .as_str()
                .map(String::from)
        })
        .collect();
    if !known.iter().any(|name| name == component) {
        let mut known = known;
        known.sort();
        known.dedup();
        return Err(format!(
            "`{component}` is not a component of {} in {ns}: {}",
            asked.release,
            known.join(", ")
        ));
    }
    let chosen = format!(
        "app.kubernetes.io/instance={},meridian.dev/component={component}",
        asked.release
    );
    let since_flag = since.map(|s| format!("--since={s}"));
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
    if let Some(flag) = &since_flag {
        arguments.push(flag);
    }
    cluster.kubectl(&arguments).await
}

#[cfg(test)]
mod tests;

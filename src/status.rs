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
//! answer it -- and when that snapshot was taken (W9.10). Read-only: it
//! changes nothing.
//!
//! **`meridian logs <component> [--since <duration>]`** prints what a
//! component's pods printed, each line under its pod's name.
//!
//! Both act through the administrator's own kubectl, as `doctor`, `up` and
//! `backup` do: the chart says what runs, and this reads it.

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

/// One account's book health: how long its replay is, and its last
/// snapshot's time where it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub account_id: String,
    pub replay: u64,
    pub snapshot_at_ns: Option<u64>,
}

/// Where core keeps its snapshots, and what of them is read. PLACEHOLDER
/// (CLI 0.1.46): core's v25 has not built its snapshots yet (W9.10, "core's
/// own and no record kind"); the table and its columns are aligned with
/// core's before this is pushed. Until a deployment has it, every account's
/// replay is its whole journal, and is said so.
pub const SNAPSHOT_TABLE: &str = "book_snapshot";

/// The query reading book health from the book's database: per account, the
/// entries after its last snapshot, and when that was taken.
pub fn health_query(with_snapshots: bool) -> String {
    if with_snapshots {
        format!(
            "SELECT e.account_id, count(*) FILTER (WHERE s.last_sequence IS NULL OR \
             e.first_sequence > s.last_sequence), coalesce(max(s.taken_at_ns), 0) \
             FROM book_entry e LEFT JOIN (SELECT account_id, max(last_sequence) AS \
             last_sequence, max(taken_at_ns) AS taken_at_ns FROM {SNAPSHOT_TABLE} GROUP BY \
             account_id) s USING (account_id) GROUP BY e.account_id ORDER BY 2 DESC, 1"
        )
    } else {
        "SELECT account_id, count(*), 0 FROM book_entry GROUP BY account_id ORDER BY 2 DESC, 1"
            .into()
    }
}

/// psql's `-At` answer, `account|replay|taken_at_ns` a line, as accounts.
pub fn accounts_of(printed: &str, with_snapshots: bool) -> Vec<Account> {
    printed
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('|');
            let account_id = parts.next()?.trim().to_string();
            let replay = parts.next()?.trim().parse().ok()?;
            let taken: u64 = parts
                .next()
                .and_then(|t| t.trim().parse().ok())
                .unwrap_or(0);
            (!account_id.is_empty()).then_some(Account {
                account_id,
                replay,
                snapshot_at_ns: (with_snapshots && taken > 0).then_some(taken),
            })
        })
        .collect()
}

/// Book health, read in the database this chart brings: the database that
/// holds the book's journal, its accounts, and whether core's snapshots are
/// there. A deployment on a database somebody else runs is said so.
pub async fn book_health(cluster: &dyn Cluster, asked: &Asked) -> Result<Vec<Account>, String> {
    let ns = asked.namespace.as_str();
    let pods = json(
        cluster,
        &[
            "get",
            "pods",
            "-n",
            ns,
            "-l",
            &format!(
                "app.kubernetes.io/instance={},meridian.dev/component=database",
                asked.release
            ),
            "-o",
            "json",
        ],
    )
    .await?;
    let Some(pod) = pods["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|pod| pod["status"]["phase"] == "Running")
        .and_then(|pod| pod["metadata"]["name"].as_str())
    else {
        return Err(
            "the book's database is not the one this chart brings, or it is not running, so its \
             health is not read here"
                .into(),
        );
    };
    let psql = |database: String, query: String| async move {
        cluster
            .kubectl(&[
                "exec", "-n", ns, pod, "--", "psql", "-X", "-At", "-U", "postgres", "-d",
                &database, "-c", &query,
            ])
            .await
    };
    let databases = psql(
        "postgres".into(),
        "SELECT datname FROM pg_database WHERE NOT datistemplate ORDER BY 1".into(),
    )
    .await?;
    for database in databases.lines().map(str::trim).filter(|d| !d.is_empty()) {
        let held = psql(
            database.to_string(),
            format!(
                "SELECT to_regclass('book_entry') IS NOT NULL, to_regclass('{SNAPSHOT_TABLE}') IS \
                 NOT NULL"
            ),
        )
        .await
        .unwrap_or_default();
        let mut flags = held.trim().split('|');
        if flags.next() != Some("t") {
            continue;
        }
        let snapshots = flags.next() == Some("t");
        let printed = psql(database.to_string(), health_query(snapshots)).await?;
        return Ok(accounts_of(&printed, snapshots));
    }
    Ok(Vec::new())
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
        accounts: book_health(cluster, asked).await,
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
                    "  {:width$}  replays {} entries, {snapshot}\n",
                    account.account_id, account.replay
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
                "account_id": a.account_id, "replay_entries": a.replay,
                "snapshot_at_ns": a.snapshot_at_ns,
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

//! What `meridian status` says of the database and of integrity at rest
//! (step 3b; spec/a-deployment-scales-out-and-fails-over-by-configuration
//! sections 4, 9 and 11): read from what core exposes, never asked of the
//! database itself.
//!
//! - **The database's members**, from its StatefulSet's pods: each one's
//!   role as Patroni labels it (`meridian.dev/database-role`, primary or
//!   standby), whether it is ready, and how far behind the primary a standby
//!   is, from Patroni's own record on its pod. A member on a node that is
//!   not ready waits for its node and is never force-deleted; that is said.
//! - **Each store member's probe** (`meridian-<component> probe`, run in its
//!   pod): its mode, lag and readiness, and its database path's
//!   `data_checksums` and `full_page_writes`, the WAL archiver's last
//!   segment and whether it is failing, and the store's last scrub with its
//!   unanswered findings, the replica comparison's among them.
//! - **The replica comparison's last run**, the database-scrub Job's answer.
//! - **Whether internal TLS is on**: the components mount its CA.
//! - **A stop an act made**: while `meridian restore --to-time` or
//!   `upgrade-deployment --enable-checksums` has everything stopped, its
//!   record on the database's StatefulSet (`meridian.dev/stopped-for-
//!   restore`): which act, since when, and the command that resumes it.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::{Asked, Cluster};
use crate::offline::{members, stopped_on, Member, Stopped};
use crate::upgrade_deployment::{database_set_of, DatabaseSet};

/// One store member's probe.
#[derive(Debug, Clone, PartialEq)]
pub struct Probed {
    pub component: String,
    pub pod: String,
    /// What it wrote of itself: mode, lag, partitions, ready.
    pub member: Value,
    /// Its database's path, where it has one.
    pub database: Option<Value>,
    /// Why it could not be read, where it could not.
    pub unread: Option<String>,
}

/// The brought database, as its StatefulSet and pods say.
#[derive(Debug, Clone, PartialEq)]
pub struct Database {
    pub set: DatabaseSet,
    pub members: Vec<Member>,
    /// Each standby's bytes behind the primary, from Patroni's record on
    /// its pod, by member.
    pub behind: BTreeMap<String, u64>,
    /// Members whose node is not ready, with the node's name.
    pub waiting_for_node: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resilience {
    /// The database this deployment brought, where it brought one.
    pub database: Option<Database>,
    pub probes: Vec<Probed>,
    /// The replica comparison's last answer, where a run of it is kept.
    pub replicas: Option<Value>,
    pub internal_tls: bool,
    /// What an act stopped, while its record is on the database's
    /// StatefulSet.
    pub stopped: Option<Stopped>,
    /// `-n <namespace>`, and `--release` where it is not the default: for
    /// the commands said.
    pub scope: String,
}

impl Resilience {
    /// The database path of the first probe that reached it.
    pub fn database_path(&self) -> Option<&Value> {
        self.probes
            .iter()
            .filter_map(|probed| probed.database.as_ref())
            .find(|path| path["ok"] == true)
    }

    /// The archiver's account, from the probe that reached the primary.
    pub fn archive(&self) -> Option<&Value> {
        self.probes
            .iter()
            .filter_map(|probed| probed.database.as_ref())
            .find(|path| path["archive"].is_object())
            .map(|path| &path["archive"])
    }
}

/// How far each standby is behind the primary, in bytes, from Patroni's
/// record on each member's pod (`metadata.annotations.status`, its
/// `xlog_location`).
pub fn behind(pods: &Value, members: &[Member]) -> BTreeMap<String, u64> {
    let location = |name: &str| -> Option<u64> {
        let pod = pods["items"]
            .as_array()?
            .iter()
            .find(|pod| pod["metadata"]["name"] == name)?;
        let said: Value =
            serde_json::from_str(pod["metadata"]["annotations"]["status"].as_str()?).ok()?;
        said["xlog_location"].as_u64()
    };
    let Some(primary) = members
        .iter()
        .find(|m| m.role.as_deref() == Some("primary"))
        .and_then(|m| location(&m.name))
    else {
        return BTreeMap::new();
    };
    members
        .iter()
        .filter(|m| m.role.as_deref() == Some("standby"))
        .filter_map(|m| Some((m.name.clone(), primary.saturating_sub(location(&m.name)?))))
        .collect()
}

/// Each node's readiness, by name, from `kubectl get nodes -o json`.
pub fn nodes_ready(nodes: &Value) -> BTreeMap<String, bool> {
    nodes["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            let ready = node["status"]["conditions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|c| c["type"] == "Ready" && c["status"] == "True");
            Some((node["metadata"]["name"].as_str()?.to_string(), ready))
        })
        .collect()
}

/// Whether the components mount internal TLS's CA: it is on.
pub fn internal_tls(workloads: &Value) -> bool {
    workloads["items"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|w| {
            w["spec"]["template"]["spec"]["volumes"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .any(|volume| {
            volume["name"] == "internal-tls-ca" || volume["name"] == "internal-tls-server"
        })
}

/// The replica comparison's answer in its Job's log: the object naming
/// `replicas`.
pub fn replicas_answer(log: &str) -> Option<Value> {
    log.lines()
        .rev()
        .find_map(|line| crate::probe::first_json(line).filter(|said| said["replicas"].is_string()))
}

/// Everything above, read.
pub async fn read(
    cluster: &dyn Cluster,
    asked: &Asked,
    workloads: &Value,
    pods: &Value,
) -> Resilience {
    let ns = asked.namespace.as_str();
    let mut found = Resilience {
        internal_tls: internal_tls(workloads),
        scope: super::scope(&asked.release, &asked.namespace),
        ..Resilience::default()
    };
    if let Some(set) = database_set_of(workloads) {
        found.stopped = workloads["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| item["kind"] == "StatefulSet" && item["metadata"]["name"] == set.name)
            .and_then(stopped_on);
        let members = members(pods, &set.name);
        let ready_nodes = match cluster.kubectl(&["get", "nodes", "-o", "json"]).await {
            Ok(said) => nodes_ready(&serde_json::from_str(&said).unwrap_or_default()),
            Err(_) => BTreeMap::new(),
        };
        let waiting_for_node = members
            .iter()
            .filter_map(|m| {
                let node = m.node.as_ref()?;
                (ready_nodes.get(node) == Some(&false)).then(|| (m.name.clone(), node.clone()))
            })
            .collect();
        found.database = Some(Database {
            behind: behind(pods, &members),
            members,
            set,
            waiting_for_node,
        });
    }
    // Each store member's probe, from inside its pod.
    for pod in pods["items"].as_array().into_iter().flatten() {
        let component = pod["metadata"]["labels"]["meridian.dev/component"]
            .as_str()
            .unwrap_or_default();
        let name = pod["metadata"]["name"].as_str().unwrap_or_default();
        if !crate::probe::PROBED.contains(&component)
            || pod["status"]["phase"] != "Running"
            || !pod["metadata"]["deletionTimestamp"].is_null()
            || !pod["metadata"]["ownerReferences"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|owner| owner["kind"] != "Job")
        {
            continue;
        }
        // An older runtime takes `probe` for nothing and starts a second
        // copy of the component: never asked of one.
        if !crate::probe::answers(pod) {
            found.probes.push(Probed {
                component: component.to_string(),
                pod: name.to_string(),
                member: Value::Null,
                database: None,
                unread: Some("its runtime predates the probe (step 3b)".into()),
            });
            continue;
        }
        let arguments = crate::probe::exec_arguments(ns, name, component);
        let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let said = cluster.kubectl_printed(&arguments).await;
        let probe = said.as_deref().ok().and_then(crate::probe::first_json);
        found.probes.push(match probe {
            Some(probe) => Probed {
                component: component.to_string(),
                pod: name.to_string(),
                member: probe["member"].clone(),
                database: crate::probe::database(&probe).cloned(),
                unread: None,
            },
            None => Probed {
                component: component.to_string(),
                pod: name.to_string(),
                member: Value::Null,
                database: None,
                unread: Some(match said {
                    Ok(_) => "its probe printed nothing readable: its runtime may predate step 3b"
                        .to_string(),
                    Err(why) => why.lines().next().unwrap_or_default().to_string(),
                }),
            },
        });
    }
    found
        .probes
        .sort_by(|a, b| (&a.component, &a.pod).cmp(&(&b.component, &b.pod)));
    // The replica comparison's newest run, where one is kept.
    let selector = format!(
        "app.kubernetes.io/instance={},meridian.dev/component=database-scrub",
        asked.release
    );
    if let Ok(listed) = cluster
        .kubectl(&["get", "jobs", "-n", ns, "-l", &selector, "-o", "json"])
        .await
    {
        let listed: Value = serde_json::from_str(&listed).unwrap_or_default();
        let newest = listed["items"]
            .as_array()
            .into_iter()
            .flatten()
            .max_by_key(|job| {
                job["metadata"]["creationTimestamp"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .and_then(|job| job["metadata"]["name"].as_str().map(String::from));
        if let Some(job) = newest {
            if let Ok(log) = cluster
                .kubectl_printed(&["logs", "-n", ns, &format!("job/{job}"), "--tail=200"])
                .await
            {
                found.replicas = replicas_answer(&log).map(|mut answer| {
                    answer["job"] = json!(job);
                    answer
                });
            }
        }
    }
    found
}

/// A time in nanoseconds since the epoch as a person reads it, and how long
/// ago from `now_ns`.
fn when(at_ns: u64, now_ns: u64) -> String {
    format!(
        "{} ({} ago)",
        crate::registry::moment(at_ns),
        super::ago(now_ns.saturating_sub(at_ns))
    )
}

/// Said first, while an act has the deployment stopped: which, since when,
/// and what resumes it. Nothing where none has.
pub fn stopped_said(resilience: &Resilience) -> String {
    let Some(stopped) = &resilience.stopped else {
        return String::new();
    };
    let suspended: Vec<&str> = stopped
        .cronjobs
        .iter()
        .filter(|(_, suspended)| !suspended)
        .map(|(name, _)| name.as_str())
        .collect();
    let mut out = format!(
        "\nStopped by `{}`{}:\n  every component and the database stay stopped",
        stopped.command(),
        stopped
            .since
            .as_deref()
            .map(|at| format!(" since {at}"))
            .unwrap_or_default()
    );
    if !suspended.is_empty() {
        out.push_str(&format!(
            ", and {} CronJob(s) suspended ({})",
            suspended.len(),
            suspended.join(", ")
        ));
    }
    out.push_str(&format!(
        ", until it finishes. While it runs, `meridian logs database-offline {scope}` says where \
         its act is. If it no longer runs, `{}` takes it from there and starts everything again.\n",
        stopped.resumed_by(&resilience.scope),
        scope = resilience.scope
    ));
    out
}

/// The sections `status` prints for all of it.
pub fn said(resilience: &Resilience, now_ns: u64, namespace: &str) -> String {
    let mut out = String::new();
    out.push_str("\nDatabase:\n");
    let brought = resilience.database.is_some();
    match &resilience.database {
        Some(db) => {
            let mut shape = vec![format!(
                "{} member(s){}",
                db.set.replicas,
                if db.set.patroni {
                    ", under Patroni"
                } else {
                    ""
                }
            )];
            shape.push(if db.set.archive {
                "WAL archive on".into()
            } else {
                "WAL archive off".into()
            });
            out.push_str(&format!("  {}  {}\n", db.set.name, shape.join(", ")));
            if db.members.is_empty() {
                out.push_str("    no member running\n");
            }
            for member in &db.members {
                let role = member.role.clone().unwrap_or_else(|| "alone".to_string());
                let mut line = format!(
                    "    {}  {role}  {}",
                    member.name,
                    if member.ready { "ready" } else { "not ready" }
                );
                if let Some(bytes) = db.behind.get(&member.name) {
                    line.push_str(&format!("  {} behind the primary", bytes_said(*bytes)));
                }
                if let Some(node) = db.waiting_for_node.get(&member.name) {
                    line.push_str(&format!(
                        "\n      waits for its node {node}, which is not ready: a database \
                         member is never force-deleted, and starts again on its own volume when \
                         the node returns"
                    ));
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
        None => {
            out.push_str("  none brought: the deployment's database is one somebody else runs\n")
        }
    }
    match resilience.database_path() {
        Some(path) => {
            let setting = |name: &str| crate::probe::setting(path, name).unwrap_or("not read");
            out.push_str(&format!(
                "  page checksums {}, full_page_writes {}{}\n",
                setting("data_checksums"),
                setting("full_page_writes"),
                path["address"]
                    .as_str()
                    .filter(|_| !brought)
                    .map(|at| format!(", at {at}"))
                    .unwrap_or_default()
            ));
            for hint in crate::probe::integrity_hints(path, brought, namespace) {
                out.push_str(&format!("  {}\n", hint.to_string().trim_start()));
            }
        }
        None => out.push_str("  its settings not read: no member's probe reached it\n"),
    }
    if let Some(archive) = resilience.archive() {
        let last = archive["last_archived_at"].as_str().unwrap_or_default();
        let ago = archive["seconds_since"]
            .as_u64()
            .map(|s| format!(" ({s}s ago)"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  WAL archive: {}{}\n",
            if last.is_empty() {
                "no segment archived yet".to_string()
            } else {
                format!("last segment at {last}{ago}")
            },
            if archive["failing"] == true {
                format!(
                    "; FAILING now ({} failures): a restore cannot reach past its last segment",
                    archive["failed"].as_u64().unwrap_or(0)
                )
            } else {
                "; not failing".to_string()
            }
        ));
    }
    match &resilience.replicas {
        Some(answer) => {
            let state = answer["replicas"].as_str().unwrap_or_default();
            out.push_str(&format!(
                "  replica comparison: {state}, {} members, primary {} (job/{})\n",
                answer["members"].as_u64().unwrap_or(0),
                answer["primary"].as_str().unwrap_or("?"),
                answer["job"].as_str().unwrap_or("?")
            ));
            for finding in answer["findings"].as_array().into_iter().flatten() {
                out.push_str(&format!(
                    "    {}: {}; {}\n",
                    finding["member"].as_str().unwrap_or("?"),
                    finding["why"].as_str().unwrap_or_default(),
                    finding["outcome"].as_str().unwrap_or_default()
                ));
            }
        }
        None if resilience
            .database
            .as_ref()
            .is_some_and(|db| db.set.replicas > 1) =>
        {
            out.push_str("  replica comparison: no run kept yet\n")
        }
        None => {}
    }
    out.push_str(&format!(
        "  internal TLS {}\n",
        if resilience.internal_tls { "on" } else { "off" }
    ));

    out.push_str("\nIntegrity and members (each store member's probe):\n");
    if resilience.probes.is_empty() {
        out.push_str("  no store member is running to probe\n");
    }
    let width = resilience
        .probes
        .iter()
        .map(|p| p.pod.len())
        .max()
        .unwrap_or(0);
    // A store's scrub is its record's, the same on every member: said once,
    // under its writer (in ha, the primary), or the first member that read
    // it where none is the writer.
    let mut record_under: BTreeMap<&str, &str> = BTreeMap::new();
    for probed in &resilience.probes {
        let read = probed
            .database
            .as_ref()
            .is_some_and(|path| path["integrity"].is_object());
        if !read {
            continue;
        }
        let writer = probed.member["mode"] == "writer";
        record_under
            .entry(probed.component.as_str())
            .and_modify(|pod| {
                if writer {
                    *pod = probed.pod.as_str();
                }
            })
            .or_insert(probed.pod.as_str());
    }
    for probed in &resilience.probes {
        let mut line = format!("  {:width$}  ", probed.pod);
        if let Some(why) = &probed.unread {
            line.push_str(&format!("not probed: {why}"));
            out.push_str(&line);
            out.push('\n');
            continue;
        }
        let member = &probed.member;
        let mut said = Vec::new();
        if let Some(mode) = member["mode"].as_str() {
            said.push(mode.to_string());
        }
        said.push(if member["ready"] == true {
            "ready".into()
        } else {
            "not ready".into()
        });
        if let Some(lag) = member["lag_ms"].as_u64() {
            said.push(format!("lag {lag} ms"));
        }
        if let Some(path) = &probed.database {
            if path["ok"] != true {
                said.push(format!(
                    "database not reached: {}",
                    path["why"].as_str().unwrap_or("no reason given")
                ));
            }
        }
        line.push_str(&said.join(", "));
        out.push_str(&line);
        out.push('\n');
        let integrity = probed.database.as_ref().map(|path| &path["integrity"]);
        if let Some(integrity) = integrity.filter(|i| i.is_object()) {
            if record_under.get(probed.component.as_str()) != Some(&probed.pod.as_str()) {
                continue;
            }
            let findings: Vec<&Value> = integrity["findings"]
                .as_array()
                .into_iter()
                .flatten()
                .collect();
            let scrub = match integrity["last_scrub_at_ns"].as_u64() {
                Some(at) => format!(
                    "last scrub {}, {} checked",
                    when(at, now_ns),
                    integrity["checked"].as_u64().unwrap_or(0)
                ),
                None => "not scrubbed yet".to_string(),
            };
            out.push_str(&format!(
                "  {:width$}    {} record: {scrub}; {}\n",
                "",
                probed.component,
                match findings.len() {
                    0 => "no findings".to_string(),
                    n => format!("{n} finding(s) not yet answered, newest first:"),
                }
            ));
            for finding in findings {
                out.push_str(&format!(
                    "  {:width$}      {} (partition {}, sequence {})\n",
                    "",
                    finding["what"].as_str().unwrap_or_default(),
                    finding["partition"].as_str().unwrap_or("?"),
                    finding["sequence"].as_i64().unwrap_or(0)
                ));
            }
        }
    }
    out
}

fn bytes_said(bytes: u64) -> String {
    if bytes < 1024 * 1024 {
        format!("{bytes} B")
    } else {
        super::said_bytes(bytes)
    }
}

/// The same, as `--json` adds it: additive keys beside the others.
pub fn json_of(resilience: &Resilience) -> Value {
    json!({
        "database": resilience.database.as_ref().map(|db| json!({
            "statefulset": db.set.name,
            "patroni": db.set.patroni,
            "wal_archive": db.set.archive,
            "wanted": db.set.replicas,
            "members": db.members.iter().map(|m| json!({
                "pod": m.name,
                "role": m.role,
                "ready": m.ready,
                "node": m.node,
                "bytes_behind_primary": db.behind.get(&m.name),
                "waits_for_node": db.waiting_for_node.get(&m.name),
            })).collect::<Vec<_>>(),
        })),
        "database_settings": resilience.database_path().map(|path| json!({
            "data_checksums": path["data_checksums"],
            "full_page_writes": path["full_page_writes"],
            "primary": path["primary"],
        })),
        "wal_archiver": resilience.archive(),
        "replica_comparison": resilience.replicas,
        "internal_tls": resilience.internal_tls,
        "stopped_by": resilience.stopped.as_ref().map(|stopped| json!({
            "act": stopped.act_name(),
            "command": stopped.command(),
            "since": stopped.since,
            "resumed_by": stopped.resumed_by(&resilience.scope),
            "database_members": stopped.database,
            "components": stopped.deployments,
            "cronjobs_suspended": stopped.cronjobs.iter()
                .filter(|(_, suspended)| !suspended)
                .map(|(name, _)| name)
                .collect::<Vec<_>>(),
        })),
        "probes": resilience.probes.iter().map(|p| json!({
            "component": p.component,
            "pod": p.pod,
            "member": p.member,
            "integrity": p.database.as_ref().map(|d| d["integrity"].clone()),
            "not_read": p.unread,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const SET: &str = "om-meridian-runtime-database";

    /// The cluster as `status` reads it, for this section alone.
    struct Fake {
        workloads: Value,
        pods: Value,
        nodes: Value,
        probe: Value,
        replicas_log: Option<String>,
        asked: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl Cluster for Fake {
        async fn kubectl(&self, arguments: &[&str]) -> Result<String, String> {
            let joined = arguments.join(" ");
            self.asked.lock().unwrap().push(joined.clone());
            if joined == "get nodes -o json" {
                return Ok(self.nodes.to_string());
            }
            if joined.starts_with("get jobs") && joined.contains("database-scrub") {
                return Ok(match &self.replicas_log {
                    Some(_) => json!({ "items": [
                        { "metadata": { "name": "om-replicas-1", "creationTimestamp": "2026-11-08T01:00:00Z" } },
                        { "metadata": { "name": "om-replicas-2", "creationTimestamp": "2026-11-08T02:00:00Z" } } ] }),
                    None => json!({ "items": [] }),
                }
                .to_string());
            }
            Err(format!("not stood in for: {joined}"))
        }

        async fn helm(&self, _: &[&str]) -> Result<String, String> {
            Err("not asked".into())
        }

        async fn kubectl_printed(&self, arguments: &[&str]) -> Result<String, String> {
            let joined = arguments.join(" ");
            self.asked.lock().unwrap().push(joined.clone());
            if joined.ends_with("probe") {
                return Ok(format!("{}\nINFO probed\n", self.probe));
            }
            if joined.starts_with("logs -n om job/om-replicas-2") {
                return self.replicas_log.clone().ok_or_else(|| "no log".into());
            }
            Err(format!("not stood in for: {joined}"))
        }
    }

    fn member(n: u64, role: &str, node: &str, xlog: u64) -> Value {
        json!({ "metadata": { "name": format!("{SET}-{n}"),
                  "labels": { "meridian.dev/component": "database", "meridian.dev/database-role": role },
                  "annotations": { "status": json!({ "role": role, "xlog_location": xlog,
                      "conn_url": "postgres://10.0.0.1:5432/postgres" }).to_string() } },
                "spec": { "nodeName": node },
                "status": { "phase": "Running", "conditions": [
                    { "type": "Ready", "status": if node == "node-c" { "False" } else { "True" } } ] } })
    }

    fn store(name: &str, component: &str) -> Value {
        json!({ "metadata": { "name": name, "labels": { "meridian.dev/component": component },
                              "ownerReferences": [ { "kind": "ReplicaSet", "name": "rs" } ] },
                "spec": { "containers": [ { "name": component,
                    "env": [ { "name": crate::probe::MARKER, "value": "7" } ] } ] },
                "status": { "phase": "Running" } })
    }

    fn ha() -> Fake {
        Fake {
            workloads: json!({ "items": [
                { "kind": "StatefulSet", "metadata": { "name": SET },
                  "spec": { "replicas": 3,
                    "selector": { "matchLabels": { "meridian.dev/component": "database" } },
                    "template": { "metadata": { "labels": { "meridian.dev/database-manager": "patroni" } },
                                  "spec": { "containers": [ { "name": "postgres" }, { "name": "archive" } ],
                                            "volumes": [ { "name": "internal-tls-server" } ] } } } },
                { "kind": "Deployment", "metadata": { "name": "om-lake" },
                  "spec": { "template": { "spec": { "volumes": [ { "name": "internal-tls-ca" } ] } } } } ] }),
            pods: json!({ "items": [
                member(0, "primary", "node-a", 5_000_000),
                member(1, "replica", "node-b", 4_999_000),
                member(2, "replica", "node-c", 3_000_000),
                store("om-lake-1", "lake"),
                // Older than the probe: never asked it.
                json!({ "metadata": { "name": "om-bor-1", "labels": { "meridian.dev/component": "bor" } },
                        "spec": { "containers": [ { "name": "bor" } ] }, "status": { "phase": "Running" } }),
            ] }),
            nodes: json!({ "items": [
                { "metadata": { "name": "node-a" }, "status": { "conditions": [ { "type": "Ready", "status": "True" } ] } },
                { "metadata": { "name": "node-b" }, "status": { "conditions": [ { "type": "Ready", "status": "True" } ] } },
                { "metadata": { "name": "node-c" }, "status": { "conditions": [ { "type": "Ready", "status": "Unknown" } ] } } ] }),
            probe: json!({ "component": "lake", "member": { "mode": "writer", "ready": true, "lag_ms": 0 },
                "paths": [ { "path": "database", "address": "om-db:5432", "ok": true,
                    "data_checksums": "on", "full_page_writes": "on", "primary": true,
                    "archive": { "last_archived_at": "2026-11-08T10:00:00Z", "seconds_since": 20,
                                 "failed": 3, "failing": true },
                    "integrity": { "last_scrub_at_ns": 1_794_000_000_000_000_000u64, "checked": 812, "filled": 0,
                        "findings": [ { "partition": "P0", "sequence": 41, "committed_at_ns": 1,
                            "what": "member om-meridian-runtime-database-2's copy differs from the primary's" } ] } } ],
                "ok": true }),
            replicas_log: Some(
                "INFO comparing\n{\"replicas\":\"differed\",\"primary\":\"om-meridian-runtime-database-0\",\"members\":3,\"findings\":[{\"member\":\"om-meridian-runtime-database-2\",\"why\":\"lake: partition P0 differs\",\"outcome\":\"re-cloned through Patroni in 9.3 s; its copy matches the primary's again\"}]}\n".into(),
            ),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked() -> Asked {
        Asked {
            release: "om".into(),
            namespace: "om".into(),
        }
    }

    #[tokio::test]
    async fn the_databases_members_integrity_archiver_and_tls_are_said() {
        let fake = ha();
        let read = read(&fake, &asked(), &fake.workloads, &fake.pods).await;
        let text = said(&read, 1_794_003_600_000_000_000, "om");
        assert!(
            text.contains(&format!(
                "{SET}  3 member(s), under Patroni, WAL archive on"
            )),
            "{text}"
        );
        assert!(text.contains(&format!("{SET}-0  primary  ready")), "{text}");
        assert!(
            text.contains(&format!(
                "{SET}-1  standby  ready  1000 B behind the primary"
            )),
            "{text}"
        );
        assert!(
            text.contains("waits for its node node-c, which is not ready: a database member is never force-deleted"),
            "{text}"
        );
        assert!(
            text.contains("page checksums on, full_page_writes on"),
            "{text}"
        );
        assert!(text.contains("WAL archive: last segment at 2026-11-08T10:00:00Z (20s ago); FAILING now (3 failures)"), "{text}");
        assert!(text.contains("replica comparison: differed, 3 members, primary om-meridian-runtime-database-0 (job/om-replicas-2)"), "{text}");
        assert!(
            text.contains("re-cloned through Patroni in 9.3 s"),
            "{text}"
        );
        assert!(text.contains("internal TLS on"), "{text}");
        assert!(
            text.contains("om-lake-1  writer, ready, lag 0 ms"),
            "{text}"
        );
        assert!(text.contains("lake record: last scrub"), "{text}");
        assert!(
            text.contains("812 checked; 1 finding(s) not yet answered"),
            "{text}"
        );
        assert!(
            text.contains("copy differs from the primary's (partition P0, sequence 41)"),
            "{text}"
        );
        assert!(
            text.contains("om-bor-1   not probed: its runtime predates the probe (step 3b)"),
            "{text}"
        );
        assert!(!text.contains("10.0.0.1"), "{text}");
        // The older runtime was never asked to probe.
        assert!(!fake
            .asked
            .lock()
            .unwrap()
            .iter()
            .any(|c| c.contains("om-bor-1")));

        let json = json_of(&read);
        assert_eq!(json["database"]["members"][1]["role"], "standby");
        assert_eq!(json["database"]["members"][1]["bytes_behind_primary"], 1000);
        assert_eq!(json["database"]["members"][2]["waits_for_node"], "node-c");
        assert_eq!(json["database_settings"]["data_checksums"], "on");
        assert_eq!(json["wal_archiver"]["failing"], true);
        assert_eq!(json["replica_comparison"]["replicas"], "differed");
        assert_eq!(json["internal_tls"], true);
        assert_eq!(json["probes"][1]["integrity"]["checked"], 812);
    }

    #[tokio::test]
    async fn an_external_databases_settings_off_are_said_plainly_without_the_guided_step() {
        let mut fake = ha();
        fake.workloads = json!({ "items": [] });
        fake.pods = json!({ "items": [ store("om-lake-1", "lake") ] });
        fake.replicas_log = None;
        fake.probe = json!({ "component": "lake", "member": { "ready": true },
            "paths": [ { "path": "database", "address": "db.firm.example:5432", "ok": true,
                         "data_checksums": "off", "full_page_writes": "off" } ] });
        let read = read(&fake, &asked(), &fake.workloads, &fake.pods).await;
        let text = said(&read, 0, "om");
        assert!(
            text.contains("none brought: the deployment's database is one somebody else runs"),
            "{text}"
        );
        assert!(
            text.contains("page checksums off, full_page_writes off, at db.firm.example:5432"),
            "{text}"
        );
        assert!(
            text.contains("worth    the database has no page checksums (data_checksums is off)"),
            "{text}"
        );
        assert!(text.contains("full_page_writes is off"), "{text}");
        assert!(!text.contains("--enable-checksums"), "{text}");
        assert!(text.contains("internal TLS off"), "{text}");
        assert!(!text.contains("replica comparison"), "{text}");
    }

    #[tokio::test]
    async fn a_stop_an_act_recorded_is_said_with_since_when_and_what_resumes_it() {
        let mut fake = ha();
        let record = Stopped {
            deployments: vec![("om-lake".into(), 1)],
            database: 3,
            cronjobs: vec![
                ("om-backup".into(), true),
                (format!("{SET}-replicas"), false),
            ],
            act: Some("restore-to-time".into()),
            since: Some("2026-11-09T03:39:15Z".into()),
        };
        fake.workloads["items"][0]["metadata"]["annotations"] =
            json!({ crate::offline::STOPPED: record.to_json() });
        fake.pods = json!({ "items": [] });
        let read = read(&fake, &asked(), &fake.workloads, &fake.pods).await;
        let text = stopped_said(&read);
        assert!(
            text.contains("Stopped by `meridian restore --to-time` since 2026-11-09T03:39:15Z:"),
            "{text}"
        );
        assert!(
            text.contains(&format!("1 CronJob(s) suspended ({SET}-replicas)")),
            "{text}"
        );
        assert!(
            text.contains("`meridian logs database-offline --release om -n om`"),
            "{text}"
        );
        assert!(
            text.contains(
                "`meridian restore --to-time <time> --release om -n om` takes it from there"
            ),
            "{text}"
        );
        let json = json_of(&read);
        assert_eq!(json["stopped_by"]["act"], "restore-to-time");
        assert_eq!(json["stopped_by"]["since"], "2026-11-09T03:39:15Z");
        assert_eq!(
            json["stopped_by"]["resumed_by"],
            "meridian restore --to-time <time> --release om -n om"
        );
        assert_eq!(
            json["stopped_by"]["cronjobs_suspended"],
            json!([format!("{SET}-replicas")])
        );
        // Every key it had, kept.
        for key in [
            "database",
            "database_settings",
            "wal_archiver",
            "replica_comparison",
            "internal_tls",
            "probes",
        ] {
            assert!(json.get(key).is_some(), "{key}");
        }

        // By --enable-checksums: its own command resumes it.
        let record = Stopped {
            act: Some("enable-checksums".into()),
            ..record
        };
        fake.workloads["items"][0]["metadata"]["annotations"] =
            json!({ crate::offline::STOPPED: record.to_json() });
        let read = super::read(&fake, &asked(), &fake.workloads, &fake.pods).await;
        let text = stopped_said(&read);
        assert!(
            text.contains("Stopped by `meridian upgrade-deployment --enable-checksums` since"),
            "{text}"
        );
        assert!(
            text.contains("`meridian upgrade-deployment --enable-checksums --release om -n om` takes it from there"),
            "{text}"
        );

        // None recorded: nothing said, and the key is null.
        let fake = ha();
        let read = super::read(&fake, &asked(), &fake.workloads, &fake.pods).await;
        assert_eq!(stopped_said(&read), "");
        assert!(json_of(&read)["stopped_by"].is_null());
    }

    #[test]
    fn a_stores_record_is_said_under_its_writer() {
        let integrity = json!({ "ok": true, "integrity": {
            "last_scrub_at_ns": 1_794_000_000_000_000_000u64, "checked": 7, "findings": [] } });
        let probed = |pod: &str, mode: &str| Probed {
            component: "lake".into(),
            pod: pod.into(),
            member: json!({ "mode": mode, "ready": true }),
            database: Some(integrity.clone()),
            unread: None,
        };
        let resilience = Resilience {
            probes: vec![
                probed("om-lake-0", "standby"),
                probed("om-lake-1", "writer"),
                probed("om-lake-2", "standby"),
            ],
            ..Resilience::default()
        };
        let text = said(&resilience, 1_794_003_600_000_000_000, "om");
        let lines: Vec<&str> = text.lines().collect();
        let record = lines
            .iter()
            .position(|line| line.contains("lake record: last scrub"))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(lines[record - 1].contains("om-lake-1  writer"), "{text}");
        assert_eq!(text.matches("lake record:").count(), 1, "{text}");

        // None the writer: under the first that read it.
        let resilience = Resilience {
            probes: vec![
                probed("om-lake-0", "standby"),
                probed("om-lake-1", "standby"),
            ],
            ..Resilience::default()
        };
        let text = said(&resilience, 1_794_003_600_000_000_000, "om");
        let lines: Vec<&str> = text.lines().collect();
        let record = lines
            .iter()
            .position(|line| line.contains("lake record:"))
            .unwrap();
        assert!(lines[record - 1].contains("om-lake-0"), "{text}");
    }
}

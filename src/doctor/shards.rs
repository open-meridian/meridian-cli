//! What `meridian doctor` says of a plugin's shards and standby pods (step
//! 3d, checkpoint 5; spec/a-deployment-scales-out-and-fails-over-by-
//! configuration section 7):
//!
//! - **Storage that cannot take a standby** (item 30): a plugin instance that
//!   keeps storage shares it between its pods only on a shared claim. On
//!   several nodes that takes a ReadWriteMany class, the chart's
//!   `pluginStorage.shared.storageClassName`; with none named there, the
//!   conductor refuses its shards or standby pod. Nodes are counted as the
//!   conductor counts them, from where the deployment's pods run, and a
//!   count that cannot be read counts as several.
//! - **An archive's account to trust** (PO items 60 and 63): a sharded or
//!   standby instance allowed a bucket archive runs under its own account,
//!   `<fullname>-shards-<instance>`, which the bucket's workload identity has
//!   to trust as well as `pluginArchive.serviceAccount`; each with the cloud's
//!   step, as values.yaml's pluginArchive says it.
//! - **Each shard's fence** (item 32): its epoch and its last takeover, from
//!   the Resilience read's history (`ShardTookOver`, kind
//!   `shard_taken_over`, its component the instance and its partition
//!   `shard <n>`). That read is a deployment admin's, through the session
//!   `meridian connect` keeps; it is asked only where a plugin runs sharded or
//!   with a standby pod.
//!
//! Everything else is read from the namespace's own workloads and pods. It
//! changes nothing.

use std::collections::BTreeSet;

use serde_json::Value;

use super::{Failure, Finding, Machine};
use crate::upgrade_deployment::shards::shape;

/// The chart value naming the ReadWriteMany class.
pub const SHARED_CLASS: &str = "pluginStorage.shared.storageClassName";

/// On a plugin's container: its prefix of the deployment's archive bucket.
pub const ARCHIVE_BUCKET: &str = "MERIDIAN_ARCHIVE_BUCKET";

/// A history item's kind for a plugin shard's take, as the read words it.
pub const SHARD_TAKEN_OVER: &str = "shard_taken_over";

/// The history items read at most, a page of 200 at a time, looking for each
/// shard's last take.
pub const MOST_PAGES: usize = 5;
pub const PAGE_SIZE: u32 = 200;

/// The Resilience read's history, a page at a time, so a test can be the
/// deployment.
#[async_trait::async_trait]
pub trait History: Send + Sync {
    /// `GET /terminal/resilience?page_size=&cursor=`'s answer, or why it
    /// could not be read.
    async fn page(&self, page_size: u32, cursor: &str) -> Result<Value, String>;
}

/// A plugin instance as its Deployment says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub instance: String,
    /// Its shards and standby pods, where its sidecar says its shard count.
    pub shards: u64,
    pub standbys: u64,
    /// Whether it mounts storage of its own (`<fullname>-storage-<instance>`
    /// or the shared `<fullname>-shared-storage-<instance>`).
    pub keeps_storage: bool,
    /// The account its pods run under.
    pub account: String,
    /// Its prefix of the archive bucket, where it is allowed one.
    pub bucket: Option<String>,
}

impl Instance {
    /// Several pods: shards, or a standby pod.
    pub fn several(&self) -> bool {
        self.shards > 1 || self.standbys > 0
    }
}

fn containers(workload: &Value) -> impl Iterator<Item = &Value> {
    workload["spec"]["template"]["spec"]["containers"]
        .as_array()
        .into_iter()
        .flatten()
}

/// Every plugin instance's Deployment in a `kubectl get deployments -o json`
/// list: those labelled with an instance, scaled to none or not.
pub fn instances(workloads: &Value) -> Vec<Instance> {
    let mut found: Vec<Instance> = workloads["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|workload| {
            let instance = workload["metadata"]["labels"]["meridian.dev/instance"]
                .as_str()
                .filter(|named| !named.is_empty())?
                .to_string();
            let (shards, standbys) = shape(workload).unwrap_or((1, 0));
            let template = &workload["spec"]["template"]["spec"];
            let keeps_storage = template["volumes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|volume| volume["persistentVolumeClaim"]["claimName"].as_str())
                .any(|claim| claim.ends_with(&format!("-storage-{instance}")));
            let bucket = containers(workload)
                .flat_map(|container| container["env"].as_array().into_iter().flatten())
                .find(|variable| variable["name"] == ARCHIVE_BUCKET)
                .and_then(|variable| variable["value"].as_str())
                .filter(|bucket| !bucket.is_empty())
                .map(String::from);
            Some(Instance {
                instance,
                shards,
                standbys,
                keeps_storage,
                account: template["serviceAccountName"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                bucket,
            })
        })
        .collect();
    found.sort_by(|a, b| a.instance.cmp(&b.instance));
    found.dedup_by(|a, b| a.instance == b.instance);
    found
}

/// The nodes the namespace's pods run on, from `kubectl get pods -o json`:
/// None where it cannot be read, which counts as several.
pub fn nodes(pods: &Value) -> Option<usize> {
    let items = pods["items"].as_array()?;
    let on: BTreeSet<&str> = items
        .iter()
        .filter(|pod| {
            !matches!(
                pod["status"]["phase"].as_str(),
                Some("Succeeded" | "Failed")
            )
        })
        .filter_map(|pod| pod["spec"]["nodeName"].as_str())
        .filter(|node| !node.is_empty())
        .collect();
    Some(on.len())
}

/// The class the deployment names for its shared storage, from `helm get
/// values -o json`: empty for none. Nothing else is read out of the values.
pub fn shared_class(values: &Value) -> String {
    values["pluginStorage"]["shared"]["storageClassName"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn named(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Item 30: the storage-holding instances that cannot take a standby, where
/// the deployment runs on several nodes and names no ReadWriteMany class.
/// `class` is None where the values could not be read, and `nodes` None
/// where the pods could not be.
pub fn standby_storage(
    held: &[Instance],
    class: Option<&str>,
    nodes: Option<usize>,
    unread: &str,
) -> Option<Finding> {
    let keeping: Vec<&str> = held
        .iter()
        .filter(|one| one.keeps_storage)
        .map(|one| one.instance.as_str())
        .collect();
    if keeping.is_empty() {
        return None;
    }
    let plugins = format!(
        "plugin{} {}",
        if keeping.len() == 1 { "" } else { "s" },
        named(&keeping)
    );
    if nodes == Some(1) || nodes == Some(0) {
        return Some(Finding::Fine(format!(
            "{plugins} keep{} storage and can take a standby: the deployment runs on one node",
            if keeping.len() == 1 { "s" } else { "" }
        )));
    }
    let Some(class) = class else {
        return Some(Finding::Unknown {
            what: format!("whether {plugins} can take a standby"),
            why: format!("{SHARED_CLASS} could not be read: {unread}"),
        });
    };
    if !class.is_empty() {
        return Some(Finding::Fine(format!(
            "{plugins} keep{} storage and can take a standby: their pods share it through {class} \
             ({SHARED_CLASS})",
            if keeping.len() == 1 { "s" } else { "" }
        )));
    }
    let runs_on = match nodes {
        Some(n) => format!("runs on {n} nodes"),
        None => "runs on nodes that could not be counted".to_string(),
    };
    Some(Finding::Worth {
        what: format!(
            "{plugins} keep{} storage and cannot take a standby or shards: this deployment \
             {runs_on} and names no ReadWriteMany class in {SHARED_CLASS}",
            if keeping.len() == 1 { "s" } else { "" }
        ),
        why: format!(
            "An instance's pods share its storage on several nodes only through a ReadWriteMany \
             storage class (a cloud's file store, CephFS, NFS, Longhorn's RWX). `meridian \
             upgrade-deployment --set {SHARED_CLASS}=<class>` names one; a launch with --standby \
             or --shards then copies its storage into the shared claim's shard-0/, and keeps the \
             old claim."
        ),
    })
}

/// The cloud's step for trusting `account`, as values.yaml's pluginArchive
/// says it, by the bucket's kind: s3://, gs://, or an Azure container URL.
pub fn trust_step(bucket: &str, namespace: &str, account: &str, instance: &str) -> String {
    let fullname = account
        .strip_suffix(&format!("-shards-{instance}"))
        .unwrap_or("<fullname>");
    if bucket.starts_with("s3://") {
        format!(
            "EKS, IAM roles for service accounts: in the role's trust policy, a StringLike \
             condition on the token's `sub` of \
             `system:serviceaccount:{namespace}:{fullname}-shards-*`; EKS Pod Identity: an \
             association for {namespace}/{account}"
        )
    } else if bucket.starts_with("gs://") {
        format!(
            "GKE Workload Identity: `roles/iam.workloadIdentityUser` on the Google service \
             account, bound for this instance's account alone, \
             `serviceAccount:<project>.svc.id.goog[{namespace}/{account}]` -- never the \
             namespace as a whole, which every pod in it would share"
        )
    } else {
        format!(
            "Azure Workload Identity: a federated identity credential for {namespace}/{account}"
        )
    }
}

/// Items 60 and 63: each sharded or standby instance allowed a bucket
/// archive, with the account its bucket must trust and the cloud's step.
pub fn archive_accounts(held: &[Instance], namespace: &str) -> Vec<Finding> {
    held.iter()
        .filter(|one| one.several())
        .filter_map(|one| {
            let bucket = one.bucket.as_deref()?;
            let account = if one.account.is_empty() {
                format!("<fullname>-shards-{}", one.instance)
            } else {
                one.account.clone()
            };
            Some(Finding::Worth {
                what: format!(
                    "plugin {} runs {} with a bucket archive ({bucket}): its bucket must trust \
                     the account {namespace}/{account}",
                    one.instance,
                    pods_said(one)
                ),
                why: format!(
                    "{}. Until it does, the plugin stays unregistered and says the bucket refuses \
                     that account (pluginArchive in values.yaml).",
                    trust_step(bucket, namespace, &account, &one.instance)
                ),
            })
        })
        .collect()
}

fn pods_said(one: &Instance) -> String {
    let mut said = Vec::new();
    if one.shards > 1 {
        said.push(format!("{} shards", one.shards));
    }
    match one.standbys {
        0 => {}
        1 => said.push("a standby pod".into()),
        n => said.push(format!("{n} standby pods")),
    }
    said.join(" and ")
}

/// A shard's last take, as the history says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Took {
    pub at_ns: u64,
    pub epoch: Option<u64>,
    pub words: String,
}

fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|said| said.trim().parse().ok()))
}

/// The epoch a take's words name: `..., epoch <n>: <why>`.
pub fn epoch_in(words: &str) -> Option<u64> {
    let (_, after) = words.rsplit_once("epoch ")?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The newest take of `instance`'s shard `shard` in a page of history, which
/// the read gives newest first.
pub fn last_take(history: &Value, instance: &str, shard: u64) -> Option<Took> {
    let partition = format!("shard {shard}");
    history["history"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| {
            item["kind"] == SHARD_TAKEN_OVER
                && item["component"] == instance
                && item["partition"] == partition.as_str()
        })
        .map(|item| {
            let words = item["words"].as_str().unwrap_or_default().to_string();
            Took {
                at_ns: number(&item["at_ns"]).unwrap_or_default(),
                epoch: epoch_in(&words),
                words,
            }
        })
}

/// Item 32: each shard's epoch and last takeover, for each instance of
/// several pods, from as much of the history as it takes, at most
/// [`MOST_PAGES`] pages.
pub async fn fences(held: &[Instance], history: &dyn History) -> Vec<Finding> {
    let several: Vec<&Instance> = held.iter().filter(|one| one.several()).collect();
    if several.is_empty() {
        return Vec::new();
    }
    let mut wanted: Vec<(String, u64)> = several
        .iter()
        .flat_map(|one| (0..one.shards.max(1)).map(|shard| (one.instance.clone(), shard)))
        .collect();
    let mut found: Vec<((String, u64), Took)> = Vec::new();
    let mut cursor = String::new();
    let mut read = 0usize;
    for _ in 0..MOST_PAGES {
        let page = match history.page(PAGE_SIZE, &cursor).await {
            Ok(page) => page,
            Err(why) => {
                return vec![Finding::Unknown {
                    what: "each plugin shard's epoch and last takeover".into(),
                    why,
                }]
            }
        };
        read += page["history"].as_array().map_or(0, Vec::len);
        wanted.retain(
            |(instance, shard)| match last_take(&page, instance, *shard) {
                Some(took) => {
                    found.push(((instance.clone(), *shard), took));
                    false
                }
                None => true,
            },
        );
        let next = page["next_cursor"].as_str().unwrap_or_default();
        if wanted.is_empty() || next.is_empty() || next == cursor {
            break;
        }
        cursor = next.to_string();
    }
    let mut findings: Vec<(String, u64, Finding)> = found
        .into_iter()
        .map(|((instance, shard), took)| {
            let epoch = took
                .epoch
                .map_or(String::new(), |epoch| format!("epoch {epoch}, "));
            let at = match took.at_ns {
                0 => "at an unstated time".to_string(),
                ns => crate::registry::moment(ns),
            };
            let finding = Finding::Fine(format!(
                "plugin {instance} shard {shard}: {epoch}last taken over {at}: {}",
                took.words
            ));
            (instance, shard, finding)
        })
        .collect();
    findings.extend(wanted.into_iter().map(|(instance, shard)| {
        let finding = Finding::Unknown {
            what: format!("plugin {instance} shard {shard}'s epoch"),
            why: format!(
                "no takeover of it among the Resilience history's newest {read} items; its pods \
                 may not have taken it yet"
            ),
        };
        (instance, shard, finding)
    }));
    findings.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    findings
        .into_iter()
        .map(|(_, _, finding)| finding)
        .collect()
}

/// The deployment's Helm release in `namespace`: the one release of the
/// meridian-runtime chart `helm list -o json` names there.
pub fn release_in(listed: &Value) -> Option<String> {
    let releases: Vec<&str> = listed
        .as_array()?
        .iter()
        .filter(|release| {
            release["chart"]
                .as_str()
                .is_some_and(|chart| chart.starts_with("meridian-runtime-"))
        })
        .filter_map(|release| release["name"].as_str())
        .collect();
    match releases.as_slice() {
        [one] => Some(one.to_string()),
        _ => None,
    }
}

/// The checks above, for the plugins launched in `namespace`. Nothing where
/// none runs there.
pub async fn shards(machine: &dyn Machine, namespace: &str, history: &dyn History) -> Vec<Finding> {
    let workloads = match machine
        .run(
            "kubectl",
            &["get", "deployments", "-n", namespace, "-o", "json"],
        )
        .await
    {
        Ok(listed) => serde_json::from_str::<Value>(&listed).unwrap_or_default(),
        // What the plugins check already says of a namespace it cannot read.
        Err(_) => return Vec::new(),
    };
    let held = instances(&workloads);
    if held.is_empty() {
        return Vec::new();
    }
    let mut findings = Vec::new();

    if held.iter().any(|one| one.keeps_storage) {
        let nodes = match machine
            .run("kubectl", &["get", "pods", "-n", namespace, "-o", "json"])
            .await
        {
            Ok(listed) => serde_json::from_str::<Value>(&listed)
                .ok()
                .as_ref()
                .and_then(nodes),
            Err(_) => None,
        };
        let (class, unread) = shared_class_of(machine, namespace).await;
        findings.extend(standby_storage(&held, class.as_deref(), nodes, &unread));
    }
    findings.extend(archive_accounts(&held, namespace));
    findings.extend(fences(&held, history).await);
    findings
}

/// The class, read from the release's own values; None, and why, where it
/// cannot be.
async fn shared_class_of(machine: &dyn Machine, namespace: &str) -> (Option<String>, String) {
    let listed = match machine
        .run("helm", &["list", "-n", namespace, "-o", "json"])
        .await
    {
        Ok(listed) => listed,
        Err(failed) => return (None, failed.to_string()),
    };
    let Some(release) = serde_json::from_str::<Value>(&listed)
        .ok()
        .as_ref()
        .and_then(release_in)
    else {
        return (
            None,
            format!(
                "`helm list -n {namespace}` names no one release of the meridian-runtime chart"
            ),
        );
    };
    match machine
        .run(
            "helm",
            &["get", "values", &release, "-n", namespace, "-o", "json"],
        )
        .await
    {
        Ok(values) => match serde_json::from_str::<Value>(&values) {
            Ok(values) => (Some(shared_class(&values)), String::new()),
            Err(failed) => (None, format!("its values do not read: {failed}")),
        },
        Err(Failure::Missing(program)) => (None, format!("{program} is not installed here")),
        Err(failed) => (None, failed.to_string()),
    }
}

/// No session to read the history through: said where it is needed, as
/// `meridian connect` would fix it.
pub struct NoHistory(pub String);

#[async_trait::async_trait]
impl History for NoHistory {
    async fn page(&self, _: u32, _: &str) -> Result<Value, String> {
        Err(self.0.clone())
    }
}

#[cfg(test)]
mod tests;

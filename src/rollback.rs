//! `meridian rollback-deployment [--to-revision <n>]` (audit D4, named by
//! plan question 237 after helm rollback's revision argument): a release
//! Helm holds as failed or pending, or an upgrade not wanted, returned to an
//! earlier revision without the administrator running helm.
//!
//! It checks first and changes nothing if a check fails: helm and the
//! cluster are here, and the release's revisions are read. Then it shows the
//! revision it returns to -- by default the last one that was deployed
//! before the current, as `helm rollback` with no revision does -- and that
//! **the database is not rolled back with it**: what a newer revision's
//! migration changed stays, and an older runtime may refuse a schema it does
//! not know. It asks, then rolls back and waits as upgrade-deployment does:
//! the migration that revision runs, if any, and every component on its
//! template.
//!
//! A rollback that would lower a store's writer groups where they are
//! raise-only (item 111) is refused among the checks: `helm rollback`
//! applies the stored manifest without rendering the chart, so the chart's
//! own refusal never runs.
//!
//! So is a rollback to a revision whose manifest carries a launched plugin's
//! credential as it was when that revision was rendered, where the broker
//! has made the credential again since: the stored manifest would put the
//! old one back under a plugin that runs on the new one (Helm 3's merge),
//! or be refused by the cluster and leave the rollback's own revision
//! failed (Helm 4's server-side apply; the dev deployment, 2026-11-20). And
//! one from a newest revision that states the credential of a plugin
//! launched since the revision returned to, which states none: Helm's merge
//! would take it out under the running plugin. Both are checked again just
//! before `helm rollback`, after the plan was approved.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::time::Instant;

use crate::doctor::{Failure, Machine};
use crate::status::{revisions_from, Release};
use crate::upgrade_deployment::counts::{environment, GROUPS_VARIABLE, STORES};
use crate::upgrade_deployment::shards::LEASE_VARIABLE;
use crate::upgrade_deployment::{jobs, pods, progress, workloads};

pub struct Asked {
    pub release: String,
    pub namespace: String,
    pub to_revision: Option<u64>,
    /// As given, for Helm.
    pub timeout: String,
}

impl Asked {
    fn selector(&self) -> String {
        format!("app.kubernetes.io/instance={}", self.release)
    }
}

/// What a rollback would do: from the release's newest revision to `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub from: Release,
    pub to: Release,
}

fn said(failed: Failure) -> String {
    match failed {
        Failure::Missing(program) => format!(
            "{program} is not installed here; `meridian doctor` says what a deployment's \
             machine needs"
        ),
        Failure::Said(said) => said,
    }
}

/// The checks, and the plan where they pass. Nothing here changes anything.
pub async fn check(machine: &dyn Machine, asked: &Asked) -> Result<Plan, String> {
    machine // S9: helm version
        .run("helm", &["version", "--short"])
        .await
        .map_err(|failed| format!("helm does not run here: {}", said(failed)))?;
    machine // S9: kubectl cluster-info
        .run("kubectl", &["cluster-info"])
        .await
        .map_err(|failed| {
            format!(
                "no cluster is reachable: {}. Point KUBECONFIG at the cluster this deployment \
                 runs in",
                said(failed)
            )
        })?;
    let history = machine // S9: helm history
        .run(
            "helm",
            &[
                "history",
                &asked.release,
                "--namespace",
                &asked.namespace,
                "--max",
                "256",
                "-o",
                "json",
            ],
        )
        .await
        .map_err(|failed| {
            format!(
                "there is no release {} in {} to roll back ({}): `meridian status -n {}` says \
                 what is there",
                asked.release,
                asked.namespace,
                said(failed),
                asked.namespace
            )
        })?;
    let revisions = revisions_from(&asked.release, &history);
    let from = revisions
        .last()
        .cloned()
        .ok_or_else(|| format!("Helm lists no revision of {}", asked.release))?;
    let to = match asked.to_revision {
        Some(wanted) => revisions
            .iter()
            .find(|revision| revision.revision == wanted)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "{} has no revision {wanted}; Helm keeps {}",
                    asked.release,
                    revisions
                        .iter()
                        .map(|revision| revision.revision.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?,
        // The last one deployed before the newest, as helm rollback's
        // default returns to the one before.
        None => revisions
            .iter()
            .rev()
            .skip(1)
            .find(|revision| matches!(revision.status.as_str(), "deployed" | "superseded"))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "{} has no earlier revision that was deployed, so there is nothing to return \
                     to; --to-revision names one",
                    asked.release
                )
            })?,
    };
    if to.revision == from.revision {
        return Err(format!(
            "revision {} is the newest already: a rollback returns to an earlier one",
            to.revision
        ));
    }
    writer_groups_kept(machine, asked, to.revision).await?;
    launched_credentials_kept(machine, asked, from.revision, to.revision).await?;
    Ok(Plan { from, to })
}

/// The Secret the broker keeps launched plugins' credentials in, by the end
/// of its name: `<release>-meridian-runtime-broker-launched`, one key pair
/// per instance, `<instance>.password` and `<instance>.url`.
const LAUNCHED_SECRET: &str = "-launched";

type Keys = serde_json::Map<String, Value>;

/// The launched plugins' Secrets a revision's manifest states, by name: the
/// keys each one states.
fn launched_stated(manifest: &str) -> BTreeMap<String, Keys> {
    serde_yaml::Deserializer::from_str(manifest)
        .filter_map(|document| Value::deserialize(document).ok())
        .filter(|document| document["kind"] == "Secret")
        .filter_map(|document| {
            let name = document["metadata"]["name"].as_str()?;
            name.ends_with(LAUNCHED_SECRET).then(|| {
                let keys = document["data"].as_object().cloned().unwrap_or_default();
                (name.to_string(), keys)
            })
        })
        .collect()
}

/// The instance a key of the Secret is for.
fn instance_of(key: &str) -> String {
    key.strip_suffix(".password")
        .or_else(|| key.strip_suffix(".url"))
        .unwrap_or(key)
        .to_string()
}

/// The instances whose credential the revision returned to states (`then`)
/// and `live` holds with another value. Values are compared and never kept
/// or said.
fn credentials_made_again(then: &Keys, live: &Value) -> Vec<String> {
    then.iter()
        .filter(|(key, then)| live["data"].get(key).is_some_and(|now| now != *then))
        .map(|(key, _)| instance_of(key))
        .collect()
}

/// The instances whose credential the newest revision states (`newest`),
/// the revision returned to does not (`then`), and `live` holds: plugins
/// launched between the two. Helm takes out of the cluster what the newest
/// manifest states and the one it returns to does not.
fn credentials_launched_since(newest: &Keys, then: &Keys, live: &Value) -> Vec<String> {
    newest
        .keys()
        .filter(|key| !then.contains_key(*key) && live["data"].get(key).is_some())
        .map(|key| instance_of(key))
        .collect()
}

/// Each named once, in order.
fn once(mut instances: Vec<String>) -> Vec<String> {
    instances.sort();
    instances.dedup();
    instances
}

/// A launched plugin's credential is the broker's: made when the plugin is
/// launched, taken out when it is stopped. The chart once rendered the
/// Secret's keys from what it held, so such a revision's manifest states
/// each credential as it was then, and `helm rollback` applies the manifest
/// of the revision it returns to as it is stored, against the newest
/// revision's. Two rollbacks are refused here, before anything changes,
/// naming the instances:
///
/// - where a credential that revision states has been made again since:
///   the old one would be put back under a plugin that runs on the new one;
/// - where the newest revision states a credential, that revision states
///   none, and the Secret holds one: a plugin launched between the two,
///   whose credential Helm's merge would take out under it.
///
/// A stopped plugin has no credential to lose, so stopping them first is
/// the way through. A credential the revision states as it still is, or for
/// a plugin that no longer runs, is left to Helm: the first changes nothing,
/// and the broker takes the second out again.
async fn launched_credentials_kept(
    machine: &dyn Machine,
    asked: &Asked,
    from: u64,
    to: u64,
) -> Result<(), String> {
    let mut stated = Vec::new();
    for revision in [to, from] {
        let manifest = manifest_of(machine, asked, revision)
            .await
            .map_err(|failed| {
                format!(
                    "Can't read revision {revision}'s manifest ({}), so can't tell whether \
                     returning to revision {to} would put back a launched plugin's old \
                     credential or take one away",
                    said(failed)
                )
            })?;
        stated.push(launched_stated(&manifest));
    }
    let (then, newest) = (&stated[0], &stated[1]);
    let none = Keys::new();
    let (mut made_again, mut launched_since) = (Vec::new(), Vec::new());
    for name in then.keys().chain(newest.keys()).collect::<BTreeSet<_>>() {
        let then = then.get(name).unwrap_or(&none);
        let newest = newest.get(name).unwrap_or(&none);
        // No key stated by either: nothing of this Secret's is Helm's to
        // change, and the cluster is not asked.
        if then.is_empty() && newest.is_empty() {
            continue;
        }
        let live = launched_held(machine, asked, name)
            .await
            .map_err(|unread| {
                format!(
                    "Can't read which launched plugins hold a credential now ({unread}), so \
                     can't tell whether returning to revision {to} would put an old one back or \
                     take one away"
                )
            })?;
        made_again.extend(credentials_made_again(then, &live));
        launched_since.extend(credentials_launched_since(newest, then, &live));
    }
    let (made_again, launched_since) = (once(made_again), once(launched_since));
    if made_again.is_empty() && launched_since.is_empty() {
        return Ok(());
    }
    let mut refused = Vec::new();
    if !made_again.is_empty() {
        let have = if made_again.len() == 1 {
            "one has"
        } else {
            "some have"
        };
        refused.push(format!(
            "revision {to} keeps the credentials of the plugins that were launched when it was \
             made, as they were then, and {have} been made again since: {}. Returning to it \
             would put the old one back under a plugin that runs on the new one, or the cluster \
             would refuse it and leave the release at a failed revision.",
            made_again.join(", "),
        ));
    }
    if !launched_since.is_empty() {
        refused.push(format!(
            "revision {to} keeps no credential for what was launched after it was made, and \
             revision {from} does: {}. Returning to revision {to} would take the credential away \
             under a plugin that runs on it.",
            launched_since.join(", "),
        ));
    }
    let stop = once([made_again, launched_since].concat());
    let them = if stop.len() == 1 { "it" } else { "them" };
    let stops = stop
        .iter()
        .map(|instance| format!("  meridian plugin stop {instance}"))
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "{} Stop {them} first, roll back, and launch {them} again (a stopped plugin has no \
         credential to lose):\n{stops}",
        refused.join(" "),
    ))
}

/// Set on the lake's members while its writes are kept in a stream.
const WRITE_AHEAD_VARIABLE: &str = "MERIDIAN_LAKE_WRITE_AHEAD";

type Writers = BTreeMap<&'static str, BTreeMap<String, String>>;

/// Each store's group-0 writer Deployment among `items`, found by its
/// component label: its environment.
fn writers(items: &[Value]) -> Writers {
    STORES
        .iter()
        .filter_map(|store| {
            let deployment = items.iter().find(|item| {
                item["kind"] == "Deployment"
                    && item["metadata"]["labels"]["meridian.dev/component"] == *store
            })?;
            let environment = environment(deployment)
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect();
            Some((*store, environment))
        })
        .collect()
}

/// A store's writer groups as its writer's environment says them: 1 where
/// it says none, as the chart reads them.
fn groups_in(writers: &Writers, store: &str) -> u64 {
    writers
        .get(store)
        .and_then(|environment| environment.get(GROUPS_VARIABLE))
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
}

fn sets(writers: &Writers, store: &str, name: &str) -> bool {
    writers
        .get(store)
        .is_some_and(|environment| environment.contains_key(name))
}

/// Writer groups in ha are raised, never lowered yet (item 111). The chart
/// refuses a lowered count when it renders; `helm rollback` applies the
/// stored manifest without rendering, so the same refusal is made here.
/// What runs is read from the live group-0 writer Deployments, as the chart
/// reads it; what the rollback would leave, from that revision's manifest
/// and values. The lake's is refused where that revision is ha or keeps the
/// lake's writes in a stream, or the running lake does; the book's where
/// that revision is ha or the running book keeps a lease.
async fn writer_groups_kept(machine: &dyn Machine, asked: &Asked, to: u64) -> Result<(), String> {
    // S9: kubectl get deployments
    let live: Value =
        serde_json::from_str(&listed(machine, asked, "deployments").await).unwrap_or_default();
    let running = writers(live["items"].as_array().map(Vec::as_slice).unwrap_or(&[]));
    // Nothing runs above one group, so nothing can be lowered.
    if STORES.iter().all(|store| groups_in(&running, store) <= 1) {
        return Ok(());
    }
    let revision = to.to_string();
    let (release, namespace) = (asked.release.as_str(), asked.namespace.as_str());
    let unread = |what: &str, failed: Failure| {
        format!(
            "Can't read revision {to}'s {what} ({}), so can't tell whether returning to it would \
             lower a store's writer groups",
            said(failed)
        )
    };
    let manifest = manifest_of(machine, asked, to)
        .await
        .map_err(|failed| unread("manifest", failed))?;
    let documents: Vec<Value> = serde_yaml::Deserializer::from_str(&manifest)
        .filter_map(|document| Value::deserialize(document).ok())
        .collect();
    let target = writers(&documents);
    // Whether that revision is ha: read once, and only where it decides.
    let mut ha = None;
    let mut refused = Vec::new();
    for store in STORES {
        let (runs, wanted) = (groups_in(&running, store), groups_in(&target, store));
        if wanted >= runs {
            continue;
        }
        let kept = if store == "lake" {
            sets(&target, store, WRITE_AHEAD_VARIABLE)
                || sets(&running, store, WRITE_AHEAD_VARIABLE)
        } else {
            sets(&running, store, LEASE_VARIABLE)
        };
        if !kept {
            if ha.is_none() {
                let values = machine // S9: helm get values
                    .run(
                        "helm",
                        &[
                            "get",
                            "values",
                            release,
                            "--revision",
                            &revision,
                            "--namespace",
                            namespace,
                            "--all",
                            "-o",
                            "json",
                        ],
                    )
                    .await
                    .map_err(|failed| unread("values", failed))?;
                let values: Value = serde_json::from_str(&values).unwrap_or_default();
                ha = Some(values["resilience"]["profile"] == "ha");
            }
            if ha != Some(true) {
                continue;
            }
        }
        // The chart's own refusal (item 112), naming the revision.
        refused.push(if store == "lake" {
            format!(
                "lake.writerGroups is {wanted} at revision {to}, but this deployment's lake runs \
                 {runs} writer groups. In the ha profile, this release can raise the lake's \
                 writer groups but can't lower them yet: a group taken away could leave writes \
                 it had acknowledged unrecorded. Keep lake.writerGroups at {runs} or more"
            )
        } else {
            format!(
                "bor.writerGroups is {wanted} at revision {to}, but this deployment's book runs \
                 {runs} writer groups. In the ha profile, this release can raise the book's \
                 writer groups but can't lower them yet. Keep bor.writerGroups at {runs} or more"
            )
        });
    }
    if refused.is_empty() {
        Ok(())
    } else {
        Err(refused.join(".\n"))
    }
}

/// What it will do, said before it asks.
pub fn plan_text(asked: &Asked, plan: &Plan) -> String {
    let (from, to) = (&plan.from, &plan.to);
    let mut out = format!(
        "This rolls {} in {} back:\n\
         \x20 from revision {}, {}, chart {} (app {})\n\
         \x20 to   revision {}, chart {} (app {}), as a new revision\n",
        asked.release,
        asked.namespace,
        from.revision,
        from.status,
        from.chart,
        from.app_version,
        to.revision,
        to.chart,
        to.app_version
    );
    if !from.description.is_empty() && from.status != "deployed" {
        out.push_str(&format!(
            "  Helm says of {}: {}\n",
            from.revision, from.description
        ));
    }
    out.push_str(
        "The database is not rolled back with it. What a newer revision's migration changed \
         stays, and the older runtime may refuse a schema it does not know: read `meridian logs \
         migrate` first, and restore a backup taken before the upgrade if the data must go back \
         too.\n",
    );
    if from.status.starts_with("pending") {
        out.push_str(&format!(
            "Helm holds revision {} as {}: if another Helm operation is running on it, let it \
             finish first; this does not check that.\n",
            from.revision, from.status
        ));
    }
    out
}

/// How often the cluster is asked while waiting, and for how long.
pub struct Pace {
    pub poll: Duration,
    pub timeout: Duration,
}

/// A revision's manifest, as Helm stored it.
async fn manifest_of(
    machine: &dyn Machine,
    asked: &Asked,
    revision: u64,
) -> Result<String, Failure> {
    // Why helm: `helm rollback` applies a stored manifest without rendering,
    // and only Helm's own record says what that manifest states.
    machine // S9: helm get manifest
        .run(
            "helm",
            &[
                "get",
                "manifest",
                &asked.release,
                "--revision",
                &revision.to_string(),
                "--namespace",
                &asked.namespace,
            ],
        )
        .await
}

/// The launched plugins' Secret, by name, as the cluster holds it now: null
/// where there is none. Why it could not be read otherwise, with nothing of
/// what it holds: its values are compared by the caller and never said.
async fn launched_held(machine: &dyn Machine, asked: &Asked, name: &str) -> Result<Value, String> {
    // Why kubectl: a stored manifest's credential is compared with the one
    // in the broker's Secret now, which only the cluster holds.
    let held = machine // S9: kubectl get secrets
        .run(
            "kubectl",
            &[
                "get",
                "secret",
                name,
                "-n",
                &asked.namespace,
                "--ignore-not-found",
                "-o",
                "json",
            ],
        )
        .await
        .map_err(said)?;
    if held.trim().is_empty() {
        return Ok(Value::Null);
    }
    // The parser's own words are left out: nothing of the answer is repeated.
    serde_json::from_str(&held).map_err(|_| "kubectl's answer is not JSON".to_string())
}

/// The release's own `what`, by its label, as the cluster holds them now:
/// nothing where they could not be listed.
async fn listed(machine: &dyn Machine, asked: &Asked, what: &str) -> String {
    machine
        .run(
            "kubectl",
            &[
                "get",
                what,
                "-n",
                &asked.namespace,
                "-l",
                &asked.selector(),
                "-o",
                "json",
            ],
        )
        .await
        .unwrap_or_default()
}

/// The rollback, then the wait. What it came to, or where it stopped.
pub async fn apply(
    machine: &dyn Machine,
    asked: &Asked,
    plan: &Plan,
    pace: &Pace,
    say: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    let (release, namespace) = (asked.release.as_str(), asked.namespace.as_str());
    let to = plan.to.revision.to_string();
    // Checked again here, as near the rollback as it can be: the plan was
    // shown and approved since the first check, and a plugin launched or
    // launched again meanwhile has a credential the first check never saw.
    launched_credentials_kept(machine, asked, plan.from.revision, plan.to.revision)
        .await
        .map_err(|refused| {
            format!("this changed after the first check, and nothing was rolled back: {refused}")
        })?;
    machine // S9: helm rollback
        .run(
            "helm",
            &[
                "rollback",
                release,
                &to,
                "--namespace",
                namespace,
                "--timeout",
                &asked.timeout,
            ],
        )
        .await
        .map_err(|failed| {
            format!(
                "helm refused the rollback: {}\n`meridian status -n {namespace}` says where the \
                 release is now",
                said(failed)
            )
        })?;
    let made = machine // S9: helm history
        .run(
            "helm",
            &[
                "history",
                release,
                "--namespace",
                namespace,
                "--max",
                "1",
                "-o",
                "json",
            ],
        )
        .await
        .ok()
        .and_then(|history| revisions_from(release, &history).pop())
        .map(|newest| newest.revision)
        .unwrap_or(plan.from.revision + 1);
    say(&format!(
        "Rolled back to revision {to}'s chart as revision {made}. Waiting for every component on \
         its template (up to {}).",
        asked.timeout
    ));
    let deadline = Instant::now() + pace.timeout;
    let mut said_waiting = std::collections::BTreeSet::new();
    loop {
        // S9: kubectl get jobs; kubectl get deployments; kubectl get statefulsets
        // S9: kubectl get replicasets; kubectl get pods
        let now = progress(
            plan.to.revision,
            &jobs(&listed(machine, asked, "jobs").await),
            &workloads(&listed(machine, asked, "deployments,statefulsets,replicasets").await),
            &pods(&listed(machine, asked, "pods").await),
        );
        if let Some(job) = now.failed {
            return Err(format!(
                "revision {to}'s migration, job/{job}, failed: the older runtime cannot read the \
                 database as the newer one left it. `meridian logs migrate -n {namespace}` says \
                 why; restoring a backup taken before the upgrade brings the data back with it"
            ));
        }
        if now.waiting.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "after {}, still waiting for:\n  - {}\nRevision {made} is applied. `meridian \
                 status -n {namespace}` says where each component is.",
                asked.timeout,
                now.waiting.join("\n  - ")
            ));
        }
        for each in now.waiting {
            if said_waiting.insert(each.clone()) {
                say(&format!("  waiting: {each}"));
            }
        }
        tokio::time::sleep(pace.poll).await;
    }
    Ok(format!(
        "{release} is at revision {made}: revision {to}'s chart, {} (app {}), every component on \
         its template.\n",
        plan.to.chart, plan.to.app_version
    ))
}

#[cfg(test)]
mod tests;

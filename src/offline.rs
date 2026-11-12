//! An act on the brought database's stopped volume (step 3b; spec/a-
//! deployment-scales-out-and-fails-over-by-configuration sections 4 and 9,
//! ruling 1058): page checksums turned on (`meridian upgrade-deployment
//! --enable-checksums`) and a restore from the WAL archive to a point in time
//! (`meridian restore --to-time`) or to its end (`meridian restore
//! --to-archive-end`, step 3c), each along the timeline the act chooses.
//!
//! The chart holds the act itself, in a suspended CronJob that never runs on
//! its own (`<release>-meridian-runtime-database-offline`, meridian-core's
//! templates/database-offline.yaml and files/database-offline.sh). This is
//! the order around it, with the person's own cluster rights (decisions/016):
//! every component stopped, then the database; under Patroni the primary
//! switched to member 0 first, since the CronJob mounts member 0's volume; a
//! Job made from the CronJob with the act named; then, under Patroni, the
//! standbys' volumes deleted so Patroni makes them again from member 0;
//! the database started, then everything else, each to the count it ran at.
//!
//! Nothing here prints a credential or a row: the act's own lines, which
//! name sizes, times and timelines, and the names of what is stopped.

pub mod checksums;
pub mod to_time;

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::time::Instant;

use crate::doctor::{Failure, Machine};
use crate::upgrade_deployment::run::Pace;
use crate::upgrade_deployment::watch::Watch;
use crate::upgrade_deployment::{pods, progress, workloads, DatabaseSet};

/// Where the data directory is on the database's volume, as the chart mounts
/// it.
pub const PGDATA: &str = "/var/lib/postgresql/data/pgdata";

/// What pg_checksums is taken to read and write a second, for the downtime
/// said before the guided step asks: a cautious figure for a cloud's
/// ordinary disk, every page read and written once.
pub const CHECKSUM_BYTES_PER_SECOND: u64 = 50 * 1024 * 1024;

/// What stopping every component and starting it again is taken to add.
pub const STOP_AND_START: Duration = Duration::from_secs(4 * 60);

/// The act the Job is made for, as the chart's script names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    EnableChecksums,
    /// To a time, as the script takes it: `YYYY-MM-DD HH:MM:SS[.ffffff]+00`.
    RestoreToTime(String),
    /// To the last commit the archive holds (step 3c).
    RestoreToArchiveEnd,
}

impl Act {
    pub fn name(&self) -> &'static str {
        match self {
            Act::EnableChecksums => "enable-checksums",
            Act::RestoreToTime(_) => "restore-to-time",
            Act::RestoreToArchiveEnd => "restore-to-archive-end",
        }
    }

    /// A restore, of either kind.
    pub fn restores(&self) -> bool {
        matches!(self, Act::RestoreToTime(_) | Act::RestoreToArchiveEnd)
    }
}

/// Where the release is, and what it is called.
#[derive(Debug, Clone)]
pub struct Release {
    pub release: String,
    pub namespace: String,
}

impl Release {
    pub fn selector(&self) -> String {
        format!("app.kubernetes.io/instance={}", self.release)
    }

    fn scope(&self) -> String {
        crate::status::scope(&self.release, &self.namespace)
    }
}

/// One of the database's members, from its pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    /// `primary` or `standby`, as Patroni labels it (`replica` is said as a
    /// standby); None where it carries no role, as alone it does not.
    pub role: Option<String>,
    pub ready: bool,
    pub node: Option<String>,
    pub terminating: bool,
}

/// The database's members among `kubectl get pods -o json`'s items: the
/// pods of its StatefulSet, by their ordinal names.
pub fn members(listed: &serde_json::Value, set: &str) -> Vec<Member> {
    let mut found: Vec<Member> = listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|pod| {
            let name = pod["metadata"]["name"].as_str().unwrap_or_default();
            pod["metadata"]["labels"]["meridian.dev/component"] == "database"
                && name
                    .strip_prefix(set)
                    .and_then(|rest| rest.strip_prefix('-'))
                    .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|pod| Member {
            name: pod["metadata"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            role: pod["metadata"]["labels"]["meridian.dev/database-role"]
                .as_str()
                .map(|role| match role {
                    "replica" => "standby".to_string(),
                    other => other.to_string(),
                }),
            ready: pod["status"]["conditions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|c| c["type"] == "Ready" && c["status"] == "True"),
            node: pod["spec"]["nodeName"].as_str().map(String::from),
            terminating: !pod["metadata"]["deletionTimestamp"].is_null(),
        })
        .collect();
    found.sort_by_key(|member| ordinal(&member.name));
    found
}

fn ordinal(name: &str) -> u64 {
    name.rsplit_once('-')
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(u64::MAX)
}

/// The Job made from the offline CronJob, with the act named: as
/// `kubectl create job --from` makes one, its environment set for the act.
pub fn offline_job(
    cronjob: &serde_json::Value,
    name: &str,
    act: &Act,
) -> Result<serde_json::Value, String> {
    let template = &cronjob["spec"]["jobTemplate"];
    let mut spec = template["spec"].clone();
    let container = spec["template"]["spec"]["containers"]
        .get_mut(0)
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("the database's offline CronJob runs no container")?;
    let named: BTreeMap<&str, String> = [
        ("MERIDIAN_DATABASE_ACT", act.name().to_string()),
        (
            "MERIDIAN_RESTORE_TO_TIME",
            match act {
                Act::RestoreToTime(to) => to.clone(),
                Act::EnableChecksums | Act::RestoreToArchiveEnd => String::new(),
            },
        ),
    ]
    .into();
    let env = container
        .entry("env")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or("the offline CronJob's environment is not a list")?;
    env.retain(|each| {
        each["name"]
            .as_str()
            .is_none_or(|name| !named.contains_key(name))
    });
    for (key, value) in &named {
        env.push(serde_json::json!({ "name": key, "value": value }));
    }
    let mut labels = template["metadata"]["labels"].clone();
    if let Some(map) = labels.as_object_mut() {
        map.insert("meridian.dev/database-act".into(), act.name().into());
    }
    Ok(serde_json::json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": name,
            "namespace": cronjob["metadata"]["namespace"],
            "labels": labels,
            "annotations": { "cronjob.kubernetes.io/instantiate": "manual" },
            "ownerReferences": [{
                "apiVersion": "batch/v1",
                "kind": "CronJob",
                "name": cronjob["metadata"]["name"],
                "uid": cronjob["metadata"]["uid"],
                "controller": true,
            }],
        },
        "spec": spec,
    }))
}

/// The downtime said before an act asks: the act proportional to the
/// database's size, plus stopping and starting, plus each standby made again
/// under Patroni, one after another.
pub fn downtime(bytes: u64, standbys: u64) -> Duration {
    let act = Duration::from_secs(bytes / CHECKSUM_BYTES_PER_SECOND + 1);
    STOP_AND_START + act * (1 + standbys as u32)
}

/// A duration as a person reads it, rounded up to the minute.
pub fn minutes(took: Duration) -> String {
    match took.as_secs().div_ceil(60) {
        0 | 1 => "about a minute".to_string(),
        n => format!("about {n} minutes"),
    }
}

// ── What it does ────────────────────────────────────────────────────────────

/// kubectl through the machine, its refusal as text.
pub async fn kubectl(machine: &dyn Machine, arguments: &[&str]) -> Result<String, String> {
    machine
        .run("kubectl", arguments)
        .await
        .map_err(|failed| failed.to_string())
}

async fn json(machine: &dyn Machine, arguments: &[&str]) -> Result<serde_json::Value, String> {
    serde_json::from_str(&kubectl(machine, arguments).await?)
        .map_err(|failed| format!("kubectl's answer could not be read: {failed}"))
}

/// The release's workloads and its database's StatefulSet, as listed now.
pub async fn database(
    machine: &dyn Machine,
    release: &Release,
) -> Result<Option<DatabaseSet>, String> {
    let listed = kubectl(
        machine,
        &[
            "get",
            "deployments,statefulsets",
            "-n",
            &release.namespace,
            "-l",
            &release.selector(),
            "-o",
            "json",
        ],
    )
    .await?;
    Ok(crate::upgrade_deployment::database_set(&listed))
}

/// The database's members as their pods are now.
pub async fn members_now(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
) -> Result<Vec<Member>, String> {
    let listed = json(
        machine,
        &[
            "get",
            "pods",
            "-n",
            &release.namespace,
            "-l",
            &format!("{},meridian.dev/component=database", release.selector()),
            "-o",
            "json",
        ],
    )
    .await?;
    Ok(members(&listed, set))
}

/// A command run in the database's member, in its server's container.
async fn in_member(
    machine: &dyn Machine,
    release: &Release,
    member: &str,
    command: &[&str],
) -> Result<String, String> {
    let mut arguments = vec![
        "exec",
        "-n",
        release.namespace.as_str(),
        member,
        "-c",
        "postgres",
        "--",
    ];
    arguments.extend_from_slice(command);
    kubectl(machine, &arguments).await
}

/// The size of the data directory on member 0, in bytes, as the act's own
/// script measures it.
pub async fn data_bytes(machine: &dyn Machine, release: &Release, set: &str) -> Option<u64> {
    let said = in_member(
        machine,
        release,
        &format!("{set}-0"),
        &["du", "-sk", PGDATA],
    )
    .await
    .ok()?;
    said.split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()
        .map(|kib| kib * 1024)
}

/// The suspended CronJob the act is made from.
pub async fn cronjob(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
) -> Result<serde_json::Value, String> {
    json(
        machine,
        &[
            "get",
            "cronjob",
            &format!("{set}-offline"),
            "-n",
            &release.namespace,
            "-o",
            "json",
        ],
    )
    .await
}

/// The offline act's script as the chart holds it, in the CronJob's
/// ConfigMap: what it knows (step 3c's `restore-to-archive-end`, and the
/// timeline it chooses, came together).
pub async fn offline_script(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
) -> Result<String, String> {
    let held = json(
        machine,
        &[
            "get",
            "configmap",
            &format!("{set}-offline"),
            "-n",
            &release.namespace,
            "-o",
            "json",
        ],
    )
    .await?;
    held["data"]["database-offline.sh"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("{set}-offline holds no database-offline.sh"))
}

/// Whether the chart's offline act chooses the timeline and restores to the
/// archive's end (step 3c).
pub fn knows_timelines(script: &str) -> bool {
    script.contains("restore-to-archive-end")
}

/// What was running before the stop, to start again as it was: each of the
/// release's Deployments with its count, and the database's; each of its
/// CronJobs with whether it was suspended already; the act it was stopped
/// for, and since when.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stopped {
    pub deployments: Vec<(String, u64)>,
    pub database: u64,
    /// Each of the release's CronJobs but the offline one, by name, with
    /// its `suspend` before the stop. Those not suspended then are while it
    /// is stopped, and are given back `suspend: false` when it starts again;
    /// those suspended already are never touched.
    pub cronjobs: Vec<(String, bool)>,
    /// The act, as its Job names it (`restore-to-time`,
    /// `restore-to-archive-end`, `enable-checksums`).
    /// None in a record written before it was kept, which only a restore
    /// to a time wrote.
    pub act: Option<String>,
    /// When it was recorded, just before anything stopped: RFC 3339, UTC.
    pub since: Option<String>,
}

/// Wait, by `pace`, until `done` says so, saying `what` if it never does.
async fn wait_for<F, Fut>(pace: &Pace, what: &str, mut done: F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + pace.timeout;
    loop {
        if done().await {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what} did not happen within {}s",
                pace.timeout.as_secs()
            ));
        }
        tokio::time::sleep(pace.poll).await;
    }
}

/// Each of the release's Deployments running, with its count.
pub async fn running_deployments(
    machine: &dyn Machine,
    release: &Release,
) -> Result<Vec<(String, u64)>, String> {
    let ns = release.namespace.as_str();
    let listed = json(
        machine,
        &[
            "get",
            "deployments",
            "-n",
            ns,
            "-l",
            &release.selector(),
            "-o",
            "json",
        ],
    )
    .await?;
    let running: Vec<(String, u64)> = listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let replicas = item["spec"]["replicas"].as_u64().unwrap_or(1);
            Some((item["metadata"]["name"].as_str()?.to_string(), replicas))
        })
        .filter(|(_, replicas)| *replicas > 0)
        .collect();
    Ok(running)
}

/// Those Deployments scaled to none, launched plugins among them, and their
/// pods gone. The broker and the registry keep running: neither holds the
/// database.
pub async fn stop_these(
    machine: &dyn Machine,
    release: &Release,
    running: &[(String, u64)],
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let ns = release.namespace.as_str();
    if !running.is_empty() {
        let mut arguments = vec!["scale", "deployment", "-n", ns, "--replicas=0"];
        arguments.extend(running.iter().map(|(name, _)| name.as_str()));
        kubectl(machine, &arguments).await?;
        watch.say(&format!(
            "  stopped {} component(s): {}",
            running.len(),
            running
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let selector = release.selector();
    wait_for(pace, "every component's pods to stop", || async {
        let held = kubectl(
            machine,
            &["get", "pods", "-n", ns, "-l", &selector, "-o", "json"],
        )
        .await
        .map(|listed| pods(&listed))
        .unwrap_or_default();
        !held.iter().any(|pod| pod.replica_set.is_some())
    })
    .await
}

/// The annotation on the database's StatefulSet that holds what a restore
/// stopped, while it is stopped: the counts to start everything again at,
/// which a restore that failed and is run again reads.
pub const STOPPED: &str = "meridian.dev/stopped-for-restore";

impl Stopped {
    /// The annotation's value. `cronjobs`, `act` and `since` were added to
    /// `database` and `deployments` (step 3b, rehearsal 2); a reader of the
    /// older record reads it as it did.
    pub fn to_json(&self) -> String {
        let mut said = serde_json::json!({
            "database": self.database,
            "deployments": self.deployments,
            "cronjobs": self.cronjobs,
        });
        if let Some(act) = &self.act {
            said["act"] = act.clone().into();
        }
        if let Some(since) = &self.since {
            said["since"] = since.clone().into();
        }
        said.to_string()
    }

    pub fn from_json(said: &str) -> Option<Stopped> {
        let said: serde_json::Value = serde_json::from_str(said).ok()?;
        Some(Stopped {
            database: said["database"].as_u64()?,
            deployments: said["deployments"]
                .as_array()?
                .iter()
                .filter_map(|each| Some((each[0].as_str()?.to_string(), each[1].as_u64()?)))
                .collect(),
            cronjobs: said["cronjobs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|each| Some((each[0].as_str()?.to_string(), each[1].as_bool()?)))
                .collect(),
            act: said["act"].as_str().map(String::from),
            since: said["since"].as_str().map(String::from),
        })
    }

    /// The act it was stopped for, a record without one being a restore's.
    pub fn act_name(&self) -> &str {
        self.act.as_deref().unwrap_or("restore-to-time")
    }

    /// The command that stopped it, as a person runs it.
    pub fn command(&self) -> &'static str {
        match self.act_name() {
            "enable-checksums" => "meridian upgrade-deployment --enable-checksums",
            "restore-to-archive-end" => "meridian restore --to-archive-end",
            _ => "meridian restore --to-time",
        }
    }

    /// The command that takes it from where it stopped, with `scope`
    /// (`-n <namespace>`, and `--release` where it is not the default).
    pub fn resumed_by(&self, scope: &str) -> String {
        match self.act_name() {
            "enable-checksums" => format!("meridian upgrade-deployment --enable-checksums {scope}"),
            "restore-to-archive-end" => format!("meridian restore --to-archive-end {scope}"),
            _ => format!("meridian restore --to-time <time> {scope}"),
        }
    }
}

/// The record on a StatefulSet as `kubectl get -o json` lists it, where it
/// holds one.
pub fn stopped_on(set: &serde_json::Value) -> Option<Stopped> {
    Stopped::from_json(set["metadata"]["annotations"][STOPPED].as_str()?)
}

/// Seconds since the epoch as RFC 3339, in UTC, to the second.
pub fn utc(unix_seconds: u64) -> String {
    time::OffsetDateTime::from_unix_timestamp(unix_seconds as i64)
        .map(|at| {
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                at.year(),
                u8::from(at.month()),
                at.day(),
                at.hour(),
                at.minute(),
                at.second()
            )
        })
        .unwrap_or_default()
}

/// The release's CronJobs, the offline one aside (it is suspended always,
/// and the act is made from it), each with whether it is suspended now: the
/// replica comparison, the scheduled backup, the scrub, and any other the
/// chart draws.
pub async fn cronjobs_now(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
) -> Result<Vec<(String, bool)>, String> {
    let offline = format!("{set}-offline");
    let listed = json(
        machine,
        &[
            "get",
            "cronjobs",
            "-n",
            &release.namespace,
            "-l",
            &release.selector(),
            "-o",
            "json",
        ],
    )
    .await
    .map_err(|failed| format!("the release's CronJobs could not be read: {failed}"))?;
    let mut found: Vec<(String, bool)> = listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|cronjob| {
            cronjob["metadata"]["labels"]["meridian.dev/component"] != "database-offline"
        })
        .filter_map(|cronjob| {
            let name = cronjob["metadata"]["name"].as_str()?;
            (name != offline).then(|| {
                (
                    name.to_string(),
                    cronjob["spec"]["suspend"].as_bool().unwrap_or(false),
                )
            })
        })
        .collect();
    found.sort();
    Ok(found)
}

/// One CronJob's `suspend` set.
async fn suspend(
    machine: &dyn Machine,
    release: &Release,
    name: &str,
    suspended: bool,
) -> Result<(), String> {
    let patch = format!("{{\"spec\":{{\"suspend\":{suspended}}}}}");
    kubectl(
        machine,
        &[
            "patch",
            "cronjob",
            name,
            "-n",
            &release.namespace,
            "--type=merge",
            "-p",
            &patch,
        ],
    )
    .await
    .map(|_| ())
}

/// The CronJobs not suspended before the stop, suspended while it lasts: a
/// replica comparison, a backup or a scrub would otherwise fire against a
/// database that is not there, or, a backup, against one half restored.
pub async fn suspend_cronjobs(
    machine: &dyn Machine,
    release: &Release,
    stopped: &Stopped,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let running: Vec<&str> = stopped
        .cronjobs
        .iter()
        .filter(|(_, suspended)| !suspended)
        .map(|(name, _)| name.as_str())
        .collect();
    for name in &running {
        suspend(machine, release, name, true)
            .await
            .map_err(|failed| format!("the CronJob {name} could not be suspended: {failed}"))?;
    }
    if !running.is_empty() {
        watch.say(&format!(
            "  suspended {} CronJob(s) while it is stopped: {}",
            running.len(),
            running.join(", ")
        ));
    }
    Ok(())
}

/// Those CronJobs given back `suspend: false`, as they were before the stop;
/// those suspended already are left as they are. A CronJob no longer there
/// is passed over.
///
/// A helm upgrade between the stop and this is not expected: nothing in the
/// CLI runs one, and `upgrade-deployment` finds the database stopped. If one
/// runs, Helm sets `suspend` on a CronJob whose template sets it (the
/// backup's, from its schedule), so that one may fire while everything is
/// stopped; and this then gives back what was recorded before the stop,
/// which can undo a schedule that upgrade changed. Running `meridian
/// upgrade-deployment` again once everything is started sets the chart's.
pub async fn resume_cronjobs(
    machine: &dyn Machine,
    release: &Release,
    stopped: &Stopped,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let mut resumed = Vec::new();
    for (name, _) in stopped.cronjobs.iter().filter(|(_, suspended)| !suspended) {
        match suspend(machine, release, name, false).await {
            Ok(()) => resumed.push(name.as_str()),
            Err(failed) if failed.contains("NotFound") || failed.contains("not found") => {}
            Err(failed) => {
                return Err(format!(
                    "the CronJob {name} could not be given back its schedule ({failed}): it \
                     stays suspended"
                ))
            }
        }
    }
    if !resumed.is_empty() {
        watch.say(&format!(
            "  resumed {} CronJob(s): {}",
            resumed.len(),
            resumed.join(", ")
        ));
    }
    Ok(())
}

/// What an act stops, recorded before anything is: each running Deployment
/// with its count, the database's, and each CronJob's `suspend`. Where an
/// earlier act recorded a stop it did not finish while the database still
/// ran, its counts and CronJobs stand over what runs now, which it had
/// stopped or suspended itself; anything running now that it did not record
/// is added.
pub async fn record_stop(
    machine: &dyn Machine,
    release: &Release,
    set: &DatabaseSet,
    act: &Act,
    earlier: Option<&Stopped>,
) -> Result<Stopped, String> {
    let mut deployments = earlier.map(|e| e.deployments.clone()).unwrap_or_default();
    for (name, count) in running_deployments(machine, release).await? {
        if !deployments.iter().any(|(had, _)| *had == name) {
            deployments.push((name, count));
        }
    }
    let mut cronjobs = earlier.map(|e| e.cronjobs.clone()).unwrap_or_default();
    for (name, suspended) in cronjobs_now(machine, release, &set.name).await? {
        if !cronjobs.iter().any(|(had, _)| *had == name) {
            cronjobs.push((name, suspended));
        }
    }
    let stopped = Stopped {
        deployments,
        database: earlier
            .map(|e| e.database)
            .filter(|n| *n > 0)
            .unwrap_or(set.replicas),
        cronjobs,
        act: Some(act.name().to_string()),
        since: Some(utc(machine.now_s())),
    };
    record_stopped(machine, release, &set.name, &stopped).await?;
    Ok(stopped)
}

/// A Job made from the offline CronJob that is still running, by name.
pub async fn act_running(machine: &dyn Machine, release: &Release) -> Option<String> {
    let listed = json(
        machine,
        &[
            "get",
            "jobs",
            "-n",
            &release.namespace,
            "-l",
            &format!("{},meridian.dev/database-act", release.selector()),
            "-o",
            "json",
        ],
    )
    .await
    .ok()?;
    listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|job| job["status"]["active"].as_u64().unwrap_or(0) > 0)
        .and_then(|job| job["metadata"]["name"].as_str().map(String::from))
}

/// What a restore stopped, kept on the database's StatefulSet before
/// anything is stopped.
pub async fn record_stopped(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
    stopped: &Stopped,
) -> Result<(), String> {
    let value = format!("{STOPPED}={}", stopped.to_json());
    kubectl(
        machine,
        &[
            "annotate",
            "statefulset",
            set,
            "-n",
            &release.namespace,
            "--overwrite",
            &value,
        ],
    )
    .await
    .map(|_| ())
    .map_err(|failed| format!("what it stops could not be recorded first: {failed}"))
}

/// What an earlier restore stopped and recorded, where it did.
pub async fn recorded_stopped(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
) -> Option<Stopped> {
    let path = format!(
        "jsonpath={{.metadata.annotations.{}}}",
        STOPPED.replace('.', "\\.")
    );
    let said = kubectl(
        machine,
        &[
            "get",
            "statefulset",
            set,
            "-n",
            &release.namespace,
            "-o",
            &path,
        ],
    )
    .await
    .ok()?;
    Stopped::from_json(said.trim())
}

/// The record removed, once everything has started again.
pub async fn clear_stopped(machine: &dyn Machine, release: &Release, set: &str) {
    let _ = kubectl(
        machine,
        &[
            "annotate",
            "statefulset",
            set,
            "-n",
            &release.namespace,
            &format!("{STOPPED}-"),
        ],
    )
    .await;
}

/// Under Patroni, member 0 made the primary, the database still running:
/// synchronous replication off first, since Patroni switches over only to a
/// synchronous standby and member 0 may be the other one (the chart holds it
/// on again as each member starts, files/database.sh). Nothing writes by
/// then: every component is stopped.
pub async fn primary_to_member_0(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let first = format!("{set}-0");
    let now = members_now(machine, release, set).await?;
    if now
        .iter()
        .any(|m| m.name == first && m.role.as_deref() == Some("primary"))
    {
        watch.say(&format!("  {first} is the primary already"));
        return Ok(());
    }
    let patronictl = ["patronictl", "-c", "/etc/patroni/patroni.yml"];
    let mut asynchronous = patronictl.to_vec();
    asynchronous.extend([
        "edit-config",
        "--force",
        "-s",
        "synchronous_mode=false",
        "-s",
        "synchronous_mode_strict=false",
    ]);
    in_member(machine, release, &first, &asynchronous)
        .await
        .map_err(|failed| {
            format!("Patroni's synchronous replication could not be turned off to switch: {failed}")
        })?;
    let mut switch = patronictl.to_vec();
    switch.extend(["switchover", "--candidate", first.as_str(), "--force"]);
    in_member(machine, release, &first, &switch)
        .await
        .map_err(|failed| format!("Patroni did not switch the primary to {first}: {failed}"))?;
    wait_for(pace, &format!("{first} to become the primary"), || async {
        members_now(machine, release, set)
            .await
            .unwrap_or_default()
            .iter()
            .any(|m| m.name == first && m.role.as_deref() == Some("primary") && m.ready)
    })
    .await?;
    watch.say(&format!("  switched the primary to {first}"));
    Ok(())
}

/// The database stopped: its StatefulSet scaled to none and every member
/// gone. Its count, to start it again.
pub async fn stop_database(
    machine: &dyn Machine,
    release: &Release,
    set: &DatabaseSet,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let ns = release.namespace.as_str();
    kubectl(
        machine,
        &["scale", "statefulset", &set.name, "-n", ns, "--replicas=0"],
    )
    .await?;
    wait_for(pace, "the database to stop", || async {
        members_now(machine, release, &set.name)
            .await
            .is_ok_and(|members| members.is_empty())
    })
    .await?;
    watch.say(&format!("  stopped the database, {}", set.name));
    Ok(())
}

/// An act that did not end well: the Job failed, with the act's own words,
/// or had not finished by the time allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActFailed {
    /// The act's FAILED line, where the Job failed with one.
    pub why: Option<String>,
    /// Whether the Job ended (failed) or is left running.
    pub ended: bool,
    /// What a person reads: which Job, how it ended, and the cause as its
    /// log gives it.
    pub said: String,
    /// The cause as its log gives it, the lines `said` shows (`cause_lines`).
    pub cause: Vec<String>,
}

impl std::fmt::Display for ActFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.said)
    }
}

impl ActFailed {
    /// Whether the act said the time named is outside the WAL archive: at or
    /// past its last commit, or before its oldest base backup ended. The
    /// chart's script says "is outside the archive" from step 3b part 7;
    /// before it, Postgres's own words for those two.
    pub fn outside_the_archive(&self) -> bool {
        self.why.as_deref().is_some_and(|why| {
            why.contains("is outside the archive")
                || self.past_the_archive()
                || why.contains("requested recovery stop point is before consistent recovery point")
        })
    }

    /// Whether the archive was replayed to its end without reaching the time
    /// named: Postgres's words when the time is at or after the archive's
    /// last commit.
    pub fn past_the_archive(&self) -> bool {
        self.why.as_deref().is_some_and(|why| {
            why.contains("recovery ended before configured recovery target was reached")
        })
    }

    /// Whether the act refused before it restored anything, as step 3c's
    /// says ("the database's volume is as it was").
    pub fn untouched(&self) -> bool {
        self.cause
            .iter()
            .any(|line| line.contains("the database's volume is as it was"))
    }

    /// The archive's first and last times, as the act names them when it
    /// refuses a time outside it (step 3c: "which holds <first> to
    /// <last>"), each as Postgres writes a time.
    pub fn holds(&self) -> Option<(&str, &str)> {
        let why = self.why.as_deref()?;
        let (_, rest) = why.split_once("which holds ")?;
        let (first, rest) = rest.split_once(" to ")?;
        // A time holds colons, never a colon and a space.
        let last = rest.split_once(": ").map_or(rest, |(last, _)| last);
        (!first.is_empty() && !last.is_empty()).then_some((first.trim(), last.trim()))
    }

    /// The archive's last commit, where Postgres's lines shown include it:
    /// `last completed transaction was at log time <time>`, as Postgres
    /// wrote the time.
    pub fn last_completed_transaction(&self) -> Option<&str> {
        const SAID: &str = "last completed transaction was at log time ";
        self.cause.iter().rev().find_map(|line| {
            line.split_once(SAID)
                .map(|(_, at)| at.trim())
                .filter(|at| !at.is_empty())
        })
    }
}

/// The lines of the act's log that say the cause: Postgres's own log, as the
/// script prints it before it fails (`database-offline: | `, step 3b part
/// 7); where it printed none, the log's last lines.
pub fn cause_lines(log: &str) -> Vec<String> {
    let postgres: Vec<String> = log
        .lines()
        .filter(|line| line.starts_with("database-offline: | "))
        .map(|line| line.trim_end().to_string())
        .collect();
    if !postgres.is_empty() {
        return postgres;
    }
    let lines: Vec<&str> = log
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.contains("FAILED"))
        .collect();
    lines[lines.len().saturating_sub(10)..]
        .iter()
        .map(|line| line.trim_end().to_string())
        .collect()
}

/// The act, in a Job made from the CronJob, run to its end: its lines said
/// as they come, and its log, or why it failed with the act's own words and
/// the cause its log gives.
pub async fn run_act(
    machine: &dyn Machine,
    release: &Release,
    cronjob: &serde_json::Value,
    act: &Act,
    unix_seconds: u64,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<String, ActFailed> {
    let ns = release.namespace.as_str();
    let name = format!(
        "{}-{unix_seconds}",
        cronjob["metadata"]["name"].as_str().unwrap_or_default()
    );
    let job = offline_job(cronjob, &name, act).map_err(|failed| ActFailed {
        why: None,
        ended: true,
        said: failed,
        cause: Vec::new(),
    })?;
    machine
        .run_with_input(
            "kubectl",
            &["create", "-n", ns, "-f", "-"],
            &job.to_string(),
        )
        .await
        .map_err(|failed| ActFailed {
            why: None,
            ended: true,
            said: format!("the {} Job could not be made: {failed}", act.name()),
            cause: Vec::new(),
        })?;
    watch.say(&format!("  job/{name}: {}", act.name()));
    let deadline = Instant::now() + pace.timeout;
    let mut said = 0;
    let logs = format!("meridian logs database-offline {}", release.scope());
    let cause = |lines: &[String]| {
        if lines.is_empty() {
            "    (it has printed nothing yet)".to_string()
        } else {
            lines
                .iter()
                .map(|line| format!("    {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    loop {
        let log = kubectl(
            machine,
            &["logs", "-n", ns, &format!("job/{name}"), "--tail=-1"],
        )
        .await
        .unwrap_or_default();
        let lines: Vec<&str> = log
            .lines()
            .filter(|line| line.starts_with("database-offline:"))
            .collect();
        for line in lines.iter().skip(said) {
            watch.say(&format!("  {line}"));
        }
        said = said.max(lines.len());
        let status = json(machine, &["get", "job", &name, "-n", ns, "-o", "json"])
            .await
            .unwrap_or_default();
        let holds = |kind: &str| {
            status["status"]["conditions"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|c| c["type"] == kind && c["status"] == "True")
        };
        if holds("Complete") {
            return Ok(log);
        }
        if holds("Failed") {
            let why = log
                .lines()
                .rev()
                .find(|line| line.contains("FAILED"))
                .map(|line| {
                    line.trim_start_matches("database-offline: ")
                        .trim_start_matches("FAILED: ")
                        .to_string()
                });
            let lines = cause_lines(&log);
            return Err(ActFailed {
                said: format!(
                    "the {} Job, job/{name}, failed: {}\nWhat its log says (`{logs}`):\n{}",
                    act.name(),
                    why.as_deref().unwrap_or("it said nothing of why"),
                    cause(&lines)
                ),
                why,
                ended: true,
                cause: lines,
            });
        }
        if Instant::now() >= deadline {
            let lines = cause_lines(&log);
            return Err(ActFailed {
                why: None,
                ended: false,
                said: format!(
                    "the {} Job, job/{name}, did not finish within {}s, and is left running. \
                     What its log says last (`{logs}`):\n{}",
                    act.name(),
                    pace.timeout.as_secs(),
                    cause(&lines)
                ),
                cause: lines,
            });
        }
        tokio::time::sleep(pace.poll).await;
    }
}

/// Under Patroni, after the act on member 0's volume: the standbys' volumes
/// deleted, so Patroni makes each again from member 0 when it starts; and,
/// for a restore, Patroni's record of where the old primary had got to,
/// which member 0's restored position is behind and would be held against.
pub async fn standbys_to_remake(
    machine: &dyn Machine,
    release: &Release,
    set: &DatabaseSet,
    members: u64,
    act: &Act,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let ns = release.namespace.as_str();
    let claims: Vec<String> = (1..members)
        .map(|n| format!("data-{}-{n}", set.name))
        .collect();
    if !claims.is_empty() {
        let mut arguments = vec!["delete", "pvc", "-n", ns, "--ignore-not-found"];
        arguments.extend(claims.iter().map(String::as_str));
        kubectl(machine, &arguments).await.map_err(|failed| {
            format!("the standbys' volumes could not be deleted, to be made again: {failed}")
        })?;
        watch.say(&format!(
            "  deleted the standbys' volumes ({}): Patroni makes each again from {}-0",
            claims.join(", "),
            set.name
        ));
    }
    if set.patroni && act.restores() {
        // Patroni's record of the old primary: its position, which member
        // 0's restored one is behind and would be refused leadership for,
        // and its synchronous standby, which member 0 may not have been.
        // Member 0 then takes its data as the cluster's, as it does when a
        // database first moves under Patroni.
        let scope = patroni_scope(&set.name);
        let records: Vec<String> = ["leader", "sync", "failover"]
            .iter()
            .map(|kind| format!("{scope}-{kind}"))
            .collect();
        let mut arguments = vec!["delete", "configmap", "-n", ns, "--ignore-not-found"];
        arguments.extend(records.iter().map(String::as_str));
        kubectl(machine, &arguments).await.map_err(|failed| {
            format!("Patroni's record of the old primary could not be cleared: {failed}")
        })?;
        watch.say(&format!(
            "  cleared Patroni's record of the old primary ({})",
            records.join(", ")
        ));
    }
    Ok(())
}

/// Patroni's scope, as the chart names it: the StatefulSet's name, cut to 63.
pub fn patroni_scope(set: &str) -> String {
    let cut: String = set.chars().take(63).collect();
    cut.trim_end_matches('-').to_string()
}

/// The database started to its count, every member ready, then every
/// component to its own, and every workload rolled out as an upgrade waits
/// for them. Whatever the database does, the components are given back
/// their counts: nothing is left stopped by this.
pub async fn start(
    machine: &dyn Machine,
    release: &Release,
    set: &DatabaseSet,
    stopped: &Stopped,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let ns = release.namespace.as_str();
    let mut failed = None;
    if stopped.database > 0 {
        let count = format!("--replicas={}", stopped.database);
        kubectl(
            machine,
            &["scale", "statefulset", &set.name, "-n", ns, &count],
        )
        .await?;
        let wanted = stopped.database as usize;
        let waited = wait_for(pace, "every member of the database to be ready", || async {
            let now = members_now(machine, release, &set.name)
                .await
                .unwrap_or_default();
            now.len() == wanted && now.iter().all(|m| m.ready && !m.terminating)
        })
        .await;
        match waited {
            Ok(()) => watch.say(&format!(
                "  the database is ready: {} member(s)",
                stopped.database
            )),
            Err(why) => failed = Some(why),
        }
    }
    started_components(machine, release, stopped, watch).await?;
    if let Some(why) = failed {
        return Err(format!(
            "{why}. Every component was started again and waits for it: `meridian status {}` \
             says where each member is",
            release.scope()
        ));
    }
    let deadline = Instant::now() + pace.timeout;
    loop {
        let listed = |what: &'static str| async move {
            kubectl(
                machine,
                &[
                    "get",
                    what,
                    "-n",
                    ns,
                    "-l",
                    &release.selector(),
                    "-o",
                    "json",
                ],
            )
            .await
            .unwrap_or_default()
        };
        let now = progress(
            0,
            &[],
            &workloads(&listed("deployments,statefulsets,replicasets").await),
            &pods(&listed("pods").await),
        );
        watch.waiting(&now);
        if now.waiting.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "after {}s, still waiting for:\n  - {}\n`meridian status {}` shows where each \
                 component is",
                pace.timeout.as_secs(),
                now.waiting.join("\n  - "),
                release.scope()
            ));
        }
        tokio::time::sleep(pace.poll).await;
    }
}

/// Each component given back the count it ran at, and each CronJob the stop
/// suspended its schedule.
pub async fn started_components(
    machine: &dyn Machine,
    release: &Release,
    stopped: &Stopped,
    watch: &mut dyn Watch,
) -> Result<(), String> {
    let mut by_count: BTreeMap<u64, Vec<&str>> = BTreeMap::new();
    for (name, replicas) in &stopped.deployments {
        by_count.entry(*replicas).or_default().push(name);
    }
    for (replicas, names) in by_count {
        let count = format!("--replicas={replicas}");
        let mut arguments = vec![
            "scale",
            "deployment",
            "-n",
            release.namespace.as_str(),
            &count,
        ];
        arguments.extend(names.iter().copied());
        kubectl(machine, &arguments).await.map_err(|failed| {
            format!(
                "the components could not be started again ({failed}): {} stay at none",
                names.join(", ")
            )
        })?;
    }
    if !stopped.deployments.is_empty() {
        watch.say(&format!(
            "  started {} component(s) again",
            stopped.deployments.len()
        ));
    }
    resume_cronjobs(machine, release, stopped, watch).await
}

/// The conductor's probe, read once it runs: what the database says of
/// itself now.
pub async fn probed_database(
    machine: &dyn Machine,
    release: &Release,
) -> Option<serde_json::Value> {
    let listed = kubectl(
        machine,
        &[
            "get",
            "pods",
            "-n",
            &release.namespace,
            "-l",
            &format!("{},meridian.dev/component=conductor", release.selector()),
            "-o",
            "json",
        ],
    )
    .await
    .ok()?;
    let pod = pods(&listed).into_iter().find(|pod| {
        pod.phase == "Running" && !pod.terminating && pod.ready_since.is_some() && pod.probes
    })?;
    let arguments = crate::probe::exec_arguments(&release.namespace, &pod.name, "conductor");
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let said = match machine.run("kubectl", &arguments).await {
        Ok(said) => said,
        Err(Failure::Said(said)) => said,
        Err(missing) => missing.to_string(),
    };
    crate::probe::first_json(&said)
        .as_ref()
        .and_then(crate::probe::database)
        .cloned()
}

#[cfg(test)]
#[path = "offline/tests.rs"]
mod tests;

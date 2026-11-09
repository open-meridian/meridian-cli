//! An act on the brought database's stopped volume (step 3b; spec/a-
//! deployment-scales-out-and-fails-over-by-configuration sections 4 and 9,
//! ruling 1058): page checksums turned on (`meridian upgrade-deployment
//! --enable-checksums`) and a restore to a point in time from the WAL archive
//! (`meridian restore --to-time`).
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
}

impl Act {
    pub fn name(&self) -> &'static str {
        match self {
            Act::EnableChecksums => "enable-checksums",
            Act::RestoreToTime(_) => "restore-to-time",
        }
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
                Act::EnableChecksums => String::new(),
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

/// What was running before the stop, to start again as it was: each of the
/// release's Deployments with its count, and the database's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stopped {
    pub deployments: Vec<(String, u64)>,
    pub database: u64,
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

/// Every component stopped: each of the release's Deployments, launched
/// plugins among them, scaled to none and its pods gone. The broker and the
/// registry keep running: neither holds the database.
pub async fn stop_components(
    machine: &dyn Machine,
    release: &Release,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<Vec<(String, u64)>, String> {
    let running = running_deployments(machine, release).await?;
    stop_these(machine, release, &running, pace, watch).await?;
    Ok(running)
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

/// Those Deployments scaled to none, and their pods gone.
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
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "database": self.database,
            "deployments": self.deployments,
        })
        .to_string()
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
        })
    }
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

/// The act, in a Job made from the CronJob, run to its end: its lines said
/// as they come, and its log, or why it failed with the act's own words.
pub async fn run_act(
    machine: &dyn Machine,
    release: &Release,
    cronjob: &serde_json::Value,
    act: &Act,
    unix_seconds: u64,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<String, String> {
    let ns = release.namespace.as_str();
    let name = format!(
        "{}-{unix_seconds}",
        cronjob["metadata"]["name"].as_str().unwrap_or_default()
    );
    let job = offline_job(cronjob, &name, act)?;
    machine
        .run_with_input(
            "kubectl",
            &["create", "-n", ns, "-f", "-"],
            &job.to_string(),
        )
        .await
        .map_err(|failed| format!("the {} Job could not be made: {failed}", act.name()))?;
    watch.say(&format!("  job/{name}: {}", act.name()));
    let deadline = Instant::now() + pace.timeout;
    let mut said = 0;
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
                .map(|line| line.trim_start_matches("database-offline: ").to_string())
                .unwrap_or_else(|| "it said nothing of why".to_string());
            return Err(format!("the {} Job, job/{name}, failed: {why}", act.name()));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the {} Job, job/{name}, had not finished after {}s; it is left running, and \
                 `meridian logs database-offline {}` says where it is",
                act.name(),
                pace.timeout.as_secs(),
                release.scope()
            ));
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
    if set.patroni && matches!(act, Act::RestoreToTime(_)) {
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

/// Each component given back the count it ran at.
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
    Ok(())
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

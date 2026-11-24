//! `meridian stop` and `meridian start`: a deployment's workloads stopped,
//! and started again, deleting nothing. Its data, its volumes and its
//! database are kept; nothing runs until it is started.
//!
//! The stop is the acts' own (an act on the stopped database's volume, this
//! module's parent), without the act: what runs is recorded first on the
//! database's StatefulSet, under a key of its own (`meridian.dev/stopped`,
//! so a restore's record and this one are never read as each other); the
//! release's CronJobs are suspended, every component stopped, launched
//! plugins among them; under Patroni member 0 is made the primary; then the
//! database is stopped. `start` gives back exactly what was recorded and
//! removes the record. Either, run again after a session that did not
//! finish, takes it from where it was: the record stands until `start` has
//! finished.
//!
//! No helm operation, no new component in the cluster: the person's own
//! kubectl, with their own rights, as the acts use it.
//!
//! What it does not stop, it says: StatefulSets that hold no database (the
//! broker's and the registry's) keep running, as they do under an act, and
//! DaemonSets are not read at all.

use serde_json::Value;

use super::{
    cronjobs_now, kubectl, members_now, running_deployments, several_databases,
    several_databases_refused, stop_database, stop_these, suspend_cronjobs, Release, Stopped,
};
use crate::doctor::Machine;
use crate::upgrade_deployment::run::Pace;
use crate::upgrade_deployment::watch::Watch;
use crate::upgrade_deployment::{database_set_of, DatabaseSet};

/// The annotation on the database's StatefulSet that holds what `meridian
/// stop` stopped, while it is stopped.
pub const STOPPED_BY_STOP: &str = "meridian.dev/stopped";

/// The act a stop's record names.
pub const ACT: &str = "stop";

/// The record a StatefulSet as `kubectl get -o json` lists it holds, where
/// it holds one.
pub fn record_on(set: &Value) -> Option<Stopped> {
    Stopped::from_json(set["metadata"]["annotations"][STOPPED_BY_STOP].as_str()?)
}

/// The database's StatefulSet among listed workloads, as listed.
fn database_item<'a>(listed: &'a Value, set: &str) -> Option<&'a Value> {
    listed["items"]
        .as_array()?
        .iter()
        .find(|item| item["kind"] == "StatefulSet" && item["metadata"]["name"] == set)
}

/// What `meridian stop` recorded, among a release's workloads as listed
/// (`kubectl get deployments,statefulsets -o json`): None where it is not
/// stopped, or brings no database.
pub fn stopped_in(listed: &Value) -> Option<Stopped> {
    let set = database_set_of(listed)?;
    database_item(listed, &set.name).and_then(record_on)
}

/// `meridian start`, with `-n` and `--release` where they are not the
/// default, as a person runs it.
pub fn start_command(scope: &str) -> String {
    match scope {
        "" | "-n meridian" => "meridian start".to_string(),
        scope => format!("meridian start {scope}"),
    }
}

/// The line `status` says in place of what is not ready, while it is
/// stopped.
pub fn status_line(stopped: &Stopped, scope: &str) -> String {
    format!(
        "Stopped since {}; run `{}`",
        stopped.since.as_deref().unwrap_or("a time not recorded"),
        start_command(scope)
    )
}

/// Why `command` is refused while the release is stopped, where it is: what
/// it would start again, the command that starts it, and that nothing was
/// changed. None where it is not stopped, or what it runs cannot be read
/// (the command's own checks say that).
pub async fn refused_while_stopped(
    machine: &dyn Machine,
    release: &Release,
    command: &str,
) -> Option<String> {
    let listed = super::release_workloads(machine, release).await.ok()?;
    let stopped = stopped_in(&listed)?;
    Some(format!(
        "{} in {} is stopped (`meridian stop`{}), and `{command}` would start parts of it \
         again. `{}` starts it as it was; then run this again. Nothing was changed",
        release.release,
        release.namespace,
        stopped
            .since
            .as_deref()
            .map(|at| format!(", since {at}"))
            .unwrap_or_default(),
        start_command(&release.scope()),
    ))
}

/// A Job of the release that is running and that a stop would cut short:
/// its name and what it is.
pub fn running_job(listed: &Value) -> Option<(String, String)> {
    listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|job| job["status"]["active"].as_u64().unwrap_or(0) > 0)
        .find_map(|job| {
            let name = job["metadata"]["name"].as_str()?.to_string();
            let labels = &job["metadata"]["labels"];
            let component = labels["meridian.dev/component"]
                .as_str()
                .unwrap_or_default();
            let what = if let Some(act) = labels["meridian.dev/database-act"].as_str() {
                format!("an act on the database's volume, {act}")
            } else if component == "migrate" || name.contains("-migrate-") {
                "a migration".to_string()
            } else if component == "restore" || name.contains("-restore") {
                "a restore".to_string()
            } else if component == "rollback" || name.contains("-rollback") {
                "a rollback".to_string()
            } else {
                return None;
            };
            Some((name, what))
        })
}

/// What a stop would do, once every check has passed.
#[derive(Debug, Clone)]
pub struct Plan {
    pub set: DatabaseSet,
    /// What a stop that did not finish recorded: it stands over what runs
    /// now, which that stop stopped or suspended itself.
    pub earlier: Option<Stopped>,
    /// Each component running now, with its count.
    pub running: Vec<(String, u64)>,
    /// The release's StatefulSets that hold no database, which keep running.
    pub kept_running: Vec<String>,
}

/// What `stop`'s checks found.
#[derive(Debug)]
pub enum Checked {
    Stop(Box<Plan>),
    /// Stopped already: said, and nothing changed.
    Already(String),
}

/// Every check, nothing changed: refused, naming why, where a restore's or
/// an act's record is there, Helm holds the release as pending, or a
/// migration, a restore, a rollback or an act runs.
pub async fn check(machine: &dyn Machine, release: &Release) -> Result<Checked, String> {
    let scope = release.scope();
    let installed = crate::upgrade_deployment::run::release(
        machine,
        &crate::upgrade_deployment::Asked {
            release: release.release.clone(),
            namespace: release.namespace.clone(),
            chart: String::new(),
            chart_version: None,
            timeout: String::new(),
            https: false,
            authority: None,
            values: Vec::new(),
            sets: Vec::new(),
        },
    )
    .await
    .map_err(|failed| {
        format!(
            "the release could not be read: {failed}. Point KUBECONFIG at the cluster this \
             deployment runs in; this drives your own helm"
        )
    })?;
    let Some(installed) = installed else {
        return Err(format!(
            "there is no release {} in {}: `meridian status -n {}` names the releases there",
            release.release, release.namespace, release.namespace
        ));
    };
    if installed.status.starts_with("pending") {
        return Err(format!(
            "{} is at revision {}, and Helm holds it as {}: another Helm operation is running \
             on it, or one was interrupted. Let it finish (`meridian status {scope}` says what \
             Helm says of it), then stop it",
            release.release, installed.revision, installed.status
        ));
    }
    let listed = super::release_workloads(machine, release)
        .await
        .map_err(|failed| {
            format!(
                "the release's workloads could not be read: {failed}. This account needs to \
                 read the namespace's Deployments and StatefulSets"
            )
        })?;
    let several = several_databases(&listed);
    if !several.is_empty() {
        return Err(format!(
            "{}. A stop records one database's count, and would leave the others running",
            several_databases_refused("a stop", &several)
        ));
    }
    let Some(set) = database_set_of(&listed) else {
        return Err(format!(
            "{} runs no database it brought, and a stop is recorded on the database's \
             StatefulSet: there is nowhere to keep what it stops",
            release.release
        ));
    };
    let item = database_item(&listed, &set.name)
        .cloned()
        .unwrap_or_default();
    if let Some(act) = super::stopped_on(&item) {
        return Err(format!(
            "{} was stopped by `{}`{}, which has not finished: its record is on {}. `{}` takes \
             it from there and starts everything again",
            release.release,
            act.command(),
            act.since
                .as_deref()
                .map(|at| format!(" at {at}"))
                .unwrap_or_default(),
            set.name,
            act.resumed_by(&scope)
        ));
    }
    let jobs = kubectl(
        machine, // S9: kubectl get jobs
        &[
            "get",
            "jobs",
            "-n",
            &release.namespace,
            "-l",
            &release.selector(),
            "-o",
            "json",
        ],
    )
    .await
    .map_err(|failed| {
        format!(
            "the release's Jobs could not be read ({failed}), and a stop never cuts a \
             migration, a restore or a rollback short"
        )
    })?;
    if let Some((job, what)) = running_job(&serde_json::from_str(&jobs).unwrap_or_default()) {
        let follow = match what.as_str() {
            "a migration" => format!("`meridian logs migrate {scope}` says where it is"),
            act if act.starts_with("an act") => {
                format!("`meridian logs database-offline {scope}` says where it is")
            }
            _ => format!("`meridian status {scope}` says what runs"),
        };
        return Err(format!(
            "job/{job}, {what}, is running, and a stop would cut it short. {follow}; once it \
             has ended, run this again"
        ));
    }
    let earlier = record_on(&item);
    let running = running_deployments(machine, release).await?;
    let kept_running = kept_running(&listed, &set.name);
    match &earlier {
        None if set.replicas == 0 => Err(format!(
            "{}'s database, {}, runs no member, and nothing recorded stopping it: a stop \
             records what runs, to start it again as it was. `meridian status {scope}` says \
             what runs",
            release.release, set.name
        )),
        Some(stopped) if set.replicas == 0 && running.is_empty() => {
            let suspended_now = cronjobs_now(machine, release, &set.name)
                .await
                .unwrap_or_default();
            let all_suspended = stopped
                .cronjobs
                .iter()
                .filter(|(_, suspended)| !suspended)
                .all(|(name, _)| {
                    suspended_now
                        .iter()
                        .find(|(now, _)| now == name)
                        .is_none_or(|(_, suspended)| *suspended)
                });
            if all_suspended {
                return Ok(Checked::Already(format!(
                    "{} in {} is stopped already{}; `{}` starts it.",
                    release.release,
                    release.namespace,
                    stopped
                        .since
                        .as_deref()
                        .map(|at| format!(", since {at}"))
                        .unwrap_or_default(),
                    start_command(&scope)
                )));
            }
            Ok(Checked::Stop(Box::new(Plan {
                set,
                earlier,
                running,
                kept_running,
            })))
        }
        _ => Ok(Checked::Stop(Box::new(Plan {
            set,
            earlier,
            running,
            kept_running,
        }))),
    }
}

/// The release's StatefulSets that hold no database, by name: a stop leaves
/// them running.
fn kept_running(listed: &Value, database: &str) -> Vec<String> {
    listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["kind"] == "StatefulSet")
        .filter_map(|item| item["metadata"]["name"].as_str())
        .filter(|name| *name != database)
        .map(String::from)
        .collect()
}

/// What it will do, said before it asks.
pub fn plan_text(release: &Release, plan: &Plan) -> String {
    let members = plan
        .earlier
        .as_ref()
        .map(|e| e.database)
        .filter(|n| *n > 0)
        .unwrap_or(plan.set.replicas);
    let mut out = format!(
        "Stopping {} in {}: nothing is deleted. Its data, its volumes and its database are \
         kept; nothing answers until `{}`.\n",
        release.release,
        release.namespace,
        start_command(&release.scope())
    );
    out.push_str(&format!(
        "  1. Recorded first, on {} ({STOPPED_BY_STOP}): each component's count, the \
         database's ({members}), each CronJob's schedule.\n",
        plan.set.name
    ));
    if plan.earlier.is_some() {
        out.push_str(
            "     A stop that did not finish recorded it already: its counts stand, and \
             anything running since is added.\n",
        );
    }
    out.push_str(
        "  2. The release's CronJobs suspended (the replica comparison, the backup, the \
         scrub).\n",
    );
    out.push_str(&match plan.running.len() {
        0 => "  3. No component runs: none to stop.\n".to_string(),
        n => format!(
            "  3. {n} component(s) stopped, launched plugins among them: {}.\n",
            plan.running
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    });
    out.push_str(&format!(
        "  4. {}the database stopped, {}.\n",
        if plan.set.patroni && plan.set.replicas > 1 {
            "Member 0 made Patroni's primary, so no member fails over as the others stop; then "
        } else {
            ""
        },
        plan.set.name
    ));
    if !plan.kept_running.is_empty() {
        out.push_str(&format!(
            "  Kept running, as under a restore: {} (they hold no database).\n",
            plan.kept_running.join(", ")
        ));
    }
    out
}

/// The record's value set on the database's StatefulSet, or removed (None).
async fn annotate(
    machine: &dyn Machine,
    release: &Release,
    set: &str,
    record: Option<&Stopped>,
) -> Result<(), String> {
    let value = match record {
        Some(record) => format!("{STOPPED_BY_STOP}={}", record.to_json()),
        None => format!("{STOPPED_BY_STOP}-"),
    };
    let mut arguments = vec!["annotate", "statefulset", set, "-n", &release.namespace];
    if record.is_some() {
        arguments.push("--overwrite");
    }
    arguments.push(&value);
    kubectl(machine, &arguments) // S9: kubectl annotate statefulsets
        .await
        .map(|_| ())
}

/// What is stopped, recorded before anything is: what a stop that did not
/// finish recorded, with anything running now that it did not.
pub async fn record(
    machine: &dyn Machine,
    release: &Release,
    plan: &Plan,
) -> Result<Stopped, String> {
    let earlier = plan.earlier.as_ref();
    let mut deployments = earlier.map(|e| e.deployments.clone()).unwrap_or_default();
    for (name, count) in &plan.running {
        if !deployments.iter().any(|(had, _)| had == name) {
            deployments.push((name.clone(), *count));
        }
    }
    let mut cronjobs = earlier.map(|e| e.cronjobs.clone()).unwrap_or_default();
    for (name, suspended) in cronjobs_now(machine, release, &plan.set.name).await? {
        if !cronjobs.iter().any(|(had, _)| *had == name) {
            cronjobs.push((name, suspended));
        }
    }
    let stopped = Stopped {
        deployments,
        database: earlier
            .map(|e| e.database)
            .filter(|n| *n > 0)
            .unwrap_or(plan.set.replicas),
        cronjobs,
        act: Some(ACT.to_string()),
        since: earlier
            .and_then(|e| e.since.clone())
            .or_else(|| Some(super::utc(machine.now_s()))),
    };
    annotate(machine, release, &plan.set.name, Some(&stopped))
        .await
        .map_err(|failed| format!("what it stops could not be recorded first: {failed}"))?;
    Ok(stopped)
}

/// Record, suspend, stop: what was done, or where it stopped and what to
/// run. The record stands either way, so `meridian stop` again finishes it
/// and `meridian start` starts what was stopped.
pub async fn stop(
    machine: &dyn Machine,
    release: &Release,
    plan: &Plan,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<String, String> {
    let scope = release.scope();
    let stopped = record(machine, release, plan).await?;
    watch.say(&format!(
        "  recorded what runs on {} ({STOPPED_BY_STOP})",
        plan.set.name
    ));
    let stopping = async {
        suspend_cronjobs(machine, release, &stopped, watch).await?;
        stop_these(machine, release, &plan.running, pace, watch).await?;
        let members = members_now(machine, release, &plan.set.name)
            .await
            .unwrap_or_default();
        if plan.set.patroni && members.len() > 1 {
            super::primary_to_member_0(machine, release, &plan.set.name, pace, watch).await?;
        }
        if plan.set.replicas > 0 || !members.is_empty() {
            stop_database(machine, release, &plan.set, pace, watch).await?;
        }
        Ok::<(), String>(())
    }
    .await;
    match stopping {
        Ok(()) => Ok(format!(
            "\n{} in {} is stopped. Its data, volumes and database are kept; `{}` starts it \
             again as it was.\n",
            release.release,
            release.namespace,
            start_command(&scope)
        )),
        Err(failed) => Err(format!(
            "stopping did not finish: {failed}\nWhat it stops is recorded on {}: `meridian stop \
             {scope}` again finishes the stop, and `{}` starts everything as it was.",
            plan.set.name,
            start_command(&scope)
        )),
    }
}

/// What `start`'s checks found.
#[derive(Debug)]
pub enum Starting {
    Start(DatabaseSet, Stopped),
    /// Not stopped by `meridian stop`: said, and nothing changed.
    Nothing(String),
}

/// Whether there is a stop to start from: refused where a restore's or an
/// act's record is there, which only that act finishes.
pub async fn check_start(machine: &dyn Machine, release: &Release) -> Result<Starting, String> {
    let scope = release.scope();
    let listed = super::release_workloads(machine, release)
        .await
        .map_err(|failed| {
            format!(
                "the release's workloads could not be read: {failed}. This account needs to \
                 read the namespace's Deployments and StatefulSets"
            )
        })?;
    let Some(set) = database_set_of(&listed) else {
        return Ok(Starting::Nothing(format!(
            "{} in {} runs no database it brought, so `meridian stop` has not stopped it; \
             nothing to start.",
            release.release, release.namespace
        )));
    };
    let item = database_item(&listed, &set.name)
        .cloned()
        .unwrap_or_default();
    if let Some(act) = super::stopped_on(&item) {
        return Err(format!(
            "{} was stopped by `{}`, not `meridian stop`, and that has not finished. `{}` takes \
             it from there and starts everything again. Nothing was changed",
            release.release,
            act.command(),
            act.resumed_by(&scope)
        ));
    }
    match record_on(&item) {
        Some(stopped) => Ok(Starting::Start(set, stopped)),
        None => Ok(Starting::Nothing(format!(
            "{} in {} is not stopped: nothing to start. `meridian status {scope}` says what \
             runs.",
            release.release, release.namespace
        ))),
    }
}

/// The database started to its recorded count, then every component to
/// its own, each CronJob given back its schedule; the record removed once
/// everything runs. Where it does not finish, the record stays, and
/// `meridian start` again finishes it.
pub async fn start(
    machine: &dyn Machine,
    release: &Release,
    set: &DatabaseSet,
    stopped: &Stopped,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<String, String> {
    let scope = release.scope();
    super::start(machine, release, set, stopped, pace, watch)
        .await
        .map_err(|failed| {
            format!(
                "starting did not finish: {failed}\nThe record of what was stopped stays on {}: \
                 `{}` again finishes it.",
                set.name,
                start_command(&scope)
            )
        })?;
    annotate(machine, release, &set.name, None)
        .await
        .map_err(|failed| {
            format!(
                "everything runs again, and its record on {} could not be removed ({failed}): \
                 `{}` again removes it",
                set.name,
                start_command(&scope)
            )
        })?;
    Ok(format!(
        "\n{} in {} runs again: the database's {} member(s), {} component(s) and each \
         CronJob's schedule, as they were.\n",
        release.release,
        release.namespace,
        stopped.database,
        stopped.deployments.len()
    ))
}

// ── The commands ─────────────────────────────────────────────────────────────

/// `meridian stop`: checked, shown, asked (`approve`), stopped. Its exit code.
pub async fn stop_command(
    machine: &dyn Machine,
    release: &Release,
    pace: &Pace,
    approve: &mut dyn FnMut(&str) -> bool,
) -> i32 {
    let plan = match check(machine, release).await {
        Ok(Checked::Stop(plan)) => plan,
        Ok(Checked::Already(said)) => {
            println!("{said}");
            return 0;
        }
        Err(refused) => {
            eprintln!("meridian stop: {refused}. Nothing was changed.");
            return 1;
        }
    };
    if !approve(&plan_text(release, &plan)) {
        eprintln!(
            "meridian stop: not approved, so nothing was changed. Where there is no terminal to \
             ask at, --yes approves"
        );
        return 1;
    }
    let mut say = |line: &str| eprintln!("{line}");
    match stop(machine, release, &plan, pace, &mut say).await {
        Ok(done) => {
            print!("{done}");
            0
        }
        Err(failed) => {
            eprintln!("meridian stop: {failed}");
            1
        }
    }
}

/// `meridian start`: what `meridian stop` recorded, started again. Its exit
/// code.
pub async fn start_command_run(machine: &dyn Machine, release: &Release, pace: &Pace) -> i32 {
    let (set, stopped) = match check_start(machine, release).await {
        Ok(Starting::Start(set, stopped)) => (set, stopped),
        Ok(Starting::Nothing(said)) => {
            println!("{said}");
            return 0;
        }
        Err(refused) => {
            eprintln!("meridian start: {refused}.");
            return 1;
        }
    };
    eprintln!(
        "Starting {} in {}: the database's {} member(s), then {} component(s), each to the \
         count it ran at, and each CronJob's schedule as it was.",
        release.release,
        release.namespace,
        stopped.database,
        stopped.deployments.len()
    );
    let mut say = |line: &str| eprintln!("{line}");
    match start(machine, release, &set, &stopped, pace, &mut say).await {
        Ok(done) => {
            print!("{done}");
            0
        }
        Err(failed) => {
            eprintln!("meridian start: {failed}");
            1
        }
    }
}

#[cfg(test)]
#[path = "stop_tests.rs"]
mod tests;

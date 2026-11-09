//! `meridian upgrade-deployment --enable-checksums` (ruling 1058; spec
//! section 9, "An existing brought database"): page checksums turned on in
//! a database the chart brought before they were on. The guided step as the
//! spec states it: whether they are on read first, and nothing changed if
//! they are; the downtime, from the database's size, said before it asks; a
//! backup first, and nothing changed if it fails; every component and the
//! database stopped and pg_checksums run on the stopped volume; everything
//! started again, and the setting read again.

use super::{
    data_bytes, database, downtime, minutes, probed_database, run_act, standbys_to_remake, start,
    started_components, stop_components, stop_database, Act, Release, Stopped,
};
use crate::doctor::{Finding, Machine};
use crate::upgrade_deployment::run::Pace;
use crate::upgrade_deployment::watch::{Step, Watch};
use crate::upgrade_deployment::{release_finding, Asked, DatabaseSet};

/// What would be done, once every check has passed.
#[derive(Debug, Clone)]
pub struct Plan {
    pub set: DatabaseSet,
    pub cronjob: serde_json::Value,
    /// The data directory's size on member 0, where it was read.
    pub bytes: Option<u64>,
}

/// What the checks found.
pub enum Checked {
    Enable(Box<Plan>),
    /// On already, or nothing to turn on: said, and nothing changed.
    Nothing(String),
    Refused,
}

fn release_of(asked: &Asked) -> Release {
    Release {
        release: asked.release.clone(),
        namespace: asked.namespace.clone(),
    }
}

/// Every check, said as made; nothing changed.
pub async fn check(machine: &dyn Machine, asked: &Asked, watch: &mut dyn Watch) -> Checked {
    watch.begin(Step::Checks);
    let checked = checks_made(machine, asked, watch).await;
    watch.end(Step::Checks, !matches!(checked, Checked::Refused));
    checked
}

async fn checks_made(machine: &dyn Machine, asked: &Asked, say: &mut dyn Watch) -> Checked {
    let release = release_of(asked);
    let mut findings = Vec::new();
    let refuse = |findings: &[Finding], say: &mut dyn Watch| {
        for finding in findings {
            say.say(&format!("{finding}"));
        }
        say.say("\nNothing was changed. Each check that stopped it says what to do above.");
        Checked::Refused
    };
    let installed = match crate::upgrade_deployment::run::release(machine, asked).await {
        Ok(installed) => installed,
        Err(failed) => {
            findings.push(Finding::Stops {
                what: format!("the release could not be read: {failed}"),
                fix: "Point KUBECONFIG at the cluster this deployment runs in; this drives your \
                      own helm."
                    .into(),
            });
            return refuse(&findings, say);
        }
    };
    findings.push(release_finding(asked, installed.as_ref()));
    if findings.iter().any(Finding::stops) {
        return refuse(&findings, say);
    }

    let set = match database(machine, &release).await {
        Ok(Some(set)) if set.replicas > 0 => set,
        Ok(_) => {
            findings.push(Finding::Stops {
                what: format!(
                    "{} runs no database it brought, so there is no volume here to turn page \
                     checksums on in",
                    asked.release
                ),
                fix: "A database somebody else runs has its checksums set where it runs: at \
                      initdb (--data-checksums), or with PostgreSQL's pg_checksums --enable on \
                      the stopped server; on a managed database, its provider's setting."
                    .into(),
            });
            return refuse(&findings, say);
        }
        Err(failed) => {
            findings.push(Finding::Stops {
                what: format!("the release's workloads could not be read: {failed}"),
                fix: "This account needs to read the namespace's Deployments and StatefulSets."
                    .into(),
            });
            return refuse(&findings, say);
        }
    };

    // Read first: on already is nothing to do.
    let probed = probed_database(machine, &release).await;
    let on = match probed
        .as_ref()
        .and_then(|database| crate::probe::setting(database, "data_checksums"))
    {
        Some(said) => said.to_string(),
        None => {
            findings.push(Finding::Stops {
                what: "whether the database has page checksums could not be read: the \
                       conductor's probe did not say"
                    .into(),
                fix: format!(
                    "The probe is step 3b's; upgrade to a chart with it first (`meridian \
                     upgrade-deployment {}`), and with every component ready run this again.",
                    crate::status::scope(&asked.release, &asked.namespace)
                ),
            });
            return refuse(&findings, say);
        }
    };
    if on == "on" {
        findings.push(Finding::Fine(format!(
            "{} has page checksums on (data_checksums)",
            set.name
        )));
        for finding in &findings {
            say.say(&format!("{finding}"));
        }
        return Checked::Nothing(format!(
            "{}'s database has page checksums on already; nothing to do.",
            asked.release
        ));
    }
    findings.push(Finding::Fine(format!(
        "{} has no page checksums (data_checksums is {on}): this turns them on",
        set.name
    )));

    let cronjob = match super::cronjob(machine, &release, &set.name).await {
        Ok(cronjob) => cronjob,
        Err(_) => {
            findings.push(Finding::Stops {
                what: format!(
                    "{} has no {}-offline CronJob, which the act runs from",
                    asked.release, set.name
                ),
                fix: format!(
                    "It is step 3b's chart's: `meridian upgrade-deployment {}` to it first.",
                    crate::status::scope(&asked.release, &asked.namespace)
                ),
            });
            return refuse(&findings, say);
        }
    };
    findings.push(Finding::Fine(format!(
        "{}-offline holds the act, enable-checksums",
        set.name
    )));

    // A backup comes first, and there must be one to take.
    let backups = super::kubectl(
        machine,
        &[
            "get",
            "cronjob",
            "-n",
            &asked.namespace,
            "-l",
            &format!("{},meridian.dev/component=backup", release.selector()),
            "-o",
            "name",
        ],
    )
    .await
    .unwrap_or_default();
    if backups.trim().is_empty() {
        findings.push(Finding::Stops {
            what: format!(
                "{} has no backup configured, and a backup is taken first",
                asked.release
            ),
            fix: format!(
                "Make a key with `meridian backup key`, set backup.recipient to its public half \
                 and backup.path or backup.existingClaim, apply them with `meridian \
                 upgrade-deployment {} -f <file>`, then run this again.",
                crate::status::scope(&asked.release, &asked.namespace)
            ),
        });
        return refuse(&findings, say);
    }
    findings.push(Finding::Fine(
        "a backup is configured, and is taken first".into(),
    ));

    let bytes = data_bytes(machine, &release, &set.name).await;
    if bytes.is_none() {
        findings.push(Finding::Unknown {
            what: "the database's size".into(),
            why: "its data directory could not be measured, so the downtime said is the \
                  least it could be"
                .into(),
        });
    }
    for finding in &findings {
        say.say(&format!("{finding}"));
    }
    Checked::Enable(Box::new(Plan {
        set,
        cronjob,
        bytes,
    }))
}

/// What it will do, said before it asks.
pub fn plan_text(asked: &Asked, plan: &Plan) -> String {
    let standbys = plan.set.replicas.saturating_sub(1);
    let size = plan
        .bytes
        .map(crate::status::said_bytes)
        .unwrap_or_else(|| "a size not read".into());
    let took = downtime(plan.bytes.unwrap_or(0), standbys);
    let cronjob = plan.cronjob["metadata"]["name"]
        .as_str()
        .unwrap_or_default();
    let patroni = plan.set.patroni;
    let mut out = format!(
        "Turning on page checksums in {set} ({release} in {namespace}):\n\
         \x20 the database  {size} on member 0's volume, {members}{under}\n\
         \x20 downtime      {about}, an estimate: pg_checksums reads and writes every page, \
         taken at 50 MiB/s, and stopping and starting everything takes a few minutes{remade}.\n\
         \x20               Nothing answers until it is done.\n\n",
        set = plan.set.name,
        release = asked.release,
        namespace = asked.namespace,
        members = match plan.set.replicas {
            1 => "1 member".to_string(),
            n => format!("{n} members"),
        },
        under = if patroni { ", under Patroni" } else { "" },
        about = minutes(took),
        remade = if standbys > 0 {
            format!("; each of its {standbys} standbys is made again from member 0 after")
        } else {
            String::new()
        },
    );
    out.push_str(
        "  1. A backup, now, as `meridian backup` takes one. If it fails, nothing is changed.\n",
    );
    out.push_str(&format!(
        "  2. Every component stopped{}, then the database.\n",
        if patroni && standbys > 0 {
            "; member 0 made the primary"
        } else {
            ""
        }
    ));
    out.push_str(&format!(
        "  3. pg_checksums --enable on member 0's volume, in a Job from {cronjob}.\n"
    ));
    if patroni && standbys > 0 {
        out.push_str(&format!(
            "  4. The standbys' volumes deleted ({}), which Patroni makes again from member 0; \
             the database started, then every component, each to the count it ran at.\n",
            (1..plan.set.replicas)
                .map(|n| format!("data-{}-{n}", plan.set.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else {
        out.push_str(
            "  4. The database started, then every component, each to the count it ran at.\n",
        );
    }
    out.push_str("  5. data_checksums read again, to say it is on.\n");
    out
}

/// What was done, said at the end.
pub fn done_text(asked: &Asked, plan: &Plan, backup: &str, act_log: &str) -> String {
    let mut out = format!(
        "\nPage checksums are on in {}'s database, {}.\n{backup}",
        asked.release, plan.set.name
    );
    for line in act_log
        .lines()
        .filter(|line| line.starts_with("database-offline: "))
    {
        out.push_str(&format!("  {line}\n"));
    }
    out
}

/// Back up, stop, act, start, verify. What was done, or at which step it
/// stopped, what state that left, and what to run.
pub async fn apply(
    machine: &dyn Machine,
    asked: &Asked,
    plan: &Plan,
    pace: &Pace,
    unix_seconds: u64,
    watch: &mut dyn Watch,
) -> Result<String, String> {
    let release = release_of(asked);
    let scope = crate::status::scope(&asked.release, &asked.namespace);

    watch.begin(Step::Backup);
    let taken = crate::backup::take_in_cluster(
        machine,
        &crate::backup::Take {
            release: asked.release.clone(),
            namespace: asked.namespace.clone(),
            to: None,
            timeout: pace.timeout,
        },
        unix_seconds,
    )
    .await;
    watch.end(Step::Backup, taken.is_ok());
    let taken =
        taken.map_err(|failed| format!("the backup failed, so nothing was changed: {failed}"))?;
    watch.say(&format!("  backed up to {}", taken.written.name));

    watch.begin(Step::Stop);
    let mut stopped = Stopped {
        deployments: Vec::new(),
        database: plan.set.replicas,
    };
    let stopping = async {
        stopped.deployments = stop_components(machine, &release, pace, watch).await?;
        if plan.set.patroni && plan.set.replicas > 1 {
            super::primary_to_member_0(machine, &release, &plan.set.name, pace, watch).await?;
        }
        stop_database(machine, &release, &plan.set, pace, watch).await
    }
    .await;
    watch.end(Step::Stop, stopping.is_ok());
    if let Err(failed) = stopping {
        // Before the act: the database is as it was, and everything is
        // given back its count.
        let back = start(machine, &release, &plan.set, &stopped, pace, watch).await;
        return Err(format!(
            "stopping everything did not finish: {failed}\nThe database was not changed. {}",
            match back {
                Ok(()) => "Everything was started again.".to_string(),
                Err(why) => format!("Starting it again: {why}"),
            }
        ));
    }

    watch.begin(Step::Act);
    let act = run_act(
        machine,
        &release,
        &plan.cronjob,
        &Act::EnableChecksums,
        unix_seconds,
        pace,
        watch,
    )
    .await;
    watch.end(Step::Act, act.is_ok());
    let log = match act {
        Ok(log) => log,
        Err(failed) => {
            // pg_checksums changes the control file last, so a run that did
            // not finish leaves the database as it was: started again.
            let back = start(machine, &release, &plan.set, &stopped, pace, watch).await;
            return Err(format!(
                "{failed}\npg_checksums marks the database last, so it is as it was, without \
                 checksums. {} The backup taken first is {}.",
                match back {
                    Ok(()) => "Everything was started again.".to_string(),
                    Err(why) => format!("Starting it again: {why}"),
                },
                taken.written.name
            ));
        }
    };

    watch.begin(Step::Start);
    let starting = async {
        if plan.set.patroni && plan.set.replicas > 1 {
            standbys_to_remake(
                machine,
                &release,
                &plan.set,
                plan.set.replicas,
                &Act::EnableChecksums,
                watch,
            )
            .await?;
        }
        start(machine, &release, &plan.set, &stopped, pace, watch).await
    }
    .await;
    watch.end(Step::Start, starting.is_ok());
    if let Err(failed) = starting {
        // Never left stopped by this: the components were given their
        // counts in start, or are given them here.
        let _ = started_components(machine, &release, &stopped, watch).await;
        return Err(format!(
            "page checksums were turned on, and starting everything again did not finish: \
             {failed}\n`meridian status {scope}` says where each part is."
        ));
    }

    watch.begin(Step::Verify);
    let now = probed_database(machine, &release).await;
    let on = now
        .as_ref()
        .and_then(|database| crate::probe::setting(database, "data_checksums"))
        .map(String::from);
    watch.end(Step::Verify, on.as_deref() == Some("on"));
    match on.as_deref() {
        Some("on") => Ok(done_text(asked, plan, &taken.said, &log)),
        Some(other) => Err(format!(
            "the act said it turned page checksums on, and the database says data_checksums is \
             {other}. `meridian status {scope}` reads it again; the backup taken first is {}.",
            taken.written.name
        )),
        None => Err(format!(
            "page checksums were turned on and everything started again, and the conductor's \
             probe did not say data_checksums. `meridian status {scope}` reads it."
        )),
    }
}

#[cfg(test)]
#[path = "checksums_tests.rs"]
mod tests;

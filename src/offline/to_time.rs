//! `meridian restore --to-time <RFC 3339>` (step 3b; spec section 4): the
//! brought database restored from its WAL archive to a point in time, in
//! place. Refused where the deployment keeps no archive (`backup.walArchive`),
//! saying how to turn it on. Every component and the database stopped, the
//! act run on member 0's volume (the archive's newest base backup before the
//! time restored and the WAL replayed to it, the database promoted there on
//! a timeline of its own), and everything started again; under Patroni the
//! standbys are made again from it. A time outside the archive fails with
//! the act's own words, the volume left restorable and everything left
//! stopped, with what to run.
//!
//! The archive's passphrase is the chart's, read by the act in its own pod;
//! nothing here reads or prints it.

use super::{
    act_running, clear_stopped, database, members_now, record_stop, run_act, standbys_to_remake,
    start, started_components, stop_database, stop_these, suspend_cronjobs, Act, Release, Stopped,
};
use crate::doctor::{Finding, Machine};
use crate::upgrade_deployment::run::Pace;
use crate::upgrade_deployment::watch::{Step, Watch};
use crate::upgrade_deployment::{release_finding, Asked, DatabaseSet};

/// A time as RFC 3339 gives it, in UTC: its whole seconds since the epoch,
/// its fraction as written, and the form the act takes
/// (`YYYY-MM-DD HH:MM:SS[.f]+00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moment {
    pub unix_seconds: i64,
    pub for_the_act: String,
    /// As it is said back: `2026-11-08T14:03:07Z`.
    pub said: String,
}

/// `2026-11-08T09:03:07.25-05:00`, `2026-11-08 14:03:07Z`: RFC 3339, with a
/// `T` or a space, a fraction or none, `Z` or an offset. Anything else is
/// refused, saying the form.
pub fn moment(given: &str) -> Result<Moment, String> {
    let form = || {
        format!(
            "--to-time takes a time as RFC 3339 writes one, its offset named: \
             2026-11-08T14:03:07Z or 2026-11-08T09:03:07-05:00, not `{given}`"
        )
    };
    let given = given.trim();
    let digits = |s: &str| -> Option<i64> {
        (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    if given.len() < 20 || !given.is_char_boundary(10) || !given.is_char_boundary(19) {
        return Err(form());
    }
    let (date, rest) = given.split_at(10);
    let separator = rest.chars().next().ok_or_else(form)?;
    if !matches!(separator, 'T' | 't' | ' ') {
        return Err(form());
    }
    let rest = &rest[1..];
    let (clock, mut zone) = rest.split_at(8.min(rest.len()));
    let mut parts = date.split('-');
    let (year, month, day) = (
        parts.next().and_then(digits).ok_or_else(form)?,
        parts.next().and_then(digits).ok_or_else(form)?,
        parts.next().and_then(digits).ok_or_else(form)?,
    );
    let mut parts = clock.split(':');
    let (hour, minute, second) = (
        parts.next().and_then(digits).ok_or_else(form)?,
        parts.next().and_then(digits).ok_or_else(form)?,
        parts.next().and_then(digits).ok_or_else(form)?,
    );
    let mut fraction = String::new();
    if let Some(after) = zone.strip_prefix('.') {
        let end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        fraction = after[..end].to_string();
        if fraction.is_empty() || fraction.len() > 6 {
            return Err(form());
        }
        zone = &after[end..];
    }
    let offset_seconds = match zone {
        "Z" | "z" => 0,
        _ => {
            let sign = match zone.chars().next() {
                Some('+') => 1,
                Some('-') => -1,
                _ => return Err(form()),
            };
            let (h, m) = zone[1..].split_once(':').ok_or_else(form)?;
            if h.len() != 2 || m.len() != 2 {
                return Err(form());
            }
            let (h, m) = (digits(h).ok_or_else(form)?, digits(m).ok_or_else(form)?);
            if h > 23 || m > 59 {
                return Err(form());
            }
            sign * (h * 3600 + m * 60)
        }
    };
    let month =
        time::Month::try_from(u8::try_from(month).map_err(|_| form())?).map_err(|_| form())?;
    let date = time::Date::from_calendar_date(
        i32::try_from(year).map_err(|_| form())?,
        month,
        u8::try_from(day).map_err(|_| form())?,
    )
    .map_err(|_| form())?;
    let clock = time::Time::from_hms(
        u8::try_from(hour).map_err(|_| form())?,
        u8::try_from(minute).map_err(|_| form())?,
        u8::try_from(second).map_err(|_| form())?,
    )
    .map_err(|_| form())?;
    let local = time::PrimitiveDateTime::new(date, clock).assume_utc();
    let unix_seconds = local.unix_timestamp() - offset_seconds;
    let utc = time::OffsetDateTime::from_unix_timestamp(unix_seconds).map_err(|_| form())?;
    let fraction_said = if fraction.is_empty() {
        String::new()
    } else {
        format!(".{fraction}")
    };
    let day_and_time = format!(
        "{:04}-{:02}-{:02}",
        utc.year(),
        u8::from(utc.month()),
        utc.day()
    );
    let hms = format!(
        "{:02}:{:02}:{:02}{fraction_said}",
        utc.hour(),
        utc.minute(),
        utc.second()
    );
    Ok(Moment {
        unix_seconds,
        for_the_act: format!("{day_and_time} {hms}+00"),
        said: format!("{day_and_time}T{hms}Z"),
    })
}

/// A time as Postgres's log writes it (`2026-11-09 14:09:42.485861+00`), as
/// `--to-time` takes it, in UTC (`2026-11-09T14:09:42.485861Z`); as written
/// where it is in no form `moment` reads.
fn said_in_utc(postgres: &str) -> String {
    let mut given = postgres.trim().to_string();
    // Postgres writes a whole-hour offset as `+00`; RFC 3339 wants `+00:00`.
    let zone = given.rfind(['+', '-']).filter(|at| *at >= 19);
    if let Some(at) = zone {
        if given.len() - at == 3 {
            given.push_str(":00");
        }
    }
    moment(&given).map_or_else(|_| postgres.trim().to_string(), |at| at.said)
}

/// What would be done, once every check has passed.
#[derive(Debug, Clone)]
pub struct Plan {
    pub to: Moment,
    pub set: DatabaseSet,
    pub cronjob: serde_json::Value,
    /// The archiver's last segment, as the probe said it.
    pub last_archived: Option<String>,
    /// Stopped by an earlier act that did not finish, and what it stopped:
    /// run again, nothing is stopped twice and everything is started again
    /// at the counts it ran at, each CronJob as it was.
    pub resume: Option<Stopped>,
    /// Recorded by an earlier act that did not finish stopping, the
    /// database still running: its counts and CronJobs stand over what runs
    /// now, which it stopped or suspended itself.
    pub earlier: Option<Stopped>,
}

fn release_of(asked: &Asked) -> Release {
    Release {
        release: asked.release.clone(),
        namespace: asked.namespace.clone(),
    }
}

/// Every check, said as made; nothing changed. `now` is this machine's
/// clock, in seconds since the epoch.
pub async fn check(
    machine: &dyn Machine,
    asked: &Asked,
    to: &Moment,
    now: i64,
    watch: &mut dyn Watch,
) -> Result<Plan, ()> {
    watch.begin(Step::Checks);
    let checked = checks_made(machine, asked, to, now, watch).await;
    watch.end(Step::Checks, checked.is_ok());
    checked
}

async fn checks_made(
    machine: &dyn Machine,
    asked: &Asked,
    to: &Moment,
    now: i64,
    say: &mut dyn Watch,
) -> Result<Plan, ()> {
    let release = release_of(asked);
    let scope = crate::status::scope(&asked.release, &asked.namespace);
    let mut findings = Vec::new();
    let refuse = |findings: &[Finding], say: &mut dyn Watch| {
        for finding in findings {
            say.say(&format!("{finding}"));
        }
        say.say("\nNothing was changed. Each check that stopped it says what to do above.");
    };
    if to.unix_seconds > now {
        findings.push(Finding::Stops {
            what: format!("{} is after now", to.said),
            fix: "A restore goes back to a time the archive has passed; name one before now."
                .into(),
        });
        refuse(&findings, say);
        return Err(());
    }
    let installed = match crate::upgrade_deployment::run::release(machine, asked).await {
        Ok(installed) => installed,
        Err(failed) => {
            findings.push(Finding::Stops {
                what: format!("the release could not be read: {failed}"),
                fix: "Point KUBECONFIG at the cluster this deployment runs in; this drives your \
                      own helm."
                    .into(),
            });
            refuse(&findings, say);
            return Err(());
        }
    };
    findings.push(release_finding(asked, installed.as_ref()));
    if findings.iter().any(Finding::stops) {
        refuse(&findings, say);
        return Err(());
    }
    let mut resume = None;
    let mut earlier = None;
    let set = match database(machine, &release).await {
        Ok(Some(set)) if set.replicas > 0 => {
            earlier = super::recorded_stopped(machine, &release, &set.name).await;
            set
        }
        // Stopped by an act that did not finish, which recorded what it
        // stopped: this one takes it from there.
        Ok(Some(set)) => match super::recorded_stopped(machine, &release, &set.name).await {
            Some(stopped) => {
                findings.push(Finding::Fine(format!(
                    "{} was stopped by `{}`{}, which did not finish: this one takes it from \
                     there, and starts everything again at the counts it ran at",
                    asked.release,
                    stopped.command(),
                    stopped
                        .since
                        .as_deref()
                        .map(|at| format!(" at {at}"))
                        .unwrap_or_default()
                )));
                resume = Some(stopped);
                set
            }
            None => {
                findings.push(Finding::Stops {
                    what: format!(
                        "{}'s database, {}, is stopped, and no restore recorded stopping it",
                        asked.release, set.name
                    ),
                    fix: "A restore runs on a deployment that is running, or one a restore \
                          or --enable-checksums stopped. `meridian status` says what runs."
                        .into(),
                });
                refuse(&findings, say);
                return Err(());
            }
        },
        Ok(None) => {
            findings.push(Finding::Stops {
                what: format!(
                    "{} runs no database it brought, so it keeps no WAL archive to restore from",
                    asked.release
                ),
                fix: "A database somebody else runs is restored to a time by whoever runs it: \
                      a managed database's point-in-time recovery is its provider's."
                    .into(),
            });
            refuse(&findings, say);
            return Err(());
        }
        Err(failed) => {
            findings.push(Finding::Stops {
                what: format!("the release's workloads could not be read: {failed}"),
                fix: "This account needs to read the namespace's Deployments and StatefulSets."
                    .into(),
            });
            refuse(&findings, say);
            return Err(());
        }
    };
    if !set.archive {
        findings.push(Finding::Stops {
            what: format!(
                "{} keeps no WAL archive (backup.walArchive is off), so there is nothing to \
                 replay to a time",
                asked.release
            ),
            fix: format!(
                "Turn it on with `meridian upgrade-deployment {scope} --set \
                 backup.walArchive.enabled=true` (backups set up first: backup.recipient and \
                 backup.path or backup.existingClaim); from its first base backup on, any time \
                 since can be restored. A backup's own file is restored with `meridian restore \
                 <file>`."
            ),
        });
        refuse(&findings, say);
        return Err(());
    }
    findings.push(Finding::Fine(format!(
        "{} archives its WAL (backup.walArchive)",
        set.name
    )));
    if let Some(job) = act_running(machine, &release).await {
        findings.push(Finding::Stops {
            what: format!("job/{job}, an act on the stopped database's volume, is still running"),
            fix: format!(
                "Two acts never share the volume. `meridian logs database-offline {scope}` says \
                 where it is; once it has ended, run this again."
            ),
        });
        refuse(&findings, say);
        return Err(());
    }
    let cronjob = match super::cronjob(machine, &release, &set.name).await {
        Ok(cronjob) => cronjob,
        Err(_) => {
            findings.push(Finding::Stops {
                what: format!(
                    "{} has no {}-offline CronJob, which the restore runs from",
                    asked.release, set.name
                ),
                fix: format!(
                    "It is step 3b's chart's: `meridian upgrade-deployment {scope}` first."
                ),
            });
            refuse(&findings, say);
            return Err(());
        }
    };
    let probed = super::probed_database(machine, &release).await;
    let archive = probed.as_ref().map(|database| &database["archive"]);
    let last_archived = archive
        .and_then(|archive| archive["last_archived_at"].as_str())
        .filter(|at| !at.is_empty())
        .map(String::from);
    match archive {
        Some(archive) if archive["failing"] == true => findings.push(Finding::Worth {
            what: "the archiver's newest attempt failed".into(),
            why: "A time after its last segment cannot be reached; `meridian status` says when \
                  that was."
                .into(),
        }),
        Some(_) => {}
        None => findings.push(Finding::Unknown {
            what: "the archive's last segment".into(),
            why: "the conductor's probe did not say it".into(),
        }),
    }
    // Before now is not inside the archive: the latest time it restores to
    // lies before its last commit, which only the replay finds.
    findings.push(Finding::Fine(format!(
        "{} is before now; whether the archive reaches it, the restore itself says",
        to.said
    )));
    for finding in &findings {
        say.say(&format!("{finding}"));
    }
    Ok(Plan {
        to: to.clone(),
        set,
        cronjob,
        last_archived,
        resume,
        earlier,
    })
}

/// What it will do, said before it asks.
pub fn plan_text(asked: &Asked, plan: &Plan) -> String {
    let members = plan
        .resume
        .as_ref()
        .map_or(plan.set.replicas, |stopped| stopped.database);
    let standbys = members.saturating_sub(1);
    let patroni = plan.set.patroni;
    let mut out = format!(
        "Restoring {release}'s database, {set}, in {namespace}, to {to}:\n\
         \x20 from      its WAL archive: the newest base backup before {to}, the WAL replayed \
         to it, and the database promoted there on a timeline of its own. What came after \
         {to} is kept in the archive, on the old timeline, and is not in the database.\n",
        release = asked.release,
        set = plan.set.name,
        namespace = asked.namespace,
        to = plan.to.said,
    );
    if let Some(at) = &plan.last_archived {
        out.push_str(&format!(
            "\x20 archived  its last segment at {at}; the newest WAL is archived before it \
             stops.\n"
        ));
    }
    out.push_str(
        "\x20 downtime  proportional to the base backup's size and the WAL replayed; nothing \
         answers until it is done.\n\n",
    );
    out.push_str(if plan.resume.is_some() {
        "  1. Everything is stopped already, by the act before; the release's CronJobs stay \
         suspended.\n"
    } else {
        "  1. The release's CronJobs suspended (the replica comparison, the backup, the \
         scrub), every component stopped, the newest WAL archived, then the database.\n"
    });
    out.push_str(&format!(
        "  2. The restore on member 0's volume, in a Job from {}.\n",
        plan.cronjob["metadata"]["name"]
            .as_str()
            .unwrap_or_default()
    ));
    if patroni && standbys > 0 {
        out.push_str(&format!(
            "  3. The standbys' volumes deleted ({}), which Patroni makes again from member 0; \
             the database started, then every component, each to the count it ran at, and \
             each CronJob as it was.\n",
            (1..members)
                .map(|n| format!("data-{}-{n}", plan.set.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else {
        out.push_str(
            "  3. The database started, then every component, each to the count it ran at, and \
             each CronJob as it was.\n",
        );
    }
    out.push_str(&format!(
        "  If {} is outside the archive, the restore fails with its own words, the volume \
         stays restorable, and everything stays stopped until a time inside it is named.\n",
        plan.to.said
    ));
    out
}

/// The newest WAL archived, best effort, once nothing writes: a segment
/// switched and the archiver waited for, briefly, so the restore can reach
/// up to the stop.
async fn archive_now(machine: &dyn Machine, release: &Release, set: &str, pace: &Pace) {
    let Some(primary) = members_now(machine, release, set)
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|m| m.role.as_deref().is_none_or(|role| role == "primary"))
    else {
        return;
    };
    let psql = |query: &'static str| {
        let member = primary.name.clone();
        async move {
            super::kubectl(
                machine,
                &[
                    "exec",
                    "-n",
                    &release.namespace,
                    &member,
                    "-c",
                    "postgres",
                    "--",
                    "psql",
                    "-X",
                    "-At",
                    "-U",
                    "postgres",
                    "-d",
                    "postgres",
                    "-c",
                    query,
                ],
            )
            .await
            .map(|said| said.trim().to_string())
        }
    };
    let Ok(switched) = psql("select pg_walfile_name(pg_switch_wal())").await else {
        return;
    };
    let deadline =
        tokio::time::Instant::now() + pace.timeout.min(std::time::Duration::from_secs(120));
    while tokio::time::Instant::now() < deadline {
        if psql("select coalesce(last_archived_wal, '') from pg_stat_archiver")
            .await
            .is_ok_and(|last| last >= switched)
        {
            return;
        }
        tokio::time::sleep(pace.poll).await;
    }
}

/// Stop, restore, start. What was done, or at which step it stopped, what
/// state that left, and what to run.
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
    let act = Act::RestoreToTime(plan.to.for_the_act.clone());

    watch.begin(Step::Stop);
    let mut stopped = plan.resume.clone().unwrap_or(Stopped {
        database: plan.set.replicas,
        ..Stopped::default()
    });
    let stopping = async {
        if plan.resume.is_some() {
            watch.say("  stopped already, by the act before");
            // The record names this act from now on: if it does not finish
            // either, the volume holds this one's, and this is what resumes.
            if stopped.act.as_deref() != Some(act.name()) {
                stopped.act = Some(act.name().to_string());
                super::record_stopped(machine, &release, &plan.set.name, &stopped).await?;
            }
            // Suspended then; again, in case anything gave one back since.
            return suspend_cronjobs(machine, &release, &stopped, watch).await;
        }
        // Recorded before anything stops, so an act that does not finish
        // leaves what to start again, and each CronJob's schedule, where
        // the next one reads it.
        stopped = record_stop(machine, &release, &plan.set, &act, plan.earlier.as_ref()).await?;
        suspend_cronjobs(machine, &release, &stopped, watch).await?;
        stop_these(machine, &release, &stopped.deployments, pace, watch).await?;
        archive_now(machine, &release, &plan.set.name, pace).await;
        // No switch to member 0 first, as checksums need: the restore makes
        // member 0's volume the archive's, whichever member was primary.
        stop_database(machine, &release, &plan.set, pace, watch).await
    }
    .await;
    watch.end(Step::Stop, stopping.is_ok());
    if let Err(failed) = stopping {
        let back = start(machine, &release, &plan.set, &stopped, pace, watch).await;
        if back.is_ok() {
            clear_stopped(machine, &release, &plan.set.name).await;
        }
        return Err(format!(
            "stopping everything did not finish: {failed}\nThe database was not restored. {}",
            match back {
                Ok(()) => "Everything was started again.".to_string(),
                Err(why) => format!("Starting it again: {why}"),
            }
        ));
    }
    watch.begin(Step::Act);
    let restored = run_act(
        machine,
        &release,
        &plan.cronjob,
        &act,
        unix_seconds,
        pace,
        watch,
    )
    .await;
    watch.end(Step::Act, restored.is_ok());
    let log = restored.map_err(|failed| {
        let again = if failed.past_the_archive() {
            // The archive replayed to its end: the time named is at or after
            // its last commit, which a restore to a time never reaches (it
            // stops before the first commit after the time named), so the
            // moment the deployment stopped is past it too.
            format!(
                "{to} is past the archive's end: the latest time that can be restored lies \
                 before the archive's last commit, which Postgres's lines above give as \
                 \"last completed transaction\" where they include it{commit}. To bring {release} \
                 back, name a time inside the archive, before that commit: `meridian restore \
                 --to-time <time> {scope}`.",
                to = plan.to.said,
                commit = failed
                    .last_completed_transaction()
                    .map(|at| format!(": {}", said_in_utc(at)))
                    .unwrap_or_default(),
                release = asked.release,
            )
        } else if failed.outside_the_archive() {
            format!(
                "Name a time inside the archive, after its oldest base backup ended and before \
                 its last commit: `meridian restore --to-time <time> {scope}`."
            )
        } else if !failed.ended {
            format!(
                "Once the Job has ended, `meridian restore --to-time <time> {scope}` takes it \
                 from there; it is refused while the Job runs."
            )
        } else {
            format!(
                "Once the cause above is dealt with, `meridian restore --to-time <time> {scope}` \
                 takes it from there."
            )
        };
        format!(
            "{failed}\nEverything stays stopped, and the release's CronJobs suspended: the \
             database's volume holds a restore that did not finish, and stays restorable. \
             {again}"
        )
    })?;

    watch.begin(Step::Start);
    let starting = async {
        if plan.set.patroni {
            standbys_to_remake(machine, &release, &plan.set, stopped.database, &act, watch).await?;
        }
        start(machine, &release, &plan.set, &stopped, pace, watch).await
    }
    .await;
    watch.end(Step::Start, starting.is_ok());
    clear_stopped(machine, &release, &plan.set.name).await;
    if let Err(failed) = starting {
        let _ = started_components(machine, &release, &stopped, watch).await;
        return Err(format!(
            "the database was restored to {}, and starting everything again did not finish: \
             {failed}\n`meridian status {scope}` says where each part is.",
            plan.to.said
        ));
    }
    let mut out = format!(
        "\nRestored {}'s database to {}, and started everything again.\n",
        asked.release, plan.to.said
    );
    for line in log
        .lines()
        .filter(|line| line.starts_with("database-offline: "))
    {
        out.push_str(&format!("  {line}\n"));
    }
    if plan.set.patroni && stopped.database > 1 {
        out.push_str(&format!(
            "  Its {} standbys were made again from member 0.\n",
            stopped.database - 1
        ));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "to_time_tests.rs"]
mod tests;

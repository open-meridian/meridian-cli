//! Upgrading: the checks, Helm, the wait, the cleanup.
//!
//! Everything here touches the world, through a `Machine` a test can be.
//! What can be decided without touching it is in the parent module.

use std::collections::BTreeSet;
use std::time::Duration;

use tokio::time::Instant;

use super::watch::{Step, Watch};
use super::{
    chart_of, direction, image_in, installed, jobs, left_by_a_restart, leftovers, migration,
    plugin_floors, pods, progress, release_finding, restarted, restarts, running_images,
    skip_policy, target_image, to_relaunch, workloads, Asked, Chart, Direction, Installed,
    JobState, Plan, Report, HELM_MINIMUM,
};
use crate::doctor::{checks, Failure, Finding, Machine};

/// What the checks found.
pub enum Checked {
    /// Every check passed: this is what would be done.
    Upgrade(Box<Plan>),
    /// Already at the target version: nothing to do, which is success.
    Current(String),
    /// A check stopped it, and nothing was changed.
    Refused,
}

async fn helm(machine: &dyn Machine) -> Finding {
    let said = match machine.run("helm", &["version", "--short"]).await {
        Ok(said) => said,
        Err(failed) => {
            return Finding::Stops {
                what: match &failed {
                    Failure::Missing(_) => "helm is not installed here".to_string(),
                    Failure::Said(said) => format!("helm does not run here: {said}"),
                },
                fix: "Install Helm 3.14 or newer. This drives your own helm.".into(),
            }
        }
    };
    match checks::version(&said) {
        Some(found) if found >= HELM_MINIMUM => Finding::Fine(format!(
            "helm {}.{} upgrades with --reset-then-reuse-values",
            found.0, found.1
        )),
        Some((major, minor)) => Finding::Stops {
            what: format!("helm is {major}.{minor}"),
            fix: format!(
                "Install {}.{} or newer: an upgrade needs --reset-then-reuse-values, which \
                 applies the new chart's defaults under the deployment's own values. \
                 --reuse-values would keep the old chart's image.",
                HELM_MINIMUM.0, HELM_MINIMUM.1
            ),
        },
        None => Finding::Unknown {
            what: "helm's version could not be read".into(),
            why: format!("`helm version --short` said {said:?}"),
        },
    }
}

async fn cluster(machine: &dyn Machine, namespace: &str) -> Vec<Finding> {
    match machine.run("kubectl", &["cluster-info"]).await {
        Ok(_) => {}
        Err(Failure::Missing(program)) => {
            return vec![Finding::Stops {
                what: format!("{program} is not installed here"),
                fix: "Install kubectl. This drives your own, so what it is pointed at is what \
                      gets upgraded."
                    .into(),
            }]
        }
        Err(Failure::Said(failed)) => {
            return vec![Finding::Stops {
                what: format!("no cluster is reachable: {failed}"),
                fix: "Point KUBECONFIG at the cluster this deployment runs in.".into(),
            }]
        }
    }
    let mut findings = vec![Finding::Fine("the cluster is reachable".into())];
    // Asked of the cluster, before anything is changed: an upgrade that can
    // replace the Deployments and then not run the migration is half done.
    for (verb, resource) in [
        ("patch", "deployments"),
        ("create", "jobs"),
        ("delete", "jobs"),
    ] {
        let allowed = machine
            .run(
                "kubectl",
                &["auth", "can-i", verb, resource, "-n", namespace],
            )
            .await;
        findings.push(match allowed {
            Ok(said) if said.trim() == "yes" => {
                Finding::Fine(format!("this account may {verb} {resource} in {namespace}"))
            }
            // kubectl says "no" and exits 1, so a refusal arrives as either.
            Ok(said) | Err(Failure::Said(said)) if said.trim().starts_with("no") => {
                Finding::Stops {
                    what: format!("this account may not {verb} {resource} in {namespace}"),
                    fix: format!(
                        "An upgrade is made with your own cluster rights. Ask for the right to \
                         {verb} {resource} in {namespace}."
                    ),
                }
            }
            other => Finding::Unknown {
                what: format!("whether this account may {verb} {resource} is unknown"),
                why: match other {
                    Ok(said) => said,
                    Err(failed) => failed.to_string(),
                },
            },
        });
    }
    findings
}

/// The nodes' disk, as `doctor` reads it, with what an upgrade adds to it:
/// every new image is pulled onto a node, so a node already short of disk is
/// made shorter, and the new pods are scheduled nowhere.
async fn disk(machine: &dyn Machine) -> Vec<Finding> {
    const PULLS: &str = "An upgrade pulls the new version's images onto the node, which takes \
                         more of the disk it is short of.";
    checks::disk(machine)
        .await
        .into_iter()
        .map(|finding| match finding {
            Finding::Stops { what, fix } => Finding::Stops {
                what,
                fix: format!("{PULLS} {fix}"),
            },
            Finding::Worth { what, why } => Finding::Worth {
                what,
                why: format!("{PULLS} {why}"),
            },
            other => other,
        })
        .collect()
}

async fn release(machine: &dyn Machine, asked: &Asked) -> Result<Option<Installed>, String> {
    let filter = format!("^{}$", asked.release);
    // Every status an upgrade must refuse as well as the one it proceeds
    // from. Named, because Helm 3 lists only deployed and failed unless told
    // and Helm 4 has no --all.
    let listed = machine
        .run(
            "helm",
            &[
                "list",
                "--namespace",
                &asked.namespace,
                "--filter",
                &filter,
                "--deployed",
                "--failed",
                "--pending",
                "--uninstalling",
                "-o",
                "json",
            ],
        )
        .await
        .map_err(|failed| failed.to_string())?;
    installed(&listed, &asked.release)
}

async fn target(machine: &dyn Machine, asked: &Asked) -> Result<Chart, Finding> {
    let mut arguments = vec!["show", "chart", asked.chart.as_str()];
    if let Some(version) = &asked.chart_version {
        arguments.extend(["--version", version.as_str()]);
    }
    let fix = "`helm show chart <chart> --version <v>` says whether a version exists; \
               --chart-version names one, and without it the latest published is used.";
    let shown = machine
        .run("helm", &arguments)
        .await
        .map_err(|failed| Finding::Stops {
            what: match &asked.chart_version {
                Some(version) => format!("{} {version} could not be found: {failed}", asked.chart),
                None => format!("{} could not be read: {failed}", asked.chart),
            },
            fix: fix.into(),
        })?;
    chart_of(&shown).ok_or_else(|| Finding::Stops {
        what: format!("{} states no name and version", asked.chart),
        fix: fix.into(),
    })
}

/// Every check, said in the order made, and what would be done if they all
/// pass. Nothing here changes anything.
pub async fn check(machine: &dyn Machine, asked: &Asked, watch: &mut dyn Watch) -> Checked {
    watch.begin(Step::Checks);
    let checked = checks_made(machine, asked, watch).await;
    watch.end(Step::Checks, !matches!(checked, Checked::Refused));
    checked
}

async fn checks_made(machine: &dyn Machine, asked: &Asked, say: &mut dyn Watch) -> Checked {
    let mut findings = vec![helm(machine).await];
    findings.extend(cluster(machine, &asked.namespace).await);
    let stopped = |findings: &[Finding]| findings.iter().any(Finding::stops);
    let refuse = |findings: &[Finding], say: &mut dyn Watch| {
        for finding in findings {
            say.say(&format!("{finding}"));
        }
        say.say("\nNothing was changed. Each check that stopped it says what to do above.");
        Checked::Refused
    };
    if stopped(&findings) {
        return refuse(&findings, say);
    }
    findings.extend(disk(machine).await);
    if stopped(&findings) {
        return refuse(&findings, say);
    }

    let from = match release(machine, asked).await {
        Ok(from) => from,
        Err(failed) => {
            findings.push(Finding::Stops {
                what: format!("the release could not be read: {failed}"),
                fix: "`helm list -n <namespace>` should list it.".into(),
            });
            return refuse(&findings, say);
        }
    };
    findings.push(release_finding(asked, from.as_ref()));
    let Some(from) = from.filter(|_| !stopped(&findings)) else {
        return refuse(&findings, say);
    };

    let to = match target(machine, asked).await {
        Ok(to) => to,
        Err(stops) => {
            findings.push(stops);
            return refuse(&findings, say);
        }
    };
    if to.name != from.chart {
        findings.push(Finding::Stops {
            what: format!(
                "{} runs the chart {}, and {} is {}",
                asked.release, from.chart, asked.chart, to.name
            ),
            fix: "An upgrade moves a release to a newer version of its own chart. --chart names \
                  the one it was installed from."
                .into(),
        });
        return refuse(&findings, say);
    }
    findings.push(Finding::Fine(format!(
        "{} {} is published ({})",
        to.name, to.version, to.app_version
    )));
    match direction(&from, &to) {
        Direction::Down(stops) => {
            findings.push(stops);
            return refuse(&findings, say);
        }
        Direction::Current => {
            for finding in &findings {
                say.say(&format!("{finding}"));
            }
            return Checked::Current(format!(
                "{} is at {} {} already; nothing to do.",
                asked.release, to.name, to.version
            ));
        }
        Direction::Up => findings.push(Finding::Fine(format!(
            "{} is newer than the installed {}",
            to.version, from.version
        ))),
    }
    findings.push(skip_policy(&from, &to));
    findings.push(plugin_floors(&to));

    // The new chart's image, and the deployment's own where it names one.
    // Only `image` is read out of the deployment's values; the rest of them,
    // its enrolment code among them, is never kept or printed.
    let mut charts = machine
        .run(
            "helm",
            &["show", "values", &asked.chart, "--version", &to.version],
        )
        .await
        .map(|values| image_in(&values))
        .unwrap_or_default();
    // A published chart's tag is its appVersion: the commit both were built at.
    charts.tag = charts.tag.or_else(|| Some(to.app_version.clone()));
    let own = machine
        .run(
            "helm",
            &[
                "get",
                "values",
                &asked.release,
                "--namespace",
                &asked.namespace,
                "-o",
                "json",
            ],
        )
        .await
        .map(|values| image_in(&values))
        .unwrap_or_default();
    let (to_image, pinned) = target_image(&charts, &own);
    findings.extend(pinned);

    let running = kubectl_json(machine, asked, WORKLOADS)
        .await
        .map(|listed| workloads(&listed))
        .unwrap_or_default();
    let held_pods = kubectl_json(machine, asked, "pods")
        .await
        .map(|listed| pods(&listed))
        .unwrap_or_default();
    for finding in &findings {
        say.say(&format!("{finding}"));
    }
    Checked::Upgrade(Box::new(Plan {
        from_images: running_images(&running, &to_image),
        left_over: left_by_a_restart(&running, &held_pods),
        from,
        to,
        to_image,
    }))
}

/// The release's workloads, and the ReplicaSets that say which of a
/// Deployment's pods are of its current template.
const WORKLOADS: &str = "deployments,statefulsets,replicasets";

async fn kubectl_json(machine: &dyn Machine, asked: &Asked, what: &str) -> Result<String, String> {
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
        .map_err(|failed| failed.to_string())
}

/// How often the cluster is asked while waiting.
pub struct Pace {
    pub poll: Duration,
    pub timeout: Duration,
}

/// Apply the plan, wait for it, clean up. What was done, or at which step it
/// stopped and what to do.
pub async fn apply(
    machine: &dyn Machine,
    asked: &Asked,
    plan: &Plan,
    pace: &Pace,
    watch: &mut dyn Watch,
) -> Result<Report, String> {
    let (release, namespace) = (asked.release.as_str(), asked.namespace.as_str());
    // Before, so a restart is counted from here and not from a pod's birth,
    // and a plugin pod made from here was made during the upgrade.
    let before_pods = pods(
        &kubectl_json(machine, asked, "pods")
            .await
            .unwrap_or_default(),
    );
    let before = restarts(&before_pods);
    let before_names: BTreeSet<String> = before_pods.into_iter().map(|pod| pod.name).collect();

    let arguments = super::helm_arguments(asked, plan);
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    watch.begin(Step::Apply);
    let applied = machine.run("helm", &arguments).await;
    watch.end(Step::Apply, applied.is_ok());
    applied.map_err(|failed| {
        format!(
            "helm refused the upgrade: {failed}\n`helm history {release} -n {namespace}` shows \
             the revision it left; `helm rollback {release} -n {namespace}` returns it to \
             revision {}.",
            plan.from.revision
        )
    })?;
    let revision = self::release(machine, asked)
        .await
        .ok()
        .flatten()
        .map(|now| now.revision)
        .unwrap_or(plan.from.revision + 1);
    watch.say(&format!(
        "Applied as revision {revision}. Waiting for the migration, then every component on its \
         new template (up to {}).",
        asked.timeout
    ));

    watch.begin(Step::Migration);
    watch.begin(Step::Components);
    let deadline = Instant::now() + pace.timeout;
    let mut said_waiting = std::collections::BTreeSet::new();
    let (held_jobs, held_workloads, held_pods) = loop {
        let held_jobs = jobs(
            &kubectl_json(machine, asked, "jobs")
                .await
                .unwrap_or_default(),
        );
        let held_workloads = workloads(
            &kubectl_json(machine, asked, WORKLOADS)
                .await
                .unwrap_or_default(),
        );
        let held_pods = pods(
            &kubectl_json(machine, asked, "pods")
                .await
                .unwrap_or_default(),
        );
        let now = progress(revision, &held_jobs, &held_workloads, &held_pods);
        watch.waiting(&now);
        // Said even when nothing else is waited for, so none goes unexplained.
        for pod in &now.left_over {
            let each = format!("pod/{pod}");
            if !said_waiting.contains(&each) {
                watch.say(&format!(
                    "  not waited for: {each}, left over from a restart; removed at the end"
                ));
                said_waiting.insert(each);
            }
        }
        if !now.migrating {
            watch.end(Step::Migration, now.failed.is_none());
        }

        if let Some(job) = now.failed {
            return Err(format!(
                "the migration failed, and the components wait for it.\n\
                 Its log: `kubectl logs -n {namespace} job/{job} --all-containers`.\n\
                 Revision {revision} stays applied. `helm history {release} -n {namespace}` \
                 lists the revisions; read the log before rolling back, since a migration that \
                 got part of the way may have changed what the older runtime would read."
            ));
        }
        if now.waiting.is_empty() {
            watch.end(Step::Components, true);
            break (held_jobs, held_workloads, held_pods);
        }
        if Instant::now() >= deadline {
            watch.end(Step::Migration, false);
            watch.end(Step::Components, false);
            return Err(format!(
                "after {}, still waiting for:\n  - {}\nRevision {revision} is applied and \
                 nothing was rolled back. `kubectl get pods -n {namespace} -l {}` shows where \
                 it is; run this again with a longer --timeout to keep waiting, since it is at \
                 {} already.",
                asked.timeout,
                now.waiting.join("\n  - "),
                asked.selector(),
                plan.to.version
            ));
        }
        // Each said once, when it is first seen, not every poll.
        for each in now.waiting {
            if !said_waiting.contains(&each) {
                watch.say(&format!("  waiting: {each}"));
                said_waiting.insert(each);
            }
        }
        tokio::time::sleep(pace.poll).await;
    };

    let mut report = Report {
        from: format!("{} {}", plan.to.name, plan.from.version),
        to: format!("{} {}", plan.to.name, plan.to.version),
        revision,
        migrated: migration(&held_jobs, revision)
            .filter(|job| job.state == JobState::Complete)
            .map(|job| job.name.clone()),
        components: held_workloads
            .iter()
            .map(|workload| {
                (
                    workload.component.clone(),
                    workload
                        .images
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                    workload.ready,
                    workload.wanted,
                )
            })
            .collect(),
        restarted: restarted(&before, &held_pods, namespace),
        // Said, and never done: relaunching a plugin is the person's call.
        relaunch: to_relaunch(&held_pods, Some(&before_names)),
        ..Report::default()
    };

    // Only finished Jobs of this release's earlier revisions, and only pods
    // left over from a restart, each found by its label. ReplicaSets are the
    // chart's revisionHistoryLimit's.
    watch.begin(Step::Cleanup);
    let (finished, running) = leftovers(&held_jobs, revision);
    report.left = running;
    let dead = left_by_a_restart(&held_workloads, &held_pods);
    for (kind, names, done) in [
        ("job", finished, &mut report.cleaned),
        ("pod", dead, &mut report.removed),
    ] {
        if names.is_empty() {
            continue;
        }
        let mut arguments = vec!["delete", kind, "-n", namespace, "--ignore-not-found"];
        arguments.extend(names.iter().map(String::as_str));
        match machine.run("kubectl", &arguments).await {
            Ok(_) => *done = names,
            Err(failed) => report.not_cleaned.push(format!(
                "{failed}. The upgrade itself is done; `kubectl delete {kind} -n {namespace} {}` \
                 removes them.",
                names.join(" ")
            )),
        }
    }
    watch.end(Step::Cleanup, report.not_cleaned.is_empty());
    Ok(report)
}

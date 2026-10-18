//! `meridian plugin gc` (ruling 166; named by plan question 237): the
//! deployment's own plugin registry's offline garbage collection, after
//! retired versions released their images.
//!
//! A retired version's manifest is deleted by digest (W8.15), and its layers
//! stay on the registry's volume until a garbage collection. CNCF
//! Distribution's `registry garbage-collect` must not run while anything can
//! push, so this sets the in-cluster registry read-only, runs the collection
//! in its pod, and sets it back: pulls go on throughout, so running plugins
//! and launches are untouched; only uploads wait, for the few minutes it
//! takes. It never deletes a manifest itself, and never an untagged one: only
//! the layers no manifest names.
//!
//! It checks first and changes nothing if a check fails, shows what it will
//! do, and asks. A run stopped half-way leaves the registry read-only, said,
//! and the next run finds it so and finishes.

use crate::doctor::{Failure, Machine};

/// The variable that sets Distribution read-only (its
/// `storage.maintenance.readonly`), as the chart's own registry reads its
/// configuration from the environment.
pub const READ_ONLY: &str = "REGISTRY_STORAGE_MAINTENANCE_READONLY";
const READ_ONLY_ON: &str = r#"{"enabled":true}"#;
/// Where registry:2 keeps its configuration in its image.
const CONFIG: &str = "/etc/docker/registry/config.yml";

pub struct Asked {
    pub release: String,
    pub namespace: String,
    /// As kubectl's rollout takes it: 5m.
    pub timeout: String,
}

/// The registry found: its StatefulSet, its pod and container, and whether a
/// run stopped half-way left it read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub statefulset: String,
    pub pod: String,
    pub container: String,
    pub read_only_already: bool,
}

fn said(failed: Failure) -> String {
    match failed {
        Failure::Missing(program) => format!(
            "{program} is not installed here; `meridian doctor` says what a deployment's machine \
             needs"
        ),
        Failure::Said(said) => said,
    }
}

/// The checks, and what would be done. Nothing here changes anything.
pub async fn check(machine: &dyn Machine, asked: &Asked) -> Result<Plan, String> {
    let ns = asked.namespace.as_str();
    let listed = machine
        .run(
            "kubectl",
            &[
                "get",
                "statefulsets",
                "-n",
                ns,
                "-l",
                &format!("app.kubernetes.io/instance={}", asked.release),
                "-o",
                "json",
            ],
        )
        .await
        .map_err(|failed| format!("the deployment could not be read: {}", said(failed)))?;
    let listed: serde_json::Value = serde_json::from_str(&listed)
        .map_err(|failed| format!("kubectl's answer could not be read: {failed}"))?;
    // The chart labels the registry's pods, and its StatefulSet only by the
    // selector that picks them.
    let Some(registry) = listed["items"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|item| crate::status::component_of(item) == Some("registry"))
    else {
        return Err(format!(
            "{} in {ns} brings no plugin registry of its own (registry.enabled is false), or is \
             not there: `meridian status {}` says what is",
            asked.release,
            crate::status::scope(&asked.release, ns)
        ));
    };
    let statefulset = registry["metadata"]["name"]
        .as_str()
        .ok_or("the registry's StatefulSet has no name")?
        .to_string();
    let containers = registry["spec"]["template"]["spec"]["containers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let container = containers
        .iter()
        .find(|c| c["name"] == "registry")
        .or_else(|| containers.first())
        .ok_or("the registry's StatefulSet runs no container")?;
    let read_only_already = container["env"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|env| env["name"] == READ_ONLY);
    for (verb, resource) in [("patch", "statefulsets"), ("create", "pods/exec")] {
        let allowed = machine
            .run("kubectl", &["auth", "can-i", verb, resource, "-n", ns])
            .await;
        match allowed {
            Ok(said) if said.trim() == "yes" => {}
            _ => {
                return Err(format!(
                    "a garbage collection needs the right to {verb} {resource} in {ns}, which \
                     this account does not hold"
                ))
            }
        }
    }
    Ok(Plan {
        pod: format!("{statefulset}-0"),
        statefulset,
        container: container["name"].as_str().unwrap_or("registry").to_string(),
        read_only_already,
    })
}

/// What it will do, said before it asks.
pub fn plan_text(asked: &Asked, plan: &Plan) -> String {
    let mut out = format!(
        "This collects the garbage of {}'s plugin registry in {}, {}:\n\
         \x20 1. sets it read-only and restarts it: uploads wait, pulls go on, so running \
         plugins and launches are untouched\n\
         \x20 2. runs `registry garbage-collect` in its pod: the layers no version's manifest \
         names, which retired versions left, are deleted; no manifest is\n\
         \x20 3. sets it back and restarts it\n",
        asked.release, asked.namespace, plan.statefulset
    );
    if plan.read_only_already {
        out.push_str(
            "It is read-only already: a collection stopped half-way left it so, and this one \
             finishes it.\n",
        );
    }
    out
}

async fn rolled_out(machine: &dyn Machine, asked: &Asked, plan: &Plan) -> Result<(), String> {
    machine
        .run(
            "kubectl",
            &[
                "rollout",
                "status",
                &format!("statefulset/{}", plan.statefulset),
                "-n",
                &asked.namespace,
                &format!("--timeout={}", asked.timeout),
            ],
        )
        .await
        .map(|_| ())
        .map_err(said)
}

/// What a collection said, in a line: what it found eligible, or that there
/// was nothing yet to collect.
pub fn collected(output: &str) -> String {
    if output.contains("Path not found") {
        return "the registry holds no image yet: nothing to collect".into();
    }
    // Distribution's summary, after the marking and before each blob swept:
    // "N blobs marked, M blobs and K manifests eligible for deletion".
    output
        .lines()
        .find(|line| line.contains("blobs marked"))
        .map(|line| line.trim().to_string())
        .unwrap_or_else(|| "it found nothing to delete".into())
}

/// Read-only, the collection, and back. What it came to, or where it
/// stopped and how the registry was left.
pub async fn apply(
    machine: &dyn Machine,
    asked: &Asked,
    plan: &Plan,
    say: &mut (dyn FnMut(&str) + Send),
) -> Result<String, String> {
    let ns = asked.namespace.as_str();
    let target = format!("statefulset/{}", plan.statefulset);
    let set = |value: String| {
        let target = target.clone();
        let container = plan.container.clone();
        async move {
            machine
                .run(
                    "kubectl",
                    &["set", "env", &target, "-n", ns, "-c", &container, &value],
                )
                .await
                .map(|_| ())
                .map_err(said)
        }
    };
    if !plan.read_only_already {
        set(format!("{READ_ONLY}={READ_ONLY_ON}"))
            .await
            .map_err(|failed| {
                format!("the registry could not be set read-only: {failed}. Nothing was changed")
            })?;
    }
    let back = format!("{READ_ONLY}-");
    let stuck = |why: String| {
        format!(
            "{why}\nThe registry is left read-only: uploads are refused, pulls go on. `meridian \
             plugin gc {}` again finishes the collection and sets it back.",
            crate::status::scope(&asked.release, ns)
        )
    };
    rolled_out(machine, asked, plan)
        .await
        .map_err(|failed| stuck(format!("the registry did not restart read-only: {failed}")))?;
    say("The registry is read-only; collecting.");
    let ran = machine
        .run(
            "kubectl",
            &[
                "exec",
                "-n",
                ns,
                &plan.pod,
                "-c",
                &plan.container,
                "--",
                "registry",
                "garbage-collect",
                CONFIG,
            ],
        )
        .await;
    let summary = match ran {
        Ok(output) => collected(&output),
        Err(Failure::Said(output)) if output.contains("Path not found") => collected(&output),
        Err(failed) => {
            // Back first: a collection that failed has deleted nothing it
            // should not, and uploads should not wait on it.
            let restored = set(back.clone()).await;
            return Err(match restored {
                Ok(()) => format!(
                    "the collection failed: {}. The registry is set back; nothing was deleted \
                     that a manifest names",
                    said(failed)
                ),
                Err(again) => stuck(format!(
                    "the collection failed ({}), and the registry could not be set back: {again}",
                    said(failed)
                )),
            });
        }
    };
    say(&format!("Collected: {summary}."));
    set(back)
        .await
        .map_err(|failed| stuck(format!("the registry could not be set back: {failed}")))?;
    rolled_out(machine, asked, plan)
        .await
        .map_err(|failed| format!("the registry was set back and has not restarted yet: {failed}. `meridian status` says when it is ready"))?;
    Ok(format!(
        "Collected {}'s plugin registry: {summary}. It takes uploads again.\n",
        asked.release
    ))
}

#[cfg(test)]
mod tests;

//! The checks themselves. Each answers one question and names one fix.

use meridian_conductor::assertions::MAX_LIFETIME_SECONDS;

use super::{Answered, Failure, Finding, Machine};

/// What the registry may answer a manifest request with.
const MANIFEST_TYPES: &str = "application/vnd.oci.image.index.v1+json,\
                              application/vnd.oci.image.manifest.v1+json,\
                              application/vnd.docker.distribution.manifest.list.v2+json,\
                              application/vnd.docker.distribution.manifest.v2+json";

/// The oldest Helm whose behaviour this chart was built against.
const HELM_MINIMUM: (u32, u32) = (3, 12);

pub async fn helm(machine: &dyn Machine) -> Finding {
    let said = match machine.run("helm", &["version", "--short"]).await {
        Ok(said) => said,
        Err(failed) => {
            return Finding::Stops {
                what: match &failed {
                    Failure::Missing(_) => "helm is not installed here".to_string(),
                    Failure::Said(said) => format!("helm does not run here: {said}"),
                },
                fix: "Install Helm 3. This drives your own helm rather than embedding one, \
                      so the chart stays the only thing that says what runs."
                    .into(),
            }
        }
    };

    match version(&said) {
        Some((major, minor)) if (major, minor) >= HELM_MINIMUM => {
            Finding::Fine(format!("helm {major}.{minor} is recent enough"))
        }
        Some((major, minor)) => Finding::Stops {
            what: format!("helm is {major}.{minor}"),
            fix: format!(
                "Install {}.{} or newer: the chart renders Secrets from what they already hold, \
                 which older Helm does not keep.",
                HELM_MINIMUM.0, HELM_MINIMUM.1
            ),
        },
        None => Finding::Unknown {
            what: "helm's version could not be read".into(),
            why: format!("`helm version --short` said {said:?}"),
        },
    }
}

/// `v3.16.2+g1234` and the other shapes Helm prints.
pub fn version(said: &str) -> Option<(u32, u32)> {
    let digits: String = said
        .trim()
        .trim_start_matches('v')
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = digits.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

pub async fn cluster(machine: &dyn Machine, namespace: &str) -> Vec<Finding> {
    let reachable = match machine.run("kubectl", &["cluster-info"]).await {
        Ok(_) => Finding::Fine("a cluster is reachable".into()),
        // Two failures with two fixes. Telling somebody to check KUBECONFIG on
        // a machine that has no kubectl to read it with is the kind of advice
        // that sends an afternoon in the wrong direction.
        Err(Failure::Missing(program)) => {
            return vec![Finding::Stops {
                what: format!("{program} is not installed here"),
                fix: "Install kubectl. This drives your own, so what it is pointed at is what \
                      gets the deployment."
                    .into(),
            }]
        }
        Err(Failure::Said(failed)) => {
            return vec![Finding::Stops {
                what: format!("no cluster is reachable: {failed}"),
                fix: "Point KUBECONFIG at the cluster this deployment is for, or start one.".into(),
            }]
        }
    };

    // Asked of the cluster rather than assumed from a role name: what an
    // install needs is the right to create what the chart contains, and the
    // cluster is the only thing that knows whether this person has it.
    let mut findings = vec![reachable];
    for (resource, what) in [
        ("deployments", "run its components"),
        ("secrets", "hold what the wizard writes"),
        ("jobs", "migrate and set itself up"),
    ] {
        let allowed = machine
            .run(
                "kubectl",
                &["auth", "can-i", "create", resource, "-n", namespace],
            )
            .await;
        findings.push(match allowed {
            Ok(said) if said.trim() == "yes" => {
                Finding::Fine(format!("this account may create {resource} in {namespace}"))
            }
            // kubectl says "no" and exits 1, so a refusal arrives as either.
            Ok(said) | Err(Failure::Said(said)) if said.trim().starts_with("no") => {
                Finding::Stops {
                    what: format!("this account may not create {resource} in {namespace}"),
                    fix: format!(
                        "Ask for the right to create {resource} in {namespace}, to {what}, or \
                         install into a namespace you administer."
                    ),
                }
            }
            other => Finding::Unknown {
                what: format!("whether this account may create {resource} is unknown"),
                why: match other {
                    Ok(said) => said,
                    Err(failed) => failed.to_string(),
                },
            },
        });
    }
    findings
}

pub async fn storage_class(machine: &dyn Machine) -> Finding {
    match machine
        .run("kubectl", &["get", "storageclass", "-o", "name"])
        .await
    {
        Ok(said) if said.trim().is_empty() => Finding::Stops {
            what: "this cluster has no storage class".into(),
            fix: "The deployment's key lives in a volume only its conductor mounts, and a \
                  cluster with no storage class cannot make one. Install a provisioner, or \
                  name a class the chart should use."
                .into(),
        },
        Ok(said) => Finding::Fine(format!(
            "{} storage class(es) for the key's volume",
            said.lines().filter(|line| !line.trim().is_empty()).count()
        )),
        Err(failed) => Finding::Unknown {
            what: "the cluster's storage classes could not be read".into(),
            why: failed.to_string(),
        },
    }
}

/// Whether the runtime image can be pulled from here.
///
/// Asked of the registry directly, because what a cluster can pull is not
/// what this machine can pull, and the one this can answer for is its own
/// network. A deployment whose image cannot be pulled fails as pods that
/// never start, minutes after the install said it had worked.
pub async fn image(machine: &dyn Machine, image: &str) -> Finding {
    let Some((repository, reference)) = image.rsplit_once(':') else {
        return Finding::Stops {
            what: format!("{image} names no tag"),
            fix: "Name the tag the deployment should run, so an upgrade is a deliberate act."
                .into(),
        };
    };
    let Some(path) = repository.strip_prefix("ghcr.io/") else {
        return Finding::Unknown {
            what: format!("{repository} is not a registry this can ask"),
            why: "Only ghcr.io is asked directly; anything else is left to the cluster.".into(),
        };
    };

    let token = machine
        .fetch(
            &format!("https://ghcr.io/token?scope=repository:{path}:pull&service=ghcr.io"),
            &[],
        )
        .await;
    let token = match token.as_ref().map(|answered| read_token(&answered.body)) {
        Ok(Some(token)) => token,
        _ => {
            return Finding::Unknown {
                what: format!("{image} could not be asked about"),
                why: "the registry did not issue a token for an anonymous pull".into(),
            }
        }
    };

    // The token goes in the header the registry asks for. As a query
    // parameter it is answered with 401, which reads as "you may not pull
    // this" and means "you did not ask properly". The Accept list is what
    // says a multi-architecture index is an acceptable answer; without it a
    // registry may refuse the only manifest it has.
    match machine
        .fetch(
            &format!("https://ghcr.io/v2/{path}/manifests/{reference}"),
            &[
                ("Authorization", &format!("Bearer {token}")),
                ("Accept", MANIFEST_TYPES),
            ],
        )
        .await
    {
        Ok(Answered { status: 200, .. }) => Finding::Fine(format!("{image} can be pulled")),
        Ok(Answered { status: 404, .. }) => Finding::Stops {
            what: format!("{image} does not exist"),
            fix: "Name a tag that was published, or check the repository.".into(),
        },
        Ok(Answered { status, .. }) => Finding::Unknown {
            what: format!("{image} answered {status}"),
            why: "which is neither a yes nor a no; the cluster may still pull it".into(),
        },
        Err(failed) => Finding::Unknown {
            what: format!("{image} could not be reached from here"),
            why: failed,
        },
    }
}

fn read_token(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("token")?
        .as_str()
        .map(String::from)
}

/// The platform, and the clock.
///
/// The clock is the check a person cannot do in their head. Every request a
/// deployment makes carries a note good for at most a minute, and a machine
/// whose clock is outside that window is refused for a reason that looks
/// exactly like a bad key: the platform says the note expired, and the
/// operator goes looking at their key.
pub async fn platform(machine: &dyn Machine, platform: &str) -> Vec<Finding> {
    let answered = match machine.fetch(platform, &[]).await {
        Ok(answered) => answered,
        Err(failed) => {
            return vec![Finding::Stops {
                what: format!("the platform at {platform} could not be reached: {failed}"),
                fix: "A deployment enrols its key and redeems its claim code through the \
                      platform. Check the address, and whether this network may reach it."
                    .into(),
            }]
        }
    };

    let mut findings = vec![Finding::Fine(format!("the platform at {platform} answers"))];

    let Some(theirs) = answered.date_s else {
        findings.push(Finding::Unknown {
            what: "this machine's clock could not be compared with the platform's".into(),
            why: "the platform stated no date".into(),
        });
        return findings;
    };

    let ours = machine.now_s();
    let skew = ours.abs_diff(theirs);
    findings.push(if skew * 2 < MAX_LIFETIME_SECONDS {
        Finding::Fine(format!(
            "this machine's clock is within {skew}s of the platform's"
        ))
    } else if skew < MAX_LIFETIME_SECONDS {
        Finding::Worth {
            what: format!("this machine's clock is {skew}s from the platform's"),
            why: format!(
                "a note is good for at most {MAX_LIFETIME_SECONDS}s, so this works and has \
                 little room left"
            ),
        }
    } else {
        Finding::Stops {
            what: format!("this machine's clock is {skew}s from the platform's"),
            fix: format!(
                "Every request carries a note good for at most {MAX_LIFETIME_SECONDS}s, and one \
                 signed by a clock this far out is refused as expired -- which reads exactly \
                 like a bad key. Turn on time synchronisation."
            ),
        }
    });
    findings
}

/// The taint the kubelet puts on a node whose free disk has fallen below its
/// eviction threshold, beside the `DiskPressure` condition it sets.
const DISK_PRESSURE_TAINT: &str = "node.kubernetes.io/disk-pressure";

const GIB: u64 = 1 << 30;

/// Free disk under the larger of these two is worth saying.
///
/// The kubelet's default hard eviction is `nodefs.available<10%` and
/// `imagefs.available<15%`, and on a single disk, as a local VM has, the
/// higher one is the one met first: the node that evicted a deployment on
/// 2026-09-30 was 86% full. Warning below 20% free comes five points before
/// that, which is a build or an image pull of room, and ten before the
/// kubelet's own disk runs out. On a small disk five points is less than one
/// pull, so the line is never under 10 GiB.
const DISK_WARN_PERCENT: u64 = 20;
const DISK_WARN_FLOOR: u64 = 10 * GIB;

/// Below how many bytes free a disk of this size is worth saying.
pub fn disk_line(capacity: u64) -> u64 {
    (capacity / 100 * DISK_WARN_PERCENT).max(DISK_WARN_FLOOR)
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / GIB as f64)
}

/// What freeing disk on a node usually means, said once for both findings.
const FREE_DISK: &str = "On a local VM, such as Rancher Desktop's, Docker's build cache is \
                         often most of it: `docker builder prune -a`.";

/// A node as far as its disk goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    /// What says it is under disk pressure: its condition, its taint, or
    /// both. Empty when neither does.
    pub pressure: Vec<String>,
}

/// From `kubectl get nodes -o json`. None when it is not a node list.
pub fn nodes(listed: &str) -> Option<Vec<Node>> {
    let listed: serde_json::Value = serde_json::from_str(listed.trim()).ok()?;
    let items = listed["items"].as_array()?;
    Some(
        items
            .iter()
            .map(|node| {
                let mut pressure = Vec::new();
                let pressed = node["status"]["conditions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|each| each["type"] == "DiskPressure" && each["status"] == "True");
                if pressed {
                    pressure.push("DiskPressure=True".to_string());
                }
                let tainted = node["spec"]["taints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|taint| taint["key"] == DISK_PRESSURE_TAINT);
                if tainted {
                    pressure.push(format!("tainted {DISK_PRESSURE_TAINT}"));
                }
                Node {
                    name: node["metadata"]["name"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    pressure,
                }
            })
            .collect(),
    )
}

/// One of a node's filesystems, as its kubelet states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    /// `disk`, or `image disk` where images are kept on a disk of their own.
    pub which: &'static str,
    pub free: u64,
    pub capacity: u64,
}

impl Space {
    pub fn low(&self) -> bool {
        self.free < disk_line(self.capacity)
    }

    fn percent(&self) -> u64 {
        self.free.saturating_mul(100) / self.capacity.max(1)
    }

    fn said(&self) -> String {
        format!(
            "{} of {} {} free ({}%)",
            gib(self.free),
            gib(self.capacity),
            self.which,
            self.percent()
        )
    }
}

/// The node's own filesystem and its image filesystem, from the kubelet's
/// stats summary. They are one disk when they state the same size, as on a
/// local VM, and said once. Nothing about the node's pods is read.
pub fn spaces(summary: &str) -> Vec<Space> {
    let summary: serde_json::Value = serde_json::from_str(summary.trim()).unwrap_or_default();
    let read = |fs: &serde_json::Value, which| {
        let capacity = fs["capacityBytes"].as_u64().filter(|bytes| *bytes > 0)?;
        Some(Space {
            which,
            free: fs["availableBytes"].as_u64()?,
            capacity,
        })
    };
    let node = read(&summary["node"]["fs"], "disk");
    let images = read(&summary["node"]["runtime"]["imageFs"], "image disk");
    match (node, images) {
        (Some(node), Some(images)) if node.capacity == images.capacity => vec![Space {
            free: node.free.min(images.free),
            ..node
        }],
        (node, images) => node.into_iter().chain(images).collect(),
    }
}

/// Why a question about nodes could not be answered, saying `refused` when
/// the cluster refused this account the right: nodes are asked about
/// cluster-wide, and a namespace's administrator may not hold that.
fn not_answered(failed: Failure, refused: &str) -> String {
    match failed {
        Failure::Said(said) if said.contains("Forbidden") || said.contains("forbidden") => {
            refused.to_string()
        }
        other => other.to_string(),
    }
}

/// "node a", or "nodes a, b".
fn named(names: &[String]) -> String {
    match names {
        [one] => format!("node {one}"),
        many => format!("nodes {}", many.join(", ")),
    }
}

/// Whether the cluster's nodes have the disk to keep a deployment running.
///
/// A node short of disk is found by Kubernetes first: the kubelet sets
/// `DiskPressure`, taints the node, evicts its pods and schedules none onto
/// it, and the deployment is gone with nothing said beforehand. This says it
/// beforehand, from the node's free disk as its kubelet states it through
/// the API server, which needs the right to get `nodes/proxy`. Without that
/// right the free disk is unknown, and the pressure itself, which needs only
/// the right to list nodes, is still read.
pub async fn disk(machine: &dyn Machine) -> Vec<Finding> {
    let unknown = |why: String| {
        vec![Finding::Unknown {
            what: "whether the cluster's nodes are short of disk is unknown".into(),
            why,
        }]
    };
    let listed = match machine
        .run("kubectl", &["get", "nodes", "-o", "json"])
        .await
    {
        Ok(listed) => listed,
        Err(failed) => {
            return unknown(not_answered(
                failed,
                "this account may not list nodes, which is a cluster-wide right",
            ))
        }
    };
    let nodes = match nodes(&listed) {
        Some(nodes) if !nodes.is_empty() => nodes,
        Some(_) => return unknown("`kubectl get nodes` listed none".into()),
        None => {
            return unknown("`kubectl get nodes -o json` said something that is not a list".into())
        }
    };

    let mut findings: Vec<Finding> = nodes
        .iter()
        .filter(|node| !node.pressure.is_empty())
        .map(|node| Finding::Stops {
            what: format!(
                "node {} is under disk pressure ({})",
                node.name,
                node.pressure.join(", ")
            ),
            fix: format!(
                "Kubernetes evicts a node's pods when it runs short of disk and schedules none \
                 onto it until it recovers, so a deployment there stops and its pods wait as \
                 Pending. Free disk on the node. {FREE_DISK} Then wait a few minutes for the \
                 taint to lift; the pods come back by themselves."
            ),
        })
        .collect();
    if findings.is_empty() {
        findings.push(Finding::Fine(match nodes.as_slice() {
            [one] => format!("node {} is not under disk pressure", one.name),
            many => format!("none of the {} nodes is under disk pressure", many.len()),
        }));
    }

    // Each node not already stopped for, asked of its kubelet.
    let mut roomy: Vec<(String, Space)> = Vec::new();
    let mut unread: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for node in nodes.iter().filter(|node| node.pressure.is_empty()) {
        let path = format!("/api/v1/nodes/{}/proxy/stats/summary", node.name);
        let spaces = match machine.run("kubectl", &["get", "--raw", &path]).await {
            Ok(summary) => spaces(&summary),
            Err(failed) => {
                unread
                    .entry(not_answered(
                        failed,
                        "this account may not get nodes/proxy, a cluster-wide right, which is \
                         how a node's free disk is asked of its kubelet. Disk pressure was read \
                         without it.",
                    ))
                    .or_default()
                    .push(node.name.clone());
                continue;
            }
        };
        if spaces.is_empty() {
            unread
                .entry("its kubelet's summary stated no size for its disk".into())
                .or_default()
                .push(node.name.clone());
            continue;
        }
        let low: Vec<&Space> = spaces.iter().filter(|space| space.low()).collect();
        for space in &low {
            findings.push(Finding::Worth {
                what: format!("node {} has {}", node.name, space.said()),
                why: format!(
                    "Kubernetes starts evicting a node's pods when its free disk falls below \
                     10% (15% for images), and this warns below {}: 20% of it, or 10 GiB where \
                     that is more. Free some before then. {FREE_DISK}",
                    gib(disk_line(space.capacity))
                ),
            });
        }
        if low.is_empty() {
            if let Some(least) = spaces.into_iter().min_by_key(Space::percent) {
                roomy.push((node.name.clone(), least));
            }
        }
    }
    if let Some((name, least)) = roomy.iter().min_by_key(|(_, space)| space.percent()) {
        findings.push(Finding::Fine(if roomy.len() == 1 {
            format!("node {name} has {}", least.said())
        } else {
            format!(
                "{} nodes have room; the least is node {name}, with {}",
                roomy.len(),
                least.said()
            )
        }));
    }
    for (why, names) in unread {
        findings.push(Finding::Unknown {
            what: format!("free disk on {} is unknown", named(&names)),
            why,
        });
    }
    findings
}

/// Whether each plugin launched in the namespace runs the sidecar its
/// deployment's components run.
///
/// A launched plugin keeps the sidecar it was launched with, through every
/// upgrade after, until it is relaunched; so what needs the newer sidecar
/// does not reach it, and nothing else says so. Worth saying, and never in
/// the way: relaunching it is the person's to decide. With nothing launched,
/// as before an install, there is nothing to say.
pub async fn plugins(machine: &dyn Machine, namespace: &str) -> Vec<Finding> {
    use crate::upgrade_deployment::{pods, to_relaunch};
    let listed = match machine
        .run("kubectl", &["get", "pods", "-n", namespace, "-o", "json"])
        .await
    {
        Ok(listed) => listed,
        Err(failed) => {
            return vec![Finding::Unknown {
                what: format!("whether the plugins launched in {namespace} run its sidecar"),
                why: failed.to_string(),
            }]
        }
    };
    let held = pods(&listed);
    let launched = held
        .iter()
        .filter(|pod| {
            pod.labels
                .get("meridian.dev/launched")
                .is_some_and(|mark| mark == "true")
                && !pod.terminating
                && !matches!(pod.phase.as_str(), "Succeeded" | "Failed")
        })
        .count();
    let stale: Vec<Finding> = to_relaunch(&held, None)
        .into_iter()
        .filter(|each| each.stale())
        .map(|each| {
            let [stop, launch] = each.commands();
            Finding::Worth {
                what: format!(
                    "plugin {} runs sidecar {}, and the deployment runs {}",
                    each.instance,
                    each.sidecar,
                    each.deployment.join(", ")
                ),
                why: format!(
                    "A launched plugin keeps the sidecar it was launched with until it is \
                     relaunched, so what needs the newer one does not reach it. `{stop}`, then \
                     `{launch}`, moves it{}.",
                    if each.live {
                        ", run in the plugin's directory"
                    } else {
                        "; `meridian plugin list` names its version"
                    }
                ),
            }
        })
        .collect();
    match (launched, stale.is_empty()) {
        (0, _) => Vec::new(),
        (1, true) => vec![Finding::Fine(format!(
            "the plugin launched in {namespace} runs its deployment's sidecar"
        ))],
        (many, true) => vec![Finding::Fine(format!(
            "the {many} plugins launched in {namespace} run their deployment's sidecar"
        ))],
        _ => stale,
    }
}

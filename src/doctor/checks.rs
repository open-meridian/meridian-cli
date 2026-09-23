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
fn version(said: &str) -> Option<(u32, u32)> {
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
            Ok(_) => Finding::Stops {
                what: format!("this account may not create {resource} in {namespace}"),
                fix: format!(
                    "Ask for rights to {what} in {namespace}, or install into a namespace you \
                     administer."
                ),
            },
            Err(failed) => Finding::Unknown {
                what: format!("whether this account may create {resource} is unknown"),
                why: failed.to_string(),
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

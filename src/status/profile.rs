//! `meridian status --profile` (step 3c; spec/a-deployment-scales-out-and-
//! fails-over-by-configuration section 12): the resilience profile as the
//! deployment computes it, ReadResilienceProfile through the dashboard's
//! terminal read, on the session `meridian connect` keeps. The conductor
//! answers it from the configuration as it is -- the chart's values as the
//! release states them, the members' reports, the nodes and zones, the
//! Leases and the source priorities -- never from a preset's name, so
//! neither does this: it prints the level, what the next level lacks, each
//! failure class and tier with its state, reason and the change that would
//! meet it, and never blind per live data type. Read-only.

use serde_json::Value;

/// The dashboard's terminal read of the profile.
pub const PROFILE_PATH: &str = "/terminal/resilience-profile";

/// The profile as the deployment answers it, or why it could not be read.
pub async fn read(address: &str, session: &crate::credential::Credential) -> Result<Value, String> {
    let http = crate::catalogue::http().map_err(|failed| failed.said().to_string())?;
    let url = format!("{address}{PROFILE_PATH}");
    let (status, body, _) = session
        .send(|bearer| http.get(url.clone()).bearer_auth(bearer))
        .await
        .map_err(|failed| failed.said().to_string())?;
    match status.as_u16() {
        404 => Err(format!(
            "{address} does not answer the resilience profile's read: its core is older than \
             contract v30"
        )),
        403 => Err("reading it takes a deployment admin".into()),
        _ if !status.is_success() => Err(format!("{address} refused ({status}): {}", body.trim())),
        _ => serde_json::from_str(&body)
            .map_err(|failed| format!("{address} answered with something unreadable: {failed}")),
    }
}

/// A word as the profile writes it (`single_node`, `met_by_restart`), as a
/// person reads it.
fn spoken(word: &str) -> String {
    match word {
        "single_node" => "Single node".into(),
        "multi_node" => "Multi-node".into(),
        "multi_zone" => "Multi-zone".into(),
        "desktop" => "Desktop".into(),
        other => other.replace('_', " "),
    }
}

fn changes(out: &mut String, changes: &Value, indent: &str) {
    for change in changes.as_array().into_iter().flatten() {
        let cost = change["cost"].as_str().unwrap_or_default();
        out.push_str(&format!(
            "{indent}change: {}{}\n",
            change["change"].as_str().unwrap_or_default(),
            if cost.is_empty() {
                String::new()
            } else {
                format!(" (costs {cost})")
            }
        ));
    }
}

fn lines(out: &mut String, title: &str, held: &Value) {
    let held: Vec<&Value> = held.as_array().into_iter().flatten().collect();
    if held.is_empty() {
        return;
    }
    out.push_str(&format!("\n{title}:\n"));
    let width = held
        .iter()
        .filter_map(|line| line["subject"].as_str())
        .map(str::len)
        .max()
        .unwrap_or(0);
    for line in held {
        let reason = line["reason"].as_str().unwrap_or_default();
        out.push_str(&format!(
            "  {:width$}  {}{}\n",
            line["subject"].as_str().unwrap_or_default(),
            spoken(line["state"].as_str().unwrap_or_default()),
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        ));
        changes(out, &line["changes"], &format!("  {:width$}    ", ""));
    }
}

/// When it was computed: core says nanoseconds since the epoch as a string.
fn computed(at: &Value) -> String {
    match at
        .as_str()
        .and_then(|ns| ns.parse::<u64>().ok())
        .or_else(|| at.as_u64())
    {
        Some(ns) if ns > 0 => crate::registry::moment(ns),
        _ => at.as_str().unwrap_or("at an unstated time").to_string(),
    }
}

/// The profile, as `status --profile` prints it.
pub fn said(profile: &Value) -> String {
    let mut out = format!(
        "Resilience profile, as the deployment computes it from its configuration as it is \
         (computed {}):\n  level: {}\n",
        computed(&profile["computed_at_ns"]),
        spoken(profile["level"].as_str().unwrap_or("not stated")),
    );
    match profile["next_level"].as_str() {
        Some(next) => {
            out.push_str(&format!("  the next, {}, lacks:\n", spoken(next)));
            changes(&mut out, &profile["next_level_changes"], "    ");
        }
        None => out.push_str("  the highest level there is\n"),
    }
    lines(&mut out, "Failure classes", &profile["failures"]);
    lines(&mut out, "Tiers", &profile["tiers"]);
    let blind: Vec<&Value> = profile["never_blind"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    if !blind.is_empty() {
        out.push_str("\nNever blind, per live data type:\n");
        let yes = |value: &Value| if value == true { "yes" } else { "no" };
        for line in blind {
            out.push_str(&format!(
                "  {}: a second source {}, standby pods {}, age and source shown {}\n",
                line["data_type"].as_str().unwrap_or_default(),
                yes(&line["second_source"]),
                yes(&line["standby_pods"]),
                yes(&line["age_and_source"]),
            ));
        }
    }
    out.push_str(
        "\nIt changes nothing. A change it names is made with `meridian upgrade-deployment --set`, \
         or at the dashboard's Plugins and Data sources.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_profile_is_said_from_the_configuration_as_the_deployment_answers_it() {
        let profile = json!({
            "level": "single_node",
            "next_level": "multi_node",
            "next_level_changes": [{"change": "resilience.profile=ha", "cost": "9 pods, 3 GiB"}],
            "failures": [
                {"subject": "pod", "state": "met_by_restart", "reason": "one book writer, no standby",
                 "changes": [{"change": "bor.standby=1", "cost": "1 pod"}]},
                {"subject": "volume lost", "state": "not_met", "reason": "backups not scheduled",
                 "changes": []},
            ],
            "tiers": [{"subject": "component", "state": "met", "reason": "", "changes": []}],
            "never_blind": [{"data_type": "fx quotes", "second_source": true, "standby_pods": false,
                             "age_and_source": true}],
            "computed_at_ns": "1794487905488194860",
        });
        let text = said(&profile);
        let at = crate::registry::moment(1_794_487_905_488_194_860);
        assert!(text.contains(&format!("(computed {at})")), "{text}");
        assert!(text.contains("  level: Single node\n"), "{text}");
        assert!(text.contains("  the next, Multi-node, lacks:\n    change: resilience.profile=ha (costs 9 pods, 3 GiB)\n"), "{text}");
        assert!(
            text.contains("  pod          met by restart: one book writer, no standby\n"),
            "{text}"
        );
        assert!(
            text.contains("change: bor.standby=1 (costs 1 pod)"),
            "{text}"
        );
        assert!(
            text.contains("  volume lost  not met: backups not scheduled\n"),
            "{text}"
        );
        assert!(text.contains("  component  met\n"), "{text}");
        assert!(
            text.contains(
                "  fx quotes: a second source yes, standby pods no, age and source shown yes"
            ),
            "{text}"
        );
        assert!(text.contains("It changes nothing."), "{text}");
    }
}

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

/// The profile, as `status --profile --wide` prints it.
pub fn said_wide(profile: &Value) -> String {
    said(profile)
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

    /// The profile the deployment answers with no advice.
    fn plain() -> Value {
        json!({
            "level": "multi_node",
            "next_level": "multi_zone",
            "next_level_changes": [{"change": "zones=3", "cost": "3 nodes"}],
            "failures": [{"subject": "pod", "state": "met", "reason": "", "changes": []}],
            "tiers": [],
            "never_blind": [],
            "computed_at_ns": "1794487905488194860",
        })
    }

    #[test]
    fn a_profile_without_advice_is_said_as_before_compact_or_wide() {
        let at = crate::registry::moment(1_794_487_905_488_194_860);
        let before = format!(
            "Resilience profile, as the deployment computes it from its configuration as it is \
             (computed {at}):\n  level: Multi-node\n  the next, Multi-zone, lacks:\n    change: \
             zones=3 (costs 3 nodes)\n\nFailure classes:\n  pod  met\n\nIt changes nothing. A \
             change it names is made with `meridian upgrade-deployment --set`, or at the \
             dashboard's Plugins and Data sources.\n"
        );
        assert_eq!(said(&plain()), before);
        assert_eq!(said_wide(&plain()), before);
        let mut empty = plain();
        empty["advice"] = json!([]);
        assert_eq!(said(&empty), before);
        assert_eq!(said_wide(&empty), before);
    }

    #[test]
    fn advice_is_a_line_each_under_the_profile_and_in_full_under_wide() {
        let mut profile = plain();
        profile["advice"] = json!([
            {"subject": "lake partitions", "state": "not_met",
             "reason": "2 ranges for 2 writer groups",
             "changes": [{"change": "lake.partitions=4", "cost": ""}]},
            {"subject": "book writer group 1", "state": "not_met",
             "reason": "its commit lane was busy 78% of the window at peak, past the 70% bound, \
                        and it refused work as overloaded twice",
             "changes": [{"change": "bor.writerGroups=3", "cost": "1 pod, 512 MiB"}]},
        ]);
        let text = said(&profile);
        // Under the profile, before the failure classes: a line each.
        let advice = text.find("\nAdvice:\n").expect(&text);
        assert!(
            advice < text.find("\nFailure classes:\n").expect(&text),
            "{text}"
        );
        assert!(
            text.contains(
                "\nAdvice:\n  lake partitions      not met: 2 ranges for 2 writer groups; \
                 change: lake.partitions=4\n"
            ),
            "{text}"
        );
        // The long one cut at the width, its detail under --wide.
        let long = text
            .lines()
            .find(|line| line.starts_with("  book writer group 1 "))
            .expect(&text);
        assert!(long.chars().count() <= crate::status::WIDTH, "{long}");
        assert!(long.ends_with('…'), "{long}");
        assert!(
            text.contains("  `meridian status --profile --wide` says each in full\n"),
            "{text}"
        );
        // Nothing else changed.
        assert!(text.contains("\nFailure classes:\n  pod  met\n"), "{text}");
        assert!(text.contains("It changes nothing."), "{text}");

        let wide = said_wide(&profile);
        assert!(
            wide.contains(
                "  book writer group 1  not met: its commit lane was busy 78% of the window at \
                 peak, past the 70% bound, and it refused work as overloaded twice\n"
            ),
            "{wide}"
        );
        assert!(
            wide.contains("change: bor.writerGroups=3 (costs 1 pod, 512 MiB)\n"),
            "{wide}"
        );
        assert!(wide.contains("change: lake.partitions=4\n"), "{wide}");
        assert!(!wide.contains('…'), "{wide}");
        assert!(!wide.contains("says each in full"), "{wide}");
    }

    #[test]
    fn short_advice_needs_no_pointer_to_wide() {
        let mut profile = plain();
        profile["advice"] = json!([{"subject": "lake partitions", "state": "not_met",
            "reason": "2 ranges for 2 writer groups",
            "changes": [{"change": "lake.partitions=4", "cost": "none"}]}]);
        let text = said(&profile);
        assert!(
            text.contains(
                "\nAdvice:\n  lake partitions  not met: 2 ranges for 2 writer groups; change: \
                 lake.partitions=4 (costs none)\n\nFailure classes:"
            ),
            "{text}"
        );
        assert!(!text.contains("--wide"), "{text}");
    }
}

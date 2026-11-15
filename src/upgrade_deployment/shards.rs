//! Plugin shards and standby pods against the deployment's own standbys
//! (step 3d; spec/a-deployment-scales-out-and-fails-over-by-configuration
//! section 7).
//!
//! A plugin launched with several shards, or with a standby pod, takes a
//! Lease per shard, under the members' account: the one the chart makes
//! while any of the lake, the book, the instrument store, the street store
//! or the conductor runs a standby (`resilience.profile` ha, or a
//! `<component>.standby` of 1). An upgrade that leaves none of them with a
//! standby deletes that account, and those plugins' pods could no longer
//! say which of them serves. It is refused while any such instance runs,
//! naming them: stop them or relaunch them unsharded first.
//!
//! Both sides are read from the release's own workloads, as the cluster
//! lists them, since an upgrade is made with cluster rights alone and holds
//! no session with the dashboard: a member that takes a lease carries
//! `MERIDIAN_LEASE_NAME`, and a plugin's sidecar carries its shard count in
//! `MERIDIAN_PLUGIN_SHARDS`, its Deployment's replicas past that count
//! being its standby pods.

use serde_json::Value;

use crate::doctor::Finding;

/// The components whose standby keeps the members' account.
pub const LEASED: [&str; 5] = ["lake", "bor", "instrument", "street", "conductor"];

/// On a member that takes a lease to say which writes.
pub const LEASE_VARIABLE: &str = "MERIDIAN_LEASE_NAME";

/// On a plugin's sidecar: the pods of its instance sharing its subjects.
pub const SHARDS_VARIABLE: &str = "MERIDIAN_PLUGIN_SHARDS";

fn variable<'a>(workload: &'a Value, name: &str) -> Option<&'a str> {
    workload["spec"]["template"]["spec"]["containers"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|container| container["env"].as_array().into_iter().flatten())
        .find(|variable| variable["name"] == name)
        .map(|variable| variable["value"].as_str().unwrap_or_default())
}

/// A plugin workload's shards and standby pods, where its sidecar says its
/// shard count: the replicas past that count stand by. None for a workload
/// that says none, as every one from before step 3d.
pub fn shape(workload: &Value) -> Option<(u64, u64)> {
    let shards = variable(workload, SHARDS_VARIABLE)?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|shards| *shards >= 1)?;
    let replicas = workload["spec"]["replicas"].as_u64().unwrap_or(shards);
    Some((shards, replicas.saturating_sub(shards)))
}

/// Whether a member of the release takes a lease now: the members' account
/// is there.
pub fn runs_leased(workloads: &Value) -> bool {
    workloads["items"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|workload| {
            let component = workload["metadata"]["labels"]["meridian.dev/component"]
                .as_str()
                .unwrap_or_default();
            // A writer group past the first is its store's.
            let component = component.split("-group-").next().unwrap_or_default();
            LEASED.contains(&component) && variable(workload, LEASE_VARIABLE).is_some()
        })
}

/// Whether the values an upgrade leaves keep a standby on any of them: a
/// `<component>.standby` given, 0 included, else the profile's.
pub fn keeps_leased(values: &Value) -> bool {
    let ha = values["resilience"]["profile"] == "ha";
    LEASED.iter().any(|component| {
        let standby = &values[*component]["standby"];
        // No members of it at all: no standby either.
        if values[*component]["replicaCount"].as_u64() == Some(0) {
            return false;
        }
        match standby {
            Value::Null => ha,
            Value::Number(n) => n.as_u64().is_some_and(|n| n >= 1),
            Value::String(said) => said.trim().parse::<u64>().is_ok_and(|n| n >= 1),
            _ => false,
        }
    })
}

/// A running plugin instance that needs the members' account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Needing {
    pub instance: String,
    pub shards: u64,
    pub standbys: u64,
}

/// Each plugin instance running with several shards or a standby pod.
pub fn needing(workloads: &Value) -> Vec<Needing> {
    let mut found: Vec<Needing> = workloads["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|workload| workload["spec"]["replicas"].as_u64() != Some(0))
        .filter_map(|workload| {
            let (shards, standbys) = shape(workload)?;
            (shards > 1 || standbys > 0).then(|| Needing {
                instance: workload["metadata"]["labels"]["meridian.dev/instance"]
                    .as_str()
                    .or(workload["metadata"]["name"].as_str())
                    .unwrap_or_default()
                    .to_string(),
                shards,
                standbys,
            })
        })
        .collect();
    found.sort_by(|a, b| a.instance.cmp(&b.instance));
    found
}

/// The refusal, where the upgrade would leave no standby while such an
/// instance runs: nothing otherwise.
pub fn finding(workloads: &Value, values: &Value) -> Option<Finding> {
    if !runs_leased(workloads) || keeps_leased(values) {
        return None;
    }
    let needing = needing(workloads);
    if needing.is_empty() {
        return None;
    }
    let named: Vec<String> = needing
        .iter()
        .map(|one| {
            let mut said = Vec::new();
            if one.shards > 1 {
                said.push(format!("{} shards", one.shards));
            }
            match one.standbys {
                0 => {}
                1 => said.push("a standby pod".to_string()),
                n => said.push(format!("{n} standby pods")),
            }
            format!("{} ({})", one.instance, said.join(", "))
        })
        .collect();
    Some(Finding::Stops {
        what: format!(
            "this upgrade leaves no store or conductor with a standby, which removes the \
             members' account that plugin shards and standby pods take their leases under, \
             and {} still run{} with them: {}",
            match needing.len() {
                1 => "1 plugin instance".to_string(),
                n => format!("{n} plugin instances"),
            },
            if needing.len() == 1 { "s" } else { "" },
            named.join(", ")
        ),
        fix: "Stop them or relaunch them unsharded first: `meridian plugin stop <instance>`, \
              or `meridian plugin launch <name> <version> --instance <instance>` without \
              --shards and --standby. Then run this again."
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn member(component: &str, leased: bool) -> Value {
        let env = if leased {
            json!([{ "name": LEASE_VARIABLE, "value": format!("om-{component}") }])
        } else {
            json!([])
        };
        json!({ "kind": "Deployment",
                "metadata": { "name": format!("om-{component}"),
                              "labels": { "meridian.dev/component": component } },
                "spec": { "replicas": 2, "template": { "spec": { "containers": [
                    { "name": component, "env": env } ] } } } })
    }

    fn plugin(instance: &str, shards: Option<u64>, replicas: u64) -> Value {
        let env = match shards {
            Some(shards) => json!([{ "name": SHARDS_VARIABLE, "value": shards.to_string() }]),
            None => json!([]),
        };
        json!({ "kind": "Deployment",
                "metadata": { "name": format!("om-plugin-{instance}"),
                              "labels": { "meridian.dev/component": "sidecar",
                                          "meridian.dev/instance": instance } },
                "spec": { "replicas": replicas, "template": { "spec": { "containers": [
                    { "name": "sidecar", "env": env }, { "name": "plugin" } ] } } } })
    }

    fn running(plugins: Vec<Value>) -> Value {
        let mut items = vec![member("lake", true), member("dashboard", false)];
        items.extend(plugins);
        json!({ "items": items })
    }

    #[test]
    fn a_plugins_replicas_past_its_shards_are_its_standby_pods() {
        assert_eq!(shape(&plugin("a", Some(2), 3)), Some((2, 1)));
        assert_eq!(shape(&plugin("a", Some(1), 1)), Some((1, 0)));
        // From before step 3d: it says no count, and nothing is read into it.
        assert_eq!(shape(&plugin("a", None, 1)), None);
    }

    #[test]
    fn a_standby_is_kept_by_the_profile_or_by_a_components_own_count() {
        assert!(keeps_leased(&json!({ "resilience": { "profile": "ha" } })));
        assert!(!keeps_leased(
            &json!({ "resilience": { "profile": "tolerant" } })
        ));
        assert!(keeps_leased(
            &json!({ "resilience": { "profile": "tolerant" }, "bor": { "standby": 1 } })
        ));
        // ha with every count given as 0: none left.
        let none: serde_json::Map<String, Value> = LEASED
            .iter()
            .map(|component| (component.to_string(), json!({ "standby": 0 })))
            .collect();
        let mut values = Value::Object(none);
        values["resilience"] = json!({ "profile": "ha" });
        assert!(!keeps_leased(&values));
    }

    #[test]
    fn leaving_ha_is_refused_while_a_plugin_runs_sharded_or_with_a_standby() {
        let workloads = running(vec![
            plugin("tiingo", Some(3), 3),
            plugin("oanda", Some(1), 2),
            plugin("plain", Some(1), 1),
            plugin("older", None, 1),
            // Stopped: it holds no lease.
            plugin("stopped", Some(2), 0),
        ]);
        let tolerant = json!({ "resilience": { "profile": "tolerant" } });
        let Some(Finding::Stops { what, fix }) = finding(&workloads, &tolerant) else {
            panic!("it stops");
        };
        assert!(
            what.ends_with(
                "2 plugin instances still run with them: oanda (a standby pod), tiingo (3 shards)"
            ),
            "{what}"
        );
        assert!(
            fix.starts_with("Stop them or relaunch them unsharded first"),
            "{fix}"
        );
        // Still ha, or one standby kept: nothing is removed.
        assert_eq!(
            finding(&workloads, &json!({ "resilience": { "profile": "ha" } })),
            None
        );
        assert_eq!(
            finding(
                &workloads,
                &json!({ "resilience": { "profile": "tolerant" }, "lake": { "standby": 1 } })
            ),
            None
        );
    }

    #[test]
    fn nothing_is_refused_with_no_such_plugin_or_where_no_member_took_a_lease() {
        let tolerant = json!({ "resilience": { "profile": "tolerant" } });
        let plain = running(vec![plugin("plain", Some(1), 1)]);
        assert_eq!(finding(&plain, &tolerant), None);
        // Never ha: the account was not there to remove.
        let never = json!({ "items": [member("lake", false), plugin("tiingo", Some(3), 3)] });
        assert_eq!(finding(&never, &tolerant), None);
    }
}

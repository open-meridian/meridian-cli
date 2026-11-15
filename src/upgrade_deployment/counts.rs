//! The counts an upgrade may not change freely (step 3d; spec/a-deployment-
//! scales-out-and-fails-over-by-configuration sections 6.3, 6.4 and 6.8):
//! the book's partitions (`bor.partitions`), and each partitioned store's
//! writer groups (`<store>.writerGroups`) and databases
//! (`<store>.databases`).
//!
//! Raising a count is an online transition the deployment makes itself.
//! Three changes are refused here, before anything is changed:
//!
//! - **A lower `bor.partitions`** while a partition past the new count
//!   holds accounts, naming each one that does. An account's placement is
//!   fixed at its creation, and it moves only by `meridian account move`.
//! - **A lower `lake.databases` or `bor.databases`** while partitions hold
//!   data in a database past the new count, naming them. Writer group g
//!   writes to database g mod the count.
//! - **A `writerGroups` or `databases` other than what runs, in the same
//!   upgrade as the deployment's first start on a release that has them.**
//!   That start would assign every partition by the new count at once, a
//!   jump with no online move: upgrade first, then change the count.
//!
//! What runs is read from the workloads' own environment, which the chart
//! sets, never from the deployment's values. What each partition holds is
//! read from the stores' probes (`partitions`, as the store's control
//! partition assigns them); where no probe says, a lowering is refused and
//! says it could not be read, never passed.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::doctor::Finding;

/// The stores that have writer groups and databases of their own.
pub const STORES: [&str; 2] = ["lake", "bor"];

/// What the chart sets on the book's writer from the release that has
/// partitions on: always, whatever the count. A book without it runs a
/// release from before them.
pub const PARTITIONS_VARIABLE: &str = "MERIDIAN_BOR_PARTITIONS";

/// Set on a store's members with several groups only.
pub const GROUPS_VARIABLE: &str = "MERIDIAN_WRITER_GROUPS";

/// `MERIDIAN_<STORE>_DATABASES`, set with several databases only.
pub fn databases_variable(store: &str) -> String {
    format!("MERIDIAN_{}_DATABASES", store.to_uppercase())
}

/// One store's counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Store {
    pub groups: u64,
    pub databases: u64,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            groups: 1,
            databases: 1,
        }
    }
}

/// The counts that run now, as the workloads' own templates say them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    /// Whether the release that runs has these counts at all: its book
    /// carries [`PARTITIONS_VARIABLE`].
    pub has_counts: bool,
    pub partitions: u64,
    pub stores: BTreeMap<&'static str, Store>,
}

/// The counts the upgrade would leave: None for one that is not a whole
/// number from 1, which the chart refuses in its own words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Wanted {
    pub partitions: Option<u64>,
    pub groups: BTreeMap<&'static str, Option<u64>>,
    pub databases: BTreeMap<&'static str, Option<u64>>,
}

fn environment(deployment: &Value) -> BTreeMap<&str, &str> {
    deployment["spec"]["template"]["spec"]["containers"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|container| container["env"].as_array().into_iter().flatten())
        .filter_map(|variable| Some((variable["name"].as_str()?, variable["value"].as_str()?)))
        .collect()
}

/// What runs, from `kubectl get deployments,... -o json`: None where the
/// listing holds no book, so nothing can be said of what runs.
pub fn running(workloads: &Value) -> Option<Running> {
    let of = |component: &str| {
        workloads["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| {
                item["kind"] == "Deployment"
                    && item["metadata"]["labels"]["meridian.dev/component"] == component
            })
            .map(environment)
    };
    let count = |said: Option<&&str>| said.and_then(|n| n.parse::<u64>().ok()).unwrap_or(1);
    let book = of("bor")?;
    let mut stores = BTreeMap::new();
    for store in STORES {
        let environment = of(store).unwrap_or_default();
        stores.insert(
            store,
            Store {
                groups: count(environment.get(GROUPS_VARIABLE)),
                databases: count(environment.get(databases_variable(store).as_str())),
            },
        );
    }
    Some(Running {
        has_counts: book.contains_key(PARTITIONS_VARIABLE),
        partitions: count(book.get(PARTITIONS_VARIABLE)),
        stores,
    })
}

/// A count as a values file or a `--set` writes it: 1 where it is not
/// said, None where it is not a whole number from 1.
fn count_in(value: &Value) -> Option<u64> {
    match value {
        Value::Null => Some(1),
        Value::Number(n) => n.as_u64().filter(|n| *n >= 1),
        Value::String(said) => said.trim().parse::<u64>().ok().filter(|n| *n >= 1),
        _ => None,
    }
}

/// What the upgrade would leave, from its values: the new chart's defaults
/// under the deployment's own and the person's.
pub fn wanted(values: &Value) -> Wanted {
    Wanted {
        partitions: count_in(&values["bor"]["partitions"]),
        groups: STORES
            .iter()
            .map(|store| (*store, count_in(&values[*store]["writerGroups"])))
            .collect(),
        databases: STORES
            .iter()
            .map(|store| (*store, count_in(&values[*store]["databases"])))
            .collect(),
    }
}

/// Whether the chart upgraded to has these counts: its own values name the
/// book's partitions.
pub fn chart_has_counts(defaults: &Value) -> bool {
    !defaults["bor"]["partitions"].is_null()
}

/// One partition as its store's control partition assigns it, from a
/// probe's `partitions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assigned {
    pub partition: String,
    pub group: u64,
    pub database: u64,
    /// The accounts placed in it: the book's partitions only.
    pub accounts: Option<u64>,
}

/// What the stores' probes said of their partitions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Held {
    /// Each store's partitions as its control partition assigns them, where
    /// a member's probe carried them.
    pub assigned: BTreeMap<String, Vec<Assigned>>,
    /// Each writer's own partitions, by store and group, as every probe
    /// says them: at most 64 named.
    pub writing: BTreeMap<(String, u64), Vec<String>>,
}

/// The store and writer group of a member, from its pod's component label:
/// `lake` is the lake's group 0, `lake-group-2` its group 2.
pub fn store_and_group(component: &str) -> Option<(&'static str, u64)> {
    STORES.iter().find_map(|store| {
        let rest = component.strip_prefix(store)?;
        if rest.is_empty() {
            return Some((*store, 0));
        }
        let group = rest.strip_prefix("-group-")?.parse().ok()?;
        Some((*store, group))
    })
}

impl Held {
    /// One member's probe, read in.
    pub fn read(&mut self, component: &str, probe: &Value) {
        let Some((store, group)) = store_and_group(component) else {
            return;
        };
        if let Some(listed) = probe["partitions"].as_array() {
            let assigned = listed
                .iter()
                .filter_map(|one| {
                    Some(Assigned {
                        partition: one["partition"].as_str()?.to_string(),
                        group: one["group"].as_u64()?,
                        database: one["database"].as_u64()?,
                        accounts: one["accounts"].as_u64(),
                    })
                })
                .collect::<Vec<_>>();
            // The longest any member carried: each reads the same control
            // partition, and one mid-start may hold less of it.
            let kept = self.assigned.entry(store.to_string()).or_default();
            if assigned.len() >= kept.len() {
                *kept = assigned;
            }
        }
        if probe["member"]["mode"] == "writer" {
            self.writing.insert(
                (store.to_string(), group),
                probe["member"]["partitions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|name| name.as_str().map(String::from))
                    .collect(),
            );
        }
    }
}

/// Whether any check here needs the stores' probes: a count that runs
/// would go down.
pub fn lowers(running: &Running, wanted: &Wanted) -> bool {
    wanted.partitions.is_some_and(|n| n < running.partitions)
        || STORES.iter().any(|store| {
            wanted.databases[store].is_some_and(|n| n < running.stores[store].databases)
        })
}

fn listed(names: &[String]) -> String {
    const MOST: usize = 12;
    let mut said = names
        .iter()
        .take(MOST)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > MOST {
        said.push_str(&format!(" and {} more", names.len() - MOST));
    }
    said
}

fn accounts_said(n: u64) -> String {
    match n {
        1 => "1 account".to_string(),
        n => format!("{n} accounts"),
    }
}

/// A count changed in the same upgrade as the first start on a release
/// that has the counts.
fn first_start(release: &str, running: &Running, wanted: &Wanted) -> Vec<Finding> {
    let mut set = Vec::new();
    for store in STORES {
        let runs = running.stores[store];
        for (key, runs, wanted) in [
            ("writerGroups", runs.groups, wanted.groups[store]),
            ("databases", runs.databases, wanted.databases[store]),
        ] {
            if let Some(wanted) = wanted.filter(|wanted| *wanted != runs) {
                set.push(format!("{store}.{key}={wanted}"));
            }
        }
    }
    if set.is_empty() {
        return Vec::new();
    }
    let keys: Vec<&str> = set.iter().filter_map(|one| one.split('=').next()).collect();
    vec![Finding::Stops {
        what: format!(
            "this upgrade is {release}'s first start on a release with writer groups and a \
             database per group, and it also sets {}",
            set.join(", ")
        ),
        fix: format!(
            "Upgrade first, then change the count: run this upgrade without {}, and once it \
             is done run `meridian upgrade-deployment --set {}`. A count changed in the same \
             upgrade as that first start would assign every partition by the new count at \
             once, with no online move.",
            keys.join(" and "),
            set.join(",")
        ),
    }]
}

/// A lower `bor.partitions`.
fn fewer_partitions(runs: u64, wanted: u64, held: &Held) -> Finding {
    let going: Vec<String> = (wanted..runs).map(|n| format!("P{n}")).collect();
    let assigned = held.assigned.get("bor");
    let mut holding = Vec::new();
    let mut unread = Vec::new();
    for partition in &going {
        match assigned
            .and_then(|all| all.iter().find(|one| &one.partition == partition))
            .and_then(|one| one.accounts)
        {
            Some(0) => {}
            Some(n) => holding.push(format!("{partition} ({})", accounts_said(n))),
            None => unread.push(partition.clone()),
        }
    }
    if holding.is_empty() && unread.is_empty() {
        return Finding::Fine(format!(
            "bor.partitions goes from {runs} to {wanted}: {} hold{} no accounts",
            listed(&going),
            if going.len() == 1 { "s" } else { "" }
        ));
    }
    let mut what = format!("bor.partitions would go from {runs} to {wanted}");
    if !holding.is_empty() {
        what.push_str(&format!(
            ", and {} still hold{} accounts",
            listed(&holding),
            {
                if holding.len() == 1 {
                    "s"
                } else {
                    ""
                }
            }
        ));
    }
    if !unread.is_empty() {
        what.push_str(&format!(
            ", and what {} hold{} could not be read: the book's probe did not say",
            listed(&unread),
            if unread.len() == 1 { "s" } else { "" }
        ));
    }
    Finding::Stops {
        what,
        fix: format!(
            "An account moves only by hand: `meridian account move <account> --to-partition \
             <partition> --reason <why>`, to a partition below P{wanted}. Move each account \
             out, then run this again. Raising the count needs none of this."
        ),
    }
}

/// A lower `<store>.databases`.
fn fewer_databases(store: &str, runs: Store, wanted: u64, held: &Held) -> Finding {
    let mut holding = Vec::new();
    let mut unread = Vec::new();
    match held.assigned.get(store) {
        // The store's own assignments: every partition, and its database.
        Some(assigned) => holding.extend(
            assigned
                .iter()
                .filter(|one| one.database >= wanted)
                .map(|one| format!("{} (database {})", one.partition, one.database)),
        ),
        // Without them, each running group whose database would go, and
        // what its writer says it writes.
        None => {
            let going: BTreeSet<u64> = (0..runs.groups)
                .filter(|group| group % runs.databases >= wanted)
                .collect();
            for group in going {
                let database = group % runs.databases;
                match held.writing.get(&(store.to_string(), group)) {
                    Some(partitions) if partitions.is_empty() => {}
                    Some(partitions) => holding.extend(
                        partitions
                            .iter()
                            .map(|partition| format!("{partition} (database {database})")),
                    ),
                    None => unread.push(format!("group {group}'s database {database}")),
                }
            }
            // A partition still moving off a group that no longer runs is
            // only in the store's assignments, which no probe carried.
            if holding.is_empty() && unread.is_empty() {
                unread.push(if wanted + 1 == runs.databases {
                    format!("database {wanted}")
                } else {
                    format!("each database from {wanted} to {}", runs.databases - 1)
                });
            }
        }
    }
    if holding.is_empty() && unread.is_empty() {
        return Finding::Fine(format!(
            "{store}.databases goes from {} to {wanted}: no partition holds data in a \
             database past the new count",
            runs.databases
        ));
    }
    let mut what = format!(
        "{store}.databases would go from {} to {wanted}",
        runs.databases
    );
    if !holding.is_empty() {
        what.push_str(&format!(
            ", and partitions still hold data in a database past the new count: {}",
            listed(&holding)
        ));
    }
    if !unread.is_empty() {
        what.push_str(&format!(
            ", and what {} holds could not be read: the {store}'s probe did not say",
            listed(&unread)
        ));
    }
    Finding::Stops {
        what,
        fix: format!(
            "Move each partition to a writer group whose database stays, `meridian partition \
             move <partition> --to-group <group> --reason <why>`, or lower \
             {store}.writerGroups first and wait for its partitions to move (`meridian \
             status` shows each partition's writer). Then run this again."
        ),
    }
}

/// Every finding of the counts: nothing where the chart upgraded to has
/// none of them.
pub fn findings(
    release: &str,
    chart_has_counts: bool,
    running: Option<&Running>,
    wanted: &Wanted,
    held: &Held,
) -> Vec<Finding> {
    if !chart_has_counts {
        return Vec::new();
    }
    let Some(running) = running else {
        let above_one = wanted.partitions.is_some_and(|n| n > 1)
            || STORES.iter().any(|store| {
                wanted.groups[store].is_some_and(|n| n > 1)
                    || wanted.databases[store].is_some_and(|n| n > 1)
            });
        let what = "the partitions, writer groups and databases that run now could not be \
                    read: the release's book was not among its workloads";
        return vec![if above_one {
            Finding::Stops {
                what: what.into(),
                fix: "A count is changed only from what runs. Check that this account may \
                      read Deployments in the namespace, then run this again."
                    .into(),
            }
        } else {
            Finding::Unknown {
                what: what.into(),
                why: "A count that runs above 1 would be lowered by this upgrade unseen.".into(),
            }
        }];
    };
    if !running.has_counts {
        return first_start(release, running, wanted);
    }
    let mut found = Vec::new();
    if let Some(wanted) = wanted.partitions.filter(|n| *n < running.partitions) {
        found.push(fewer_partitions(running.partitions, wanted, held));
    }
    for store in STORES {
        let runs = running.stores[store];
        if let Some(wanted) = wanted.databases[store].filter(|n| *n < runs.databases) {
            found.push(fewer_databases(store, runs, wanted, held));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn deployment(component: &str, environment: &[(&str, &str)]) -> Value {
        json!({
            "kind": "Deployment",
            "metadata": { "name": format!("meridian-{component}"),
                          "labels": { "meridian.dev/component": component } },
            "spec": { "template": { "spec": { "containers": [{
                "name": component,
                "env": environment.iter()
                    .map(|(name, value)| json!({ "name": name, "value": value }))
                    .collect::<Vec<_>>(),
            }] } } },
        })
    }

    fn runs(book: &[(&str, &str)], lake: &[(&str, &str)]) -> Running {
        running(&json!({ "items": [deployment("bor", book), deployment("lake", lake)] }))
            .expect("a book is listed")
    }

    fn stops(found: &[Finding]) -> (String, String) {
        match found {
            [Finding::Stops { what, fix }] => (what.clone(), fix.clone()),
            other => panic!("one check stops it: {other:?}"),
        }
    }

    #[test]
    fn what_runs_is_read_from_the_workloads_own_environment() {
        let before = runs(&[], &[]);
        assert!(!before.has_counts);
        assert_eq!(before.partitions, 1);
        assert_eq!(before.stores["lake"], Store::default());

        let now = runs(
            &[
                ("MERIDIAN_BOR_PARTITIONS", "4"),
                ("MERIDIAN_WRITER_GROUPS", "2"),
                ("MERIDIAN_BOR_DATABASES", "2"),
            ],
            &[("MERIDIAN_WRITER_GROUPS", "3")],
        );
        assert!(now.has_counts);
        assert_eq!(now.partitions, 4);
        assert_eq!(
            now.stores["bor"],
            Store {
                groups: 2,
                databases: 2
            }
        );
        assert_eq!(
            now.stores["lake"],
            Store {
                groups: 3,
                databases: 1
            }
        );
        // No book among the workloads: nothing is said of what runs.
        assert_eq!(
            running(&json!({ "items": [deployment("lake", &[])] })),
            None
        );
    }

    #[test]
    fn a_count_is_one_where_unsaid_and_none_where_it_is_no_count() {
        let read = wanted(&json!({
            "bor": { "partitions": "8", "writerGroups": 2 },
            "lake": { "databases": 0, "writerGroups": "two" },
        }));
        assert_eq!(read.partitions, Some(8));
        assert_eq!(read.groups["bor"], Some(2));
        assert_eq!(read.databases["bor"], Some(1));
        assert_eq!(read.databases["lake"], None);
        assert_eq!(read.groups["lake"], None);
    }

    #[test]
    fn a_member_is_its_stores_group_by_its_component() {
        assert_eq!(store_and_group("lake"), Some(("lake", 0)));
        assert_eq!(store_and_group("bor-group-3"), Some(("bor", 3)));
        assert_eq!(store_and_group("lake-reader"), None);
        assert_eq!(store_and_group("conductor"), None);
    }

    #[test]
    fn a_chart_without_the_counts_is_checked_for_none() {
        let running = runs(&[], &[]);
        let wanted = wanted(&json!({ "lake": { "writerGroups": 2 } }));
        assert!(findings("om", false, Some(&running), &wanted, &Held::default()).is_empty());
    }

    #[test]
    fn a_count_changed_with_the_first_start_is_refused_upgrade_first() {
        let running = runs(&[], &[]);
        let wanted = wanted(&json!({
            "lake": { "writerGroups": 2 },
            "bor": { "databases": 2, "partitions": 4 },
        }));
        let (what, fix) = stops(&findings(
            "om",
            true,
            Some(&running),
            &wanted,
            &Held::default(),
        ));
        assert!(
            what.contains("first start") && what.contains("lake.writerGroups=2, bor.databases=2"),
            "{what}"
        );
        assert!(
            fix.starts_with("Upgrade first, then change the count"),
            "{fix}"
        );
        assert!(
            fix.contains("without lake.writerGroups and bor.databases"),
            "{fix}"
        );
        assert!(
            fix.contains("--set lake.writerGroups=2,bor.databases=2"),
            "{fix}"
        );
        // The book's partitions place new accounts only: never part of it.
        assert!(!what.contains("partitions"), "{what}");
    }

    #[test]
    fn the_first_start_with_the_counts_left_alone_passes() {
        let running = runs(&[], &[]);
        let wanted = wanted(&json!({ "bor": { "partitions": 4 } }));
        assert!(findings("om", true, Some(&running), &wanted, &Held::default()).is_empty());
    }

    #[test]
    fn a_count_raised_after_the_first_start_passes() {
        let running = runs(&[("MERIDIAN_BOR_PARTITIONS", "1")], &[]);
        let wanted = wanted(&json!({
            "lake": { "writerGroups": 2, "databases": 2 },
            "bor": { "partitions": 4 },
        }));
        assert!(!lowers(&running, &wanted));
        assert!(findings("om", true, Some(&running), &wanted, &Held::default()).is_empty());
    }

    fn book_probe(accounts: &[u64]) -> Value {
        json!({
            "member": { "mode": "writer", "partitions": ["P0", "control"] },
            "partitions": accounts.iter().enumerate().map(|(n, accounts)| json!({
                "partition": format!("P{n}"), "group": 0, "database": 0, "accounts": accounts,
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn fewer_partitions_is_refused_naming_each_that_still_holds_accounts() {
        let running = runs(&[("MERIDIAN_BOR_PARTITIONS", "4")], &[]);
        let wanted = wanted(&json!({ "bor": { "partitions": 2 } }));
        assert!(lowers(&running, &wanted));
        let mut held = Held::default();
        held.read("bor", &book_probe(&[5, 9, 0, 1]));
        let (what, fix) = stops(&findings("om", true, Some(&running), &wanted, &held));
        assert_eq!(
            what,
            "bor.partitions would go from 4 to 2, and P3 (1 account) still holds accounts"
        );
        assert!(
            fix.contains("meridian account move") && fix.contains("below P2"),
            "{fix}"
        );

        held.read("bor", &book_probe(&[5, 9, 7, 12]));
        let (what, _) = stops(&findings("om", true, Some(&running), &wanted, &held));
        assert!(
            what.ends_with("P2 (7 accounts), P3 (12 accounts) still hold accounts"),
            "{what}"
        );
    }

    #[test]
    fn fewer_partitions_passes_once_each_one_going_is_empty() {
        let running = runs(&[("MERIDIAN_BOR_PARTITIONS", "4")], &[]);
        let wanted = wanted(&json!({ "bor": { "partitions": 2 } }));
        let mut held = Held::default();
        held.read("bor", &book_probe(&[5, 9, 0, 0]));
        assert_eq!(
            findings("om", true, Some(&running), &wanted, &held),
            vec![Finding::Fine(
                "bor.partitions goes from 4 to 2: P2, P3 hold no accounts".into()
            )]
        );
    }

    #[test]
    fn fewer_partitions_is_refused_where_the_probe_does_not_say_what_they_hold() {
        let running = runs(&[("MERIDIAN_BOR_PARTITIONS", "2")], &[]);
        let wanted = wanted(&json!({ "bor": { "partitions": 1 } }));
        let mut held = Held::default();
        // A book whose probe carries no `partitions`.
        held.read(
            "bor",
            &json!({ "member": { "mode": "writer", "partitions": ["P0", "P1", "control"] } }),
        );
        let (what, _) = stops(&findings("om", true, Some(&running), &wanted, &held));
        assert_eq!(
            what,
            "bor.partitions would go from 2 to 1, and what P1 holds could not be read: the \
             book's probe did not say"
        );
    }

    #[test]
    fn fewer_databases_is_refused_naming_the_partitions_in_a_database_that_would_go() {
        let running = runs(
            &[("MERIDIAN_BOR_PARTITIONS", "1")],
            &[
                ("MERIDIAN_WRITER_GROUPS", "2"),
                ("MERIDIAN_LAKE_DATABASES", "2"),
            ],
        );
        let wanted = wanted(&json!({ "lake": { "writerGroups": 2, "databases": 1 } }));
        assert!(lowers(&running, &wanted));
        let mut held = Held::default();
        held.read(
            "lake",
            &json!({
                "member": { "mode": "writer", "partitions": ["control", "fx:rates"] },
                "partitions": [
                    { "partition": "control", "group": 0, "database": 0 },
                    { "partition": "fx:rates", "group": 0, "database": 0 },
                    { "partition": "eq:bars", "group": 1, "database": 1 },
                ],
            }),
        );
        let (what, fix) = stops(&findings("om", true, Some(&running), &wanted, &held));
        assert_eq!(
            what,
            "lake.databases would go from 2 to 1, and partitions still hold data in a \
             database past the new count: eq:bars (database 1)"
        );
        assert!(
            fix.contains("meridian partition move") && fix.contains("lake.writerGroups"),
            "{fix}"
        );
    }

    #[test]
    fn fewer_databases_passes_once_no_partition_is_assigned_past_the_new_count() {
        let running = runs(
            &[("MERIDIAN_BOR_PARTITIONS", "1")],
            &[("MERIDIAN_LAKE_DATABASES", "2")],
        );
        let wanted = wanted(&json!({ "lake": { "databases": 1 } }));
        let mut held = Held::default();
        held.read(
            "lake",
            &json!({
                "member": { "mode": "writer", "partitions": ["control", "eq:bars"] },
                "partitions": [
                    { "partition": "control", "group": 0, "database": 0 },
                    { "partition": "eq:bars", "group": 0, "database": 0 },
                ],
            }),
        );
        assert!(matches!(
            findings("om", true, Some(&running), &wanted, &held).as_slice(),
            [Finding::Fine(_)]
        ));
    }

    #[test]
    fn fewer_databases_without_assignments_names_what_each_group_going_writes() {
        let running = runs(
            &[
                ("MERIDIAN_BOR_PARTITIONS", "4"),
                ("MERIDIAN_WRITER_GROUPS", "2"),
                ("MERIDIAN_BOR_DATABASES", "2"),
            ],
            &[],
        );
        let wanted = wanted(&json!({ "bor": { "partitions": 4, "writerGroups": 2 } }));
        let mut held = Held::default();
        held.read(
            "bor",
            &json!({ "member": { "mode": "writer", "partitions": ["P0", "P2", "control"] } }),
        );
        held.read(
            "bor-group-1",
            &json!({ "member": { "mode": "writer", "partitions": ["P1", "P3"] } }),
        );
        // A standby's partitions are those it follows: never its group's.
        held.read(
            "bor-group-1",
            &json!({ "member": { "mode": "standby", "partitions": ["P1", "P3"] } }),
        );
        let (what, _) = stops(&findings("om", true, Some(&running), &wanted, &held));
        assert!(
            what.ends_with("past the new count: P1 (database 1), P3 (database 1)"),
            "{what}"
        );

        // A group whose writer did not answer, and nothing at all read:
        // refused, saying so.
        let mut silent = Held::default();
        silent.read(
            "bor",
            &json!({ "member": { "mode": "writer", "partitions": ["P0", "control"] } }),
        );
        let (what, _) = stops(&findings("om", true, Some(&running), &wanted, &silent));
        assert!(
            what.contains("what group 1's database 1 holds could not be read"),
            "{what}"
        );
        let one_group = runs(
            &[
                ("MERIDIAN_BOR_PARTITIONS", "1"),
                ("MERIDIAN_BOR_DATABASES", "2"),
            ],
            &[],
        );
        let (what, _) = stops(&findings("om", true, Some(&one_group), &wanted, &silent));
        assert!(
            what.contains("what database 1 holds could not be read"),
            "{what}"
        );
    }

    #[test]
    fn what_runs_unread_stops_a_count_above_one_and_is_said_otherwise() {
        let above = wanted(&json!({ "lake": { "writerGroups": 2 } }));
        assert!(matches!(
            findings("om", true, None, &above, &Held::default()).as_slice(),
            [Finding::Stops { .. }]
        ));
        let ones = wanted(&json!({}));
        assert!(matches!(
            findings("om", true, None, &ones, &Held::default()).as_slice(),
            [Finding::Unknown { .. }]
        ));
    }
}

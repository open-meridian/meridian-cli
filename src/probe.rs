//! What a member's probe says (meridian-core's `meridian-<component> probe`,
//! step 3b): one JSON object, read from inside the member's pod, which
//! `status`, `doctor` and `upgrade-deployment` read rather than asking the
//! database themselves. It names each path's address as `host:port` only,
//! never a credential; nothing here prints more of it than that.
//!
//! The database's path carries what integrity at rest asks of it
//! (`data_checksums`, `full_page_writes`), whether it is a primary, the WAL
//! archiver's last segment and whether it is failing (`archive`), and the
//! store's last scrub with its unanswered findings, the replica
//! comparison's among them (`integrity`).

use serde_json::Value;

use crate::doctor::Finding;

/// The components that answer `probe`, each from its own binary in its own
/// container of the same name.
pub const PROBED: [&str; 6] = [
    "conductor",
    "lake",
    "bor",
    "instrument",
    "street",
    "launcher",
];

/// What a member's container carries when its runtime answers `probe`: the
/// scrub's interval, which came with integrity at rest, after the probe. An
/// older runtime takes `probe` for no argument at all and starts a second
/// copy of the component, so it is never asked of one.
pub const MARKER: &str = "MERIDIAN_SCRUB_INTERVAL_DAYS";

/// Whether a pod, as `kubectl get pods -o json` lists it, runs a runtime
/// that answers `probe`: a container of it carries the marker.
pub fn answers(pod: &Value) -> bool {
    pod["spec"]["containers"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|container| container["env"].as_array().into_iter().flatten())
        .any(|env| env["name"] == MARKER)
}

/// How long a probe is given in its pod before it is stopped there: its own
/// paths are bounded at a few seconds each.
pub const WITHIN_SECONDS: &str = "30";

/// `kubectl exec` of one member's probe, as its arguments: bounded in the
/// pod by `timeout`, so nothing it starts outlives the read.
pub fn exec_arguments<'a>(namespace: &'a str, pod: &'a str, component: &'a str) -> Vec<String> {
    vec![
        "exec".into(),
        "-n".into(),
        namespace.into(),
        pod.into(),
        "-c".into(),
        component.into(),
        "--".into(),
        "timeout".into(),
        WITHIN_SECONDS.into(),
        format!("meridian-{component}"),
        "probe".into(),
    ]
}

/// The first JSON object in what a probe printed. It exits 1 when a path did
/// not answer, and its object is printed all the same, with its log after
/// it, so it is read from wherever it starts.
pub fn first_json(said: &str) -> Option<Value> {
    let from = said.find('{')?;
    serde_json::Deserializer::from_str(&said[from..])
        .into_iter::<Value>()
        .next()?
        .ok()
        .filter(Value::is_object)
}

/// The database's path in a probe's answer, where the member has one.
pub fn database(probe: &Value) -> Option<&Value> {
    probe["paths"]
        .as_array()?
        .iter()
        .find(|path| path["path"] == "database")
}

/// One of the database's settings as the probe read it: `on`, `off`, or
/// None where it was not read.
pub fn setting<'a>(database: &'a Value, name: &str) -> Option<&'a str> {
    database[name].as_str().filter(|said| !said.is_empty())
}

/// What `doctor` and `status` say of the database's page checksums and
/// full-page writes, plainly, when either is off: nothing when both are on
/// or neither was read. `brought` is the database this deployment brought,
/// whose checksums the guided step turns on.
pub fn integrity_hints(database: &Value, brought: bool, namespace: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    if setting(database, "data_checksums") == Some("off") {
        found.push(if brought {
            Finding::Worth {
                what: "the database this deployment brought has no page checksums \
                       (data_checksums is off): it was made before they were turned on, so a \
                       page corrupted on its disk is not caught when it is read"
                    .into(),
                why: format!(
                    "`meridian upgrade-deployment --enable-checksums -n {namespace}` turns them \
                     on: a backup first, then everything stopped while pg_checksums reads every \
                     page, the time it takes said before it asks."
                ),
            }
        } else {
            Finding::Worth {
                what: "the database has no page checksums (data_checksums is off): a page \
                       corrupted on its disk is not caught when it is read"
                    .into(),
                why: "They are set when a database is made (initdb --data-checksums), or turned \
                      on with PostgreSQL's pg_checksums --enable on the stopped server; on a \
                      managed database, its provider's setting."
                    .into(),
            }
        });
    }
    if setting(database, "full_page_writes") == Some("off") {
        found.push(Finding::Worth {
            what: "the database's full_page_writes is off: a page torn by a crash mid-write \
                   cannot be repaired from the write-ahead log"
                .into(),
            why: if brought {
                "The database this deployment brought sets it on; something has set it off \
                 since. Set full_page_writes = on, which takes a reload and no restart."
                    .into()
            } else {
                "Set full_page_writes = on, which takes a reload and no restart; on a managed \
                 database, its provider's setting."
                    .into()
            },
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_probe_that_exited_one_is_read_from_where_its_object_starts() {
        let said = "{\"component\":\"lake\",\"paths\":[{\"path\":\"database\",\"address\":\"db:5432\",\
                    \"ok\":false,\"why\":\"refused\"}],\"ok\":false}\n2026-11-08T10:00:00Z ERROR lake: \
                    a path it needs did not answer";
        let probe = first_json(said).expect("its object");
        assert_eq!(probe["component"], "lake");
        assert_eq!(database(&probe).unwrap()["address"], "db:5432");
        assert_eq!(first_json("Error from server: pods not found"), None);
    }

    #[test]
    fn settings_off_are_said_plainly_and_on_says_nothing() {
        let on = json!({ "data_checksums": "on", "full_page_writes": "on" });
        assert!(integrity_hints(&on, true, "meridian").is_empty());
        assert!(integrity_hints(&json!({}), false, "meridian").is_empty());

        let off = json!({ "data_checksums": "off", "full_page_writes": "off" });
        let brought: Vec<String> = integrity_hints(&off, true, "om")
            .iter()
            .map(|f| f.to_string())
            .collect();
        assert_eq!(brought.len(), 2);
        assert!(
            brought[0].contains("--enable-checksums -n om"),
            "{brought:?}"
        );
        assert!(
            brought[1].contains("full_page_writes is off"),
            "{brought:?}"
        );

        let external: Vec<String> = integrity_hints(&off, false, "om")
            .iter()
            .map(|f| f.to_string())
            .collect();
        assert!(!external[0].contains("--enable-checksums"), "{external:?}");
        assert!(external[0].contains("provider"), "{external:?}");
    }
}

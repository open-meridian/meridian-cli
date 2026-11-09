//! A cluster that acts as one: scaling changes what runs, the offline Job
//! does its act, the probe reads the database as it now is. Every command
//! asked is kept, so a test reads what was done and in what order, and what
//! a refusal did not do.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::doctor::Answered;
use crate::upgrade_deployment::Asked;

pub const NS: &str = "meridian";
pub const SET: &str = "meridian-meridian-runtime-database";

#[derive(Debug, Clone)]
pub struct State {
    pub deployments: BTreeMap<String, u64>,
    pub database: u64,
    pub patroni: bool,
    pub archive: bool,
    pub primary: usize,
    pub checksums: bool,
    pub claims: BTreeSet<String>,
    pub configmaps: BTreeSet<String>,
    pub backup_configured: bool,
    pub backup_fails: bool,
    /// The act's own failure, as its script says it.
    pub act_fails: Option<String>,
    pub cron: bool,
    pub probe: bool,
    pub synchronous: bool,
    /// Jobs made, by name: the act they were made for, if one.
    pub jobs: BTreeMap<String, Option<String>>,
    /// The time the restore was told, as its Job's environment named it.
    pub restored_to: Option<String>,
    /// What a restore recorded it stopped, on the database's StatefulSet.
    pub stopped_record: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            deployments: [
                ("meridian-meridian-runtime-conductor", 1),
                ("meridian-meridian-runtime-lake", 1),
                ("meridian-meridian-runtime-dashboard", 2),
            ]
            .into_iter()
            .map(|(name, n)| (name.to_string(), n))
            .collect(),
            database: 1,
            patroni: false,
            archive: false,
            primary: 0,
            checksums: false,
            claims: [format!("data-{SET}-0")].into(),
            configmaps: BTreeSet::new(),
            backup_configured: true,
            backup_fails: false,
            act_fails: None,
            cron: true,
            probe: true,
            synchronous: true,
            jobs: BTreeMap::new(),
            restored_to: None,
            stopped_record: None,
        }
    }
}

impl State {
    /// Under Patroni with two standbys, member 1 the primary.
    pub fn ha(mut self) -> Self {
        self.patroni = true;
        self.database = 3;
        self.primary = 1;
        self.claims = (0..3).map(|n| format!("data-{SET}-{n}")).collect();
        self.configmaps = ["leader", "sync", "config"]
            .iter()
            .map(|kind| format!("{SET}-{kind}"))
            .collect();
        self
    }
}

#[derive(Default)]
pub struct Sim {
    pub state: Mutex<State>,
    pub asked: Mutex<Vec<String>>,
}

impl Sim {
    pub fn new(state: State) -> Self {
        Sim {
            state: Mutex::new(state),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    pub fn now(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    /// Whether anything that changes the cluster was asked.
    pub fn changed_anything(&self) -> bool {
        self.asked().iter().any(|c| {
            c.starts_with("kubectl scale")
                || c.starts_with("kubectl delete")
                || c.contains("patronictl")
                || c.contains("pg_switch_wal")
                || (c.starts_with("kubectl create") && !c.contains("backup"))
        })
    }

    pub fn pods(state: &State, only: Option<&str>) -> Value {
        let mut items = Vec::new();
        for (name, replicas) in &state.deployments {
            let component = name.rsplit('-').next().unwrap_or_default();
            if only.is_some_and(|c| c != component) {
                continue;
            }
            for n in 0..*replicas {
                items.push(json!({
                    "metadata": { "name": format!("{name}-{n}"),
                        "labels": { "app.kubernetes.io/instance": "meridian",
                                    "meridian.dev/component": component },
                        "ownerReferences": [ { "kind": "ReplicaSet", "name": format!("{name}-rs") } ] },
                    "spec": { "containers": [ { "name": component, "image": "runtime:1",
                        "env": [ { "name": crate::probe::MARKER, "value": "7" } ] } ] },
                    "status": { "phase": "Running", "conditions": [
                        { "type": "Ready", "status": "True", "lastTransitionTime": "2026-11-08T10:00:00Z" } ] }
                }));
            }
        }
        if only.is_none_or(|c| c == "database") {
            for n in 0..state.database as usize {
                let mut labels = json!({ "app.kubernetes.io/instance": "meridian",
                                          "meridian.dev/component": "database" });
                if state.patroni {
                    labels["meridian.dev/database-role"] = json!(if n == state.primary {
                        "primary"
                    } else {
                        "replica"
                    });
                }
                items.push(json!({
                    "metadata": { "name": format!("{SET}-{n}"), "labels": labels },
                    "spec": { "nodeName": "node-1" },
                    "status": { "phase": "Running", "conditions": [ { "type": "Ready", "status": "True" } ] }
                }));
            }
        }
        json!({ "items": items })
    }

    pub fn workloads(state: &State, sets: bool) -> Value {
        let mut items: Vec<Value> = state
            .deployments
            .iter()
            .map(|(name, replicas)| {
                let component = name.rsplit('-').next().unwrap_or_default();
                json!({
                    "kind": "Deployment",
                    "metadata": { "name": name, "generation": 1,
                                  "labels": { "meridian.dev/component": component } },
                    "spec": { "replicas": replicas,
                        "selector": { "matchLabels": { "app.kubernetes.io/instance": "meridian",
                                                       "meridian.dev/component": component } },
                        "template": { "spec": { "containers": [ { "name": component, "image": "runtime:1" } ] } } },
                    "status": { "observedGeneration": 1, "replicas": replicas, "updatedReplicas": replicas,
                                "readyReplicas": replicas, "availableReplicas": replicas }
                })
            })
            .collect();
        if sets {
            let mut containers =
                vec![json!({ "name": "postgres", "image": "meridian-postgres:1" })];
            if state.archive {
                containers.push(json!({ "name": "archive", "image": "meridian-postgres:1" }));
            }
            let mut labels = json!({});
            if state.patroni {
                labels["meridian.dev/database-manager"] = json!("patroni");
            }
            items.push(json!({
                "kind": "StatefulSet",
                "metadata": { "name": SET, "generation": 1 },
                "spec": { "replicas": state.database,
                    "selector": { "matchLabels": { "app.kubernetes.io/instance": "meridian",
                                                   "meridian.dev/component": "database" } },
                    "template": { "metadata": { "labels": labels },
                                  "spec": { "containers": containers } } },
                "status": { "observedGeneration": 1, "replicas": state.database,
                            "updatedReplicas": state.database, "readyReplicas": state.database,
                            "currentRevision": "a", "updateRevision": "a" }
            }));
        }
        json!({ "items": items })
    }

    pub fn cronjob(name: &str) -> Value {
        json!({
            "metadata": { "name": name, "namespace": NS, "uid": "u-1" },
            "spec": { "jobTemplate": {
                "metadata": { "labels": { "meridian.dev/component": name } },
                "spec": { "template": { "spec": { "containers": [ {
                    "name": "offline", "image": "meridian-postgres:1",
                    "env": [ { "name": "MERIDIAN_DATABASE_ACT", "value": "" },
                             { "name": "MERIDIAN_RESTORE_TO_TIME", "value": "" },
                             { "name": "PGDATA", "value": PGDATA } ] } ] } } } } }
        })
    }
}

#[async_trait::async_trait]
impl Machine for Sim {
    async fn run(&self, program: &str, arguments: &[&str]) -> Result<String, Failure> {
        let command = format!("{program} {}", arguments.join(" "));
        self.asked.lock().unwrap().push(command.clone());
        let mut state = self.state.lock().unwrap();
        let said = |text: String| Ok(text);
        let refused = |text: &str| Err(Failure::Said(text.to_string()));
        let sel = "app.kubernetes.io/instance=meridian";
        if command.starts_with("helm list") {
            return said(
                json!([{ "name": "meridian", "revision": "9", "status": "deployed",
                         "chart": "meridian-runtime-0.1.469", "app_version": "515969a" }])
                .to_string(),
            );
        }
        if command == format!("kubectl get deployments,statefulsets -n {NS} -l {sel} -o json")
            || command
                == format!(
                    "kubectl get deployments,statefulsets,replicasets -n {NS} -l {sel} -o json"
                )
        {
            return said(Sim::workloads(&state, true).to_string());
        }
        if command == format!("kubectl get deployments -n {NS} -l {sel} -o json") {
            return said(Sim::workloads(&state, false).to_string());
        }
        if command == format!("kubectl get pods -n {NS} -l {sel} -o json") {
            return said(Sim::pods(&state, None).to_string());
        }
        if let Some(component) = command
            .strip_prefix(&format!(
                "kubectl get pods -n {NS} -l {sel},meridian.dev/component="
            ))
            .and_then(|rest| rest.strip_suffix(" -o json"))
        {
            return said(Sim::pods(&state, Some(component)).to_string());
        }
        if command.contains("meridian-conductor probe") {
            if !state.probe {
                return refused("error: unable to upgrade connection");
            }
            let on = if state.checksums { "on" } else { "off" };
            let mut database = json!({ "path": "database", "address": "db:5432", "ok": true,
                                       "data_checksums": on, "full_page_writes": "on", "primary": true });
            if state.archive {
                database["archive"] = json!({ "last_archived_at": "2026-11-08T10:00:00Z",
                                              "seconds_since": 12, "failed": 0, "failing": false });
            }
            return said(
                json!({ "component": "conductor", "member": { "ready": true },
                        "paths": [ database ], "ok": true })
                .to_string(),
            );
        }
        if command.contains("-- du -sk") {
            return said(format!("2097152\t{PGDATA}\n"));
        }
        if command.contains("patronictl") && command.contains("edit-config") {
            state.synchronous = false;
            return said("Configuration changed".into());
        }
        if command.contains("patronictl") && command.contains("switchover") {
            if state.synchronous {
                return refused("Error: candidate name does not match with sync_standby");
            }
            state.primary = 0;
            return said("Successfully switched over".into());
        }
        if command.contains("pg_switch_wal") || command.contains("last_archived_wal") {
            return said("000000010000000000000007\n".into());
        }
        if command == format!("kubectl get cronjob {SET}-offline -n {NS} -o json") {
            return if state.cron {
                said(Sim::cronjob(&format!("{SET}-offline")).to_string())
            } else {
                refused("Error from server (NotFound): cronjobs.batch not found")
            };
        }
        let backup_selector = format!("{sel},meridian.dev/component=backup");
        if command == format!("kubectl get cronjob -n {NS} -l {backup_selector} -o name") {
            return said(if state.backup_configured {
                "cronjob.batch/meridian-meridian-runtime-backup\n".into()
            } else {
                String::new()
            });
        }
        if command == format!("kubectl get cronjob -n {NS} -l {backup_selector} -o json") {
            return said(if state.backup_configured {
                json!({ "items": [ Sim::cronjob("meridian-meridian-runtime-backup") ] }).to_string()
            } else {
                json!({ "items": [] }).to_string()
            });
        }
        if command.starts_with("kubectl get pvc") {
            return said(json!({ "items": [] }).to_string());
        }
        if let Some(job) = command
            .strip_prefix(&format!("kubectl logs -n {NS} job/"))
            .map(|rest| rest.trim_end_matches(" --tail=-1"))
        {
            return match state.jobs.get(job) {
                Some(None) if state.backup_fails => {
                    said("backup: pg_dump: connection refused\n".into())
                }
                Some(None) => said(
                    "backup: dumped the database\nbackup: wrote \
                     meridian-20261108T100000Z.meridian-backup 4096 bytes sha256 ab12\n"
                        .into(),
                ),
                Some(Some(act)) => {
                    let mut log = format!("database-offline: the act {act}\n");
                    match &state.act_fails {
                        Some(why) => log.push_str(&format!("database-offline: FAILED: {why}\n")),
                        None => log.push_str("database-offline: done in 3s\n"),
                    }
                    said(log)
                }
                None => refused("Error from server (NotFound): jobs.batch not found"),
            };
        }
        if command.contains("-o jsonpath={.status.failed}") {
            return said(if state.backup_fails {
                "1".into()
            } else {
                String::new()
            });
        }
        if let Some(job) = command
            .strip_prefix("kubectl get job ")
            .and_then(|rest| rest.strip_suffix(&format!(" -n {NS} -o json")))
        {
            let condition = match state.jobs.get(job).cloned() {
                Some(Some(act)) if state.act_fails.is_none() => {
                    if act == "enable-checksums" {
                        state.checksums = true;
                    }
                    "Complete"
                }
                Some(Some(_)) => "Failed",
                _ => "Complete",
            };
            return said(
                json!({ "status": { "conditions": [ { "type": condition, "status": "True" } ] } })
                    .to_string(),
            );
        }
        if let Some(rest) =
            command.strip_prefix(&format!("kubectl scale deployment -n {NS} --replicas="))
        {
            let mut words = rest.split_whitespace();
            let count: u64 = words.next().unwrap().parse().unwrap();
            for name in words {
                state.deployments.insert(name.to_string(), count);
            }
            return said("scaled".into());
        }
        if let Some(rest) = command.strip_prefix(&format!(
            "kubectl scale statefulset {SET} -n {NS} --replicas="
        )) {
            state.database = rest.trim().parse().unwrap();
            if state.database > 0 && state.primary as u64 >= state.database {
                state.primary = 0;
            }
            for n in 0..state.database {
                state.claims.insert(format!("data-{SET}-{n}"));
            }
            return said("scaled".into());
        }
        let record = format!("kubectl annotate statefulset {SET} -n {NS} --overwrite {STOPPED}=");
        if let Some(said_json) = command.strip_prefix(&record) {
            state.stopped_record = Some(said_json.to_string());
            return said("annotated".into());
        }
        if command == format!("kubectl annotate statefulset {SET} -n {NS} {STOPPED}-") {
            state.stopped_record = None;
            return said("annotated".into());
        }
        if command.starts_with(&format!(
            "kubectl get statefulset {SET} -n {NS} -o jsonpath="
        )) {
            return said(state.stopped_record.clone().unwrap_or_default());
        }
        if let Some(rest) =
            command.strip_prefix(&format!("kubectl delete pvc -n {NS} --ignore-not-found "))
        {
            for claim in rest.split_whitespace() {
                state.claims.remove(claim);
            }
            return said("deleted".into());
        }
        if let Some(rest) = command.strip_prefix(&format!(
            "kubectl delete configmap -n {NS} --ignore-not-found "
        )) {
            for name in rest.split_whitespace() {
                state.configmaps.remove(name);
            }
            return said("deleted".into());
        }
        refused(&format!("not stood in for: {command}"))
    }

    async fn run_with_input(
        &self,
        program: &str,
        arguments: &[&str],
        input: &str,
    ) -> Result<String, Failure> {
        let document: Value = serde_json::from_str(input).unwrap_or_default();
        let name = document["metadata"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let env = document["spec"]["template"]["spec"]["containers"][0]["env"].clone();
        let named = |key: &str| {
            env.as_array()
                .into_iter()
                .flatten()
                .find(|e| e["name"] == key)
                .and_then(|e| e["value"].as_str())
                .filter(|value| !value.is_empty())
                .map(String::from)
        };
        let act = named("MERIDIAN_DATABASE_ACT");
        let to = named("MERIDIAN_RESTORE_TO_TIME");
        self.asked.lock().unwrap().push(format!(
            "{program} {} <<{} {name} {}",
            arguments.join(" "),
            document["kind"].as_str().unwrap_or_default(),
            act.clone().unwrap_or_default()
        ));
        let mut state = self.state.lock().unwrap();
        if to.is_some() {
            state.restored_to = to;
        }
        state.jobs.insert(name, act);
        Ok(String::new())
    }

    async fn fetch(&self, url: &str, _: &[(&str, &str)]) -> Result<Answered, String> {
        Err(format!("nothing here fetches, and {url} was asked"))
    }

    fn now_s(&self) -> u64 {
        1_794_000_000
    }
}

pub fn asked() -> Asked {
    Asked {
        release: "meridian".into(),
        namespace: NS.into(),
        chart: String::new(),
        chart_version: None,
        timeout: "10m".into(),
        https: false,
        authority: None,
        values: Vec::new(),
        sets: Vec::new(),
    }
}

pub fn pace() -> Pace {
    Pace {
        poll: Duration::ZERO,
        timeout: Duration::from_secs(5),
    }
}

#[test]
fn the_offline_job_names_its_act_and_keeps_the_rest_of_the_cronjobs_template() {
    let cronjob = Sim::cronjob(&format!("{SET}-offline"));
    let job = offline_job(
        &cronjob,
        "j-1",
        &Act::RestoreToTime("2026-11-08 10:00:00+00".into()),
    )
    .unwrap();
    let env = job["spec"]["template"]["spec"]["containers"][0]["env"]
        .as_array()
        .unwrap()
        .clone();
    let named = |key: &str| {
        env.iter()
            .filter(|e| e["name"] == key)
            .map(|e| e["value"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(named("MERIDIAN_DATABASE_ACT"), ["restore-to-time"]);
    assert_eq!(
        named("MERIDIAN_RESTORE_TO_TIME"),
        ["2026-11-08 10:00:00+00"]
    );
    assert_eq!(named("PGDATA"), [PGDATA]);
    assert_eq!(
        job["metadata"]["ownerReferences"][0]["name"],
        format!("{SET}-offline")
    );
    assert_eq!(
        job["metadata"]["labels"]["meridian.dev/database-act"],
        "restore-to-time"
    );
}

#[test]
fn members_are_the_sets_pods_in_order_their_roles_said_as_standby_or_primary() {
    let state = State::default().ha();
    let found = members(&Sim::pods(&state, Some("database")), SET);
    let said: Vec<(String, Option<String>)> = found
        .iter()
        .map(|m| (m.name.clone(), m.role.clone()))
        .collect();
    assert_eq!(
        said,
        [
            (format!("{SET}-0"), Some("standby".into())),
            (format!("{SET}-1"), Some("primary".into())),
            (format!("{SET}-2"), Some("standby".into())),
        ]
    );
}

#[test]
fn the_downtime_grows_with_the_size_and_each_standby_made_again() {
    let small = downtime(10 * 1024 * 1024, 0);
    let large = downtime(50 * 1024 * 1024 * 600, 0);
    let ha = downtime(50 * 1024 * 1024 * 600, 2);
    assert!(small < large && large < ha, "{small:?} {large:?} {ha:?}");
    assert_eq!(minutes(Duration::from_secs(30)), "about a minute");
    assert_eq!(
        minutes(Duration::from_secs(14 * 60 + 1)),
        "about 15 minutes"
    );
}

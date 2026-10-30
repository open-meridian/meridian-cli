use super::*;
use std::io::Write;

const MANIFEST: &str = r#"{
  "format": 1,
  "release": "meridian",
  "namespace": "meridian",
  "chart_version": "0.1.300",
  "app_version": "abc1234",
  "deployment_id": "DEP-X",
  "taken_at": "20261011T020000Z",
  "key": "generated",
  "database": { "included": true, "brought": true, "name": "meridian", "secret": "meridian-meridian-runtime-database" },
  "chart_secrets": ["meridian-meridian-runtime-settings-key", "meridian-meridian-runtime-database"],
  "operator_secrets": ["meridian-tls"],
  "config_maps": ["meridian-meridian-runtime-dashboard-keys"]
}"#;

/// A backup as the chart's script makes one: members, their checksums, a tar
/// with `./` paths, sealed to `recipient`.
fn sealed(members: &[(&str, &[u8])], recipient: &str, tamper: Option<&str>) -> Vec<u8> {
    let mut checksums = String::new();
    for (path, bytes) in members {
        checksums.push_str(&format!("{}  {path}\n", hex(&Sha256::digest(bytes))));
    }
    let mut builder = tar::Builder::new(Vec::new());
    let mut add = |path: &str, bytes: &[u8]| {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        builder
            .append_data(&mut header, format!("./{path}"), bytes)
            .unwrap();
    };
    for (path, bytes) in members {
        let bytes: &[u8] = if Some(*path) == tamper {
            b"changed"
        } else {
            bytes
        };
        add(path, bytes);
    }
    add("checksums.sha256", checksums.as_bytes());
    let plain = builder.into_inner().unwrap();
    let recipient: age::x25519::Recipient = recipient.parse().unwrap();
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
            .unwrap();
    let mut out = Vec::new();
    let mut writer = encryptor.wrap_output(&mut out).unwrap();
    writer.write_all(&plain).unwrap();
    writer.finish().unwrap();
    out
}

fn members() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("manifest.json", MANIFEST.as_bytes()),
        ("values/values.yaml", b"deployment:\n  id: DEP-X\n"),
        (
            "secrets/meridian-meridian-runtime-settings-key/key",
            b"thirty-two bytes, as a key is...",
        ),
        ("secrets/meridian-tls/tls.crt", b"crt"),
        ("secrets/meridian-tls/tls.key", b"key"),
        ("key/key.pem", b"-----BEGIN PRIVATE KEY-----"),
        (
            "database/counts.tsv",
            b"public.book_entry\t3\npublic.config_account\t2\n",
        ),
    ]
}

#[test]
fn a_backup_opens_with_the_identity_it_was_sealed_to_and_every_member_is_checked() {
    let (identity, recipient) = new_identity("2026-10-11T00:00:00Z");
    assert!(recipient.starts_with("age1"), "{recipient}");
    assert!(identity.contains(&format!("# public key: {recipient}")));
    assert!(identity.contains("AGE-SECRET-KEY-1"));

    let opened = open(&sealed(&members(), &recipient, None), &identity).unwrap();
    assert_eq!(opened.manifest.release, "meridian");
    assert_eq!(opened.manifest.key, "generated");
    assert!(opened.manifest.database.included && opened.manifest.database.brought);
    assert_eq!(opened.manifest.operator_secrets, vec!["meridian-tls"]);
    let tls = opened.object("secrets", "meridian-tls");
    assert_eq!(tls.keys().collect::<Vec<_>>(), vec!["tls.crt", "tls.key"]);
    assert_eq!(
        opened.member("key/key.pem").unwrap(),
        b"-----BEGIN PRIVATE KEY-----"
    );
    assert!(opened.member("database/database.dump").is_err());
}

#[test]
fn another_identity_does_not_open_it_and_says_so() {
    let (_, recipient) = new_identity("now");
    let (other, _) = new_identity("now");
    let refused = open(&sealed(&members(), &recipient, None), &other)
        .err()
        .unwrap();
    assert!(refused.contains("sealed to another"), "{refused}");
    let refused = open(b"not age at all", &other).err().unwrap();
    assert!(refused.contains("not a backup"), "{refused}");
}

#[test]
fn a_member_that_is_not_what_was_written_is_refused() {
    let (identity, recipient) = new_identity("now");
    let refused = open(
        &sealed(&members(), &recipient, Some("key/key.pem")),
        &identity,
    )
    .err()
    .unwrap();
    assert!(
        refused.contains("key/key.pem is not what the backup wrote"),
        "{refused}"
    );
}

#[test]
fn a_member_checksums_does_not_list_is_refused() {
    let mut held = BTreeMap::new();
    held.insert("checksums.sha256".to_string(), b"".to_vec());
    held.insert("extra".to_string(), b"x".to_vec());
    assert!(verify(&held).unwrap_err().contains("does not list"));
}

#[test]
fn a_member_outside_the_backup_is_refused() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(1);
    header.set_cksum();
    // tar::Builder refuses `..` itself, so the name is written raw.
    header.as_gnu_mut().unwrap().name[..12].copy_from_slice(b"../escape.tx");
    header.set_cksum();
    builder.append(&header, &b"x"[..]).unwrap();
    let plain = builder.into_inner().unwrap();
    assert!(untar(&plain).unwrap_err().contains("outside itself"));
}

#[test]
fn only_what_differs_from_the_charts_defaults_is_given_to_helm() {
    let values: serde_yaml::Value = serde_yaml::from_str(
        "image: {repository: r, tag: abc}\ndeployment: {id: DEP-X, enrolmentCodeKey: code}\n\
         ingress: {enabled: true, host: meridian.localhost, tls: {secretName: meridian-tls}}\n\
         sidecars: []\nbackup: {recipient: age1x, keep: 14}\n",
    )
    .unwrap();
    let defaults: serde_yaml::Value = serde_yaml::from_str(
        "image: {repository: r, tag: latest}\ndeployment: {id: '', enrolmentCodeKey: code}\n\
         ingress: {enabled: false, host: '', tls: {secretName: ''}}\nsidecars: []\n\
         backup: {recipient: '', keep: 14}\n",
    )
    .unwrap();
    let given: serde_yaml::Value = serde_yaml::from_str(
        "image: {tag: abc}\ndeployment: {id: DEP-X}\n\
         ingress: {enabled: true, host: meridian.localhost, tls: {secretName: meridian-tls}}\n\
         backup: {recipient: age1x}\n",
    )
    .unwrap();
    assert_eq!(chosen(&values, &defaults), given);
}

#[test]
fn the_full_name_is_the_charts() {
    let none = serde_yaml::from_str("{}").unwrap();
    assert_eq!(fullname("meridian", &none), "meridian-meridian-runtime");
    let named = serde_yaml::from_str("nameOverride: rt").unwrap();
    assert_eq!(fullname("trial", &named), "trial-rt");
    let full = serde_yaml::from_str("fullnameOverride: firm").unwrap();
    assert_eq!(fullname("trial", &full), "firm");
}

#[test]
fn a_login_is_read_from_first_runs_url() {
    let login = login(
        "postgres://meridian_app:p%40ss%3Aw@db.ns.svc.cluster.local:5432/meridian?sslmode=disable",
    )
    .unwrap();
    assert_eq!(
        login,
        Login {
            role: "meridian_app".into(),
            password: "p@ss:w".into(),
            database: "meridian".into()
        }
    );
    assert!(super::login("postgres://Robert'); drop table x;--:p@h/db").is_err());
    assert!(super::login("no scheme").is_err());
}

#[test]
fn the_database_and_its_roles_are_made_as_first_run_makes_them() {
    let serving = Login {
        role: "meridian_app".into(),
        password: "it's".into(),
        database: "meridian".into(),
    };
    let migrating = Login {
        role: "meridian_migrate".into(),
        password: "m".into(),
        database: "meridian".into(),
    };
    let sql = provision_sql(&serving, &migrating).unwrap();
    assert!(sql.contains("alter role meridian_app with login password 'it''s';"));
    assert!(sql.contains("create database meridian"));
    assert!(sql.contains("\\connect meridian"));
    assert!(sql.contains("grant create on schema public to meridian_migrate;"));
    assert!(sql.contains("revoke create on schema public from meridian_app, public;"));
}

#[test]
fn counts_that_differ_are_named() {
    let backed_up = counts("public.a\t3\npublic.b\t0\npublic.gone\t1\n");
    let restored = counts("public.a\t2\npublic.b\t0\npublic.new\t5\n");
    assert_eq!(
        differences(&backed_up, &restored),
        vec![
            "public.a: 3 backed up, 2 restored",
            "public.gone: 1 backed up, and no such table restored",
            "public.new: restored, and not in the backup",
        ]
    );
    assert!(differences(&backed_up, &backed_up).is_empty());
}

#[test]
fn a_kept_secret_is_made_for_helm_to_adopt_and_an_administrators_is_not() {
    let data = BTreeMap::from([("key".to_string(), b"k".to_vec())]);
    let kept = secret_object("s", "ns", &data, Some("meridian"));
    assert_eq!(
        kept["metadata"]["labels"]["app.kubernetes.io/managed-by"],
        "Helm"
    );
    assert_eq!(
        kept["metadata"]["annotations"]["meta.helm.sh/release-name"],
        "meridian"
    );
    assert_eq!(
        kept["metadata"]["annotations"]["meta.helm.sh/release-namespace"],
        "ns"
    );
    assert_eq!(kept["data"]["key"], "aw==");
    assert_eq!(kept["type"], "Opaque");
    let tls = BTreeMap::from([
        ("tls.crt".to_string(), b"c".to_vec()),
        ("tls.key".to_string(), b"k".to_vec()),
    ]);
    let theirs = secret_object("t", "ns", &tls, None);
    assert_eq!(theirs["type"], "kubernetes.io/tls");
    assert!(theirs["metadata"]["labels"].is_null());
}

#[test]
fn a_job_from_the_cronjob_holds_its_file_only_when_asked() {
    let cronjob = serde_json::json!({
        "metadata": { "name": "m-meridian-runtime-backup", "namespace": "ns", "uid": "u-1" },
        "spec": { "jobTemplate": {
            "metadata": { "labels": { "meridian.dev/component": "backup" } },
            "spec": { "template": { "spec": { "containers": [ { "name": "backup", "env": [] } ] } } }
        } }
    });
    let plain = job_from(&cronjob, "m-meridian-runtime-backup-1", false, &[]).unwrap();
    assert_eq!(plain["metadata"]["ownerReferences"][0]["uid"], "u-1");
    assert_eq!(
        plain["spec"]["template"]["spec"]["containers"][0]["env"],
        serde_json::json!([])
    );
    let held = job_from(&cronjob, "m-meridian-runtime-backup-1", true, &[]).unwrap();
    assert_eq!(
        held["spec"]["template"]["spec"]["containers"][0]["env"][0]["name"],
        "MERIDIAN_BACKUP_HOLD"
    );
}

#[test]
fn first_runs_job_is_found_with_what_the_chart_names_for_it() {
    let jobs = serde_json::json!({ "items": [
        { "metadata": { "name": "m-meridian-runtime-migrate-1" },
          "spec": { "template": {
            "metadata": { "labels": {} },
            "spec": { "containers": [ { "name": "instrument" } ] } } } },
        { "metadata": { "name": "m-meridian-runtime-first-run-1" },
          "spec": { "template": {
            "metadata": { "labels": { "meridian.dev/component": "first-run" } },
            "spec": { "containers": [ { "name": "first-run", "env": [
                { "name": "MERIDIAN_FIRST_RUN_BINDING", "value": "m-meridian-runtime-first-run-grant" },
                { "name": "MERIDIAN_FIRST_RUN_RESTART",
                  "value": "m-meridian-runtime-conductor, m-meridian-runtime-dashboard" },
                { "name": "MERIDIAN_BROKER_URL", "valueFrom": { "secretKeyRef": { "name": "b", "key": "k" } } }
            ] } ] } } } }
    ] });
    assert_eq!(
        first_run_job(&jobs),
        Some(FirstRunJob {
            name: "m-meridian-runtime-first-run-1".into(),
            binding: "m-meridian-runtime-first-run-grant".into(),
            restart: vec![
                "m-meridian-runtime-conductor".into(),
                "m-meridian-runtime-dashboard".into()
            ],
        })
    );
    assert_eq!(first_run_job(&serde_json::json!({ "items": [] })), None);
}

#[test]
fn what_the_job_wrote_is_read_from_its_log() {
    let log = "backup: secrets: a b\nbackup: wrote meridian-20261011T020000Z.meridian-backup 1234 bytes sha256 abcd\nbackup: holding\n";
    assert_eq!(
        wrote(log),
        Some(Wrote {
            name: "meridian-20261011T020000Z.meridian-backup".into(),
            bytes: 1234,
            sha256: "abcd".into()
        })
    );
    assert_eq!(
        wrote("backup: wrote ../x.meridian-backup 1 bytes sha256 a"),
        None
    );
    assert_eq!(wrote("nothing"), None);
}

#[test]
fn an_identity_is_never_written_over() {
    let dir = std::env::temp_dir().join(format!("meridian-backup-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("identity.txt");
    let _ = std::fs::remove_file(&path);
    let recipient = write_identity(&path, "now").unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains(&recipient));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(write_identity(&path, "now")
        .unwrap_err()
        .contains("never written over"));
    std::fs::remove_dir_all(&dir).unwrap();
}

// ── Restore's checks and plan (the gaps found 2026-10-12) ─────────────────

fn manifest_with(database: &str) -> Manifest {
    let text = MANIFEST.replace(
        r#""database": { "included": true, "brought": true, "name": "meridian", "secret": "meridian-meridian-runtime-database" }"#,
        database,
    );
    serde_json::from_str(&text).unwrap()
}

fn db_secret(url: Option<&str>, migrate: Option<&str>) -> BTreeMap<String, Vec<u8>> {
    let mut secret = BTreeMap::new();
    if let Some(url) = url {
        secret.insert("url".to_string(), url.as_bytes().to_vec());
    }
    if let Some(migrate) = migrate {
        secret.insert("migrate-url".to_string(), migrate.as_bytes().to_vec());
    }
    secret
}

const EXTERNAL: &str =
    "postgres://meridian_app:s3cr%40t@db.firm.example:5432/meridian?sslmode=require";
const EXTERNAL_MIGRATE: &str =
    "postgres://meridian_migrate:m1gr%40te@db.firm.example:5432/meridian?sslmode=require";

#[test]
fn a_databases_place_is_its_host_and_name_never_its_role_or_password() {
    assert_eq!(
        place(EXTERNAL).as_deref(),
        Some("db.firm.example:5432/meridian")
    );
    assert_eq!(place("not a url"), None);
}

#[test]
fn what_restore_does_with_the_database_is_decided_before_anything_changes() {
    let values: serde_yaml::Value =
        serde_yaml::from_str("backup:\n  image: ghcr.io/open-meridian/meridian-backup:abc\n")
            .unwrap();
    let brought = manifest_with(
        r#""database": { "included": true, "brought": true, "name": "meridian", "secret": "d" }"#,
    );
    assert_eq!(
        database_plan(&brought, &db_secret(None, None), &values).unwrap(),
        DatabasePlan::Brought {
            database: "meridian".into()
        }
    );

    // A dump of a database somebody else runs: loaded by restore itself.
    let external = manifest_with(
        r#""database": { "included": true, "brought": false, "name": "meridian", "secret": "d" }"#,
    );
    let secret = db_secret(Some(EXTERNAL), Some(EXTERNAL_MIGRATE));
    assert_eq!(
        database_plan(&external, &secret, &values).unwrap(),
        DatabasePlan::Load {
            place: "db.firm.example:5432/meridian".into(),
            database: "meridian".into(),
            image: "ghcr.io/open-meridian/meridian-backup:abc".into(),
        }
    );
    // Without backup.image, or without a URL to load with: refused first.
    let none: serde_yaml::Value = serde_yaml::from_str("deployment:\n  id: DEP-X\n").unwrap();
    assert!(database_plan(&external, &secret, &none)
        .unwrap_err()
        .contains("backup.image"));
    assert!(database_plan(&external, &db_secret(None, None), &values)
        .unwrap_err()
        .contains("holds no URL"));

    // Not in the backup, somebody else's: reconnected, and named.
    let reconnect = manifest_with(
        r#""database": { "included": false, "brought": false, "name": "meridian", "secret": "d" }"#,
    );
    assert_eq!(
        database_plan(&reconnect, &db_secret(Some(EXTERNAL), None), &values).unwrap(),
        DatabasePlan::Reconnect {
            place: "db.firm.example:5432/meridian".into()
        }
    );
    let empty = manifest_with(
        r#""database": { "included": false, "brought": true, "name": "meridian", "secret": "d" }"#,
    );
    assert_eq!(
        database_plan(&empty, &db_secret(None, None), &values).unwrap(),
        DatabasePlan::BroughtEmpty
    );
    let before = manifest_with(
        r#""database": { "included": false, "brought": false, "name": "", "secret": "d" }"#,
    );
    assert_eq!(
        database_plan(&before, &db_secret(None, None), &values).unwrap(),
        DatabasePlan::None
    );
}

#[test]
fn the_chart_installed_is_never_older_than_the_backups_and_newer_is_said() {
    let older = chart_version_said("0.1.300", "0.1.299").unwrap_err();
    assert!(older.contains("older than 0.1.300") && older.contains("nothing was changed"));
    let newer = chart_version_said("0.1.300", "0.1.302").unwrap().unwrap();
    assert!(newer.contains("an upgrade"), "{newer}");
    assert_eq!(chart_version_said("0.1.300", "0.1.300").unwrap(), None);
    assert!(chart_version_said("", "0.1.300")
        .unwrap()
        .unwrap()
        .contains("records no chart version"));
}

#[test]
fn what_a_deployment_kept_or_would_take_up_is_found_before_anything_changes() {
    let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
    let full = "meridian-meridian-runtime";
    let values: serde_yaml::Value = serde_yaml::from_str(
        "backup:\n  recipient: age1x\n  path: /Users/me/backups\nregistry:\n  enabled: false\n",
    )
    .unwrap();
    assert_eq!(
        backup_volume("firm", full, &values).as_deref(),
        Some("firm-meridian-meridian-runtime-backup")
    );
    assert_eq!(
        claims_taken(full, &values),
        ["data-meridian-meridian-runtime-database-0"]
    );
    assert!(left_behind(&manifest, full, &values, &Present::default()).is_empty());

    let present = Present {
        secrets: vec![format!("{full}-key"), "unrelated".into()],
        config_maps: vec![
            format!("{full}-backup-values"),
            format!("{full}-dashboard-keys"),
        ],
        claims: vec![
            format!("data-{full}-database-0"),
            format!("images-{full}-registry-0"),
        ],
        volume: Ok(Some("firm-meridian-meridian-runtime-backup".into())),
    };
    let found = left_behind(&manifest, full, &values, &present);
    // The backup volume is not refused: restore offers to delete it within
    // its own confirmation (Plan::delete_volume).
    assert_eq!(found.len(), 4, "{found:?}");
    assert_eq!(found[0], format!("the Secret {full}-key"));
    assert_eq!(found[1], format!("the ConfigMap {full}-dashboard-keys"));
    assert_eq!(found[2], format!("the ConfigMap {full}-backup-values"));
    assert!(found[3].starts_with(&format!("the claim data-{full}-database-0")));
    assert!(!found.iter().any(|said| said.contains("kubectl")));
    let only_the_volume = Present {
        volume: Ok(Some("firm-meridian-meridian-runtime-backup".into())),
        ..Present::default()
    };
    assert!(left_behind(&manifest, full, &values, &only_the_volume).is_empty());

    // No path, no volume to look for; a claim of the operator's names none.
    let claimed: serde_yaml::Value =
        serde_yaml::from_str("backup:\n  recipient: age1x\n  existingClaim: mine\n").unwrap();
    assert_eq!(backup_volume("firm", full, &claimed), None);
    // Unsaid, the chart's defaults bring both claims.
    let unsaid: serde_yaml::Value = serde_yaml::from_str("{}").unwrap();
    assert_eq!(claims_taken(full, &unsaid).len(), 2);
}

#[test]
fn the_counts_are_grouped_by_store_from_each_tables_prefix() {
    let counts = counts(
        "public.book_entry\t3\npublic.book_lot\t1\npublic.config_account\t2\n\
         public.holding\t5\npublic.statement\t1\npublic.schema_migration\t12\npublic.lake_price\t7\n\
         public.instrument_record\t4\npublic.dashboard_ticket\t2\npublic.mystery\t1\n",
    );
    assert_eq!(
        by_store(&counts),
        [
            ("config", 1, 2),
            ("book", 2, 4),
            ("street", 3, 18),
            ("lake", 1, 7),
            ("instrument", 1, 4),
            ("dashboard", 1, 2),
            ("other", 1, 1),
        ]
    );
}

fn launched_pod(instance: &str, waiting: Option<&str>, ready: bool) -> serde_json::Value {
    let mut state = serde_json::json!({ "running": {} });
    if let Some(reason) = waiting {
        state = serde_json::json!({ "waiting": { "reason": reason } });
    }
    serde_json::json!({
        "metadata": { "name": format!("meridian-plugin-{instance}-abc"),
                      "labels": { "meridian.dev/instance": instance, "meridian.dev/launched": "true" } },
        "spec": { "containers": [
            { "name": "sidecar", "image": "ghcr.io/open-meridian/meridian-runtime:abc" },
            { "name": "plugin", "image": format!("registry.local/plugins/{instance}@sha256:1") } ] },
        "status": {
            "phase": if ready { "Running" } else { "Pending" },
            "containerStatuses": [
                { "name": "sidecar", "restartCount": 0, "state": { "running": {} } },
                { "name": "plugin", "restartCount": 0, "state": state } ],
            "conditions": if ready {
                serde_json::json!([{ "type": "Ready", "status": "True", "lastTransitionTime": "2026-10-12T00:00:00Z" }])
            } else { serde_json::json!([]) }
        }
    })
}

#[test]
fn only_launched_plugins_whose_image_could_not_be_pulled_are_named() {
    let listed = serde_json::json!({ "items": [
        launched_pod("snaptrade-1", Some("ImagePullBackOff"), false),
        launched_pod("ops-1", None, true),
        launched_pod("tiingo-1", Some("ErrImagePull"), false),
    ]});
    let pods = crate::upgrade_deployment::pods(&listed.to_string());
    let (settled, missing) = unpulled(&pods);
    assert!(settled);
    assert_eq!(
        missing,
        [
            (
                "snaptrade-1".to_string(),
                "registry.local/plugins/snaptrade-1@sha256:1".to_string()
            ),
            (
                "tiingo-1".to_string(),
                "registry.local/plugins/tiingo-1@sha256:1".to_string()
            ),
        ]
    );
    // One still starting is waited for; one crashing is no image's fault.
    let listed = serde_json::json!({ "items": [
        launched_pod("ops-1", Some("ContainerCreating"), false),
    ]});
    let (settled, missing) = unpulled(&crate::upgrade_deployment::pods(&listed.to_string()));
    assert!(!settled && missing.is_empty());
}

fn a_plan(database: DatabasePlan) -> Plan {
    Plan {
        file: "meridian-20261011T020000Z.meridian-backup".into(),
        release: "meridian".into(),
        deployment_id: "DEP-X".into(),
        taken_at: "20261011T020000Z".into(),
        cluster: "k3d-scratch".into(),
        namespace: "firm".into(),
        namespace_exists: false,
        chart: PUBLISHED_CHART.into(),
        version: "0.1.302".into(),
        version_said: chart_version_said("0.1.300", "0.1.302").unwrap(),
        secrets: 3,
        config_maps: 1,
        key_generated: true,
        storage: Vec::new(),
        database,
        unchecked: None,
        delete_volume: None,
    }
}

#[test]
fn the_volume_a_deleted_namespace_left_is_offered_for_deletion_in_the_plan() {
    let mut plan = a_plan(DatabasePlan::None);
    assert!(!plan_text(&plan).contains("deletes the backup volume"));
    plan.delete_volume = Some("firm-meridian-meridian-runtime-backup".into());
    let text = plan_text(&plan);
    assert!(
        text.contains("First it deletes the backup volume firm-meridian-meridian-runtime-backup"),
        "{text}"
    );
    assert!(
        text.contains("the node's directory, with its backups, stays"),
        "{text}"
    );
    assert!(!text.contains("kubectl"), "{text}");
}

#[test]
fn the_plan_names_the_backup_the_cluster_the_chart_and_what_it_will_make() {
    let text = plan_text(&a_plan(DatabasePlan::Brought {
        database: "meridian".into(),
    }));
    for said in [
        "the backup  meridian (DEP-X), taken 20261011T020000Z",
        "into        the cluster k3d-scratch, namespace firm (made)",
        &format!("from        {PUBLISHED_CHART} 0.1.302"),
        "restoring is then an upgrade",
        "It will make 3 Secrets and the deployment's key, 1 ConfigMaps",
        "start the database the chart brings",
    ] {
        assert!(text.contains(said), "{said}: {text}");
    }
}

#[test]
fn an_external_database_is_named_and_whoever_else_runs_against_it_told_to_stop() {
    let secret = db_secret(Some(EXTERNAL), Some(EXTERNAL_MIGRATE));
    let reconnect = manifest_with(
        r#""database": { "included": false, "brought": false, "name": "meridian", "secret": "d" }"#,
    );
    let values: serde_yaml::Value = serde_yaml::from_str("{}").unwrap();
    let plan = a_plan(database_plan(&reconnect, &secret, &values).unwrap());
    let text = plan_text(&plan);
    assert!(
        text.contains("it is db.firm.example:5432/meridian, which somebody else runs")
            && text.contains("Stop any other deployment running against it first"),
        "{text}"
    );
    let load = a_plan(DatabasePlan::Load {
        place: "db.firm.example:5432/meridian".into(),
        database: "meridian".into(),
        image: "ghcr.io/open-meridian/meridian-backup:abc".into(),
    });
    let text = format!("{}\n{}", text, plan_text(&load));
    assert!(text.contains("which must be empty"), "{text}");
    for secret in ["s3cr", "m1gr", "meridian_app", "meridian_migrate"] {
        assert!(!text.contains(secret), "{secret}: {text}");
    }
}

// ── Plugins' storage (ruling 828) ────────────────────────────────────────

fn a_claim(name: &str, launched: bool) -> StorageClaim {
    let mut labels = BTreeMap::from([
        (
            "meridian.dev/component".to_string(),
            "plugin-storage".to_string(),
        ),
        ("app.kubernetes.io/instance".to_string(), "m".to_string()),
    ]);
    if launched {
        labels.insert("meridian.dev/launched".into(), "true".into());
        labels.insert("meridian.dev/plugin".into(), "snaptrade".into());
    }
    StorageClaim {
        name: name.into(),
        labels,
        size: "5Gi".into(),
    }
}

#[test]
fn the_plugins_claims_are_read_by_name_with_their_labels_and_size() {
    let listed = serde_json::json!({ "items": [
        { "metadata": { "name": "m-meridian-runtime-storage-zeta", "labels": { "meridian.dev/instance": "zeta" } },
          "spec": { "resources": { "requests": { "storage": "5Gi" } } } },
        { "metadata": { "name": "m-meridian-runtime-storage-alpha", "labels": { "meridian.dev/launched": "true" } },
          "spec": { "resources": { "requests": { "storage": "1Gi" } } } },
        { "metadata": { "name": "m-meridian-runtime-storage-gone", "deletionTimestamp": "2026-10-30T00:00:00Z" },
          "spec": { "resources": { "requests": { "storage": "1Gi" } } } }
    ] });
    let claims = storage_claims(&listed);
    assert_eq!(
        claims.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        [
            "m-meridian-runtime-storage-alpha",
            "m-meridian-runtime-storage-zeta"
        ]
    );
    assert_eq!(claims[0].size, "1Gi");
    assert_eq!(claims[0].labels["meridian.dev/launched"], "true");
    assert_eq!(
        storage_selector("m"),
        "app.kubernetes.io/instance=m,meridian.dev/component=plugin-storage"
    );
}

#[test]
fn a_job_reads_each_plugins_storage_read_only_and_names_it_for_the_script() {
    let cronjob = serde_json::json!({
        "metadata": { "name": "m-meridian-runtime-backup", "namespace": "ns", "uid": "u-1" },
        "spec": { "jobTemplate": {
            "metadata": { "labels": { "meridian.dev/component": "backup" } },
            "spec": { "template": { "spec": {
                "containers": [ { "name": "backup", "env": [], "volumeMounts": [ { "name": "work", "mountPath": "/work" } ] } ],
                "volumes": [ { "name": "work", "emptyDir": {} } ]
            } } }
        } }
    });
    let claims = [
        a_claim("m-meridian-runtime-storage-a", true),
        a_claim("m-meridian-runtime-storage-b", false),
    ];
    let job = job_from(&cronjob, "m-meridian-runtime-backup-1", true, &claims).unwrap();
    let pod = &job["spec"]["template"]["spec"];
    let env = pod["containers"][0]["env"].as_array().unwrap();
    let value = |name: &str| env.iter().find(|e| e["name"] == name).unwrap()["value"].clone();
    assert_eq!(value("MERIDIAN_BACKUP_HOLD"), "1");
    assert_eq!(
        value("MERIDIAN_BACKUP_STORAGE"),
        "m-meridian-runtime-storage-a m-meridian-runtime-storage-b"
    );
    let described: Vec<StorageClaim> =
        serde_json::from_str(value("MERIDIAN_BACKUP_STORAGE_CLAIMS").as_str().unwrap()).unwrap();
    assert_eq!(described, claims);
    let mounts = pod["containers"][0]["volumeMounts"].as_array().unwrap();
    assert_eq!(mounts.len(), 3);
    assert_eq!(
        mounts[2]["mountPath"],
        "/in/storage/m-meridian-runtime-storage-b"
    );
    assert_eq!(mounts[2]["readOnly"], true);
    let volumes = pod["volumes"].as_array().unwrap();
    assert_eq!(
        volumes[1]["persistentVolumeClaim"]["claimName"],
        "m-meridian-runtime-storage-a"
    );
    assert_eq!(volumes[1]["persistentVolumeClaim"]["readOnly"], true);
    // None to read: the Job is the CronJob's own.
    let plain = job_from(&cronjob, "m-meridian-runtime-backup-2", false, &[]).unwrap();
    assert_eq!(
        plain["spec"]["template"]["spec"]["volumes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_claim_is_made_again_as_the_launchers_or_the_charts_own() {
    let values: serde_yaml::Value =
        serde_yaml::from_str("pluginStorage: { storageClassName: fast }").unwrap();
    let launched = storage_claim_object(
        &a_claim("m-meridian-runtime-storage-a", true),
        "firm",
        "m",
        &values,
    );
    assert_eq!(
        launched["metadata"]["labels"]["meridian.dev/plugin"],
        "snaptrade"
    );
    assert!(launched["metadata"]["annotations"].is_null(), "{launched}");
    assert_eq!(launched["spec"]["resources"]["requests"]["storage"], "5Gi");
    assert_eq!(launched["spec"]["storageClassName"], "fast");
    let charts = storage_claim_object(
        &a_claim("m-meridian-runtime-storage-b", false),
        "firm",
        "m",
        &serde_yaml::from_str("{}").unwrap(),
    );
    assert_eq!(
        charts["metadata"]["labels"]["app.kubernetes.io/managed-by"],
        "Helm"
    );
    assert_eq!(
        charts["metadata"]["annotations"]["meta.helm.sh/release-name"],
        "m"
    );
    assert_eq!(
        charts["metadata"]["annotations"]["helm.sh/resource-policy"],
        "keep"
    );
    assert!(charts["spec"]["storageClassName"].is_null());
}

fn opened_with(storage: &[&str], claims: &str, tars: &[&str]) -> Opened {
    let mut manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
    manifest.storage = storage.iter().map(|s| s.to_string()).collect();
    let mut members = BTreeMap::new();
    members.insert(
        "storage/claims.json".to_string(),
        claims.as_bytes().to_vec(),
    );
    for tar in tars {
        members.insert(format!("storage/{tar}.tar"), Vec::new());
    }
    Opened { manifest, members }
}

#[test]
fn every_claim_a_backup_holds_is_described_or_restore_stops_first() {
    let described =
        r#"[{"name": "c-a", "labels": {"meridian.dev/launched": "true"}, "size": "5Gi"}]"#;
    let claims = described_claims(&opened_with(&["c-a"], described, &["c-a"])).unwrap();
    assert_eq!(claims[0].size, "5Gi");
    let undescribed = described_claims(&opened_with(&["c-a", "c-b"], described, &["c-a", "c-b"]));
    assert!(undescribed
        .unwrap_err()
        .contains("does not describe its claim"));
    let missing = described_claims(&opened_with(&["c-a"], described, &[]));
    assert!(missing.unwrap_err().contains("storage/c-a.tar"));
    // A backup the schedule took holds none, and needs no description.
    assert!(described_claims(&opened_with(&[], "", &[]))
        .unwrap()
        .is_empty());
}

#[test]
fn a_tars_files_are_listed_as_the_filling_pod_lists_them() {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, bytes) in [
        ("./session/messages.log", &b"fix"[..]),
        ("./sequence", &b"4127"[..]),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o640);
        header.set_cksum();
        builder.append_data(&mut header, path, bytes).unwrap();
    }
    let listed = listing(&builder.into_inner().unwrap()).unwrap();
    assert_eq!(
        listed,
        format!(
            "{} ./sequence\n{} ./session/messages.log\n",
            hex(&Sha256::digest(b"4127")),
            hex(&Sha256::digest(b"fix"))
        )
    );
}

#[test]
fn a_copy_is_what_the_job_wrote_or_it_is_read_again() {
    let written = Wrote {
        name: "m-20261030T000000Z.meridian-backup".into(),
        bytes: 3,
        sha256: hex(&Sha256::digest(b"age")),
    };
    assert!(is_written(b"age", &written));
    assert!(!is_written(b"ag", &written), "short");
    assert!(!is_written(b"agf", &written), "same size, other bytes");
}

#[test]
fn a_pod_that_cannot_be_placed_says_why() {
    let pods = serde_json::json!({ "items": [ { "status": { "conditions": [
        { "type": "PodScheduled", "status": "False", "reason": "Unschedulable",
          "message": "0/2 nodes are available: 2 node(s) had volume node affinity conflict." }
    ] } } ] });
    assert_eq!(
        unschedulable(&pods).as_deref(),
        Some("0/2 nodes are available: 2 node(s) had volume node affinity conflict.")
    );
    let placed = serde_json::json!({ "items": [ { "status": { "conditions": [
        { "type": "PodScheduled", "status": "True" } ] } } ] });
    assert_eq!(unschedulable(&placed), None);
}

#[test]
fn a_plugins_storage_left_in_the_namespace_stops_restore_and_the_plan_names_each() {
    let mut manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
    manifest.storage = vec!["meridian-meridian-runtime-storage-snap-1".into()];
    let values: serde_yaml::Value = serde_yaml::from_str("{}").unwrap();
    let present = Present {
        claims: vec!["meridian-meridian-runtime-storage-snap-1".into()],
        ..Present::default()
    };
    let found = left_behind(&manifest, "meridian-meridian-runtime", &values, &present);
    assert!(
        found
            .iter()
            .any(|said| said.contains("storage-snap-1, a plugin's storage")),
        "{found:?}"
    );
    let mut plan = a_plan(DatabasePlan::None);
    plan.storage = manifest.storage.clone();
    let text = plan_text(&plan);
    assert!(
        text.contains(
            "1 plugin storage claim filled as backed up (meridian-meridian-runtime-storage-snap-1)"
        ),
        "{text}"
    );
}

#[test]
fn a_claim_is_filled_by_a_pod_that_may_set_owners_and_modes_and_nothing_more() {
    let pod = filler_pod(
        "m-restore-storage-0",
        "firm",
        "m-meridian-runtime-storage-a",
        "ghcr.io/x/backup:1",
    );
    let container = &pod["spec"]["containers"][0];
    assert_eq!(
        container["securityContext"]["capabilities"]["drop"],
        serde_json::json!(["ALL"])
    );
    assert_eq!(
        container["securityContext"]["capabilities"]["add"],
        serde_json::json!(["CHOWN", "FOWNER", "DAC_OVERRIDE"])
    );
    assert_eq!(pod["spec"]["automountServiceAccountToken"], false);
    assert_eq!(
        pod["spec"]["volumes"][0]["persistentVolumeClaim"]["claimName"],
        "m-meridian-runtime-storage-a"
    );
}

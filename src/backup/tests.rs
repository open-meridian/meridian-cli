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
    let plain = job_from(&cronjob, "m-meridian-runtime-backup-1", false).unwrap();
    assert_eq!(plain["metadata"]["ownerReferences"][0]["uid"], "u-1");
    assert_eq!(
        plain["spec"]["template"]["spec"]["containers"][0]["env"],
        serde_json::json!([])
    );
    let held = job_from(&cronjob, "m-meridian-runtime-backup-1", true).unwrap();
    assert_eq!(
        held["spec"]["template"]["spec"]["containers"][0]["env"][0]["name"],
        "MERIDIAN_BACKUP_HOLD"
    );
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

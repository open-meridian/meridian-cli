//! `meridian backup` and `meridian restore` (task
//! kernel/a-deployment-is-backed-up-and-restored).
//!
//! A backup is made in the cluster by the chart's CronJob, in an image of
//! its own holding pg_dump and age (deploy/chart/files/backup.sh in
//! meridian-core): one file, `<release>-<UTC time>.meridian-backup`, a tar
//! sealed with age to the administrator's recipient. `meridian backup` runs
//! that CronJob once, now, and with `--to` copies the file out to this
//! machine; `meridian backup key` makes the pair it is sealed with.
//!
//! Restoring is a person's act with their own cluster rights, never the
//! deployment's (decisions/016), and only into a namespace holding no
//! release of that name and none of the backup's Secrets. The order is what
//! makes it safe: the kept Secrets first, so the chart renders with them;
//! the database's URL withheld, so every component and the migration wait
//! at it; the database restored and its row counts checked against the
//! backup's own; and only then the URL, so nothing ever runs against a
//! half-restored store. Then what first run does once it has written that
//! URL: the conductor and the dashboard, which read it only when they start,
//! restarted, and the first-run Job's rights given up.
//!
//! Nothing here prints a secret's value, a password or the identity: names,
//! counts, sizes and hashes only. Every Secret reaches kubectl on its
//! standard input, never as an argument.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Where `up` installs from, and so where restore does unless told.
pub const PUBLISHED_CHART: &str = "oci://ghcr.io/open-meridian/charts/meridian-runtime";
/// What every backup file's name ends with.
pub const SUFFIX: &str = ".meridian-backup";
/// What `meridian backup key` writes when not told where.
pub const IDENTITY_FILE: &str = "meridian-backup-identity.txt";

/// Every table's row count, as the backup counted them in its dump's own
/// snapshot. The same statement as the chart's files/backup.sh: restore
/// counts again with it and compares.
pub const COUNTS: &str = "select n.nspname || '.' || c.relname || chr(9) || \
     (xpath('/row/n/text()', query_to_xml(format('select count(*) as n from %I.%I', n.nspname, c.relname), false, true, '')))[1]::text \
     from pg_class c join pg_namespace n on n.oid = c.relnamespace \
     where c.relkind in ('r', 'p') and n.nspname not in ('pg_catalog', 'information_schema') \
     and n.nspname not like 'pg\\_toast%' order by n.nspname || '.' || c.relname collate \"C\"";

// ── The file ─────────────────────────────────────────────────────────────────

/// What a backup says of itself (manifest.json).
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub release: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub chart_version: String,
    #[serde(default)]
    pub deployment_id: String,
    #[serde(default)]
    pub taken_at: String,
    /// `generated`: the conductor made the deployment's key and it is in the
    /// backup as key/key.pem; `secret`: it is an administrator's Secret,
    /// backed up with the others they named.
    pub key: String,
    pub database: Database,
    #[serde(default)]
    pub chart_secrets: Vec<String>,
    #[serde(default)]
    pub operator_secrets: Vec<String>,
    #[serde(default)]
    pub config_maps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Database {
    pub included: bool,
    pub brought: bool,
    #[serde(default)]
    pub name: String,
    /// The Secret holding its URL, which restore withholds until the
    /// database is back.
    #[serde(default)]
    pub secret: String,
}

/// A backup, opened and checked.
pub struct Opened {
    pub manifest: Manifest,
    pub members: BTreeMap<String, Vec<u8>>,
}

impl Opened {
    /// One object's keys, from `secrets/<name>/` or `configmaps/<name>/`.
    pub fn object(&self, kind: &str, name: &str) -> BTreeMap<String, Vec<u8>> {
        let prefix = format!("{kind}/{name}/");
        self.members
            .iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix(&prefix)
                    .filter(|key| !key.is_empty() && !key.contains('/'))
                    .map(|key| (key.to_string(), bytes.clone()))
            })
            .collect()
    }

    pub fn member(&self, path: &str) -> Result<&[u8], String> {
        self.members
            .get(path)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("the backup holds no {path}"))
    }
}

/// A new identity, as `age-keygen` writes one, and its recipient.
pub fn new_identity(now: &str) -> (String, String) {
    use age::secrecy::ExposeSecret;
    let identity = age::x25519::Identity::generate();
    let recipient = identity.to_public().to_string();
    let file = format!(
        "# created: {now}\n# public key: {recipient}\n# The identity meridian restore opens this deployment's backups with.\n# Keep it off the cluster, and somewhere a reset of this machine does not reach.\n{}\n",
        identity.to_string().expose_secret()
    );
    (file, recipient)
}

/// The identity written to `path`, readable by this user alone, and never
/// over a file already there. Its recipient, to set as backup.recipient.
pub fn write_identity(path: &Path, now: &str) -> Result<String, String> {
    use std::io::Write;
    let (file, recipient) = new_identity(now);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut written = options.open(path).map_err(|failed| match failed.kind() {
        std::io::ErrorKind::AlreadyExists => format!(
            "{} is there already, and an identity is never written over: a backup sealed to it \
             would be lost with it",
            path.display()
        ),
        _ => format!("{} could not be written: {failed}", path.display()),
    })?;
    written
        .write_all(file.as_bytes())
        .map_err(|failed| format!("{} could not be written: {failed}", path.display()))?;
    Ok(recipient)
}

/// Open a backup with the identity file's text: decrypt, untar, check every
/// member against checksums.sha256, and read the manifest.
pub fn open(sealed: &[u8], identity: &str) -> Result<Opened, String> {
    let identities = age::IdentityFile::from_buffer(identity.as_bytes())
        .map_err(|failed| format!("the identity file could not be read: {failed}"))?
        .into_identities()
        .map_err(|failed| format!("the identity file could not be read: {failed}"))?;
    if identities.is_empty() {
        return Err("the identity file holds no identity".into());
    }
    let decryptor = age::Decryptor::new(sealed)
        .map_err(|failed| format!("this is not a backup age can open: {failed}"))?;
    let mut plain = Vec::new();
    decryptor
        .decrypt(
            identities
                .iter()
                .map(|each| each.as_ref() as &dyn age::Identity),
        )
        .map_err(|failed| {
            format!("it does not open with this identity ({failed}): it was sealed to another")
        })?
        .read_to_end(&mut plain)
        .map_err(|failed| format!("the backup could not be read to its end: {failed}"))?;
    let members = untar(&plain)?;
    verify(&members)?;
    let manifest: Manifest = serde_json::from_slice(
        members
            .get("manifest.json")
            .ok_or("the backup holds no manifest.json")?,
    )
    .map_err(|failed| format!("the backup's manifest could not be read: {failed}"))?;
    if manifest.format != 1 {
        return Err(format!(
            "the backup is in format {}, which this meridian does not know: upgrade it",
            manifest.format
        ));
    }
    Ok(Opened { manifest, members })
}

/// Every file in the tar, by its path without a leading `./`.
pub fn untar(plain: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut members = BTreeMap::new();
    let mut archive = tar::Archive::new(plain);
    for entry in archive
        .entries()
        .map_err(|failed| format!("the backup is not a tar: {failed}"))?
    {
        let mut entry = entry.map_err(|failed| format!("the backup's tar is broken: {failed}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|failed| format!("a member's name could not be read: {failed}"))?
            .to_string_lossy()
            .trim_start_matches("./")
            .to_string();
        if path.split('/').any(|part| part == "..") {
            return Err(format!("the backup names a member outside itself: {path}"));
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|failed| format!("{path} could not be read: {failed}"))?;
        members.insert(path, bytes);
    }
    Ok(members)
}

/// Every member listed in checksums.sha256, with that hash; none missing and
/// none unlisted.
pub fn verify(members: &BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let listed = members
        .get("checksums.sha256")
        .ok_or("the backup holds no checksums.sha256")?;
    let listed = String::from_utf8_lossy(listed);
    let mut seen = 0;
    for line in listed.lines().filter(|line| !line.trim().is_empty()) {
        let (hash, path) = line.split_once("  ").ok_or_else(|| {
            format!("checksums.sha256 has a line that is not `<hash>  <path>`: {line}")
        })?;
        let bytes = members
            .get(path)
            .ok_or_else(|| format!("the backup lists {path} and does not hold it"))?;
        if hex(&Sha256::digest(bytes)) != hash {
            return Err(format!(
                "{path} is not what the backup wrote: its hash differs"
            ));
        }
        seen += 1;
    }
    if seen + 1 != members.len() {
        return Err("the backup holds a member checksums.sha256 does not list".into());
    }
    Ok(())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ── The values ───────────────────────────────────────────────────────────────

/// What was chosen: the values that differ from the chart's defaults, so a
/// default the chart has since changed still applies, and Helm is given what
/// a person gave it (`helm get values`, not `--all`).
pub fn chosen(values: &serde_yaml::Value, defaults: &serde_yaml::Value) -> serde_yaml::Value {
    use serde_yaml::Value;
    match (values, defaults) {
        (Value::Mapping(held), Value::Mapping(default)) => {
            let mut out = serde_yaml::Mapping::new();
            for (key, value) in held {
                match default.get(key) {
                    Some(was) if was == value => {}
                    Some(was @ Value::Mapping(_)) if value.is_mapping() => {
                        let inner = chosen(value, was);
                        if inner.as_mapping().is_some_and(|m| !m.is_empty()) {
                            out.insert(key.clone(), inner);
                        }
                    }
                    _ => {
                        out.insert(key.clone(), value.clone());
                    }
                }
            }
            Value::Mapping(out)
        }
        _ => values.clone(),
    }
}

/// The chart's full name, as its helpers make it: what every object's name
/// begins with.
pub fn fullname(release: &str, values: &serde_yaml::Value) -> String {
    let said = |key: &str| {
        values
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(String::from)
    };
    let full = said("fullnameOverride").unwrap_or_else(|| {
        let name = said("nameOverride").unwrap_or_else(|| "meridian-runtime".into());
        let name: String = name.chars().take(63).collect();
        format!("{release}-{}", name.trim_end_matches('-'))
    });
    let full: String = full.chars().take(63).collect();
    full.trim_end_matches('-').to_string()
}

// ── The database ─────────────────────────────────────────────────────────────

/// A role's login, read from the URL first run wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub role: String,
    pub password: String,
    pub database: String,
}

/// `postgres://<role>:<password>@<host>:<port>/<database>?...`
pub fn login(url: &str) -> Result<Login, String> {
    let rest = url
        .split_once("://")
        .map(|(_, rest)| rest)
        .ok_or("the database URL has no scheme")?;
    let (who, place) = rest
        .rsplit_once('@')
        .ok_or("the database URL names no role")?;
    let (role, password) = who.split_once(':').unwrap_or((who, ""));
    let database = place
        .split_once('/')
        .map(|(_, rest)| rest.split('?').next().unwrap_or(""))
        .unwrap_or("");
    let login = Login {
        role: percent_decoded(role)?,
        password: percent_decoded(password)?,
        database: percent_decoded(database)?,
    };
    for name in [&login.role, &login.database] {
        identifier(name)?;
    }
    Ok(login)
}

fn percent_decoded(said: &str) -> Result<String, String> {
    let bytes = said.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let pair = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            let value = u8::from_str_radix(pair, 16)
                .map_err(|_| "the database URL has a broken escape".to_string())?;
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "the database URL is not UTF-8".into())
}

/// A name that becomes SQL. Refused rather than escaped, as first run does.
pub fn identifier(name: &str) -> Result<&str, String> {
    let ok = !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(name)
    } else {
        Err(format!("{name} is not a name restore will put into SQL"))
    }
}

fn literal(said: &str) -> Result<String, String> {
    if said.contains('\0') {
        return Err("a password holds a NUL".into());
    }
    Ok(format!("'{}'", said.replace('\'', "''")))
}

/// The database and its two roles, made as first run makes them, with the
/// passwords the backup's URLs hold. For psql, on its standard input.
pub fn provision_sql(serving: &Login, migrating: &Login) -> Result<String, String> {
    let database = identifier(&serving.database)?;
    let mut sql = String::from("\\set ON_ERROR_STOP 1\n");
    for role in [serving, migrating] {
        let name = identifier(&role.role)?;
        let password = literal(&role.password)?;
        sql.push_str(&format!(
            "select 'create role {name}' where not exists (select 1 from pg_roles where rolname = '{name}') \\gexec\n\
             alter role {name} with login password {password};\n"
        ));
    }
    sql.push_str(&format!(
        "select 'create database {database}' where not exists (select 1 from pg_database where datname = '{database}') \\gexec\n\
         \\connect {database}\n"
    ));
    for role in [serving, migrating] {
        let name = identifier(&role.role)?;
        sql.push_str(&format!(
            "grant connect on database {database} to {name};\ngrant usage on schema public to {name};\n"
        ));
    }
    sql.push_str(&format!(
        "grant create on schema public to {};\nrevoke create on schema public from {}, public;\n",
        identifier(&migrating.role)?,
        identifier(&serving.role)?
    ));
    Ok(sql)
}

/// `<schema>.<table>\t<count>` lines, as the backup and COUNTS write them.
pub fn counts(tsv: &str) -> BTreeMap<String, u64> {
    tsv.lines()
        .filter_map(|line| {
            let (table, count) = line.split_once('\t')?;
            Some((table.to_string(), count.trim().parse().ok()?))
        })
        .collect()
}

/// Each table whose count differs, or that one side has and the other not.
pub fn differences(
    backed_up: &BTreeMap<String, u64>,
    restored: &BTreeMap<String, u64>,
) -> Vec<String> {
    let mut said = Vec::new();
    for (table, count) in backed_up {
        match restored.get(table) {
            Some(found) if found == count => {}
            Some(found) => said.push(format!("{table}: {count} backed up, {found} restored")),
            None => said.push(format!(
                "{table}: {count} backed up, and no such table restored"
            )),
        }
    }
    for table in restored
        .keys()
        .filter(|table| !backed_up.contains_key(*table))
    {
        said.push(format!("{table}: restored, and not in the backup"));
    }
    said
}

// ── The objects ──────────────────────────────────────────────────────────────

/// A Secret as kubectl takes it on its standard input. `adopt` names the
/// release that is to own it: Helm adopts an object carrying these, and
/// refuses to install over one that does not.
pub fn secret_object(
    name: &str,
    namespace: &str,
    data: &BTreeMap<String, Vec<u8>>,
    adopt: Option<&str>,
) -> serde_json::Value {
    let encoded: serde_json::Map<String, serde_json::Value> = data
        .iter()
        .map(|(key, bytes)| {
            (
                key.clone(),
                base64::engine::general_purpose::STANDARD
                    .encode(bytes)
                    .into(),
            )
        })
        .collect();
    let keys: Vec<&str> = data.keys().map(String::as_str).collect();
    let kind = match keys.as_slice() {
        ["tls.crt", "tls.key"] => "kubernetes.io/tls",
        [".dockerconfigjson"] => "kubernetes.io/dockerconfigjson",
        _ => "Opaque",
    };
    let mut object = serde_json::json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": { "name": name, "namespace": namespace },
        "type": kind,
        "data": encoded,
    });
    if let Some(release) = adopt {
        object["metadata"]["labels"] = owned_labels();
        object["metadata"]["annotations"] = owned_annotations(release, namespace);
    }
    object
}

pub fn config_map_object(
    name: &str,
    namespace: &str,
    data: &BTreeMap<String, Vec<u8>>,
    release: &str,
) -> serde_json::Value {
    let mut text = serde_json::Map::new();
    let mut binary = serde_json::Map::new();
    for (key, bytes) in data {
        match std::str::from_utf8(bytes) {
            Ok(said) => {
                text.insert(key.clone(), said.into());
            }
            Err(_) => {
                binary.insert(
                    key.clone(),
                    base64::engine::general_purpose::STANDARD
                        .encode(bytes)
                        .into(),
                );
            }
        }
    }
    serde_json::json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": owned_labels(),
            "annotations": owned_annotations(release, namespace),
        },
        "data": text,
        "binaryData": binary,
    })
}

fn owned_labels() -> serde_json::Value {
    serde_json::json!({ "app.kubernetes.io/managed-by": "Helm" })
}

fn owned_annotations(release: &str, namespace: &str) -> serde_json::Value {
    serde_json::json!({
        "meta.helm.sh/release-name": release,
        "meta.helm.sh/release-namespace": namespace,
    })
}

/// A Job from the backup's CronJob, as `kubectl create job --from` makes
/// one, and holding its file for `--to` to copy when asked.
pub fn job_from(
    cronjob: &serde_json::Value,
    name: &str,
    hold: bool,
) -> Result<serde_json::Value, String> {
    let template = &cronjob["spec"]["jobTemplate"];
    let mut spec = template["spec"].clone();
    if !spec.is_object() {
        return Err("the backup's CronJob has no job template".into());
    }
    if hold {
        let container = spec["template"]["spec"]["containers"]
            .get_mut(0)
            .ok_or("the backup's CronJob runs no container")?;
        let env = container
            .as_object_mut()
            .ok_or("the backup's container is not an object")?
            .entry("env")
            .or_insert_with(|| serde_json::json!([]));
        env.as_array_mut()
            .ok_or("the backup's environment is not a list")?
            .push(serde_json::json!({ "name": "MERIDIAN_BACKUP_HOLD", "value": "1" }));
    }
    Ok(serde_json::json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": {
            "name": name,
            "namespace": cronjob["metadata"]["namespace"],
            "labels": template["metadata"]["labels"],
            "annotations": { "cronjob.kubernetes.io/instantiate": "manual" },
            "ownerReferences": [{
                "apiVersion": "batch/v1",
                "kind": "CronJob",
                "name": cronjob["metadata"]["name"],
                "uid": cronjob["metadata"]["uid"],
                "controller": true,
            }],
        },
        "spec": spec,
    }))
}

/// The chart's first-run Job, as restore finishes it: its name, the
/// RoleBinding it holds its rights through, and the Deployments it restarts
/// once it has written the database's URL -- each from the Job's own
/// environment, where the chart names them for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstRunJob {
    pub name: String,
    pub binding: String,
    pub restart: Vec<String>,
}

/// The first-run Job among a `kubectl get jobs -o json` answer, if the
/// release has one (the chart renders it with the dashboard).
pub fn first_run_job(jobs: &serde_json::Value) -> Option<FirstRunJob> {
    let job = jobs["items"].as_array()?.iter().find(|job| {
        job["spec"]["template"]["metadata"]["labels"]["meridian.dev/component"] == "first-run"
    })?;
    let env = job["spec"]["template"]["spec"]["containers"][0]["env"].as_array()?;
    let value = |name: &str| {
        env.iter()
            .find(|e| e["name"] == name)
            .and_then(|e| e["value"].as_str())
            .unwrap_or_default()
            .to_string()
    };
    Some(FirstRunJob {
        name: job["metadata"]["name"].as_str()?.to_string(),
        binding: value("MERIDIAN_FIRST_RUN_BINDING"),
        restart: value("MERIDIAN_FIRST_RUN_RESTART")
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(String::from)
            .collect(),
    })
}

/// What the Job's log says it wrote: `backup: wrote <name> <bytes> bytes sha256 <hex>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrote {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

pub fn wrote(log: &str) -> Option<Wrote> {
    log.lines().rev().find_map(|line| {
        let mut words = line.strip_prefix("backup: wrote ")?.split_whitespace();
        let name = words.next()?.to_string();
        let bytes = words.next()?.parse().ok()?;
        (words.next()? == "bytes" && words.next()? == "sha256").then_some(())?;
        let sha256 = words.next()?.to_string();
        (name.ends_with(SUFFIX) && !name.contains('/')).then_some(Wrote {
            name,
            bytes,
            sha256,
        })
    })
}

// ── Running kubectl and helm ─────────────────────────────────────────────────

async fn run(program: &str, arguments: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new(program);
    command
        .args(arguments)
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|failed| format!("{program} could not be run: {failed}. Is it installed?"))?;
    if let Some(input) = input {
        let mut stdin = child.stdin.take().ok_or("no standard input to write to")?;
        let input = input.to_vec();
        // Written beside the read, so a large dump never fills a pipe both
        // ends are waiting on.
        tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        });
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|failed| format!("{program} could not be run: {failed}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "`{program} {}`: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

async fn text(program: &str, arguments: &[&str]) -> Result<String, String> {
    run(program, arguments, None)
        .await
        .map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// Ask until `ready` answers, every two seconds, for at most `within`.
async fn until<F, Fut, T>(what: &str, within: Duration, mut ready: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let started = Instant::now();
    loop {
        if let Some(found) = ready().await {
            return Ok(found);
        }
        if started.elapsed() >= within {
            return Err(format!(
                "{what} did not happen within {}s",
                within.as_secs()
            ));
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

// ── meridian backup ──────────────────────────────────────────────────────────

pub struct Take {
    pub release: String,
    pub namespace: String,
    /// A directory on this machine to copy the file to.
    pub to: Option<PathBuf>,
    pub timeout: Duration,
}

/// Run the deployment's backup once, now, and say what it wrote.
pub async fn take(asked: &Take, unix_seconds: u64) -> Result<String, String> {
    let ns = asked.namespace.as_str();
    if let Some(to) = &asked.to {
        if !to.is_dir() {
            return Err(format!("{} is not a directory here", to.display()));
        }
    }
    let selector = format!(
        "app.kubernetes.io/instance={},meridian.dev/component=backup",
        asked.release
    );
    let found: serde_json::Value = serde_json::from_str(
        &text(
            "kubectl",
            &["get", "cronjob", "-n", ns, "-l", &selector, "-o", "json"],
        )
        .await?,
    )
    .map_err(|failed| format!("kubectl's answer could not be read: {failed}"))?;
    let Some(cronjob) = found["items"].get(0) else {
        return Err(format!(
            "{} in {ns} has no backup configured. Make a key with `meridian backup key`, then set \
             backup.recipient to its public half and backup.path (a directory on the node) or \
             backup.existingClaim in a values file, and apply it with `meridian \
             upgrade-deployment --release {} -n {ns} -f <file>`",
            asked.release, asked.release
        ));
    };
    let cronjob_name = cronjob["metadata"]["name"].as_str().unwrap_or_default();
    let job = format!("{cronjob_name}-{unix_seconds}");
    let manifest = job_from(cronjob, &job, asked.to.is_some())?;
    run(
        "kubectl",
        &["create", "-n", ns, "-f", "-"],
        Some(manifest.to_string().as_bytes()),
    )
    .await?;

    let job_name = job.as_str();
    let log = until("the backup", asked.timeout, move || async move {
        let log = text("kubectl", &["logs", "-n", ns, &format!("job/{job_name}")])
            .await
            .unwrap_or_default();
        if wrote(&log).is_some() {
            return Some(Ok(log));
        }
        let failed = text(
            "kubectl",
            &[
                "get",
                "job",
                job_name,
                "-n",
                ns,
                "-o",
                "jsonpath={.status.failed}",
            ],
        )
        .await
        .unwrap_or_default();
        (!failed.trim().is_empty() && failed.trim() != "0").then_some(Err(log))
    })
    .await?
    .map_err(|log| format!("the backup failed. It said:\n{log}"))?;
    let written = wrote(&log).expect("waited for it");

    let mut said: String = log
        .lines()
        .filter(|line| line.starts_with("backup: ") && !line.contains("holding"))
        .map(|line| format!("  {line}\n"))
        .collect();
    said.insert_str(
        0,
        &format!(
            "Backed up {} to {} ({} bytes), on the deployment's backup volume.\n",
            asked.release, written.name, written.bytes
        ),
    );

    if let Some(to) = &asked.to {
        let pod = text(
            "kubectl",
            &[
                "get",
                "pods",
                "-n",
                ns,
                "-l",
                &format!("job-name={job}"),
                "-o",
                "jsonpath={.items[0].metadata.name}",
            ],
        )
        .await?;
        let bytes = run(
            "kubectl",
            &[
                "exec",
                "-n",
                ns,
                pod.trim(),
                "--",
                "cat",
                &format!("/backups/{}", written.name),
            ],
            None,
        )
        .await?;
        let released = run(
            "kubectl",
            &["exec", "-n", ns, pod.trim(), "--", "touch", "/tmp/copied"],
            None,
        )
        .await;
        if bytes.len() as u64 != written.bytes || hex(&Sha256::digest(&bytes)) != written.sha256 {
            return Err("the copy is not what the backup wrote: its size or hash differs".into());
        }
        let path = to.join(&written.name);
        std::fs::write(&path, &bytes)
            .map_err(|failed| format!("{} could not be written: {failed}", path.display()))?;
        said.push_str(&format!(
            "Copied to {}, its SHA-256 checked. It opens only with the identity whose public half \
             is backup.recipient.\n",
            path.display()
        ));
        released?;
    }
    Ok(said)
}

// ── meridian restore ─────────────────────────────────────────────────────────

pub struct Restore {
    pub file: PathBuf,
    pub identity: PathBuf,
    /// None: the namespace the backup was taken in.
    pub namespace: Option<String>,
    /// None: the release the backup names. Another is refused: every kept
    /// object's name is made from it.
    pub release: Option<String>,
    pub chart: String,
    pub chart_version: Option<String>,
    pub timeout: Duration,
}

/// `<host>:<port>/<database>` of a database URL, never its role or password:
/// what a person is told when restore is to reconnect to, or load into, a
/// database somebody else runs.
pub fn place(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, rest)| rest)?;
    let after = rest.rsplit_once('@').map_or(rest, |(_, after)| after);
    let place = after.split('?').next().unwrap_or_default();
    (!place.is_empty()).then(|| place.to_string())
}

/// What restore does with the deployment's database, decided from the
/// manifest before anything is changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabasePlan {
    /// The database the chart brings, started and loaded from the dump.
    Brought { database: String },
    /// A database somebody else runs, loaded from the dump by restore
    /// itself, in `backup.image`, as the migrating role (gap 4): it must be
    /// empty.
    Load {
        place: String,
        database: String,
        image: String,
    },
    /// A database somebody else runs, not in the backup: the deployment
    /// reaches it again with the restored URL, so nothing else may run
    /// against it (gap 3).
    Reconnect { place: String },
    /// The chart brings one, and the backup holds no dump of it
    /// (`backup.database: false`): it starts empty.
    BroughtEmpty,
    /// The backup was taken before first run made one.
    None,
}

/// The database plan, or why restore cannot carry it out, said before any
/// change. `secret` is the backup's copy of the database's Secret.
pub fn database_plan(
    manifest: &Manifest,
    secret: &BTreeMap<String, Vec<u8>>,
    values: &serde_yaml::Value,
) -> Result<DatabasePlan, String> {
    let db = &manifest.database;
    let url = |key: &str| {
        secret
            .get(key)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .filter(|url| !url.trim().is_empty())
    };
    let where_ = || {
        url("url")
            .as_deref()
            .and_then(place)
            .unwrap_or_else(|| db.name.clone())
    };
    match (db.included, db.brought) {
        (true, true) => Ok(DatabasePlan::Brought {
            database: db.name.clone(),
        }),
        (true, false) => {
            let migrating = url("migrate-url").or_else(|| url("url")).ok_or_else(|| {
                format!(
                    "the backup holds a dump of a database this chart does not bring, and its \
                     Secret {} holds no URL to load it with: nothing was changed",
                    db.secret
                )
            })?;
            let database = login(&migrating)?.database;
            let image = values
                .get("backup")
                .and_then(|backup| backup.get("image"))
                .and_then(|image| image.as_str())
                .filter(|image| !image.is_empty())
                .ok_or(
                    "the backup's values name no backup.image, the image restore loads a \
                     database somebody else runs in: nothing was changed",
                )?
                .to_string();
            Ok(DatabasePlan::Load {
                place: where_(),
                database,
                image,
            })
        }
        (false, false) if !db.name.is_empty() => Ok(DatabasePlan::Reconnect { place: where_() }),
        (false, true) if !db.name.is_empty() => Ok(DatabasePlan::BroughtEmpty),
        _ => Ok(DatabasePlan::None),
    }
}

/// The chart version restore installs, against the one the backup was taken
/// on (gap 6): never older, since a database a newer chart migrated is not
/// served by an older runtime; newer is an upgrade, and said. What to say,
/// or why not.
pub fn chart_version_said(recorded: &str, installs: &str) -> Result<Option<String>, String> {
    use std::cmp::Ordering;
    if recorded.is_empty() {
        return Ok(Some(
            "the backup records no chart version, so whether this one is older is not known".into(),
        ));
    }
    match crate::release::compare(installs, recorded) {
        Ordering::Less => Err(format!(
            "the chart restore would install is {installs}, older than {recorded}, the version \
             the backup was taken on: its database may hold what only a newer runtime serves. \
             Name {recorded} or newer with --chart-version; nothing was changed"
        )),
        Ordering::Greater => Ok(Some(format!(
            "{installs} is newer than {recorded}, the version the backup was taken on: restoring \
             is then an upgrade, the migration moving the restored database to it"
        ))),
        Ordering::Equal => Ok(None),
    }
}

/// What is in the namespace, and on the cluster, before restore changes
/// anything: the names kubectl lists.
#[derive(Debug, Clone)]
pub struct Present {
    pub secrets: Vec<String>,
    pub config_maps: Vec<String>,
    pub claims: Vec<String>,
    /// The backup volume a `backup.path` makes, where one is there; `Err`
    /// when whether it is could not be read.
    pub volume: Result<Option<String>, String>,
}

impl Default for Present {
    fn default() -> Self {
        Present {
            secrets: Vec::new(),
            config_maps: Vec::new(),
            claims: Vec::new(),
            volume: Ok(None),
        }
    }
}

/// The backup volume the chart makes from `backup.path`, by name: the
/// node's directory as a volume, kept when a namespace is deleted.
pub fn backup_volume(ns: &str, full: &str, values: &serde_yaml::Value) -> Option<String> {
    let backup = values.get("backup")?;
    let said = |key: &str| {
        backup
            .get(key)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
    };
    said("path")?;
    said("recipient")?;
    let claim = said("existingClaim")
        .map(String::from)
        .unwrap_or_else(|| format!("{full}-backup"));
    Some(format!("{ns}-{claim}"))
}

/// The claims the release's StatefulSets take up when installed: the
/// brought database's and the registry's, each where the values enable it.
pub fn claims_taken(full: &str, values: &serde_yaml::Value) -> Vec<String> {
    let enabled = |path: &[&str]| {
        let mut at = values;
        for key in path {
            match at.get(*key) {
                Some(next) => at = next,
                // Not said: the chart's default, which brings both.
                None => return true,
            }
        }
        at.as_bool().unwrap_or(true)
    };
    let mut claims = Vec::new();
    if enabled(&["database", "brought", "enabled"]) {
        claims.push(format!("data-{full}-database-0"));
    }
    if enabled(&["registry", "enabled"]) {
        claims.push(format!("images-{full}-registry-0"));
    }
    claims
}

/// Every kept object of a deployment restore would make or take up, found
/// already there (gap 7), each with what to do: checked before any change.
/// The backup volume a deleted namespace left is not among them: restore
/// offers to delete it within its own confirmation (`Plan::delete_volume`,
/// audit D6), since its Retain policy keeps the node's directory, backups
/// and all.
pub fn left_behind(
    manifest: &Manifest,
    full: &str,
    values: &serde_yaml::Value,
    present: &Present,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut secrets: Vec<String> = manifest
        .chart_secrets
        .iter()
        .chain(&manifest.operator_secrets)
        .cloned()
        .collect();
    if manifest.key == "generated" {
        secrets.push(format!("{full}-key"));
    }
    for name in secrets.iter().filter(|name| present.secrets.contains(name)) {
        found.push(format!("the Secret {name}"));
    }
    let maps = manifest
        .config_maps
        .iter()
        .cloned()
        .chain(std::iter::once(format!("{full}-backup-values")));
    for name in maps.filter(|name| present.config_maps.contains(name)) {
        found.push(format!("the ConfigMap {name}"));
    }
    for name in claims_taken(full, values)
        .into_iter()
        .filter(|name| present.claims.contains(name))
    {
        found.push(format!(
            "the claim {name}, which the release would mount and find full"
        ));
    }
    found
}

/// Every table's store, read from its name's prefix: `config_`, `book_`,
/// `lake_`, the street's (its tables' own names, and `schema_migration`, the
/// street's record of its migrations), the instrument store's, `dashboard_`.
pub fn store_of(table: &str) -> &'static str {
    let name = table.rsplit_once('.').map_or(table, |(_, name)| name);
    const STREET: [&str; 9] = [
        "street_",
        "holding",
        "statement",
        "custodial_",
        "account_",
        "activity",
        "partition_",
        "sync_",
        "position",
    ];
    if name.starts_with("config_") {
        "config"
    } else if name.starts_with("book_") {
        "book"
    } else if name.starts_with("lake_") {
        "lake"
    } else if name.starts_with("dashboard_") {
        "dashboard"
    } else if name.starts_with("instrument") || name.starts_with("venue_") {
        "instrument"
    } else if name == "schema_migration" || STREET.iter().any(|prefix| name.starts_with(prefix)) {
        "street"
    } else {
        "other"
    }
}

/// The counts grouped by store (gap 5): each store with its tables and rows,
/// in a fixed order, a store with no table left out.
pub fn by_store(counts: &BTreeMap<String, u64>) -> Vec<(&'static str, usize, u64)> {
    const ORDER: [&str; 7] = [
        "config",
        "book",
        "street",
        "lake",
        "instrument",
        "dashboard",
        "other",
    ];
    ORDER
        .iter()
        .filter_map(|store| {
            let held: Vec<u64> = counts
                .iter()
                .filter(|(table, _)| store_of(table) == *store)
                .map(|(_, rows)| *rows)
                .collect();
            (!held.is_empty()).then(|| (*store, held.len(), held.iter().sum()))
        })
        .collect()
}

fn stores_text(counts: &BTreeMap<String, u64>) -> String {
    by_store(counts)
        .iter()
        .map(|(store, tables, rows)| format!("    {store}: {tables} tables, {rows} rows\n"))
        .collect()
}

/// Why a launched plugin's pod cannot start for its image, as Kubernetes
/// says it while waiting.
const UNPULLED: [&str; 4] = [
    "ErrImagePull",
    "ImagePullBackOff",
    "InvalidImageName",
    "ErrImageNeverPull",
];

/// The launched plugins' pods: each instance whose image could not be
/// pulled, with that image; and whether every pod has settled, ready or
/// so refused (gap 5). A registry's claim can outlive a namespace, so some
/// images may be there; only the missing are named.
pub fn unpulled(pods: &[crate::upgrade_deployment::Pod]) -> (bool, Vec<(String, String)>) {
    let mut missing = Vec::new();
    let mut settled = true;
    for pod in pods.iter().filter(|pod| !pod.terminating) {
        let instance = pod
            .labels
            .get("meridian.dev/instance")
            .cloned()
            .unwrap_or_else(|| pod.name.clone());
        let refused = pod.init.iter().chain(&pod.containers).find(|container| {
            container
                .waiting
                .as_deref()
                .is_some_and(|why| UNPULLED.contains(&why))
        });
        match refused {
            Some(container) => {
                let image = pod.images.get(&container.name).cloned().unwrap_or_default();
                if !missing.iter().any(|(held, _)| *held == instance) {
                    missing.push((instance, image));
                }
            }
            None if pod.ready_since.is_some() => {}
            None => settled = false,
        }
    }
    (settled, missing)
}

/// What restore will do, shown before it does any of it (gap 2).
#[derive(Debug, Clone)]
pub struct Plan {
    pub file: String,
    pub release: String,
    pub deployment_id: String,
    pub taken_at: String,
    pub cluster: String,
    pub namespace: String,
    pub namespace_exists: bool,
    pub chart: String,
    pub version: String,
    /// What is said of the version: newer than the backup's, or unknown.
    pub version_said: Option<String>,
    pub secrets: usize,
    pub config_maps: usize,
    pub key_generated: bool,
    pub database: DatabasePlan,
    /// Whether the backup volume could be checked for, where it could not.
    pub unchecked: Option<String>,
    /// The backup volume a deleted namespace left, which the release would
    /// make again: deleted once approved, its node directory kept.
    pub delete_volume: Option<String>,
}

pub fn plan_text(plan: &Plan) -> String {
    let mut out = format!(
        "Restoring {file}:\n\
         \x20 the backup  {release} ({id}), taken {taken}\n\
         \x20 into        the cluster {cluster}, namespace {ns}{made}\n\
         \x20 from        {chart} {version}\n",
        file = plan.file,
        release = plan.release,
        id = plan.deployment_id,
        taken = plan.taken_at,
        cluster = plan.cluster,
        ns = plan.namespace,
        made = if plan.namespace_exists { "" } else { " (made)" },
        chart = plan.chart,
        version = plan.version,
    );
    if let Some(said) = &plan.version_said {
        out.push_str(&format!("  {said}.\n"));
    }
    out.push_str(&format!(
        "\n  It will make {} Secrets{}, {} ConfigMaps, and install {} with the backup's values; ",
        plan.secrets,
        if plan.key_generated {
            " and the deployment's key"
        } else {
            ""
        },
        plan.config_maps,
        plan.release,
    ));
    out.push_str(&match &plan.database {
        DatabasePlan::Brought { database } => format!(
            "then start the database the chart brings, make {database} and its roles, and load the \
             dump, every table's count checked before the components are given its URL.\n"
        ),
        DatabasePlan::Load {
            place, database, ..
        } => format!(
            "then load the dump into {place}, a database somebody else runs, which must be \
             empty, as the migrating role, in backup.image, every table's count checked before \
             the components are given its URL. The restored deployment writes to {database} \
             there: stop anything else running against it first.\n"
        ),
        DatabasePlan::Reconnect { place } => format!(
            "the database is not in the backup: it is {place}, which somebody else runs, and the \
             restored deployment will write to it again. Stop any other deployment running \
             against it first: two writing one book is the worst outcome.\n"
        ),
        DatabasePlan::BroughtEmpty => "the backup holds no dump of the database the chart \
             brings (backup.database is false), so it starts empty.\n"
            .to_string(),
        DatabasePlan::None => {
            "the backup holds no database: it was taken before first run.\n".to_string()
        }
    });
    if let Some(volume) = &plan.delete_volume {
        out.push_str(&format!(
            "  First it deletes the backup volume {volume}, which a deleted namespace left and \
             the release makes again. It is kept with Retain, so the node's directory, with its \
             backups, stays where it is.\n"
        ));
    }
    if let Some(unchecked) = &plan.unchecked {
        out.push_str(&format!("  Not checked: {unchecked}.\n"));
    }
    out
}

/// Restore a backup into an empty namespace, saying each step on `step`,
/// asking `approve` with the plan once every check has passed and before
/// anything is changed, and returning what was restored.
pub async fn restore(
    asked: &Restore,
    step: &mut dyn FnMut(&str),
    approve: &mut dyn FnMut(&str) -> bool,
) -> Result<String, String> {
    let sealed = std::fs::read(&asked.file)
        .map_err(|failed| format!("{} could not be read: {failed}", asked.file.display()))?;
    let identity = std::fs::read_to_string(&asked.identity)
        .map_err(|failed| format!("{} could not be read: {failed}", asked.identity.display()))?;
    let opened = open(&sealed, &identity)?;
    drop(identity);
    let manifest = opened.manifest.clone();
    step(&format!(
        "Opened {}: {} ({}) taken {} of chart {}, every member's hash checked",
        asked.file.display(),
        manifest.release,
        manifest.deployment_id,
        manifest.taken_at,
        if manifest.chart_version.is_empty() {
            "?"
        } else {
            manifest.chart_version.as_str()
        }
    ));

    let release = asked
        .release
        .clone()
        .unwrap_or_else(|| manifest.release.clone());
    if release != manifest.release {
        return Err(format!(
            "the backup is of the release {}, and every object it restores is named for it: \
             restore it as {} (leave out --release)",
            manifest.release, manifest.release
        ));
    }
    let namespace = asked
        .namespace
        .clone()
        .filter(|ns| !ns.is_empty())
        .unwrap_or_else(|| manifest.namespace.clone());
    let ns = namespace.as_str();
    if ns.is_empty() {
        return Err("the backup names no namespace; say which with -n".into());
    }

    let values: serde_yaml::Value = serde_yaml::from_slice(opened.member("values/values.yaml")?)
        .map_err(|failed| format!("the backup's values could not be read: {failed}"))?;
    let full = fullname(&release, &values);
    let db_secret = opened.object("secrets", &manifest.database.secret);
    let database = database_plan(&manifest, &db_secret, &values)?;

    // Where, said before anything is changed.
    let context = text("kubectl", &["config", "current-context"]).await?;
    let cluster = context.trim().to_string();
    step(&format!(
        "Restoring into the cluster {cluster}, namespace {ns}"
    ));

    // An empty namespace, or none: no release, and nothing a deployment
    // keeps or would take up.
    if text("helm", &["status", &release, "-n", ns]).await.is_ok() {
        return Err(format!(
            "{ns} holds a release {release} already. Restore goes only into a namespace without \
             one: nothing was changed"
        ));
    }
    let names = |kind: &'static str| async move {
        text("kubectl", &["get", kind, "-n", ns, "-o", "name"])
            .await
            .unwrap_or_default()
            .lines()
            .map(|line| {
                line.split_once('/')
                    .map_or(line, |(_, name)| name)
                    .to_string()
            })
            .collect::<Vec<_>>()
    };
    let volume = match backup_volume(ns, &full, &values) {
        None => Ok(None),
        Some(name) => match text("kubectl", &["get", "pv", &name, "-o", "name"]).await {
            Ok(_) => Ok(Some(name)),
            Err(failed) if failed.contains("NotFound") || failed.contains("not found") => Ok(None),
            Err(failed) => Err(format!(
                "whether the backup volume {name} is left from a deleted namespace ({failed})"
            )),
        },
    };
    let present = Present {
        secrets: names("secrets").await,
        config_maps: names("configmaps").await,
        claims: names("persistentvolumeclaims").await,
        volume,
    };
    let left = left_behind(&manifest, &full, &values, &present);
    if !left.is_empty() {
        return Err(format!(
            "{ns} holds what a deployment keeps, so it was one once, and restore would make or \
             take up each:\n  {}\nRestore into a namespace without them; nothing was changed",
            left.join("\n  ")
        ));
    }
    let delete_volume = present.volume.clone().ok().flatten();

    // Restore's own rights, where it loads a database somebody else runs:
    // a container of backup.image in the conductor's pod, which reaches the
    // database by the cluster's own path, and kubectl exec into it.
    if matches!(database, DatabasePlan::Load { .. }) {
        for (verb, resource) in [
            ("patch", "pods/ephemeralcontainers"),
            ("create", "pods/exec"),
        ] {
            let allowed = text("kubectl", &["auth", "can-i", verb, resource, "-n", ns])
                .await
                .unwrap_or_default();
            if allowed.trim() != "yes" {
                return Err(format!(
                    "loading the backup's database into the one somebody else runs needs the \
                     right to {verb} {resource} in {ns}, which this account does not hold: \
                     nothing was changed"
                ));
            }
        }
    }

    // The chart restore installs, at the version the backup was taken on
    // unless told, from whatever reference: read as installed, and never
    // older than the backup's.
    let version = asked
        .chart_version
        .clone()
        .or_else(|| (!manifest.chart_version.is_empty()).then(|| manifest.chart_version.clone()));
    let with_version = |command: &[&'static str]| {
        let mut said: Vec<String> = command.iter().map(|word| word.to_string()).collect();
        said.push(asked.chart.clone());
        if let Some(version) = &version {
            said.extend(["--version".to_string(), version.clone()]);
        }
        said
    };
    let shown = with_version(&["show", "chart"]);
    let shown: Vec<&str> = shown.iter().map(String::as_str).collect();
    let chart =
        crate::upgrade_deployment::chart_of(&text("helm", &shown).await.map_err(|failed| {
            format!(
                "{} could not be read{}: {failed}. Nothing was changed",
                asked.chart,
                version
                    .as_deref()
                    .map(|v| format!(" at {v}"))
                    .unwrap_or_default()
            )
        })?)
        .ok_or_else(|| {
            format!(
                "{} states no name and version: nothing was changed",
                asked.chart
            )
        })?;
    let version_said = chart_version_said(&manifest.chart_version, &chart.version)?;

    // What was chosen, against the chart restore installs.
    let show = with_version(&["show", "values"]);
    let show: Vec<&str> = show.iter().map(String::as_str).collect();
    let defaults: serde_yaml::Value = serde_yaml::from_str(&crate::upgrade_deployment::document(
        &text("helm", &show).await?,
    ))
    .map_err(|failed| format!("the chart's defaults could not be read: {failed}"))?;
    let mut given = chosen(&values, &defaults);

    let namespace_exists = text("kubectl", &["get", "namespace", ns]).await.is_ok();
    let plan = Plan {
        file: asked.file.display().to_string(),
        release: release.clone(),
        deployment_id: manifest.deployment_id.clone(),
        taken_at: manifest.taken_at.clone(),
        cluster: cluster.clone(),
        namespace: namespace.clone(),
        namespace_exists,
        chart: asked.chart.clone(),
        version: chart.version.clone(),
        version_said,
        secrets: manifest.chart_secrets.len() + manifest.operator_secrets.len(),
        config_maps: manifest.config_maps.len(),
        key_generated: manifest.key == "generated",
        database: database.clone(),
        unchecked: present.volume.clone().err(),
        delete_volume,
    };
    if !approve(&plan_text(&plan)) {
        return Err(
            "not approved, so nothing was changed. Where there is no terminal to ask at, --yes \
             approves"
                .into(),
        );
    }

    // The volume a deleted namespace left, as approved: the object, never
    // the node's directory, which Retain keeps.
    if let Some(volume) = &plan.delete_volume {
        text("kubectl", &["delete", "pv", volume, "--wait=true"])
            .await
            .map_err(|failed| {
                format!("the backup volume {volume} could not be deleted ({failed}): nothing else was changed")
            })?;
        step(&format!(
            "Deleted the backup volume {volume}; the node's directory, with its backups, is kept"
        ));
    }

    if !namespace_exists {
        text("kubectl", &["create", "namespace", ns]).await?;
    }

    // The kept objects, before the chart, so it renders with them; the
    // database's URL withheld while a dump is to be loaded, so nothing runs
    // against a half-restored store.
    let withheld = manifest
        .database
        .included
        .then(|| manifest.database.secret.clone());
    let mut made = 0;
    for (name, adopt) in manifest
        .chart_secrets
        .iter()
        .map(|name| (name, Some(release.as_str())))
        .chain(manifest.operator_secrets.iter().map(|name| (name, None)))
    {
        let mut data = opened.object("secrets", name);
        if withheld.as_deref() == Some(name.as_str()) {
            data.clear();
        }
        create(&secret_object(name, ns, &data, adopt)).await?;
        made += 1;
    }
    for name in &manifest.config_maps {
        create(&config_map_object(
            name,
            ns,
            &opened.object("configmaps", name),
            &release,
        ))
        .await?;
    }
    let key_secret = format!("{full}-key");
    if manifest.key == "generated" {
        // The key the conductor made, now in a Secret of the administrator's
        // kind (values.yaml's key.existingSecret): a claim cannot be
        // pre-filled before the chart's own Job would make it.
        let key = BTreeMap::from([(
            "key.pem".to_string(),
            opened.member("key/key.pem")?.to_vec(),
        )]);
        create(&secret_object(&key_secret, ns, &key, None)).await?;
        let map = given
            .as_mapping_mut()
            .ok_or("the values are not a mapping")?;
        let mut key = serde_yaml::Mapping::new();
        key.insert("generate".into(), false.into());
        key.insert("existingSecret".into(), key_secret.clone().into());
        key.insert("key".into(), "key.pem".into());
        map.insert("key".into(), serde_yaml::Value::Mapping(key));
    }
    step(&format!(
        "Made {made} Secrets{}, the deployment's key{}",
        if withheld.is_some() {
            " (the database's URL withheld)"
        } else {
            ""
        },
        if manifest.key == "generated" {
            " among them"
        } else {
            " in the administrator's own"
        }
    ));

    // The chart, with what was chosen. Written to a file of this user's
    // alone, since the values may name credentials' Secrets, and removed.
    let values_file =
        std::env::temp_dir().join(format!("meridian-restore-{}.yaml", std::process::id()));
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&values_file)
            .map_err(|failed| format!("the values could not be written: {failed}"))?;
        file.write_all(
            serde_yaml::to_string(&given)
                .map_err(|failed| format!("the values could not be written: {failed}"))?
                .as_bytes(),
        )
        .map_err(|failed| format!("the values could not be written: {failed}"))?;
    }
    let values_path = values_file.to_string_lossy().into_owned();
    let mut install = vec![
        "install",
        release.as_str(),
        asked.chart.as_str(),
        "-n",
        ns,
        "-f",
        values_path.as_str(),
    ];
    if let Some(version) = &version {
        install.extend(["--version", version.as_str()]);
    }
    let installed = text("helm", &install).await;
    let _ = std::fs::remove_file(&values_file);
    installed?;
    step(&format!(
        "Installed {release} from {} {}",
        asked.chart, chart.version
    ));

    let mut report = format!(
        "Restored {release} into {ns} from {}.\n",
        asked.file.display()
    );
    let backed_up = || -> Result<BTreeMap<String, u64>, String> {
        Ok(counts(&String::from_utf8_lossy(
            opened.member("database/counts.tsv")?,
        )))
    };
    let checked = |database: &str,
                   restored: BTreeMap<String, u64>,
                   step: &mut dyn FnMut(&str),
                   report: &mut String|
     -> Result<(), String> {
        let differ = differences(&backed_up()?, &restored);
        if !differ.is_empty() {
            return Err(format!(
                "the restored database does not hold what the backup counted, so its URL is \
                 withheld and nothing runs against it:\n  {}",
                differ.join("\n  ")
            ));
        }
        let rows: u64 = restored.values().sum();
        step(&format!(
            "Restored {database}: {} tables and {rows} rows, each table's count as the backup counted it",
            restored.len()
        ));
        report.push_str(&format!(
            "  The database {database}: {} tables, {rows} rows, every count as backed up:\n{}",
            restored.len(),
            stores_text(&restored)
        ));
        Ok(())
    };
    match &database {
        DatabasePlan::Brought { .. } => {
            let serving = login(&utf8(&db_secret, "url")?)?;
            let migrating = login(&utf8(&db_secret, "migrate-url")?)?;
            let database = serving.database.clone();

            // The database this chart brings: started, as first run starts it.
            let workload_name = format!("{full}-database");
            let pod_name = format!("{workload_name}-0");
            let (workload, pod) = (workload_name.as_str(), pod_name.as_str());
            until(
                "the database's StatefulSet to exist",
                asked.timeout,
                move || async move {
                    text("kubectl", &["get", "statefulset", workload, "-n", ns])
                        .await
                        .ok()
                },
            )
            .await?;
            text(
                "kubectl",
                &["scale", "statefulset", workload, "-n", ns, "--replicas=1"],
            )
            .await?;
            until(
                "the database to answer",
                asked.timeout,
                move || async move {
                    // Over TCP: the image answers on its socket during its first,
                    // initialising start, and restarts after.
                    text(
                        "kubectl",
                        &[
                            "exec",
                            "-n",
                            ns,
                            pod,
                            "--",
                            "pg_isready",
                            "-h",
                            "127.0.0.1",
                            "-U",
                            "postgres",
                        ],
                    )
                    .await
                    .ok()
                },
            )
            .await?;
            let psql = [
                "exec", "-i", "-n", ns, pod, "--", "psql", "-X", "-q", "-U", "postgres", "-d",
                "postgres",
            ];
            run(
                "kubectl",
                &psql,
                Some(provision_sql(&serving, &migrating)?.as_bytes()),
            )
            .await
            .map_err(|_| {
                "the database's roles could not be made. psql's output is not shown, since it can \
                     quote a password; the database's URL is withheld"
                    .to_string()
            })?;
            let tables = text(
                "kubectl",
                &[
                    "exec", "-n", ns, pod, "--", "psql", "-X", "-At", "-U", "postgres", "-d",
                    &database, "-c", TABLES,
                ],
            )
            .await?;
            if tables.trim() != "0" {
                return Err(format!(
                    "the database {database} holds tables already, so it is not restored over. The \
                     database's URL is withheld, and nothing is running against it"
                ));
            }
            step(&format!(
                "Started the database, and made {database} and its two roles"
            ));

            let dump = opened.member("database/database.dump")?;
            run(
                "kubectl",
                &[
                    "exec",
                    "-i",
                    "-n",
                    ns,
                    pod,
                    "--",
                    "pg_restore",
                    "-U",
                    "postgres",
                    "-d",
                    &database,
                    "--exit-on-error",
                    "--single-transaction",
                ],
                Some(dump),
            )
            .await
            .map_err(|failed| {
                format!("the dump did not restore: {failed}. The database's URL is withheld")
            })?;
            let restored = counts(
                &text(
                    "kubectl",
                    &[
                        "exec", "-n", ns, pod, "--", "psql", "-X", "-At", "-U", "postgres", "-d",
                        &database, "-c", COUNTS,
                    ],
                )
                .await?,
            );
            checked(&database, restored, step, &mut report)?;
        }
        DatabasePlan::Load {
            place,
            database,
            image,
        } => {
            let migrating = utf8(&db_secret, "migrate-url").or_else(|_| utf8(&db_secret, "url"))?;
            let (pod, container) = loader(&release, ns, image, asked.timeout).await?;
            step(&format!(
                "Started {image} beside the conductor, to reach {place} as the conductor does"
            ));
            let tables = in_loader(
                &pod,
                &container,
                ns,
                &migrating,
                format!("{TABLES};\n").as_bytes(),
                true,
            )
            .await
            .map_err(|_| {
                format!(
                    "{place} could not be asked whether it is empty, as the migrating role. \
                         psql's output is not shown, since it can quote the URL; the database's \
                         URL is withheld, and nothing is running against it"
                )
            })?;
            if String::from_utf8_lossy(&tables).trim() != "0" {
                return Err(format!(
                    "{place} holds tables already, so the dump is not loaded over it. The \
                     database's URL is withheld, and nothing is running against it"
                ));
            }
            in_loader(
                &pod,
                &container,
                ns,
                &migrating,
                opened.member("database/database.dump")?,
                false,
            )
            .await
            .map_err(|failed| {
                format!(
                    "the dump did not load into {place}: {failed}. The database's URL is withheld"
                )
            })?;
            let restored = in_loader(
                &pod,
                &container,
                ns,
                &migrating,
                format!("{COUNTS};\n").as_bytes(),
                true,
            )
            .await
            .map_err(|_| {
                format!("{place}'s tables could not be counted. The database's URL is withheld")
            })?;
            checked(
                database,
                counts(&String::from_utf8_lossy(&restored)),
                step,
                &mut report,
            )?;
        }
        DatabasePlan::Reconnect { place } => report.push_str(&format!(
            "  The database {place} was not in the backup: it is the one your provider runs, which \
             the deployment reaches again with the restored URL.\n"
        )),
        DatabasePlan::BroughtEmpty => report.push_str(
            "  The database the chart brings was not in the backup (backup.database is false), \
             and starts empty.\n",
        ),
        DatabasePlan::None => {}
    }
    if matches!(
        database,
        DatabasePlan::Brought { .. } | DatabasePlan::Load { .. }
    ) {
        // Only now the URL: the migration and every component start.
        let name = manifest.database.secret.as_str();
        let adopt = manifest
            .chart_secrets
            .contains(&manifest.database.secret)
            .then_some(release.as_str());
        run(
            "kubectl",
            &["replace", "-n", ns, "-f", "-"],
            Some(
                secret_object(name, ns, &db_secret, adopt)
                    .to_string()
                    .as_bytes(),
            ),
        )
        .await?;
        step("Gave the components the database's URL");
        finish_first_run(&release, &full, ns, asked.timeout).await?;
        step("Restarted the conductor and the dashboard with it; first run gave up its rights");
    }

    // Every component of the release, ready; never the plugins the launcher
    // makes again, whose images were in the registry a backup does not hold.
    let selector_text = format!("app.kubernetes.io/instance={release},!meridian.dev/launched");
    let selector = selector_text.as_str();
    let seconds = format!("--timeout={}s", asked.timeout.as_secs());
    until(
        "the components to be made",
        asked.timeout,
        move || async move {
            text(
                "kubectl",
                &["get", "deployments", "-n", ns, "-l", selector, "-o", "name"],
            )
            .await
            .ok()
            .filter(|names| !names.trim().is_empty())
        },
    )
    .await?;
    text(
        "kubectl",
        &[
            "wait",
            "--for=condition=Available",
            "deployment",
            "-n",
            ns,
            "-l",
            selector,
            &seconds,
        ],
    )
    .await
    .map_err(|failed| format!("the components did not all become ready: {failed}"))?;
    step("Every component is ready");

    report.push_str(&format!(
        "  {} Secrets and the deployment's key, as they were: secret settings open with the \
         settings key they were sealed with, and every sidecar trusts the dashboard's key.\n",
        manifest.chart_secrets.len() + manifest.operator_secrets.len()
    ));

    // The launched plugins, each pod read for an image it could not pull,
    // for at most two minutes: only those to upload again are named.
    let launched_text = format!("app.kubernetes.io/instance={release},meridian.dev/launched=true");
    let launched = launched_text.as_str();
    let read = || async move {
        crate::upgrade_deployment::pods(
            &text(
                "kubectl",
                &["get", "pods", "-n", ns, "-l", launched, "-o", "json"],
            )
            .await
            .unwrap_or_default(),
        )
    };
    let wait = asked.timeout.min(Duration::from_secs(120));
    let started = Instant::now();
    let (pods, settled, missing) = loop {
        let pods = read().await;
        let (settled, missing) = unpulled(&pods);
        if settled || started.elapsed() >= wait {
            break (pods, settled, missing);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    if !missing.is_empty() {
        report.push_str(&format!(
            "  Launched plugins whose images are not in the deployment's registry, which a backup \
             does not hold: {}. Upload each again with `meridian plugin upload` from its \
             directory; an edge plugin's own storage starts empty, and it gathers again what \
             its vendor still serves.\n",
            missing
                .iter()
                .map(|(instance, image)| format!("{instance} ({image})"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !settled {
        let named: BTreeSet<String> = pods
            .iter()
            .filter(|pod| !pod.terminating && pod.ready_since.is_none())
            .filter_map(|pod| pod.labels.get("meridian.dev/instance").cloned())
            .filter(|instance| !missing.iter().any(|(held, _)| held == instance))
            .collect();
        if !named.is_empty() {
            report.push_str(&format!(
                "  Launched plugins not running within {}s, for a reason other than their image: \
                 {}. `meridian status {scope}` lists each plugin's readiness and restarts, and \
                 `meridian logs <instance> {scope}` what it printed.\n",
                wait.as_secs(),
                named.into_iter().collect::<Vec<_>>().join(", "),
                scope = crate::status::scope(&release, ns)
            ));
        }
    }
    Ok(report)
}

/// Whether a database holds a table of its own.
const TABLES: &str =
    "select count(*) from pg_tables where schemaname not in ('pg_catalog', 'information_schema')";

fn utf8(secret: &BTreeMap<String, Vec<u8>>, key: &str) -> Result<String, String> {
    secret
        .get(key)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .ok_or_else(|| format!("the backup's database Secret holds no {key}"))
}

/// A container of `image` in the conductor's running pod, as `kubectl debug`
/// adds one: no object of its own in the cluster, gone when the conductor
/// is restarted with the database's URL. It reaches the database by the
/// conductor's own path. Its pod and its name, once it runs.
async fn loader(
    release: &str,
    ns: &str,
    image: &str,
    timeout: Duration,
) -> Result<(String, String), String> {
    let selector_text =
        format!("app.kubernetes.io/instance={release},meridian.dev/component=conductor");
    let selector = selector_text.as_str();
    let pod = until("the conductor to run", timeout, move || async move {
        text(
            "kubectl",
            &[
                "get",
                "pods",
                "-n",
                ns,
                "-l",
                selector,
                "--field-selector=status.phase=Running",
                "-o",
                "jsonpath={.items[0].metadata.name}",
            ],
        )
        .await
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
    })
    .await?;
    let ephemeral = |pod: String| async move {
        text(
            "kubectl",
            &[
                "get",
                "pod",
                &pod,
                "-n",
                ns,
                "-o",
                "jsonpath={range .spec.ephemeralContainers[*]}{.name}{\"\\n\"}{end}",
            ],
        )
        .await
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect::<BTreeSet<String>>()
    };
    let before = ephemeral(pod.clone()).await;
    // Long enough for the load; the pod it is in is replaced at the end.
    let sleep = (timeout.as_secs() * 4 + 600).to_string();
    text(
        "kubectl",
        &[
            "debug",
            &pod,
            "-n",
            ns,
            &format!("--image={image}"),
            "--profile=baseline",
            "--",
            "sleep",
            &sleep,
        ],
    )
    .await
    .map_err(|failed| {
        format!("a container of {image} could not be added beside the conductor: {failed}")
    })?;
    let after = ephemeral(pod.clone()).await;
    let container = after
        .difference(&before)
        .next()
        .cloned()
        .ok_or("the container kubectl debug added could not be found")?;
    let (pod_ref, name_ref) = (pod.as_str(), container.as_str());
    until("the loading container to run", timeout, move || async move {
        text(
            "kubectl",
            &[
                "get",
                "pod",
                pod_ref,
                "-n",
                ns,
                "-o",
                &format!(
                    "jsonpath={{.status.ephemeralContainerStatuses[?(@.name==\"{name_ref}\")].state.running.startedAt}}"
                ),
            ],
        )
        .await
        .ok()
        .filter(|started| !started.trim().is_empty())
    })
    .await?;
    Ok((pod, container))
}

/// psql (`query`) or pg_restore in the loading container, against `url`,
/// which reaches it on its standard input's first line, never as an
/// argument kubectl sees; `input` follows it.
async fn in_loader(
    pod: &str,
    container: &str,
    ns: &str,
    url: &str,
    input: &[u8],
    query: bool,
) -> Result<Vec<u8>, String> {
    let script = if query {
        "IFS= read -r url && exec psql \"$url\" -X -q -At -v ON_ERROR_STOP=1"
    } else {
        "IFS= read -r url && exec pg_restore --dbname=\"$url\" --no-password --exit-on-error --single-transaction"
    };
    let mut stdin = Vec::with_capacity(url.len() + 1 + input.len());
    stdin.extend_from_slice(url.trim().as_bytes());
    stdin.push(b'\n');
    stdin.extend_from_slice(input);
    run(
        "kubectl",
        &[
            "exec", "-i", "-n", ns, pod, "-c", container, "--", "sh", "-c", script,
        ],
        Some(&stdin),
    )
    .await
}

/// What first run does once it has written the database's URL, done for the
/// URL restore gives last (meridian-core's first-run Job, its apply).
///
/// The conductor and the dashboard read the URL only when they start, and
/// both started before it was given: the conductor kept its configuration in
/// memory, answering for nobody whatever the restored database holds. And the
/// first-run Job, started when the URL was not there, was waiting with its
/// rights for a wizard nobody opens (decisions/016). So the Deployments the
/// chart names for the Job are restarted, its pod stopped, as the Job stops
/// once it has applied, and its RoleBinding deleted once it is done -- never
/// before: a pod stopped while running may be counted failed and made again,
/// and the new one finds the URL and gives up the rights itself, which it can
/// do only while it holds them. With no first-run Job (no dashboard), the
/// conductor alone is restarted.
async fn finish_first_run(
    release: &str,
    full: &str,
    ns: &str,
    timeout: Duration,
) -> Result<(), String> {
    let jobs: serde_json::Value = serde_json::from_str(
        &text(
            "kubectl",
            &[
                "get",
                "jobs",
                "-n",
                ns,
                "-l",
                &format!("app.kubernetes.io/instance={release}"),
                "-o",
                "json",
            ],
        )
        .await?,
    )
    .map_err(|failed| format!("kubectl's answer could not be read: {failed}"))?;
    let first_run = first_run_job(&jobs);
    let restart = match &first_run {
        Some(job) => job.restart.clone(),
        None => vec![format!("{full}-conductor")],
    };
    for name in &restart {
        text(
            "kubectl",
            &[
                "rollout",
                "restart",
                &format!("deployment/{name}"),
                "-n",
                ns,
            ],
        )
        .await?;
    }
    if let Some(job) = &first_run {
        text(
            "kubectl",
            &[
                "delete",
                "pod",
                "-n",
                ns,
                "-l",
                &format!("job-name={}", job.name),
                "--wait",
            ],
        )
        .await?;
        let name = job.name.as_str();
        until("first run to finish", timeout, move || async move {
            text(
                "kubectl",
                &[
                    "get",
                    "job",
                    name,
                    "-n",
                    ns,
                    "-o",
                    "jsonpath={.status.succeeded}",
                ],
            )
            .await
            .ok()
            .filter(|succeeded| !succeeded.trim().is_empty() && succeeded.trim() != "0")
        })
        .await?;
        if !job.binding.is_empty() {
            text(
                "kubectl",
                &[
                    "delete",
                    "rolebinding",
                    &job.binding,
                    "-n",
                    ns,
                    "--ignore-not-found",
                ],
            )
            .await?;
        }
    }
    let seconds = format!("--timeout={}s", timeout.as_secs());
    for name in &restart {
        text(
            "kubectl",
            &[
                "rollout",
                "status",
                &format!("deployment/{name}"),
                "-n",
                ns,
                &seconds,
            ],
        )
        .await
        .map_err(|failed| format!("{name} did not come back with the database's URL: {failed}"))?;
    }
    Ok(())
}

async fn create(object: &serde_json::Value) -> Result<(), String> {
    let name = object["metadata"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let ns = object["metadata"]["namespace"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    run(
        "kubectl",
        &["create", "-n", &ns, "-f", "-"],
        Some(object.to_string().as_bytes()),
    )
    .await
    .map(|_| ())
    .map_err(|failed| format!("{name} could not be made: {failed}"))
}

#[cfg(test)]
mod tests;

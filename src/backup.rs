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
//! half-restored store.
//!
//! Nothing here prints a secret's value, a password or the identity: names,
//! counts, sizes and hashes only. Every Secret reaches kubectl on its
//! standard input, never as an argument.

use std::collections::BTreeMap;
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
             backup.existingClaim in a values file, and apply it with \
             `meridian upgrade-deployment -f <file>`",
            asked.release
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

/// Restore a backup into an empty namespace, saying each step on `step` and
/// returning what was restored.
pub async fn restore(asked: &Restore, step: &mut dyn FnMut(&str)) -> Result<String, String> {
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

    // Where, said before anything is changed.
    let context = text("kubectl", &["config", "current-context"]).await?;
    step(&format!(
        "Restoring into the cluster {}, namespace {ns}",
        context.trim()
    ));

    // An empty namespace, or none.
    if text("helm", &["status", &release, "-n", ns]).await.is_ok() {
        return Err(format!(
            "{ns} holds a release {release} already. Restore goes only into a namespace without \
             one: nothing was changed"
        ));
    }
    let present: Vec<String> = text("kubectl", &["get", "secrets", "-n", ns, "-o", "name"])
        .await
        .unwrap_or_default()
        .lines()
        .map(|line| line.trim_start_matches("secret/").to_string())
        .collect();
    let clash: Vec<&String> = manifest
        .chart_secrets
        .iter()
        .chain(&manifest.operator_secrets)
        .filter(|name| present.contains(name))
        .collect();
    if !clash.is_empty() {
        return Err(format!(
            "{ns} holds Secrets the backup would restore, so it was a deployment once: {}. \
             Restore into a namespace without them; nothing was changed",
            clash
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if text("kubectl", &["get", "namespace", ns]).await.is_err() {
        text("kubectl", &["create", "namespace", ns]).await?;
    }

    // What was chosen, against the chart restore installs.
    let mut show = vec!["show", "values", asked.chart.as_str()];
    let version = asked.chart_version.clone().or_else(|| {
        (asked.chart == PUBLISHED_CHART && !manifest.chart_version.is_empty())
            .then(|| manifest.chart_version.clone())
    });
    if let Some(version) = &version {
        show.extend(["--version", version.as_str()]);
    }
    let defaults: serde_yaml::Value = serde_yaml::from_str(&text("helm", &show).await?)
        .map_err(|failed| format!("the chart's defaults could not be read: {failed}"))?;
    let mut given = chosen(&values, &defaults);

    // The kept objects, before the chart, so it renders with them.
    let withheld = manifest
        .database
        .included
        .then(|| manifest.database.secret.clone());
    let mut made = 0;
    for name in &manifest.chart_secrets {
        let mut data = opened.object("secrets", name);
        if withheld.as_deref() == Some(name.as_str()) {
            data.clear();
        }
        create(&secret_object(name, ns, &data, Some(&release))).await?;
        made += 1;
    }
    for name in &manifest.operator_secrets {
        create(&secret_object(
            name,
            ns,
            &opened.object("secrets", name),
            None,
        ))
        .await?;
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
        "Installed {release} from {}{}",
        asked.chart,
        version
            .as_deref()
            .map(|v| format!(" {v}"))
            .unwrap_or_default()
    ));

    let mut report = format!(
        "Restored {release} into {ns} from {}.\n",
        asked.file.display()
    );
    if manifest.database.included {
        if !manifest.database.brought {
            return Err(
                "the backup holds a dump of a database this chart does not bring, which restore \
                 does not load yet: restore it with your database provider's tools, then fill the \
                 database's Secret"
                    .into(),
            );
        }
        let secret = opened.object("secrets", &manifest.database.secret);
        let utf8 = |key: &str| {
            secret
                .get(key)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .ok_or_else(|| format!("the backup's database Secret holds no {key}"))
        };
        let serving = login(&utf8("url")?)?;
        let migrating = login(&utf8("migrate-url")?)?;
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
            &["exec", "-n", ns, pod, "--", "psql", "-X", "-At", "-U", "postgres", "-d", &database,
              "-c", "select count(*) from pg_tables where schemaname not in ('pg_catalog', 'information_schema')"],
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

        let backed_up = counts(&String::from_utf8_lossy(
            opened.member("database/counts.tsv")?,
        ));
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
        let differ = differences(&backed_up, &restored);
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
            "  The database {database}: {} tables, {rows} rows, every count as backed up.\n",
            restored.len()
        ));

        // Only now the URL: the migration and every component start.
        let name = manifest.database.secret.as_str();
        run(
            "kubectl",
            &["replace", "-n", ns, "-f", "-"],
            Some(
                secret_object(name, ns, &secret, Some(&release))
                    .to_string()
                    .as_bytes(),
            ),
        )
        .await?;
        step("Gave the components the database's URL");
    } else if !manifest.database.name.is_empty() {
        report.push_str(&format!(
            "  The database {} was not in the backup: it is the one your provider runs, which the \
             deployment reaches again with the restored URL.\n",
            manifest.database.name
        ));
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
    let launched = text(
        "kubectl",
        &[
            "get",
            "deployments",
            "-n",
            ns,
            "-l",
            &format!("app.kubernetes.io/instance={release},meridian.dev/launched=true"),
            "-o",
            "jsonpath={range .items[*]}{.metadata.labels.meridian\\.dev/instance}{\"\\n\"}{end}",
        ],
    )
    .await
    .unwrap_or_default();
    let launched: Vec<&str> = launched.lines().filter(|l| !l.is_empty()).collect();
    if !launched.is_empty() {
        report.push_str(&format!(
            "  Launched plugins, whose images were in the deployment's registry and are not in a \
             backup: {}. Upload each again with `meridian plugin upload`; an edge plugin's own \
             storage starts empty, and it gathers again what its vendor still serves.\n",
            launched.join(", ")
        ));
    }
    Ok(report)
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

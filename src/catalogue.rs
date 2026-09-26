//! `meridian plugin upload`, `list`, `launch` and `stop` (W8, spec/the-local-
//! plugin-registry), through a connected deployment's dashboard as its
//! administrator.
//!
//! Upload builds the plugin's image on this machine, as its Dockerfile says
//! -- on the base image of the SDK it pins -- reads it back with `docker
//! save`, and pushes it into the deployment's registry through the
//! dashboard, blob by blob in the registry's own protocol, skipping every
//! blob the registry already holds: the base, after the first plugin on it.
//! Then it sends the metadata from the plugin's pyproject.toml and the
//! image's digest, and the conductor records the version. What a plugin may
//! do is its roles', from that metadata; nothing here writes a grant.
//!
//! Launch shows the roles and tags a version declares and asks before
//! sending them as approved: the approval is the person's (W8.3).

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest as _, Sha256};

/// What a plugin says about itself, from its pyproject.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub name: String,
    pub version: String,
    pub roles: Vec<String>,
    pub tags: Vec<String>,
    pub interface: bool,
    pub sdk_version: String,
}

/// A host label, a letter first: a plugin's name, a role, a tag, an instance.
pub fn is_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1] != b'-'
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && !name.contains("--")
}

/// The metadata in a pyproject.toml, held to the forms the deployment will
/// hold it to. Which roles exist is the deployment's to say, and it does,
/// refusing one it does not have when the version is recorded.
pub fn metadata(pyproject: &str) -> Result<Metadata, String> {
    let project: toml::Table = pyproject
        .parse()
        .map_err(|failed| format!("pyproject.toml does not read: {failed}"))?;
    let text = |table: &toml::Table, key: &str| {
        table
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let list = |table: &toml::Table, key: &str| -> Result<Vec<String>, String> {
        match table.get(key) {
            None => Ok(Vec::new()),
            Some(value) => value
                .as_array()
                .ok_or(format!("[tool.meridian] {key} is not a list"))?
                .iter()
                .map(|item| {
                    item.as_str().map(String::from).ok_or(format!(
                        "[tool.meridian] {key} holds something that is not a name"
                    ))
                })
                .collect(),
        }
    };
    let section = project
        .get("project")
        .and_then(|v| v.as_table())
        .ok_or("pyproject.toml has no [project]")?;
    let name = text(section, "name");
    let version = text(section, "version");
    let sdk_version = section
        .get("dependencies")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|d| d.as_str())
        .find_map(|d| d.strip_prefix("open-meridian=="))
        .map(|v| v.trim().to_string())
        .ok_or("[project] dependencies do not pin open-meridian==<version>")?;
    let meridian = project
        .get("tool")
        .and_then(|t| t.get("meridian"))
        .and_then(|m| m.as_table())
        .ok_or(
            "pyproject.toml has no [tool.meridian]: what the plugin declares for its deployment",
        )?;
    let metadata = Metadata {
        roles: list(meridian, "roles")?,
        tags: list(meridian, "tags")?,
        interface: meridian
            .get("interface")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        name,
        version,
        sdk_version,
    };
    if !is_name(&metadata.name) {
        return Err(format!(
            "`{}` is not a plugin's name: lowercase letters, digits and single hyphens",
            metadata.name
        ));
    }
    if metadata.version.is_empty() {
        return Err("[project] has no version".into());
    }
    for part in metadata.roles.iter().chain(&metadata.tags) {
        if !is_name(part) {
            return Err(format!("`{part}` is not a role or a tag's form"));
        }
    }
    Ok(metadata)
}

/// An image's manifest and the blobs it names, from an OCI image layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub manifest: Vec<u8>,
    pub media_type: String,
    /// `sha256:<hex>` of the manifest: what a launch runs, never a tag.
    pub digest: String,
    /// The config and the layers, by digest, each a file in the layout.
    pub blobs: Vec<(String, PathBuf)>,
}

fn blob_path(layout: &Path, digest: &str) -> Result<PathBuf, String> {
    let hex = digest
        .strip_prefix("sha256:")
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or(format!("{digest} is not a sha256 digest"))?;
    Ok(layout.join("blobs").join("sha256").join(hex))
}

fn read_json(path: &Path) -> Result<(Vec<u8>, serde_json::Value), String> {
    let bytes = std::fs::read(path)
        .map_err(|failed| format!("{} could not be read: {failed}", path.display()))?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|failed| format!("{} is not JSON: {failed}", path.display()))?;
    Ok((bytes, value))
}

/// The image in a layout `docker save` wrote: its one manifest, following an
/// index to the manifest whose blobs the layout holds.
pub fn image(layout: &Path) -> Result<Image, String> {
    let (_, index) = read_json(&layout.join("index.json"))?;
    let mut candidates: Vec<serde_json::Value> =
        index["manifests"].as_array().cloned().unwrap_or_default();
    while let Some(descriptor) = candidates.first().cloned() {
        candidates.remove(0);
        let digest = descriptor["digest"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let path = blob_path(layout, &digest)?;
        if !path.exists() {
            continue;
        }
        let (bytes, manifest) = read_json(&path)?;
        let media_type = manifest["mediaType"]
            .as_str()
            .or_else(|| descriptor["mediaType"].as_str())
            .unwrap_or_default()
            .to_string();
        if media_type.contains("index") || media_type.contains("manifest.list") {
            let mut inner = manifest["manifests"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            inner.append(&mut candidates);
            candidates = inner;
            continue;
        }
        let computed = format!("sha256:{:x}", Sha256::digest(&bytes));
        if computed != digest {
            return Err(format!("the manifest's bytes are not {digest}"));
        }
        let mut blobs = Vec::new();
        for named in std::iter::once(&manifest["config"])
            .chain(manifest["layers"].as_array().into_iter().flatten())
        {
            let blob = named["digest"]
                .as_str()
                .ok_or("a manifest names a blob with no digest")?;
            let path = blob_path(layout, blob)?;
            if !path.exists() {
                return Err(format!("the layout does not hold {blob}"));
            }
            blobs.push((blob.to_string(), path));
        }
        return Ok(Image {
            manifest: bytes,
            media_type,
            digest,
            blobs,
        });
    }
    Err("the saved image holds no manifest this can push".into())
}

/// Where an upload goes next: the registry's Location, on the dashboard,
/// with the blob's digest added.
pub fn upload_url(address: &str, location: &str, digest: &str) -> String {
    let joined = if location.starts_with("http://") || location.starts_with("https://") {
        location.to_string()
    } else {
        format!("{address}{location}")
    };
    let glue = if joined.contains('?') { '&' } else { '?' };
    format!("{joined}{glue}digest={digest}")
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        // Long enough for a layer of a few hundred megabytes on a slow link.
        .timeout(Duration::from_secs(900))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|failed| failed.to_string())
}

fn said(status: reqwest::StatusCode, body: &str) -> String {
    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(String::from))
        .unwrap_or_else(|| body.trim().to_string());
    format!("{status}: {reason}")
}

/// Run a command, saying what it was when it fails.
async fn docker(args: &[&str]) -> Result<(), String> {
    let status = tokio::process::Command::new("docker")
        .args(args)
        .status()
        .await
        .map_err(|failed| format!("docker could not be run: {failed}"))?;
    if !status.success() {
        return Err(format!("`docker {}` failed", args.join(" ")));
    }
    Ok(())
}

/// Build, push and record: the version's digest once recorded.
pub async fn upload(address: &str, session: &str, dir: &Path) -> Result<String, String> {
    let pyproject = std::fs::read_to_string(dir.join("pyproject.toml"))
        .map_err(|failed| format!("{} has no pyproject.toml: {failed}", dir.display()))?;
    let metadata = metadata(&pyproject)?;
    let tag = format!("meridian-plugin/{}:{}", metadata.name, metadata.version);
    println!("Building {tag} from {} ...", dir.display());
    docker(&["build", "-t", &tag, &dir.display().to_string()]).await?;

    let scratch = std::env::temp_dir().join(format!("meridian-upload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|failed| failed.to_string())?;
    let saved = scratch.join("image.tar");
    docker(&["save", "-o", &saved.display().to_string(), &tag]).await?;
    let unpacked = tokio::process::Command::new("tar")
        .args([
            "-xf",
            &saved.display().to_string(),
            "-C",
            &scratch.display().to_string(),
        ])
        .status()
        .await
        .map_err(|failed| format!("tar could not be run: {failed}"))?;
    if !unpacked.success() {
        return Err("the saved image could not be unpacked".into());
    }
    let pushed = push(address, session, &metadata, &image(&scratch)?).await;
    let _ = std::fs::remove_dir_all(&scratch);
    let digest = pushed?;
    record(address, session, &metadata, &digest).await?;
    Ok(digest)
}

async fn push(
    address: &str,
    session: &str,
    metadata: &Metadata,
    image: &Image,
) -> Result<String, String> {
    let http = client()?;
    let repository = format!("{address}/terminal/registry/v2/plugins/{}", metadata.name);
    for (digest, path) in &image.blobs {
        let held = http
            .head(format!("{repository}/blobs/{digest}"))
            .bearer_auth(session)
            .send()
            .await
            .map_err(|failed| format!("could not reach {address}: {failed}"))?;
        if held.status().is_success() {
            println!("  {digest}: already there");
            continue;
        }
        let started = http
            .post(format!("{repository}/blobs/uploads/"))
            .bearer_auth(session)
            .send()
            .await
            .map_err(|failed| format!("could not reach {address}: {failed}"))?;
        let status = started.status();
        let location = started
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let Some(location) = location.filter(|_| status.is_success()) else {
            return Err(format!(
                "the registry did not start an upload: {}",
                said(status, &started.text().await.unwrap_or_default())
            ));
        };
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|failed| failed.to_string())?;
        let size = bytes.len();
        let sent = http
            .put(upload_url(address, &location, digest))
            .bearer_auth(session)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(|failed| format!("could not reach {address}: {failed}"))?;
        let status = sent.status();
        if !status.is_success() {
            return Err(format!(
                "{digest} was not taken: {}",
                said(status, &sent.text().await.unwrap_or_default())
            ));
        }
        println!("  {digest}: sent, {size} bytes");
    }
    let named = http
        .put(format!("{repository}/manifests/{}", metadata.version))
        .bearer_auth(session)
        .header("content-type", &image.media_type)
        .body(image.manifest.clone())
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = named.status();
    if !status.is_success() {
        return Err(format!(
            "the manifest was not taken: {}",
            said(status, &named.text().await.unwrap_or_default())
        ));
    }
    Ok(image.digest.clone())
}

async fn record(
    address: &str,
    session: &str,
    metadata: &Metadata,
    digest: &str,
) -> Result<(), String> {
    let answer = client()?
        .post(format!("{address}/terminal/plugins"))
        .bearer_auth(session)
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "name": metadata.name, "version": metadata.version, "roles": metadata.roles,
                "tags": metadata.tags, "interface": metadata.interface,
                "sdk_version": metadata.sdk_version, "image_digest": digest,
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = answer.status();
    let body = answer.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "the version was not recorded: {}",
            said(status, &body)
        ));
    }
    Ok(())
}

/// The catalogue, as the dashboard answers it.
pub async fn catalogue(address: &str, session: &str) -> Result<serde_json::Value, String> {
    let answer = client()?
        .get(format!("{address}/terminal/plugins"))
        .bearer_auth(session)
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = answer.status();
    let body = answer.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(said(status, &body));
    }
    serde_json::from_str(&body).map_err(|failed| failed.to_string())
}

/// The catalogue, for a person to read.
pub fn listed(catalogue: &serde_json::Value) -> String {
    let names = |value: &serde_json::Value| {
        let held: Vec<&str> = value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .collect();
        if held.is_empty() {
            "none".to_string()
        } else {
            held.join(", ")
        }
    };
    let mut out = String::from("Versions:\n");
    let versions = catalogue["versions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if versions.is_empty() {
        out.push_str("  none uploaded\n");
    }
    for v in &versions {
        out.push_str(&format!(
            "  {} {}  roles: {}  tags: {}  page: {}  SDK {}\n",
            v["name"].as_str().unwrap_or_default(),
            v["version"].as_str().unwrap_or_default(),
            names(&v["roles"]),
            names(&v["tags"]),
            if v["interface"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            v["sdk_version"].as_str().unwrap_or_default(),
        ));
    }
    out.push_str("Launches:\n");
    let launches = catalogue["launches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if launches.is_empty() {
        out.push_str("  none\n");
    }
    for l in &launches {
        let failure = l["failure"].as_str().unwrap_or_default();
        out.push_str(&format!(
            "  {}  {} {}  {}{}\n",
            l["instance_id"].as_str().unwrap_or_default(),
            l["name"].as_str().unwrap_or_default(),
            l["version"].as_str().unwrap_or_default(),
            l["state"].as_str().unwrap_or_default(),
            if failure.is_empty() {
                String::new()
            } else {
                format!(": {failure}")
            },
        ));
    }
    out
}

/// A recorded version's roles and tags, which a launch approves.
pub fn declared(
    catalogue: &serde_json::Value,
    name: &str,
    version: &str,
) -> Option<(Vec<String>, Vec<String>)> {
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    };
    catalogue["versions"].as_array()?.iter().find_map(|v| {
        (v["name"] == name && v["version"] == version)
            .then(|| (strings(&v["roles"]), strings(&v["tags"])))
    })
}

async fn post(
    address: &str,
    session: &str,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let answer = client()?
        .post(format!("{address}{path}"))
        .bearer_auth(session)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(said(status, &text));
    }
    serde_json::from_str(&text).map_err(|failed| failed.to_string())
}

pub async fn launch(
    address: &str,
    session: &str,
    name: &str,
    version: &str,
    instance: &str,
    roles: &[String],
    tags: &[String],
) -> Result<serde_json::Value, String> {
    post(
        address,
        session,
        "/terminal/plugins/launch",
        serde_json::json!({ "name": name, "version": version, "instance_id": instance,
                            "approved_roles": roles, "approved_tags": tags }),
    )
    .await
}

pub async fn stop(
    address: &str,
    session: &str,
    instance: &str,
) -> Result<serde_json::Value, String> {
    post(
        address,
        session,
        "/terminal/plugins/stop",
        serde_json::json!({ "instance_id": instance }),
    )
    .await
}

#[cfg(test)]
mod tests;

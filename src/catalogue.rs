//! `meridian plugin upload`, `list`, `launch` and `stop` (W8, spec/the-local-
//! plugin-registry), through a connected deployment's dashboard as its
//! administrator.
//!
//! Upload builds the plugin's image on this machine, as its Dockerfile says
//! -- on the base image of the SDK it pins -- reads it back with `docker
//! save`, and pushes it into the deployment's registry through the
//! dashboard, blob by blob in the registry's own protocol, sending no blob the
//! registry already holds. The registry keeps blobs per repository, so one
//! another plugin's repository holds -- the SDK's base, after the first
//! plugin on it -- is mounted from there rather than sent again.
//! Then it sends the metadata from the plugin's pyproject.toml, the
//! version's declaration as the built image prints it (contract v11: its
//! secret settings' names, what it receives and does not carry, the storage
//! it asks for, read with the SDK's `meridian-declaration` from the
//! `[tool.meridian] declaration` it names), and the image's digest, and the
//! conductor records the version. What a plugin may
//! do is its roles', from that metadata; nothing here writes a grant.
//!
//! From contract v25 the declaration is the whole of what the version says
//! about itself, and names each section of the vendor's documentation, its
//! `docs/`, by digest and size; upload runs `meridian-declaration
//! --documentation` in the same image, holds each text to its digest and the
//! bounds, sends the texts with the metadata, and says where the version's
//! documentation is read (W8.1, W8.2).
//!
//! Launch shows the roles a version declares and asks before sending them as
//! approved: the approval is the person's (W8.3). A plugin declares no tags
//! (decisions/026), and a pyproject.toml that still does is refused here,
//! before anything is built or sent.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest as _, Sha256};

use crate::check::EDGE_ROLES;
use crate::credential::Credential;
use crate::live::Failed;

/// What a plugin says about itself, from its pyproject.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub name: String,
    pub version: String,
    pub roles: Vec<String>,
    pub interface: bool,
    pub sdk_version: String,
    /// The `module:attribute` of its meridian.Declaration, where it names one.
    pub declaration: Option<String>,
}

/// A host label, a letter first: a plugin's name, a role, an instance.
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
    if meridian.contains_key("tags") {
        return Err(
            "[tool.meridian] declares `tags`, and a plugin declares none: a person's access \
             to a plugin is read or write, the same for every plugin, granted in the \
             deployment's access groups (decisions/026). Remove the `tags` line from \
             pyproject.toml"
                .into(),
        );
    }
    let metadata = Metadata {
        roles: list(meridian, "roles")?,
        interface: meridian
            .get("interface")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        declaration: meridian
            .get("declaration")
            .and_then(|v| v.as_str())
            .map(String::from),
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
    for role in &metadata.roles {
        if !is_name(role) {
            return Err(format!("`{role}` is not a role's form"));
        }
    }
    if let Some(named) = &metadata.declaration {
        let (module, attribute) = named.split_once(':').unwrap_or((named, ""));
        if module.is_empty() || attribute.is_empty() {
            return Err(format!(
                "[tool.meridian] declaration `{named}` is not `<module>:<attribute>`"
            ));
        }
    }
    Ok(metadata)
}

/// The declaration the image printed, held to what the deployment will hold
/// it to before anything is sent: JSON, and storage asked for only by a
/// plugin holding an edge role.
pub fn declaration(printed: &str, roles: &[String]) -> Result<serde_json::Value, String> {
    let declared: serde_json::Value = serde_json::from_str(printed.trim())
        .map_err(|failed| format!("the declaration the image printed is not JSON: {failed}"))?;
    if !declared.is_object() {
        return Err("the declaration the image printed is not an object".into());
    }
    let storage = declared.get("storage").is_some_and(|s| !s.is_null());
    if storage && !roles.iter().any(|role| EDGE_ROLES.contains(&role.as_str())) {
        return Err(format!(
            "the declaration asks for storage, and the plugin holds no edge role: only {} own \
             storage for their raw records (decisions/028)",
            EDGE_ROLES.join(", ")
        ));
    }
    Ok(declared)
}

/// The documentation's texts the image printed (`meridian-declaration
/// --documentation`), held to what its declaration names before anything is
/// sent: each a section of the fixed set, its text's digest and size the
/// declaration's, within the bounds the conductor holds them to. None where
/// the declaration names no documentation.
pub fn documentation(
    printed: &str,
    declared: &serde_json::Value,
) -> Result<Option<serde_json::Value>, String> {
    let named = declared["documentation"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if named.is_empty() {
        return Ok(None);
    }
    let texts: Vec<serde_json::Value> = serde_json::from_str(printed.trim()).map_err(|failed| {
        format!("the documentation the image printed is not a JSON list: {failed}")
    })?;
    if texts.len() != named.len() {
        return Err(format!(
            "the declaration names {} section(s), and the image printed {}",
            named.len(),
            texts.len()
        ));
    }
    let mut total = 0;
    for (text, held) in texts.iter().zip(&named) {
        let section = text["section"].as_str().unwrap_or_default();
        let markdown = text["markdown"].as_str().unwrap_or_default();
        if !crate::check::SECTIONS.contains(&section) {
            return Err(format!(
                "`{section}` is not a section: overview, setup, troubleshooting, terms or pages"
            ));
        }
        if held["name"].as_str() != Some(section) {
            return Err(format!(
                "the declaration names {} where the image printed {section}",
                held["name"]
            ));
        }
        let bytes = markdown.len();
        let digest = format!("sha256:{:x}", Sha256::digest(markdown.as_bytes()));
        if held["digest"].as_str() != Some(digest.as_str())
            || held["bytes"].as_u64() != Some(bytes as u64)
        {
            return Err(format!(
                "docs/{section}.md is not what the declaration names: {digest}, {bytes} bytes"
            ));
        }
        if bytes == 0 || bytes > crate::check::SECTION_BYTES {
            return Err(format!(
                "docs/{section}.md is {bytes} bytes, and a section is 1 to {}",
                crate::check::SECTION_BYTES
            ));
        }
        total += bytes;
    }
    if total > crate::check::BUNDLE_BYTES {
        return Err(format!(
            "the sections hold {total} bytes together, and the most is {}",
            crate::check::BUNDLE_BYTES
        ));
    }
    Ok(Some(serde_json::Value::Array(
        texts
            .into_iter()
            .map(|text| serde_json::json!({ "section": text["section"], "markdown": text["markdown"] }))
            .collect(),
    )))
}

/// Where a deployment admin reads a version's documentation before it is
/// launched: the portal's versions list, each version's Documentation page
/// (W8.2, W6.9). PLACEHOLDER (CLI 0.1.46): core's v25 has not drawn that page
/// yet, and design names no path; aligned with core's route before this is
/// pushed.
pub fn version_documentation(address: &str, name: &str, version: &str) -> String {
    format!("{address}/admin/plugins/versions/{name}/{version}/documentation")
}

/// Where anybody holding a level on a launched instance reads its
/// documentation: the Documentation tab of its area (W6.9), beside Summary.
pub fn instance_documentation(address: &str, instance: &str) -> String {
    format!("{address}/plugins/{instance}?tab=documentation")
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

fn client() -> Result<reqwest::Client, Failed> {
    crate::authority::trusted_here(reqwest::Client::builder())
        // Long enough for a layer of a few hundred megabytes on a slow link.
        .timeout(Duration::from_secs(900))
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(crate::release::naming_this_version())
        .build()
        .map_err(|failed| Failed::Refused(failed.to_string()))
}

/// What the dashboard refused, said as live.rs says it: a missing or lapsed
/// session (401, on any `/terminal/` path, the registry's included) apart
/// from everything else, and anything else after what was being done.
fn refused(address: &str, doing: &str, status: reqwest::StatusCode, body: &str) -> Failed {
    match crate::live::refusal(address, status, body) {
        Failed::Refused(said) if !doing.is_empty() => Failed::Refused(format!("{doing}: {said}")),
        other => other,
    }
}

fn unreachable(address: &str, failed: reqwest::Error) -> Failed {
    Failed::Refused(format!("could not reach {address}: {failed}"))
}

/// Run a command, saying what it was when it fails.
/// What `docker <args>` printed on its standard output.
async fn docker_output(args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("docker")
        .args(args)
        .output()
        .await
        .map_err(|failed| format!("docker could not be run: {failed}"))?;
    if !output.status.success() {
        return Err(format!(
            "`docker {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

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

/// What an upload recorded: the image's digest, and whether the version
/// carried its vendor's documentation.
pub struct Uploaded {
    pub digest: String,
    pub documented: bool,
    pub name: String,
    pub version: String,
}

/// Build, push and record: the version once recorded.
pub async fn upload(address: &str, session: &Credential, dir: &Path) -> Result<Uploaded, Failed> {
    let pyproject = std::fs::read_to_string(dir.join("pyproject.toml"))
        .map_err(|failed| format!("{} has no pyproject.toml: {failed}", dir.display()))?;
    let metadata = metadata(&pyproject)?;
    let tag = format!("meridian-plugin/{}:{}", metadata.name, metadata.version);
    eprintln!("Building {tag} from {} ...", dir.display());
    docker(&["build", "-t", &tag, &dir.display().to_string()]).await?;
    let declared = match &metadata.declaration {
        None => None,
        Some(named) => {
            let printed = docker_output(&[
                "run",
                "--rm",
                "--network",
                "none",
                "--entrypoint",
                "meridian-declaration",
                &tag,
                named,
            ])
            .await?;
            Some(declaration(&printed, &metadata.roles)?)
        }
    };
    let texts = match (&metadata.declaration, &declared) {
        (Some(named), Some(held))
            if held["documentation"]
                .as_array()
                .is_some_and(|d| !d.is_empty()) =>
        {
            let printed = docker_output(&[
                "run",
                "--rm",
                "--network",
                "none",
                "--entrypoint",
                "meridian-declaration",
                &tag,
                "--documentation",
                named,
            ])
            .await?;
            documentation(&printed, held)?
        }
        _ => None,
    };

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
        return Err(Failed::Refused(
            "the saved image could not be unpacked".into(),
        ));
    }
    let pushed = push(address, session, &metadata, &image(&scratch)?).await;
    let _ = std::fs::remove_dir_all(&scratch);
    let digest = pushed?;
    record(
        address,
        session,
        &metadata,
        declared.as_ref(),
        texts.as_ref(),
        &digest,
    )
    .await?;
    Ok(Uploaded {
        digest,
        documented: texts.is_some(),
        name: metadata.name,
        version: metadata.version,
    })
}

async fn push(
    address: &str,
    session: &Credential,
    metadata: &Metadata,
    image: &Image,
) -> Result<String, Failed> {
    let http = client()?;
    let plugins = format!("{address}/terminal/registry/v2/plugins");
    let repository = format!("{plugins}/{}", metadata.name);
    // A blob is held or not; a session the dashboard no longer knows is
    // neither, and is said as what it is rather than sent for.
    let holds = |url: String| {
        let http = http.clone();
        async move {
            let answer = http
                .head(url)
                .bearer_auth(session.bearer().await?)
                .send()
                .await
                .map_err(|failed| unreachable(address, failed))?;
            if answer.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(refused(address, "", answer.status(), ""));
            }
            Ok(answer.status().is_success())
        }
    };
    let others = others(address, session, &metadata.name).await;
    for (digest, path) in &image.blobs {
        if holds(format!("{repository}/blobs/{digest}")).await? {
            eprintln!("  {digest}: already there");
            continue;
        }
        let mut from = None;
        for other in &others {
            if holds(format!("{plugins}/{other}/blobs/{digest}")).await? {
                from = Some(other.as_str());
                break;
            }
        }
        let started = http
            .post(match from {
                Some(other) => mount_url(&repository, digest, other),
                None => format!("{repository}/blobs/uploads/"),
            })
            .bearer_auth(session.bearer().await?)
            .send()
            .await
            .map_err(|failed| unreachable(address, failed))?;
        let status = started.status();
        // Mounted: linked into this repository, nothing sent. A registry that
        // will not mount opens an upload instead, which is sent as ever.
        if let (Some(other), reqwest::StatusCode::CREATED) = (from, status) {
            eprintln!("  {digest}: already there, in plugins/{other}");
            continue;
        }
        let location = started
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let Some(location) = location.filter(|_| status.is_success()) else {
            return Err(refused(
                address,
                "the registry did not start an upload",
                status,
                &started.text().await.unwrap_or_default(),
            ));
        };
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|failed| Failed::Refused(failed.to_string()))?;
        let size = bytes.len();
        let sent = http
            .put(upload_url(address, &location, digest))
            .bearer_auth(session.bearer().await?)
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(|failed| unreachable(address, failed))?;
        let status = sent.status();
        if !status.is_success() {
            return Err(refused(
                address,
                &format!("{digest} was not taken"),
                status,
                &sent.text().await.unwrap_or_default(),
            ));
        }
        eprintln!("  {digest}: sent, {size} bytes");
    }
    let named = http
        .put(format!("{repository}/manifests/{}", metadata.version))
        .bearer_auth(session.bearer().await?)
        .header("content-type", &image.media_type)
        .body(image.manifest.clone())
        .send()
        .await
        .map_err(|failed| unreachable(address, failed))?;
    let status = named.status();
    if !status.is_success() {
        return Err(refused(
            address,
            "the manifest was not taken",
            status,
            &named.text().await.unwrap_or_default(),
        ));
    }
    Ok(image.digest.clone())
}

/// The other plugins in the catalogue, whose repositories may hold blobs
/// this one shares. None when the catalogue cannot be read: then every blob
/// is sent, which is slower and no less right.
async fn others(address: &str, session: &Credential, name: &str) -> Vec<String> {
    let held = catalogue(address, session).await.unwrap_or_default();
    let named: std::collections::BTreeSet<String> = held["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["name"].as_str())
        .filter(|other| *other != name && is_name(other))
        .map(String::from)
        .collect();
    named.into_iter().collect()
}

/// Starting an upload that is a mount: `digest`, linked from another
/// plugin's repository into this one.
pub fn mount_url(repository: &str, digest: &str, other: &str) -> String {
    format!("{repository}/blobs/uploads/?mount={digest}&from=plugins/{other}")
}

/// What recording a version sends: its metadata, the image's digest, and the
/// declaration as the image printed it, whole. Every key the SDK writes is
/// carried as written -- from contract v22 `algorithms`, which SDK 0.26.0
/// writes only for a version declaring one -- so a key the deployment reads
/// needs no release of this binary to reach it. From contract v25 the
/// documentation's texts beside it (`documentation`, each a section and its
/// Markdown: RecordPluginUploadRequest.documentation's DocumentationText).
pub fn record_body(
    metadata: &Metadata,
    declaration: Option<&serde_json::Value>,
    texts: Option<&serde_json::Value>,
    digest: &str,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "name": metadata.name, "version": metadata.version, "roles": metadata.roles,
        "interface": metadata.interface,
        "sdk_version": metadata.sdk_version, "image_digest": digest,
    });
    if let Some(declaration) = declaration {
        body["declaration"] = declaration.clone();
    }
    if let Some(texts) = texts {
        body["documentation"] = texts.clone();
    }
    body
}

async fn record(
    address: &str,
    session: &Credential,
    metadata: &Metadata,
    declaration: Option<&serde_json::Value>,
    texts: Option<&serde_json::Value>,
    digest: &str,
) -> Result<(), Failed> {
    let http = client()?;
    let body = record_body(metadata, declaration, texts, digest).to_string();
    let (status, body, _) = session
        .send(|bearer| {
            http.post(format!("{address}/terminal/plugins"))
                .bearer_auth(bearer)
                .header("content-type", "application/json")
                .body(body.clone())
        })
        .await?;
    if !status.is_success() {
        return Err(refused(
            address,
            "the version was not recorded",
            status,
            &body,
        ));
    }
    Ok(())
}

/// The catalogue, as the dashboard answers it.
pub async fn catalogue(address: &str, session: &Credential) -> Result<serde_json::Value, Failed> {
    let http = client()?;
    let (status, body, _) = session
        .send(|bearer| {
            http.get(format!("{address}/terminal/plugins"))
                .bearer_auth(bearer)
        })
        .await?;
    if !status.is_success() {
        return Err(refused(address, "", status, &body));
    }
    serde_json::from_str(&body).map_err(|failed| Failed::Refused(failed.to_string()))
}

/// The catalogue, for a person to read: from contract v25 each version
/// carrying its vendor's documentation with where it is read.
pub fn listed(catalogue: &serde_json::Value, address: &str) -> String {
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
            "  {} {}  roles: {}  page: {}  SDK {}\n",
            v["name"].as_str().unwrap_or_default(),
            v["version"].as_str().unwrap_or_default(),
            names(&v["roles"]),
            if v["interface"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            v["sdk_version"].as_str().unwrap_or_default(),
        ));
        if v["declaration"]["documentation"]
            .as_array()
            .is_some_and(|sections| !sections.is_empty())
        {
            out.push_str(&format!(
                "    documentation: {}\n",
                version_documentation(
                    address,
                    v["name"].as_str().unwrap_or_default(),
                    v["version"].as_str().unwrap_or_default()
                )
            ));
        }
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

/// A recorded version's roles, which a launch approves.
pub fn declared(catalogue: &serde_json::Value, name: &str, version: &str) -> Option<Vec<String>> {
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    };
    catalogue["versions"]
        .as_array()?
        .iter()
        .find_map(|v| (v["name"] == name && v["version"] == version).then(|| strings(&v["roles"])))
}

async fn post(
    address: &str,
    session: &Credential,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, Failed> {
    let http = client()?;
    let body = body.to_string();
    let (status, text, _) = session
        .send(|bearer| {
            http.post(format!("{address}{path}"))
                .bearer_auth(bearer)
                .header("content-type", "application/json")
                .body(body.clone())
        })
        .await?;
    if !status.is_success() {
        return Err(refused(address, "", status, &text));
    }
    serde_json::from_str(&text).map_err(|failed| Failed::Refused(failed.to_string()))
}

/// What a launch asks for: a recorded version, the instance it runs as, what
/// was approved of it, and whether it runs live (W8.3).
pub struct Launch<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub instance: &'a str,
    pub roles: &'a [String],
    pub live: bool,
}

pub async fn launch(
    address: &str,
    session: &Credential,
    asked: &Launch<'_>,
) -> Result<serde_json::Value, Failed> {
    post(
        address,
        session,
        "/terminal/plugins/launch",
        serde_json::json!({ "name": asked.name, "version": asked.version,
                            "instance_id": asked.instance, "approved_roles": asked.roles,
                            "live": asked.live }),
    )
    .await
}

pub async fn stop(
    address: &str,
    session: &Credential,
    instance: &str,
) -> Result<serde_json::Value, Failed> {
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

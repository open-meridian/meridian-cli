//! A `plugin bundle`: a listed version carried to a deployment that reaches
//! nothing (contract v25; spec/plugins-are-shared-through-registries,
//! requirements 44 and 45; W8.14).
//!
//! **`meridian plugin download organisation/plugin@version`**, on a connected
//! machine, resolves the version at the platform under the person's sign-in
//! (DownloadListedVersion, counted there as a bundle), pulls the image and
//! its referrers by digest from where the answer says -- the provenance, the
//! bill of materials, the scan results, the listing statement and the image's
//! metadata, each an OCI referrer of the image -- with the platform's signed
//! key status beside them, and **verifies everything before it writes**:
//!
//! - every manifest and blob is the digest it is named by;
//! - the provenance and the listing statement are DSSE envelopes the
//!   platform's key signed, and name this image's digest;
//! - the listing statement names this listing, version and registry;
//! - the key status is the platform's, signed, and the provenance's signer
//!   key is in it, not revoked before the version was listed.
//!
//! Any check that fails or cannot be made refuses the bundle, naming it, and
//! nothing is written. The platform's keys are the ones this release of the
//! CLI carries, as each chart release carries them (requirement 23).
//!
//! The bundle is a tar holding an OCI image layout -- the image, its blobs,
//! its referrers -- and `meridian-bundle.json`: the listing, version and
//! registry, the image's digest, its metadata and the documentation's texts,
//! and the dated key status.
//!
//! **`meridian plugin upload --bundle <file>`** carries it into a
//! deployment's local registry, under `plugins/{organisation}/{plugin}`,
//! through the dashboard as upload does, and records the version with its
//! listing, source registry and key status; the conductor verifies it again
//! against the chart's keys and shows the key status's age.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use crate::credential::Credential;
use crate::live::Failed;

/// The listing statement's predicate type (approved, item 105).
pub const LISTING: &str = "https://open-meridian.dev/attestation/plugin-listing/v1";
/// The builder's provenance, SLSA's.
pub const PROVENANCE: &str = "https://slsa.dev/provenance/v1";
/// The image's metadata the builder attests: its PluginMetadata and the
/// documentation's texts. The platform's build names it, not yet approved:
/// `<NAME?:https://open-meridian.dev/attestation/plugin-metadata/v1>`.
pub const METADATA: &str = "https://open-meridian.dev/attestation/plugin-metadata/v1";
/// DSSE's payload type for an in-toto statement.
pub const IN_TOTO: &str = "application/vnd.in-toto+json";
/// What SigningKeyStatus's signature is made under (the platform's encoding).
pub const KEY_STATUS_TYPE: &str = "application/vnd.open-meridian.signing-key-status+json";
/// The bundle's own record, beside its image layout.
pub const RECORD: &str = "meridian-bundle.json";

/// The platform's public keys this release carries, PEM, the current one and
/// the next. PLACEHOLDER (CLI 0.1.46): the KMS key is created at the
/// platform's v25 deploy (the v25 plan, step 2, item 143), and its public
/// half is written here before this release; until then a bundle cannot be
/// verified, and none is written.
pub const PLATFORM_KEYS: [&str; 0] = [];

const OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";
const OCI_INDEX: &str = "application/vnd.oci.image.index.v1+json";
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.manifest.v1+json, \
     application/vnd.oci.image.index.v1+json, \
     application/vnd.docker.distribution.manifest.v2+json";

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn hex_of(digest: &str) -> Result<&str, String> {
    digest
        .strip_prefix("sha256:")
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("{digest} is not a sha256 digest"))
}

// ── Canonical JSON and signatures ────────────────────────────────────────

/// A document as the platform signs it: sorted keys, no spaces, every
/// character past ASCII escaped (Python's `json.dumps(sort_keys=True,
/// separators=(",", ":"))`).
pub fn canonical(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (n, key) in keys.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_canonical(&map[*key], out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        serde_json::Value::String(text) => write_string(text, out),
        other => out.push_str(&other.to_string()),
    }
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            other => {
                let mut units = [0u16; 2];
                for unit in other.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// DSSE's pre-authentication encoding.
pub fn pae(payload_type: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "DSSEv1 {} {payload_type} {} ",
        payload_type.len(),
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// A P-256 public key's point, from its PEM (SubjectPublicKeyInfo).
fn p256_point(pem: &str) -> Result<Vec<u8>, String> {
    const PREFIX: [u8; 26] = [
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
    ];
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    let der = STANDARD
        .decode(body.trim())
        .map_err(|failed| format!("a platform key is not PEM: {failed}"))?;
    if der.len() != PREFIX.len() + 65 || der[..PREFIX.len()] != PREFIX {
        return Err("a platform key is not an ECDSA P-256 public key".into());
    }
    Ok(der[PREFIX.len()..].to_vec())
}

/// Whether `signature` (ASN.1 DER, base64) over `message` verifies with one
/// of `keys`.
pub fn signed_by(keys: &[String], message: &[u8], signature: &str) -> bool {
    use aws_lc_rs::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
    let Ok(signature) = STANDARD.decode(signature.trim()) else {
        return false;
    };
    keys.iter().any(|pem| {
        p256_point(pem).is_ok_and(|point| {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, point)
                .verify(message, &signature)
                .is_ok()
        })
    })
}

/// The in-toto statement a DSSE envelope carries, if a signature on it
/// verifies with one of `keys`.
pub fn verified_statement(
    envelope: &serde_json::Value,
    keys: &[String],
) -> Option<serde_json::Value> {
    let payload_type = envelope["payloadType"].as_str()?;
    let body = STANDARD.decode(envelope["payload"].as_str()?).ok()?;
    let message = pae(payload_type, &body);
    let signed = envelope["signatures"]
        .as_array()?
        .iter()
        .filter_map(|signature| signature["sig"].as_str())
        .any(|signature| signed_by(keys, &message, signature));
    signed.then(|| serde_json::from_slice(&body).ok()).flatten()
}

/// Whether the platform signed the key status (SigningKeyStatus.signature).
pub fn key_status_signed(status: &serde_json::Value, keys: &[String]) -> bool {
    let Some(signature) = status["signature"].as_str() else {
        return false;
    };
    let mut document = status.clone();
    if let Some(map) = document.as_object_mut() {
        map.remove("signature");
    }
    signed_by(
        keys,
        &pae(KEY_STATUS_TYPE, canonical(&document).as_bytes()),
        signature,
    )
}

/// A statement's predicate type, unverified, to tell envelopes apart.
fn predicate_of(envelope: &serde_json::Value) -> String {
    envelope["payload"]
        .as_str()
        .and_then(|payload| STANDARD.decode(payload).ok())
        .and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok())
        .and_then(|statement| statement["predicateType"].as_str().map(String::from))
        .unwrap_or_default()
}

fn names_subject(statement: &serde_json::Value, digest: &str) -> bool {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    statement["subject"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|subject| subject["digest"]["sha256"].as_str() == Some(hex))
}

// ── The bundle, in memory ───────────────────────────────────────────────

/// A version's bytes, by digest, and what is known about it.
#[derive(Debug, Clone, Default)]
pub struct Bundle {
    pub listing: String,
    pub version: String,
    pub registry: String,
    pub image_digest: String,
    pub image_media_type: String,
    /// Every manifest and blob, by digest: the image's and its referrers'.
    pub blobs: BTreeMap<String, Vec<u8>>,
    /// The referrers' manifests, by digest, with their media types.
    pub referrers: Vec<(String, String)>,
    /// PluginMetadata, as the image's metadata attestation carries it.
    pub metadata: serde_json::Value,
    /// Each section of the documentation: section and markdown.
    pub documentation: serde_json::Value,
    pub key_status: serde_json::Value,
    /// The listing statement as the platform carries it on the listed
    /// version (`ListedVersion.listing_statement`, a DSSE envelope as JSON;
    /// DESIGN25 amended at b0c4459), where it reached this beside the
    /// image's referrer.
    pub listing_statement: Option<serde_json::Value>,
}

/// What a referrer's layers hold that is a DSSE envelope, from the blobs.
fn envelopes(bundle: &Bundle) -> Vec<serde_json::Value> {
    let mut found = Vec::new();
    for (digest, _) in &bundle.referrers {
        let Some(manifest) = bundle
            .blobs
            .get(digest)
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        else {
            continue;
        };
        for layer in manifest["layers"].as_array().into_iter().flatten() {
            let Some(bytes) = layer["digest"].as_str().and_then(|d| bundle.blobs.get(d)) else {
                continue;
            };
            if let Ok(envelope) = serde_json::from_slice::<serde_json::Value>(bytes) {
                if envelope.get("payloadType").is_some() {
                    found.push(envelope);
                }
            }
        }
    }
    found
}

/// Every check a bundle is held to before it is written, or carried in.
/// The metadata and texts it carries, from the verified metadata attestation.
pub fn verify(bundle: &mut Bundle, keys: &[String]) -> Result<(), String> {
    if keys.is_empty() {
        return Err(
            "this meridian carries none of the platform's keys yet, so nothing it downloads can be \
             verified, and no bundle is written"
                .into(),
        );
    }
    for (digest, bytes) in &bundle.blobs {
        if sha256(bytes) != *digest {
            return Err(format!("{digest} is not the bytes it is named by"));
        }
    }
    let manifest: serde_json::Value = bundle
        .blobs
        .get(&bundle.image_digest)
        .and_then(|bytes| serde_json::from_slice(bytes).ok())
        .ok_or_else(|| format!("the image's manifest, {}, is missing", bundle.image_digest))?;
    for named in std::iter::once(&manifest["config"])
        .chain(manifest["layers"].as_array().into_iter().flatten())
    {
        let digest = named["digest"].as_str().unwrap_or_default();
        if !bundle.blobs.contains_key(digest) {
            return Err(format!("the image's blob {digest} is missing"));
        }
    }
    let mut found = envelopes(bundle);
    found.extend(bundle.listing_statement.clone());
    let by = |predicate: &str| {
        found
            .iter()
            .filter(|envelope| {
                envelope["payloadType"].as_str() == Some(IN_TOTO)
                    && predicate_of(envelope) == predicate
            })
            .find_map(|envelope| verified_statement(envelope, keys))
    };
    let provenance = by(PROVENANCE).ok_or(
        "no provenance the platform's key signed is attached to the image: the builder's is \
         required",
    )?;
    let listing = by(LISTING)
        .ok_or("no listing statement the platform's key signed is attached to the image")?;
    for (statement, what) in [(&provenance, "provenance"), (&listing, "listing statement")] {
        if !names_subject(statement, &bundle.image_digest) {
            return Err(format!(
                "the {what} names another image than {}",
                bundle.image_digest
            ));
        }
    }
    let predicate = &listing["predicate"];
    for (field, wanted) in [
        ("listing", &bundle.listing),
        ("version", &bundle.version),
        ("registry", &bundle.registry),
    ] {
        if predicate[field].as_str() != Some(wanted.as_str()) {
            return Err(format!(
                "the listing statement says {field} {}, not {wanted}",
                predicate[field]
            ));
        }
    }
    if !key_status_signed(&bundle.key_status, keys) {
        return Err("the key status is not the platform's: its signature does not verify".into());
    }
    let signer = provenance["predicate"]["signer"]
        .as_str()
        .or_else(|| predicate["signer_key"].as_str())
        .unwrap_or_default()
        .to_string();
    let key = bundle.key_status["keys"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|key| key["fingerprint"].as_str() == Some(signer.as_str()))
        .ok_or_else(|| format!("the tag's signer, {signer}, is no key in the key status"))?;
    let listed_ns = predicate["listed_at"]
        .as_str()
        .and_then(|at| crate::registry::ns_of(at.split('.').next().unwrap_or(at)))
        .unwrap_or(0);
    let not_after = key["not_trusted_after_ns"].as_u64().unwrap_or(0);
    if key["revoked_at_ns"].as_u64().unwrap_or(0) != 0 && not_after <= listed_ns {
        return Err(format!(
            "the tag's signer, {signer}, was revoked, not trusted after {}, before this version \
             was listed",
            crate::registry::moment(not_after)
        ));
    }
    if let Some(metadata) = by(METADATA) {
        bundle.metadata = metadata["predicate"]["metadata"].clone();
        bundle.documentation = serde_json::Value::Array(
            metadata["predicate"]["documentation"]
                .as_array()
                .into_iter()
                .flatten()
                .map(
                    |text| serde_json::json!({ "section": text["name"], "markdown": text["text"] }),
                )
                .collect(),
        );
    }
    Ok(())
}

// ── Pulling from a registry ──────────────────────────────────────────────

/// An OCI registry's repository, read anonymously or with a pull token.
pub struct Oci {
    base: String,
    name: String,
    token: tokio::sync::Mutex<Option<String>>,
    http: reqwest::Client,
}

impl Oci {
    /// `ghcr.io/open-meridian/plugins/acme/snaptrade`, as a resolution's
    /// `pull_from` names it; HTTPS, or HTTP to this machine only.
    pub fn at(pull_from: &str, token: Option<String>) -> Result<Oci, String> {
        let (host, name) = pull_from
            .split_once('/')
            .ok_or_else(|| format!("{pull_from} names no repository"))?;
        let local = host.starts_with("127.0.0.1") || host.starts_with("localhost");
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(900))
            .default_headers(crate::release::naming_this_version())
            .build()
            .map_err(|failed| failed.to_string())?;
        Ok(Oci {
            base: format!("{}://{host}", if local { "http" } else { "https" }),
            name: name.to_string(),
            token: tokio::sync::Mutex::new(token.filter(|t| !t.is_empty())),
            http,
        })
    }

    /// An anonymous token, as the registry's challenge says to ask for one.
    async fn challenged(&self, header: &str) -> Result<String, String> {
        let field = |key: &str| {
            header
                .split(',')
                .filter_map(|part| part.trim().trim_start_matches("Bearer ").split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().trim_matches('"').to_string())
        };
        let realm = field("realm").ok_or("the registry asked for a token and named no realm")?;
        let mut url = reqwest::Url::parse(&realm).map_err(|failed| failed.to_string())?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(service) = field("service") {
                query.append_pair("service", &service);
            }
            query.append_pair(
                "scope",
                &field("scope").unwrap_or_else(|| format!("repository:{}:pull", self.name)),
            );
        }
        let text = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|failed| format!("the registry's token service did not answer: {failed}"))?
            .text()
            .await
            .unwrap_or_default();
        let answer: serde_json::Value = serde_json::from_str(&text)
            .map_err(|failed| format!("the registry's token is unreadable: {failed}"))?;
        answer["token"]
            .as_str()
            .or_else(|| answer["access_token"].as_str())
            .map(String::from)
            .ok_or_else(|| "the registry's token service gave no token".into())
    }

    /// GET a path of the repository; its bytes and content type.
    async fn get(&self, path: &str, accept: &str) -> Result<(Vec<u8>, String), String> {
        for _ in 0..2 {
            let mut request = self
                .http
                .get(format!("{}/v2/{}/{path}", self.base, self.name))
                .header("accept", accept);
            if let Some(token) = self.token.lock().await.as_deref() {
                request = request.bearer_auth(token);
            }
            let answer = request
                .send()
                .await
                .map_err(|failed| format!("{} did not answer: {failed}", self.base))?;
            let status = answer.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                let header = answer
                    .headers()
                    .get("www-authenticate")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                let token = self.challenged(&header).await?;
                *self.token.lock().await = Some(token);
                continue;
            }
            let media = answer
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            if !status.is_success() {
                return Err(format!(
                    "{}/v2/{}/{path} answered {status}",
                    self.base, self.name
                ));
            }
            let bytes = answer
                .bytes()
                .await
                .map_err(|failed| format!("{path} could not be read: {failed}"))?;
            return Ok((bytes.to_vec(), media));
        }
        Err(format!("{} refused the token it gave", self.base))
    }

    /// A manifest by digest, its bytes checked against it.
    pub async fn manifest(&self, digest: &str) -> Result<(Vec<u8>, String), String> {
        let (bytes, media) = self
            .get(&format!("manifests/{digest}"), MANIFEST_ACCEPT)
            .await?;
        if sha256(&bytes) != digest {
            return Err(format!("the manifest the registry sent is not {digest}"));
        }
        Ok((bytes, media))
    }

    pub async fn blob(&self, digest: &str) -> Result<Vec<u8>, String> {
        let (bytes, _) = self
            .get(&format!("blobs/{digest}"), "application/octet-stream")
            .await?;
        if sha256(&bytes) != digest {
            return Err(format!("the blob the registry sent is not {digest}"));
        }
        Ok(bytes)
    }

    /// The image's referrers' descriptors (OCI distribution 1.1), or the
    /// tag schema's index where the registry has no referrers API.
    pub async fn referrers(&self, digest: &str) -> Result<Vec<serde_json::Value>, String> {
        let index = match self.get(&format!("referrers/{digest}"), OCI_INDEX).await {
            Ok((bytes, _)) => bytes,
            Err(_) => {
                let tag = format!("sha256-{}", hex_of(digest)?);
                match self.get(&format!("manifests/{tag}"), OCI_INDEX).await {
                    Ok((bytes, _)) => bytes,
                    Err(_) => return Ok(Vec::new()),
                }
            }
        };
        let index: serde_json::Value = serde_json::from_slice(&index)
            .map_err(|failed| format!("the referrers' index is unreadable: {failed}"))?;
        Ok(index["manifests"].as_array().cloned().unwrap_or_default())
    }
}

/// The image and its referrers, every byte by digest.
pub async fn pull(oci: &Oci, digest: &str, bundle: &mut Bundle) -> Result<(), String> {
    let (bytes, media) = oci.manifest(digest).await?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|failed| format!("the image's manifest is unreadable: {failed}"))?;
    bundle.image_media_type = manifest["mediaType"]
        .as_str()
        .map(String::from)
        .unwrap_or(media);
    bundle.blobs.insert(digest.to_string(), bytes);
    for named in std::iter::once(&manifest["config"])
        .chain(manifest["layers"].as_array().into_iter().flatten())
    {
        let blob = named["digest"]
            .as_str()
            .ok_or("the image's manifest names a blob with no digest")?;
        let held = oci.blob(blob).await?;
        bundle.blobs.insert(blob.to_string(), held);
    }
    for referrer in oci.referrers(digest).await? {
        let Some(named) = referrer["digest"].as_str() else {
            continue;
        };
        let (bytes, media) = oci.manifest(named).await?;
        let held: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|failed| format!("the referrer {named} is unreadable: {failed}"))?;
        for layer in
            std::iter::once(&held["config"]).chain(held["layers"].as_array().into_iter().flatten())
        {
            if let Some(blob) = layer["digest"].as_str() {
                let bytes = oci.blob(blob).await?;
                bundle.blobs.insert(blob.to_string(), bytes);
            }
        }
        let media = held["mediaType"]
            .as_str()
            .map(String::from)
            .unwrap_or(media);
        bundle.blobs.insert(named.to_string(), bytes);
        bundle.referrers.push((named.to_string(), media));
    }
    Ok(())
}

// ── Writing and reading the file ─────────────────────────────────────────

/// `organisation/plugin@version`, held to its forms.
pub fn named(given: &str) -> Result<(String, String, String), String> {
    let (listing, version) = given
        .split_once('@')
        .ok_or_else(|| format!("{given} is not organisation/plugin@version"))?;
    let (organisation, plugin) = listing
        .split_once('/')
        .ok_or_else(|| format!("{given} is not organisation/plugin@version"))?;
    if !crate::registry::is_slug(organisation) || !crate::catalogue::is_name(plugin) {
        return Err(format!(
            "{given} is not organisation/plugin@version: a slug, a plugin's name and its version"
        ));
    }
    if crate::registry::semver_of_tag(&format!("v{version}")).is_none() {
        return Err(format!("{version} is not a SemVer 2.0 version"));
    }
    Ok((
        organisation.to_string(),
        plugin.to_string(),
        version.to_string(),
    ))
}

/// The record a bundle keeps beside its layout.
fn record(bundle: &Bundle, written_at_ns: u64) -> serde_json::Value {
    serde_json::json!({
        "listing": bundle.listing,
        "version": bundle.version,
        "registry": bundle.registry,
        "image_digest": bundle.image_digest,
        "image_media_type": bundle.image_media_type,
        "referrers": bundle.referrers.iter().map(|(digest, media)| serde_json::json!({
            "digest": digest, "mediaType": media,
        })).collect::<Vec<_>>(),
        "metadata": bundle.metadata,
        "documentation": bundle.documentation,
        "key_status": bundle.key_status,
        "listing_statement": bundle.listing_statement,
        "written_at_ns": written_at_ns,
    })
}

/// The bundle, as a tar holding an OCI image layout and its record.
pub fn write(bundle: &Bundle, to: &Path, written_at_ns: u64) -> Result<(), String> {
    if to.exists() {
        return Err(format!(
            "{} is there already; a bundle is never written over a file",
            to.display()
        ));
    }
    let partial = to.with_extension("partial");
    let file = std::fs::File::create(&partial)
        .map_err(|failed| format!("{}: {failed}", partial.display()))?;
    let mut tar = tar::Builder::new(file);
    let mut add = |path: &str, bytes: &[u8]| -> Result<(), String> {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(written_at_ns / 1_000_000_000);
        header.set_cksum();
        tar.append_data(&mut header, path, bytes)
            .map_err(|failed| format!("{path}: {failed}"))
    };
    add("oci-layout", br#"{"imageLayoutVersion":"1.0.0"}"#)?;
    let mut manifests = vec![serde_json::json!({
        "mediaType": bundle.image_media_type,
        "digest": bundle.image_digest,
        "size": bundle.blobs.get(&bundle.image_digest).map_or(0, Vec::len),
        "annotations": {"org.opencontainers.image.ref.name": bundle.version},
    })];
    for (digest, media) in &bundle.referrers {
        manifests.push(serde_json::json!({
            "mediaType": media, "digest": digest,
            "size": bundle.blobs.get(digest).map_or(0, Vec::len),
        }));
    }
    let index = serde_json::json!({
        "schemaVersion": 2, "mediaType": OCI_INDEX, "manifests": manifests,
    });
    add("index.json", index.to_string().as_bytes())?;
    for (digest, bytes) in &bundle.blobs {
        add(&format!("blobs/sha256/{}", hex_of(digest)?), bytes)?;
    }
    add(RECORD, record(bundle, written_at_ns).to_string().as_bytes())?;
    tar.into_inner()
        .and_then(|mut file| {
            use std::io::Write as _;
            file.flush()?;
            file.sync_all()
        })
        .map_err(|failed| format!("{}: {failed}", partial.display()))?;
    std::fs::rename(&partial, to).map_err(|failed| format!("{}: {failed}", to.display()))
}

/// A bundle file, read back: every blob, and its record.
pub fn read(from: &Path) -> Result<Bundle, String> {
    let file =
        std::fs::File::open(from).map_err(|failed| format!("{}: {failed}", from.display()))?;
    let mut archive = tar::Archive::new(file);
    let mut blobs = BTreeMap::new();
    let mut held: Option<serde_json::Value> = None;
    for entry in archive
        .entries()
        .map_err(|failed| format!("{} is not a plugin bundle: {failed}", from.display()))?
    {
        let mut entry = entry.map_err(|failed| format!("{}: {failed}", from.display()))?;
        let path = entry
            .path()
            .map_err(|failed| failed.to_string())?
            .to_string_lossy()
            .into_owned();
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|failed| format!("{path}: {failed}"))?;
        if let Some(hex) = path.strip_prefix("blobs/sha256/") {
            blobs.insert(format!("sha256:{hex}"), bytes);
        } else if path == RECORD {
            held = Some(serde_json::from_slice(&bytes).map_err(|failed| {
                format!("{}'s {RECORD} is unreadable: {failed}", from.display())
            })?);
        }
    }
    let held = held.ok_or_else(|| {
        format!(
            "{} holds no {RECORD}, so it is not a plugin bundle",
            from.display()
        )
    })?;
    let text = |key: &str| held[key].as_str().unwrap_or_default().to_string();
    Ok(Bundle {
        listing: text("listing"),
        version: text("version"),
        registry: text("registry"),
        image_digest: text("image_digest"),
        image_media_type: text("image_media_type"),
        blobs,
        referrers: held["referrers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                Some((
                    r["digest"].as_str()?.to_string(),
                    r["mediaType"].as_str().unwrap_or(OCI_MANIFEST).to_string(),
                ))
            })
            .collect(),
        metadata: held["metadata"].clone(),
        documentation: held["documentation"].clone(),
        key_status: held["key_status"].clone(),
        listing_statement: Some(held["listing_statement"].clone()).filter(|s| s.is_object()),
    })
}

/// The file a bundle is written to where `--to` names none.
pub fn default_file(organisation: &str, plugin: &str, version: &str) -> PathBuf {
    PathBuf::from(format!(
        "{organisation}-{plugin}-{version}.plugin-bundle.tar"
    ))
}

// ── meridian plugin download ─────────────────────────────────────────────

pub struct Download {
    pub organisation: String,
    pub plugin: String,
    pub version: String,
    pub registry: String,
    pub to: PathBuf,
}

/// Resolve, pull, verify, write: what was written.
pub async fn download(
    asked: &Download,
    signed_in: &crate::registry::SignedIn,
    keys: &[String],
) -> Result<Bundle, String> {
    if asked.to.exists() {
        return Err(format!(
            "{} is there already; a bundle is never written over a file",
            asked.to.display()
        ));
    }
    let platform = signed_in.platform().to_string();
    let listing = format!("{}/{}", asked.organisation, asked.plugin);
    let bearer = signed_in.bearer().await?;
    let resolved = crate::registry::call(
        &platform,
        reqwest::Method::POST,
        &format!(
            "/api/v1/registry/listings/{listing}/versions/{}/resolve",
            asked.version
        ),
        &bearer,
        Some(&serde_json::json!({
            "listing": listing, "version": asked.version, "registry": asked.registry,
        })),
    )
    .await?;
    let digest = resolved["image_digest"]
        .as_str()
        .filter(|d| hex_of(d).is_ok())
        .ok_or_else(|| format!("{platform} resolved {listing} to no digest"))?
        .to_string();
    let pull_from = resolved["pull_from"]
        .as_str()
        .ok_or_else(|| format!("{platform} named nowhere to pull {listing} from"))?;
    let oci = Oci::at(pull_from, resolved["pull_token"].as_str().map(String::from))?;
    let mut bundle = Bundle {
        listing,
        version: asked.version.clone(),
        registry: asked.registry.clone(),
        image_digest: digest.clone(),
        // Where the resolution carries the listed version's statement, as
        // the catalogue does (ListedVersion.listing_statement); otherwise
        // the image's referrer holds it.
        listing_statement: resolved["listing_statement"]
            .as_str()
            .and_then(|text| serde_json::from_str(text).ok()),
        ..Bundle::default()
    };
    eprintln!("Pulling {pull_from}@{digest} ...");
    pull(&oci, &digest, &mut bundle).await?;
    let status = reqwest::Client::new()
        .get(format!("{platform}/api/v1/registry/signing-keys"))
        .send()
        .await
        .map_err(|failed| format!("could not reach {platform}: {failed}"))?
        .text()
        .await
        .unwrap_or_default();
    bundle.key_status = serde_json::from_str(&status)
        .map_err(|failed| format!("{platform}'s key status is unreadable: {failed}"))?;
    verify(&mut bundle, keys)?;
    write(
        &bundle,
        &asked.to,
        crate::credential::now_s() * 1_000_000_000,
    )?;
    Ok(bundle)
}

// ── meridian plugin upload --bundle ──────────────────────────────────────

/// The metadata a bundle's upload records: the version's own, from its
/// metadata attestation, with its listing, source registry and key status.
pub fn record_body(bundle: &Bundle) -> Result<serde_json::Value, String> {
    let (_, plugin) = bundle
        .listing
        .split_once('/')
        .ok_or_else(|| format!("{} is not organisation/plugin", bundle.listing))?;
    let metadata = &bundle.metadata;
    let mut body = serde_json::json!({
        "name": plugin,
        "version": bundle.version,
        "roles": metadata["roles"].as_array().cloned().unwrap_or_default(),
        "interface": metadata["interface"].as_bool().unwrap_or(false),
        "sdk_version": metadata["sdk_version"].as_str().unwrap_or_default(),
        "image_digest": bundle.image_digest,
        "listing": bundle.listing,
        "source_registry": bundle.registry,
        "key_status": bundle.key_status,
    });
    if let Some(statement) = &bundle.listing_statement {
        body["listing_statement"] = canonical(statement).into();
    }
    if !metadata["declaration"].is_null() {
        body["declaration"] = metadata["declaration"].clone();
    }
    if bundle
        .documentation
        .as_array()
        .is_some_and(|texts| !texts.is_empty())
    {
        body["documentation"] = bundle.documentation.clone();
    }
    Ok(body)
}

/// Carry a bundle into the deployment, verified first: the version's digest.
pub async fn upload(
    address: &str,
    session: &Credential,
    from: &Path,
    keys: &[String],
) -> Result<String, Failed> {
    let mut bundle = read(from)?;
    verify(&mut bundle, keys)?;
    let repository = format!("{address}/terminal/registry/v2/plugins/{}", bundle.listing);
    let http = crate::catalogue::http()?;
    let image: serde_json::Value = serde_json::from_slice(&bundle.blobs[&bundle.image_digest])
        .map_err(|failed| Failed::Refused(failed.to_string()))?;
    // Every blob the image and its referrers name, then their manifests: the
    // image's under its version, each referrer's by its digest.
    let mut manifests: Vec<(String, String, String)> = vec![(
        bundle.image_digest.clone(),
        bundle.image_media_type.clone(),
        bundle.version.clone(),
    )];
    let mut named = Vec::new();
    for value in
        std::iter::once(&image["config"]).chain(image["layers"].as_array().into_iter().flatten())
    {
        named.extend(value["digest"].as_str().map(String::from));
    }
    for (digest, media) in &bundle.referrers {
        let held: serde_json::Value = serde_json::from_slice(&bundle.blobs[digest])
            .map_err(|failed| Failed::Refused(failed.to_string()))?;
        for value in
            std::iter::once(&held["config"]).chain(held["layers"].as_array().into_iter().flatten())
        {
            named.extend(value["digest"].as_str().map(String::from));
        }
        manifests.push((digest.clone(), media.clone(), digest.clone()));
    }
    named.sort();
    named.dedup();
    for digest in &named {
        let bytes = bundle
            .blobs
            .get(digest)
            .cloned()
            .ok_or_else(|| Failed::Refused(format!("the bundle does not hold {digest}")))?;
        crate::catalogue::send_blob(&http, address, session, &repository, digest, bytes).await?;
    }
    for (digest, media, reference) in &manifests {
        crate::catalogue::send_manifest(
            &http,
            address,
            session,
            &repository,
            reference,
            media,
            bundle.blobs[digest].clone(),
        )
        .await?;
    }
    // The tag schema's index of the referrers, for a registry with no
    // referrers API (OCI distribution 1.1).
    if !bundle.referrers.is_empty() {
        let index = serde_json::json!({
            "schemaVersion": 2, "mediaType": OCI_INDEX,
            "manifests": bundle.referrers.iter().map(|(digest, media)| serde_json::json!({
                "mediaType": media, "digest": digest,
                "size": bundle.blobs.get(digest).map_or(0, Vec::len),
            })).collect::<Vec<_>>(),
        });
        crate::catalogue::send_manifest(
            &http,
            address,
            session,
            &repository,
            &format!("sha256-{}", hex_of(&bundle.image_digest)?),
            OCI_INDEX,
            index.to_string().into_bytes(),
        )
        .await?;
    }
    crate::catalogue::record_json(address, session, &record_body(&bundle)?).await?;
    Ok(bundle.image_digest)
}

#[cfg(test)]
mod tests;

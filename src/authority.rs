//! This machine's own certificate authority, which a local deployment's HTTPS
//! is signed by (task kernel/a-development-deployment-serves-https, ruled
//! 2026-10-03).
//!
//! An MCP client signs in over HTTPS or not at all, so a deployment at
//! `meridian.localhost` has to serve it, and the chart makes no certificate
//! (spec/live-plugin-development, requirement 1). The CLI does, as the
//! operator's tool on the operator's machine: a root made once, in-process
//! (no mkcert, nothing downloaded), kept in this CLI's own directory with its
//! key readable by the person alone, named for Open Meridian and this machine,
//! and valid for ten years. Each local deployment gets a certificate from it
//! for its name and `*.plugins.` below it, valid for a year and renewed within
//! 30 days of its end.
//!
//! The root may sign names under `.localhost` and nothing else: a name
//! constraint in the root itself, so a copy of its key cannot vouch for
//! somebody's bank to this machine.
//!
//! Trusting it is the machine's, asked once: on macOS the login keychain,
//! which asks for the person's password itself, so this never handles one,
//! and then apps on Node, the Claude app among them, which read neither the
//! keychain nor a shell's profile: `NODE_EXTRA_CA_CERTS` named to them through
//! launchd now and by a LaunchAgent at each login, never replacing one the
//! person named already (ruled 2026-10-04, with the install script asking).
//! Elsewhere each step is printed as a command, and on macOS too where no
//! dialog can reach the person (an SSH session, or a session that is not on
//! the Mac's screen): there the keychain's dialog would wait for ever, found
//! on a GitHub macOS runner, so `security` is not run at all. Where it is run,
//! it is given [`DIALOG_LIMIT`] and stopped after it. Nothing here does any
//! of it without `Trust`, which a test replaces.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints,
};

const DAY_S: u64 = 86_400;
/// The root's life (ruling 5).
const ROOT_DAYS: u64 = 3_650;
/// A deployment's certificate's life.
const LEAF_DAYS: u64 = 365;
/// `up` and `upgrade-deployment` issue a new one this close to the end.
pub const RENEW_WITHIN_DAYS: u64 = 30;

const ROOT: &str = "root.pem";
const KEY: &str = "root-key.pem";
/// Written once the machine has been asked to trust the root and it worked,
/// or, off macOS, where this never runs the step itself, once the commands
/// were printed. Never where macOS could show no dialog: asked again there.
const ASKED: &str = "asked";

/// The name constraint: every name a root of this CLI's may sign.
const SIGNS_UNDER: &str = "localhost";

/// Where it lives: `<this CLI's directory>/authority`.
pub fn directory() -> Result<PathBuf, String> {
    Ok(crate::sessions::configuration()?.join("authority"))
}

/// The root's certificate, which is what anything trusting it is pointed at.
pub fn root_path(dir: &Path) -> PathBuf {
    dir.join(ROOT)
}

/// A name under `.localhost`, which every browser sends to this machine and
/// which only this machine's own authority signs for.
pub fn is_local(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost")
}

/// The root and its key. Never printed: Debug leaves the key out.
pub struct Authority {
    root_pem: String,
    key_pem: String,
}

impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authority")
            .field("root_pem", &self.root_pem)
            .finish_non_exhaustive()
    }
}

/// A deployment's certificate and its key, as its Secret holds them.
#[derive(Clone)]
pub struct Leaf {
    pub cert_pem: String,
    key_pem: String,
    pub ends_s: u64,
}

impl std::fmt::Debug for Leaf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Leaf")
            .field("cert_pem", &self.cert_pem)
            .field("ends_s", &self.ends_s)
            .finish_non_exhaustive()
    }
}

fn at(seconds: u64) -> Result<time::OffsetDateTime, String> {
    time::OffsetDateTime::from_unix_timestamp(seconds as i64)
        .map_err(|failed| format!("{seconds} is not a time: {failed}"))
}

/// `2027-10-03`, for saying when a certificate ends.
pub fn date(seconds: u64) -> String {
    at(seconds)
        .map(|when| when.date().to_string())
        .unwrap_or_else(|_| seconds.to_string())
}

fn failed(what: &str) -> impl Fn(rcgen::Error) -> String + '_ {
    move |error| format!("{what}: {error}")
}

impl Authority {
    /// A new root, named for Open Meridian and this machine.
    pub fn make(machine: &str, now_s: u64) -> Result<Authority, String> {
        let key = KeyPair::generate().map_err(failed("no key could be made"))?;
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::OrganizationName, "Open Meridian");
        name.push(
            DnType::CommonName,
            format!("Open Meridian local authority for {machine}"),
        );
        params.distinguished_name = name;
        // It signs deployments' certificates and never another authority.
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.name_constraints = Some(NameConstraints {
            permitted_subtrees: vec![GeneralSubtree::DnsName(SIGNS_UNDER.into())],
            excluded_subtrees: Vec::new(),
        });
        // A day of leeway, for a clock somewhere that is behind this one.
        params.not_before = at(now_s.saturating_sub(DAY_S))?;
        params.not_after = at(now_s + ROOT_DAYS * DAY_S)?;
        let root = params
            .self_signed(&key)
            .map_err(failed("the root could not be signed"))?;
        Ok(Authority {
            root_pem: root.pem(),
            key_pem: key.serialize_pem(),
        })
    }

    /// What `dir` holds: the root and its key, or nothing made yet. One
    /// without the other is refused rather than remade, because a root
    /// already trusted somewhere would be left trusted with nobody holding
    /// its key.
    pub fn read(dir: &Path) -> Result<Option<Authority>, String> {
        let read = |name: &str| match std::fs::read_to_string(dir.join(name)) {
            Ok(text) => Ok(Some(text)),
            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(failed) => Err(format!("{}: {failed}", dir.join(name).display())),
        };
        match (read(ROOT)?, read(KEY)?) {
            (None, None) => Ok(None),
            (Some(root_pem), Some(key_pem)) => {
                let held = Authority { root_pem, key_pem };
                held.key_id().ok_or_else(|| {
                    format!(
                        "{} is not a certificate this CLI made. `meridian authority remove` \
                         removes it, and the next `meridian up` makes another",
                        dir.join(ROOT).display()
                    )
                })?;
                KeyPair::from_pem(&held.key_pem).map_err(|_| {
                    format!(
                        "{} is not a key. `meridian authority remove` removes the authority, \
                         and the next `meridian up` makes another",
                        dir.join(KEY).display()
                    )
                })?;
                Ok(Some(held))
            }
            _ => Err(format!(
                "{} holds half of a certificate authority. `meridian authority remove` \
                 removes it, and the next `meridian up` makes another",
                dir.display()
            )),
        }
    }

    /// Written into `dir`, the key readable by its owner alone from the moment
    /// it exists: made with those permissions and renamed into place.
    pub fn write(&self, dir: &Path) -> Result<(), String> {
        crate::sessions::create_private_directory(dir)?;
        for (name, text) in [(KEY, &self.key_pem), (ROOT, &self.root_pem)] {
            let path = dir.join(name);
            let partial = dir.join(format!(".{name}.partial"));
            let _ = std::fs::remove_file(&partial);
            let mut file = crate::sessions::private_file(&partial)
                .map_err(|failed| format!("could not write {}: {failed}", partial.display()))?;
            file.write_all(text.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|failed| format!("could not write {}: {failed}", partial.display()))?;
            std::fs::rename(&partial, &path)
                .map_err(|failed| format!("could not write {}: {failed}", path.display()))?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn root_pem(&self) -> &str {
        &self.root_pem
    }

    /// The root's SHA-1, as macOS's `security` names a certificate.
    pub fn sha1(&self) -> String {
        use sha1::Digest as _;
        let der = pem_der(&self.root_pem).unwrap_or_default();
        sha1::Sha1::digest(der)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect()
    }

    /// The root's subject key identifier, which a certificate it signed names.
    fn key_id(&self) -> Option<Vec<u8>> {
        let der = pem_der(&self.root_pem)?;
        let (_, root) = x509_parser::parse_x509_certificate(&der).ok()?;
        if !root.is_ca() {
            return None;
        }
        let id = root
            .iter_extensions()
            .find_map(|extension| match extension.parsed_extension() {
                x509_parser::extensions::ParsedExtension::SubjectKeyIdentifier(id) => {
                    Some(id.0.to_vec())
                }
                _ => None,
            });
        id
    }

    /// A certificate for `host` and every plugin's page below it, for a year.
    pub fn issue(&self, host: &str, now_s: u64) -> Result<Leaf, String> {
        if !is_local(host) {
            return Err(format!(
                "{host} is not a name under .localhost, and this machine's authority signs \
                 nothing else: a firm's own name has a certificate of its own"
            ));
        }
        let key = KeyPair::from_pem(&self.key_pem).map_err(failed("the root's key"))?;
        let issuer = Issuer::from_ca_cert_pem(&self.root_pem, key).map_err(failed("the root"))?;
        let leaf_key = KeyPair::generate().map_err(failed("no key could be made"))?;
        let mut params = CertificateParams::new(vec![host.to_string(), plugins_of(host)])
            .map_err(failed("those names"))?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, host);
        params.distinguished_name = name;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        params.not_before = at(now_s.saturating_sub(3_600))?;
        let ends_s = now_s + LEAF_DAYS * DAY_S;
        params.not_after = at(ends_s)?;
        let leaf = params
            .signed_by(&leaf_key, &issuer)
            .map_err(failed("the certificate could not be signed"))?;
        Ok(Leaf {
            cert_pem: leaf.pem(),
            key_pem: leaf_key.serialize_pem(),
            ends_s,
        })
    }

    /// Whether a deployment's certificate, as its Secret holds it, still
    /// serves: this authority's, naming `host` and its plugins' wildcard, and
    /// more than 30 days from its end.
    pub fn standing(&self, cert_pem: &str, host: &str, now_s: u64) -> Standing {
        let Some(der) = pem_der(cert_pem) else {
            return Standing::Due("it is not a certificate".into());
        };
        let Ok((_, leaf)) = x509_parser::parse_x509_certificate(&der) else {
            return Standing::Due("it is not a certificate".into());
        };
        let ends_s = leaf.validity().not_after.timestamp().max(0) as u64;
        let names: Vec<String> = leaf
            .subject_alternative_name()
            .ok()
            .flatten()
            .map(|names| {
                names
                    .value
                    .general_names
                    .iter()
                    .filter_map(|name| match name {
                        x509_parser::extensions::GeneralName::DNSName(dns) => Some(dns.to_string()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let signed_by =
            leaf.iter_extensions()
                .find_map(|extension| match extension.parsed_extension() {
                    x509_parser::extensions::ParsedExtension::AuthorityKeyIdentifier(aki) => {
                        aki.key_identifier.as_ref().map(|id| id.0.to_vec())
                    }
                    _ => None,
                });
        if signed_by.is_none() || signed_by != self.key_id() {
            return Standing::Due("it was signed by another authority than this machine's".into());
        }
        if !names.iter().any(|name| name == host) || !names.contains(&plugins_of(host)) {
            return Standing::Due(format!("it does not name {host} and {}", plugins_of(host)));
        }
        if ends_s <= now_s + RENEW_WITHIN_DAYS * DAY_S {
            return Standing::Due(format!("it ends on {}", date(ends_s)));
        }
        Standing::Current { ends_s }
    }
}

fn plugins_of(host: &str) -> String {
    format!("*.plugins.{host}")
}

fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let (_, parsed) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).ok()?;
    Some(parsed.contents)
}

/// Whether a deployment's certificate still serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    Current {
        ends_s: u64,
    },
    /// A new one is due, and why.
    Due(String),
}

/// The Secret a release's certificate is kept in, which the chart's
/// `ingress.tls.secretName` names.
pub fn secret_name(release: &str) -> String {
    format!("{release}-tls")
}

/// The Secret, as `kubectl apply --server-side -f -` takes it on its standard
/// input: never an argument, since every process on the machine can read
/// those, and server-side so no copy of the key is kept in an annotation.
pub fn secret_manifest(release: &str, namespace: &str, leaf: &Leaf) -> String {
    let encoded = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
    serde_json::json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "type": "kubernetes.io/tls",
        "metadata": {
            "name": secret_name(release),
            "namespace": namespace,
            "labels": { "app.kubernetes.io/instance": release },
        },
        "data": {
            "tls.crt": encoded(&leaf.cert_pem),
            "tls.key": encoded(&leaf.key_pem),
        },
    })
    .to_string()
}

/// The certificate a Secret holds, from `kubectl get secret -o
/// jsonpath={.data.tls\.crt}`: base64, as Kubernetes keeps it.
pub fn certificate_in(said: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(said.trim())
        .ok()?;
    String::from_utf8(bytes).ok().filter(|pem| !pem.is_empty())
}

/// A client that trusts this machine's own root beside the system's, where
/// there is one: so the CLI reaches a local deployment whether or not the
/// person has added the root to their keychain yet. Safe to add to any
/// client, since the root signs names under `.localhost` and nothing else.
pub fn trusted_here(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let root = directory()
        .ok()
        .and_then(|dir| std::fs::read(root_path(&dir)).ok())
        .and_then(|pem| reqwest::Certificate::from_pem(&pem).ok());
    match root {
        Some(root) => builder.tls_certs_merge([root]),
        None => builder,
    }
}

// ── In the cluster ─────────────────────────────────────────────────────────

/// The certificate a release's Secret holds now, if it has one. The
/// certificate alone: its key is never read back.
pub async fn held(
    machine: &dyn crate::doctor::Machine,
    release: &str,
    namespace: &str,
) -> Option<String> {
    let said = machine
        .run(
            "kubectl",
            &[
                "get",
                "secret",
                &secret_name(release),
                "-n",
                namespace,
                "--ignore-not-found",
                "-o",
                "jsonpath={.data.tls\\.crt}",
            ],
        )
        .await
        .ok()?;
    certificate_in(&said)
}

/// The Secret written, or replaced, with a new certificate.
pub async fn write_secret(
    machine: &dyn crate::doctor::Machine,
    release: &str,
    namespace: &str,
    leaf: &Leaf,
) -> Result<(), String> {
    machine
        .run_with_input(
            "kubectl",
            &[
                "apply",
                "--server-side",
                "--force-conflicts",
                "--field-manager",
                "meridian",
                "-f",
                "-",
            ],
            &secret_manifest(release, namespace, leaf),
        )
        .await
        .map(|_| ())
        .map_err(|failed| {
            format!(
                "the certificate's Secret {} could not be written in {namespace}: {failed}",
                secret_name(release)
            )
        })
}

/// What the certificate step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Certified {
    /// The Secret's certificate still serves.
    Kept { ends_s: u64 },
    /// A new one was written, and why.
    Issued { ends_s: u64, why: String },
}

impl Certified {
    pub fn said(&self, host: &str) -> String {
        match self {
            Certified::Kept { ends_s } => format!(
                "The certificate for {host} serves until {}; kept.",
                date(*ends_s)
            ),
            Certified::Issued { ends_s, why } => format!(
                "Issued a certificate for {host} and {}, until {} ({why}).",
                plugins_of(host),
                date(*ends_s)
            ),
        }
    }
}

/// What a release's certificate needs.
#[derive(Debug)]
pub enum Needs {
    /// Nothing: it serves until then.
    Nothing { ends_s: u64 },
    /// A new one, issued here and not yet written, and why.
    New { leaf: Leaf, why: String },
}

/// What a release's certificate needs. Changes nothing.
pub async fn needs(
    machine: &dyn crate::doctor::Machine,
    authority: &Authority,
    release: &str,
    namespace: &str,
    host: &str,
) -> Result<Needs, String> {
    let now = machine.now_s();
    let why = match held(machine, release, namespace).await {
        None => "there was none".to_string(),
        Some(pem) => match authority.standing(&pem, host, now) {
            Standing::Current { ends_s } => return Ok(Needs::Nothing { ends_s }),
            Standing::Due(why) => why,
        },
    };
    Ok(Needs::New {
        leaf: authority.issue(host, now)?,
        why,
    })
}

/// The certificate step: the Secret kept while its certificate serves, and
/// otherwise a new one issued and written.
pub async fn certify(
    machine: &dyn crate::doctor::Machine,
    authority: &Authority,
    release: &str,
    namespace: &str,
    host: &str,
) -> Result<Certified, String> {
    match needs(machine, authority, release, namespace, host).await? {
        Needs::Nothing { ends_s } => Ok(Certified::Kept { ends_s }),
        Needs::New { leaf, why } => {
            write_secret(machine, release, namespace, &leaf).await?;
            Ok(Certified::Issued {
                ends_s: leaf.ends_s,
                why,
            })
        }
    }
}

// ── Trusting it ────────────────────────────────────────────────────────────

/// A path as a shell reads it back.
fn shell(path: &Path) -> String {
    shell_word(&path.display().to_string())
}

fn shell_word(said: &str) -> String {
    if said
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c))
    {
        said.to_string()
    } else {
        format!("'{}'", said.replace('\'', r"'\''"))
    }
}

/// What trusting the root is on each system, as one command.
pub fn trust_command(os: &str, root: &Path) -> String {
    let root = shell(root);
    match os {
        "macos" => format!(
            "security add-trusted-cert -r trustRoot -k ~/Library/Keychains/login.keychain-db {root}"
        ),
        "linux" => format!(
            "sudo cp {root} /usr/local/share/ca-certificates/open-meridian-local.crt && sudo update-ca-certificates"
        ),
        "windows" => format!("certutil -user -addstore Root {root}"),
        _ => format!("add {root} to this system's trusted roots"),
    }
}

/// And taking that back.
pub fn untrust_command(os: &str, sha1: &str) -> String {
    match os {
        "macos" => format!(
            "security delete-certificate -t -Z {sha1} ~/Library/Keychains/login.keychain-db"
        ),
        "linux" => "sudo rm /usr/local/share/ca-certificates/open-meridian-local.crt && sudo update-ca-certificates --fresh".into(),
        "windows" => format!("certutil -user -delstore Root {sha1}"),
        _ => "remove it from this system's trusted roots".into(),
    }
}

// ── For apps on Node ───────────────────────────────────────────────────────

/// What an app on Node reads extra roots from. The Claude desktop app is one,
/// and reads neither the login keychain nor a shell's profile: found
/// 2026-10-04, its sign-in to a local deployment failed with "unable to verify
/// the first certificate" until launchd named the root to it and the app was
/// restarted. The Claude Code CLI reads the login keychain and needs none.
pub const NODE_EXTRA_CA_CERTS: &str = "NODE_EXTRA_CA_CERTS";

/// The file holding the certificates `NODE_EXTRA_CA_CERTS` already named and
/// this root, where one was named before: that one is never replaced, so both
/// are named together, in this file, once the person says so.
const COMBINED: &str = "node-extra-ca-certs.pem";
/// What launchd named before the combined file replaced it, put back when
/// the authority is removed. Empty when launchd named nothing.
const BEFORE: &str = "node-extra-ca-certs.before";
/// The LaunchAgent that names the root to apps again at each login.
pub const AGENT: &str = "com.open-meridian.authority";

/// Naming a file to apps on Node, as one command a person runs: launchd's
/// environment for apps started from now on on macOS, the profile a session
/// starts from on Linux, the user's environment on Windows.
pub fn apps_command(os: &str, file: &Path) -> String {
    match os {
        "macos" => format!("launchctl setenv {NODE_EXTRA_CA_CERTS} {}", shell(file)),
        "linux" => format!(
            "echo \"export {NODE_EXTRA_CA_CERTS}={}\" >> ~/.profile",
            shell(file)
        ),
        "windows" => format!("setx {NODE_EXTRA_CA_CERTS} \"{}\"", file.display()),
        _ => format!(
            "set {NODE_EXTRA_CA_CERTS}={} where apps are started",
            shell(file)
        ),
    }
}

/// And taking that back.
pub fn unapps_command(os: &str) -> String {
    match os {
        "macos" => format!("launchctl unsetenv {NODE_EXTRA_CA_CERTS}"),
        "linux" => format!("remove the {NODE_EXTRA_CA_CERTS} line from ~/.profile"),
        "windows" => format!("reg delete HKCU\\Environment /v {NODE_EXTRA_CA_CERTS} /f"),
        _ => format!("unset {NODE_EXTRA_CA_CERTS} where apps are started"),
    }
}

/// A file holding `theirs` and the root, as one command.
fn combine_command(os: &str, theirs: &str, root: &Path, combined: &Path) -> String {
    match os {
        "windows" => format!(
            "type \"{theirs}\" \"{}\" > \"{}\"",
            root.display(),
            combined.display()
        ),
        _ => format!(
            "cat {} {} > {}",
            shell_word(theirs),
            shell(root),
            shell(combined)
        ),
    }
}

/// Whether `named` is this authority's own file: its root, or the file
/// holding both.
fn ours(dir: &Path, named: &str) -> bool {
    let named = Path::new(named);
    named == root_path(dir) || named == dir.join(COMBINED)
}

/// Another file `NODE_EXTRA_CA_CERTS` already names, which is never replaced:
/// as launchd gives it to apps on macOS, or else as this process was given it.
fn theirs(trust: &dyn Trust, dir: &Path) -> Option<String> {
    let launchd = match trust.os() {
        "macos" => trust.apps_env(),
        _ => None,
    };
    launchd
        .filter(|named| !named.is_empty())
        .or_else(|| trust.own_env())
        .filter(|named| !named.is_empty() && !ours(dir, named))
}

/// The commands that name the root to apps, or a file holding it and theirs.
fn apps_commands(os: &str, dir: &Path, theirs: Option<&str>) -> Vec<String> {
    let root = root_path(dir);
    match theirs {
        None => vec![apps_command(os, &root)],
        Some(theirs) => {
            let combined = dir.join(COMBINED);
            vec![
                combine_command(os, theirs, &root, &combined),
                apps_command(os, &combined),
            ]
        }
    }
}

/// Every step of trusting it, as commands, for a person to run themselves:
/// all there is off macOS, and what is said there when they say no.
fn by_hand(trust: &dyn Trust, dir: &Path) -> String {
    let os = trust.os();
    let mut said = format!(
        "To trust it, run:\n  {}\n\
         The Claude app, like any app on Node, reads its roots from {NODE_EXTRA_CA_CERTS} \
         instead; to name it there:\n",
        trust_command(os, &root_path(dir))
    );
    for line in apps_commands(os, dir, theirs(trust, dir).as_deref()) {
        said.push_str(&format!("  {line}\n"));
    }
    if os == "macos" {
        said.push_str(
            "`meridian authority trust` does all of this, and names it to apps again at each login.",
        );
    }
    said.trim_end().to_string()
}

/// The LaunchAgent: `launchctl setenv NODE_EXTRA_CA_CERTS <file>` when it is
/// loaded, which launchd does at each login.
pub fn agent_plist(file: &Path) -> String {
    let file = file
        .display()
        .to_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by `meridian authority trust`, so apps on Node, the Claude app among
     them, trust this machine's own certificate authority. `meridian authority
     remove` and `meridian uninstall` take it away. -->
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{AGENT}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/launchctl</string>
    <string>setenv</string>
    <string>{NODE_EXTRA_CA_CERTS}</string>
    <string>{file}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}

fn agent_path(agents: &Path) -> PathBuf {
    agents.join(format!("{AGENT}.plist"))
}

fn write_agent(agents: &Path, file: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(agents)
        .map_err(|failed| format!("could not make {}: {failed}", agents.display()))?;
    let plist = agent_path(agents);
    std::fs::write(&plist, agent_plist(file))
        .map_err(|failed| format!("could not write {}: {failed}", plist.display()))?;
    Ok(plist)
}

/// `theirs` and the root, in one file: theirs first, as it was.
fn combine(theirs: &str, root: &Path, combined: &Path) -> Result<(), String> {
    let mut held = std::fs::read_to_string(theirs)
        .map_err(|failed| format!("{theirs} could not be read: {failed}"))?;
    if !held.is_empty() && !held.ends_with('\n') {
        held.push('\n');
    }
    let root = std::fs::read_to_string(root)
        .map_err(|failed| format!("{} could not be read: {failed}", root.display()))?;
    held.push_str(&root);
    std::fs::write(combined, held)
        .map_err(|failed| format!("could not write {}: {failed}", combined.display()))
}

/// The step for the Claude app, on macOS: `NODE_EXTRA_CA_CERTS` named for
/// apps started from now on, and again at each login by a LaunchAgent. One
/// already naming another file is never replaced: a file holding both is
/// offered, and asked for first. Whether it is in place afterwards.
fn point_apps(
    dir: &Path,
    trust: &dyn Trust,
    ask: &dyn Fn(&str) -> bool,
    say: &mut dyn FnMut(&str),
) -> bool {
    let root = root_path(dir);
    let combined = dir.join(COMBINED);
    let launchd = trust.apps_env().filter(|named| !named.is_empty());
    let file = match theirs(trust, dir) {
        None => match &launchd {
            Some(named) if Path::new(named) == combined => combined,
            _ => root,
        },
        Some(other) => {
            say(&format!(
                "{NODE_EXTRA_CA_CERTS} already names {other}, and this never replaces it. A file \
                 holding its certificates and this root, kept at {}, would serve both.",
                combined.display()
            ));
            if !ask(&format!(
                "Make that file, and name it to apps instead of {other}?"
            )) {
                say(&format!(
                    "Left as it is. To do it yourself, and restart the Claude app:\n  {}",
                    apps_commands("macos", dir, Some(&other)).join("\n  ")
                ));
                return false;
            }
            if let Err(failed) = combine(&other, &root, &combined) {
                say(&format!(
                    "Nothing was changed: {failed}. `meridian authority trust` asks again."
                ));
                return false;
            }
            // What launchd named before the first switch, put back on removal.
            if !dir.join(BEFORE).exists() {
                let _ = std::fs::write(dir.join(BEFORE), launchd.unwrap_or_default());
            }
            combined
        }
    };
    let named = file.display().to_string();
    if let Err(refused) = trust.set_apps_env(&named) {
        say(&format!(
            "Apps were not pointed at it: {refused}. To do it yourself:\n  {}",
            apps_command("macos", &file)
        ));
        return false;
    }
    let restart = "Restart the Claude app, if it is open, for it to sign in to a local \
                   deployment; the Claude Code CLI reads the login keychain and needs none of this.";
    match trust
        .launch_agents()
        .and_then(|agents| write_agent(&agents, &file))
    {
        Ok(plist) => {
            // Reloaded, so launchd holds what the file now says.
            let _ = trust.unload(&plist);
            let _ = trust.load(&plist);
            say(&format!(
                "Apps started from now on are pointed at it with {NODE_EXTRA_CA_CERTS}={named}, \
                 and {} names it again at each login. {restart}",
                plist.display()
            ));
            true
        }
        Err(failed) => {
            say(&format!(
                "Apps started from now on are pointed at it with {NODE_EXTRA_CA_CERTS}={named}, \
                 but nothing names it again at the next login: {failed}. {restart}"
            ));
            false
        }
    }
}

/// The step for the Claude app undone: the LaunchAgent unloaded and deleted,
/// and `NODE_EXTRA_CA_CERTS` put back as it was, where it still names this
/// authority's file. One the person has since changed is left alone.
fn unpoint_apps(dir: &Path, trust: &dyn Trust) -> String {
    let mut said = String::new();
    if let Ok(agents) = trust.launch_agents() {
        let plist = agent_path(&agents);
        if plist.exists() {
            let _ = trust.unload(&plist);
            match std::fs::remove_file(&plist) {
                Ok(()) => said.push_str(&format!("Removed {}.\n", plist.display())),
                Err(failed) => said.push_str(&format!(
                    "Could not remove {}: {failed}. Delete it, or each login names a file that \
                     is gone.\n",
                    plist.display()
                )),
            }
        }
    }
    let Some(named) = trust
        .apps_env()
        .filter(|named| !named.is_empty() && ours(dir, named))
    else {
        return said;
    };
    let before = std::fs::read_to_string(dir.join(BEFORE))
        .ok()
        .map(|before| before.trim().to_string())
        .filter(|before| !before.is_empty());
    let done = match &before {
        Some(before) => trust.set_apps_env(before).map(|()| {
            format!("Apps are pointed at {before} again with {NODE_EXTRA_CA_CERTS}, as before.")
        }),
        None => trust
            .unset_apps_env()
            .map(|()| format!("{NODE_EXTRA_CA_CERTS} no longer names it to apps.")),
    };
    match done {
        Ok(line) => said.push_str(&format!("{line} Restart the Claude app, if it is open.\n")),
        Err(refused) => {
            let fix = match &before {
                Some(before) => apps_command("macos", Path::new(before)),
                None => unapps_command("macos"),
            };
            said.push_str(&format!(
                "{NODE_EXTRA_CA_CERTS} still names {named} to apps ({refused}); `{fix}` puts it \
                 right.\n"
            ));
        }
    }
    said
}

/// Whether apps on Node, the Claude app among them, are pointed at the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Apps {
    /// `NODE_EXTRA_CA_CERTS` names the root, or the file holding it and
    /// theirs; and whether the LaunchAgent names it again at each login.
    Named { file: PathBuf, at_login: bool },
    /// It names another file.
    Other(String),
    /// It names nothing.
    Unnamed,
}

/// What launchd names to apps now, and whether the LaunchAgent is there.
pub fn apps(dir: &Path, trust: &dyn Trust) -> Apps {
    let at_login = trust
        .launch_agents()
        .is_ok_and(|agents| agent_path(&agents).exists());
    match trust.apps_env().filter(|named| !named.is_empty()) {
        Some(named) if ours(dir, &named) => Apps::Named {
            file: PathBuf::from(named),
            at_login,
        },
        Some(named) => Apps::Other(named),
        None => Apps::Unnamed,
    }
}

impl Apps {
    pub fn said(&self) -> String {
        match self {
            Apps::Named {
                file,
                at_login: true,
            } => format!(
                "For the Claude app: in place. Apps are pointed at it with \
                 {NODE_EXTRA_CA_CERTS}={}, named again at each login by the LaunchAgent {AGENT}.",
                file.display()
            ),
            Apps::Named {
                file,
                at_login: false,
            } => format!(
                "For the Claude app: apps are pointed at it now with {NODE_EXTRA_CA_CERTS}={}, \
                 but nothing names it again at the next login. `meridian authority trust` does.",
                file.display()
            ),
            Apps::Other(named) => format!(
                "For the Claude app: not in place. {NODE_EXTRA_CA_CERTS} names {named} to apps; \
                 `meridian authority trust` offers a file holding both."
            ),
            Apps::Unnamed => format!(
                "For the Claude app: not in place. It reads its roots from \
                 {NODE_EXTRA_CA_CERTS}, which names nothing to apps here; `meridian authority \
                 trust` names it."
            ),
        }
    }
}

/// What the root is for, said before anything asks to trust it.
pub fn purpose(dir: &Path) -> String {
    format!(
        "This machine has its own certificate authority for Open Meridian:\n\
         \x20 {root}\n\
         It signs the HTTPS certificates of deployments on this machine, at names under \
         .localhost and nothing else, so a browser, the Claude Code CLI and the Claude app reach \
         them over HTTPS without a warning. Its key stays in {dir}, readable by you alone. \
         `meridian authority remove` takes it away again.",
        root = root_path(dir).display(),
        dir = dir.display(),
    )
}

/// What the person's machine is asked, so a test can be the machine.
pub trait Trust {
    /// `macos`, `linux`, `windows`, as `std::env::consts::OS` says.
    fn os(&self) -> &str;
    /// Why macOS could show no dialog here for the person to answer, so the
    /// keychain is not asked at all; `None` where it could.
    fn no_dialog(&self) -> Option<String>;
    /// Add the root to the login keychain, which asks for the person's
    /// password itself. Only on macOS, and only after the person said yes.
    fn add(&self, root: &Path) -> Result<(), String>;
    /// Take it out of the login keychain, with its trust settings.
    fn remove(&self, sha1: &str) -> Result<(), String>;
    /// `NODE_EXTRA_CA_CERTS` as launchd gives it to apps: `launchctl getenv`.
    fn apps_env(&self) -> Option<String>;
    /// And as this process was given it.
    fn own_env(&self) -> Option<String>;
    /// `launchctl setenv NODE_EXTRA_CA_CERTS <file>`: for apps started from
    /// now on.
    fn set_apps_env(&self, file: &str) -> Result<(), String>;
    /// `launchctl unsetenv NODE_EXTRA_CA_CERTS`.
    fn unset_apps_env(&self) -> Result<(), String>;
    /// Where the person's LaunchAgents are: `~/Library/LaunchAgents`.
    fn launch_agents(&self) -> Result<PathBuf, String>;
    /// `launchctl load <plist>`, and `unload`.
    fn load(&self, plist: &Path) -> Result<(), String>;
    fn unload(&self, plist: &Path) -> Result<(), String>;
}

/// The person's own machine: `security` and `launchctl`, run where they can
/// answer them.
pub struct ThisMachine;

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "there is no home directory".into())
}

fn login_keychain() -> Result<PathBuf, String> {
    Ok(home()?.join("Library/Keychains/login.keychain-db"))
}

/// How long `security` is given for its dialog to be answered: time for a
/// person to find it and type their password, and an end where nobody can.
const DIALOG_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);

/// Why no dialog of macOS's could reach the person from this session, or
/// `None` where one could: an SSH session (`SSH_CONNECTION` or `SSH_TTY`), or
/// one launchd does not call `Aqua`, the session on the Mac's own screen.
/// Where launchd cannot say, `None`: the time limit is the backstop.
fn no_dialog_here(ssh: bool, manager: Option<&str>) -> Option<String> {
    if ssh {
        return Some(
            "this is an SSH session, and macOS shows its dialog on the Mac's own screen".into(),
        );
    }
    match manager.map(str::trim) {
        Some(name) if !name.is_empty() && name != "Aqua" => Some(format!(
            "launchd calls this session {name}, not Aqua, the session on the Mac's own screen"
        )),
        _ => None,
    }
}

/// A limit as it is said: `2 minutes`, `a second`.
fn spoken(limit: std::time::Duration) -> String {
    let seconds = limit.as_secs();
    match (seconds / 60, seconds % 60) {
        (1, 0) => "a minute".into(),
        (minutes, 0) if minutes > 1 => format!("{minutes} minutes"),
        (0, 1) => "a second".into(),
        _ => format!("{seconds} seconds"),
    }
}

/// `program`, its three streams inherited, stopped if it has not finished
/// within `limit`: a dialog nobody answers fails the step instead of
/// waiting for ever.
fn run_within(
    program: &str,
    arguments: &[&std::ffi::OsStr],
    limit: std::time::Duration,
) -> Result<(), String> {
    let mut child = std::process::Command::new(program)
        .args(arguments)
        .spawn()
        .map_err(|failed| format!("{program} could not be run: {failed}"))?;
    let ends = std::time::Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("{program} said no ({status})")),
            Ok(None) if std::time::Instant::now() >= ends => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "macOS's dialog for your password was not answered within {}, so {program} \
                     was stopped",
                    spoken(limit)
                ));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(failed) => return Err(format!("{program} could not be waited for: {failed}")),
        }
    }
}

fn security(arguments: &[&std::ffi::OsStr]) -> Result<(), String> {
    // Inherited, all three: macOS asks for the password in its own dialog,
    // and says why it refused here.
    run_within("security", arguments, DIALOG_LIMIT)
}

/// `launchctl`, its output kept: what it says is the answer, or why not.
fn launchctl(arguments: &[&std::ffi::OsStr]) -> Result<String, String> {
    let output = std::process::Command::new("launchctl")
        .args(arguments)
        .output()
        .map_err(|failed| format!("launchctl could not be run: {failed}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(match said.is_empty() {
        true => format!("launchctl said no ({})", output.status),
        false => format!("launchctl said: {said}"),
    })
}

impl Trust for ThisMachine {
    fn os(&self) -> &str {
        std::env::consts::OS
    }

    fn no_dialog(&self) -> Option<String> {
        if self.os() != "macos" {
            return None;
        }
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        let ssh = set("SSH_CONNECTION") || set("SSH_TTY");
        let manager = match ssh {
            true => None,
            false => launchctl(&["managername".as_ref()]).ok(),
        };
        no_dialog_here(ssh, manager.as_deref())
    }

    fn add(&self, root: &Path) -> Result<(), String> {
        let keychain = login_keychain()?;
        security(&[
            "add-trusted-cert".as_ref(),
            "-r".as_ref(),
            "trustRoot".as_ref(),
            "-k".as_ref(),
            keychain.as_os_str(),
            root.as_os_str(),
        ])
    }

    fn remove(&self, sha1: &str) -> Result<(), String> {
        let keychain = login_keychain()?;
        security(&[
            "delete-certificate".as_ref(),
            "-t".as_ref(),
            "-Z".as_ref(),
            sha1.as_ref(),
            keychain.as_os_str(),
        ])
    }

    fn apps_env(&self) -> Option<String> {
        if self.os() != "macos" {
            return None;
        }
        launchctl(&["getenv".as_ref(), NODE_EXTRA_CA_CERTS.as_ref()])
            .ok()
            .filter(|named| !named.is_empty())
    }

    fn own_env(&self) -> Option<String> {
        std::env::var(NODE_EXTRA_CA_CERTS)
            .ok()
            .filter(|named| !named.is_empty())
    }

    fn set_apps_env(&self, file: &str) -> Result<(), String> {
        launchctl(&[
            "setenv".as_ref(),
            NODE_EXTRA_CA_CERTS.as_ref(),
            file.as_ref(),
        ])
        .map(|_| ())
    }

    fn unset_apps_env(&self) -> Result<(), String> {
        launchctl(&["unsetenv".as_ref(), NODE_EXTRA_CA_CERTS.as_ref()]).map(|_| ())
    }

    fn launch_agents(&self) -> Result<PathBuf, String> {
        Ok(home()?.join("Library/LaunchAgents"))
    }

    fn load(&self, plist: &Path) -> Result<(), String> {
        launchctl(&["load".as_ref(), plist.as_os_str()]).map(|_| ())
    }

    fn unload(&self, plist: &Path) -> Result<(), String> {
        launchctl(&["unload".as_ref(), plist.as_os_str()]).map(|_| ())
    }
}

/// The authority, ready to sign.
pub struct Prepared {
    pub authority: Authority,
    pub dir: PathBuf,
}

/// The authority in `dir`, or a new one made and written there.
fn held_or_made(dir: &Path, machine: &str, now_s: u64) -> Result<Authority, String> {
    match Authority::read(dir)? {
        Some(held) => Ok(held),
        None => {
            let made = Authority::make(machine, now_s)?;
            made.write(dir)?;
            Ok(made)
        }
    }
}

/// Where macOS could show no dialog: what is not done, and each step as a
/// command, for a session on the Mac's own screen.
fn no_dialog_said(why: &str, trust: &dyn Trust, dir: &Path) -> String {
    format!(
        "Not trusted yet: macOS asks for your password in a dialog on this Mac's own screen, and \
         none can be shown here ({why}), so the keychain was not asked. In Terminal on this Mac's \
         screen, run `meridian authority trust`, or each step yourself. {}",
        by_hand(trust, dir)
    )
}

/// What the root is for, and then whether to trust it: on macOS the login
/// keychain and, after it, apps on Node, when the person says yes; anywhere
/// else, or when they say no, or where macOS could show no dialog, each step
/// as a command instead. Remembered once the keychain took it, or, off
/// macOS, once the commands were said.
fn ask_once(dir: &Path, trust: &dyn Trust, ask: &dyn Fn(&str) -> bool, say: &mut dyn FnMut(&str)) {
    say(&purpose(dir));
    let asked = if trust.os() == "macos" {
        if let Some(why) = trust.no_dialog() {
            say(&no_dialog_said(&why, trust, dir));
            false
        } else if ask(
            "Trust it now? macOS asks for your password to add it to your login keychain, and \
             apps started from now on, the Claude app among them, are pointed at it",
        ) {
            match trust.add(&root_path(dir)) {
                Ok(()) => {
                    say("Added to your login keychain, trusted for HTTPS.");
                    point_apps(dir, trust, ask, say);
                    true
                }
                Err(refused) => {
                    say(&format!(
                        "It was not added: {refused}. {}",
                        by_hand(trust, dir)
                    ));
                    false
                }
            }
        } else {
            say(&format!("Not trusted. {}", by_hand(trust, dir)));
            false
        }
    } else {
        say(&by_hand(trust, dir));
        true
    };
    if asked {
        let _ = std::fs::write(dir.join(ASKED), "");
    }
}

/// The authority in `dir`, made if there is none; and the machine asked to
/// trust it, once, having been told what it is for. `up` and
/// `upgrade-deployment --https` come here.
pub fn prepare(
    dir: &Path,
    machine: &str,
    now_s: u64,
    trust: &dyn Trust,
    ask: &dyn Fn(&str) -> bool,
    say: &mut dyn FnMut(&str),
) -> Result<Prepared, String> {
    let authority = held_or_made(dir, machine, now_s)?;
    if !dir.join(ASKED).exists() {
        ask_once(dir, trust, ask, say);
    }
    Ok(Prepared {
        authority,
        dir: dir.to_path_buf(),
    })
}

/// `meridian authority trust`, which the install script runs: the authority
/// made if there is none, and trusted here. Asked as `up` asks, the first
/// time; where the keychain took it before and apps are not pointed at it
/// yet, asked about apps alone; where both are done, said so. Off macOS,
/// each step as a command, every time.
pub fn trust_here(
    dir: &Path,
    machine: &str,
    now_s: u64,
    trust: &dyn Trust,
    ask: &dyn Fn(&str) -> bool,
    say: &mut dyn FnMut(&str),
) -> Result<(), String> {
    held_or_made(dir, machine, now_s)?;
    if !dir.join(ASKED).exists() {
        ask_once(dir, trust, ask, say);
    } else if trust.os() != "macos" {
        say(&by_hand(trust, dir));
    } else if let Apps::Named { at_login: true, .. } = apps(dir, trust) {
        say(&format!(
            "This machine's certificate authority is trusted here already: in your login \
             keychain, and named to apps, the Claude app among them, with {NODE_EXTRA_CA_CERTS}."
        ));
    } else {
        say(&format!(
            "This machine's certificate authority, {}, is in your login keychain, which a browser \
             and the Claude Code CLI read. The Claude app, like any app on Node, reads \
             {NODE_EXTRA_CA_CERTS} instead.",
            root_path(dir).display()
        ));
        if ask("Point apps started from now on at it, and again at each login?") {
            point_apps(dir, trust, ask, say);
        } else {
            say(&format!(
                "Not pointed. To do it yourself, and restart the Claude app:\n  {}",
                apps_commands("macos", dir, theirs(trust, dir).as_deref()).join("\n  ")
            ));
        }
    }
    Ok(())
}

/// The authority taken away: on macOS, apps no longer pointed at it, as
/// before it, and out of the login keychain; elsewhere, the commands that
/// undo what the person was told to run; and its files removed. What was
/// done, to say.
pub fn remove(dir: &Path, trust: &dyn Trust) -> Result<String, String> {
    let mut said = match trust.os() {
        "macos" => unpoint_apps(dir, trust),
        _ => String::new(),
    };
    let root = std::fs::read_to_string(root_path(dir)).ok();
    if root.is_none() && !dir.exists() {
        said.push_str(&format!(
            "There is no certificate authority at {}; nothing to remove.",
            dir.display()
        ));
        return Ok(said);
    }
    if let Some(root_pem) = root {
        let sha1 = Authority {
            root_pem,
            key_pem: String::new(),
        }
        .sha1();
        if trust.os() == "macos" {
            match trust.no_dialog() {
                Some(why) => said.push_str(&format!(
                    "Not taken out of your login keychain: macOS asks for your password in a \
                     dialog on this Mac's own screen, and none can be shown here ({why}). In \
                     Terminal on this Mac's screen, `{}` takes it out.\n",
                    untrust_command("macos", &sha1)
                )),
                None => match trust.remove(&sha1) {
                    Ok(()) => said.push_str("Removed it from your login keychain.\n"),
                    Err(refused) => said.push_str(&format!(
                        "It was not in your login keychain, or could not be taken out \
                         ({refused}). If Keychain Access lists it, `{}` removes it.\n",
                        untrust_command("macos", &sha1)
                    )),
                },
            }
        } else {
            said.push_str(&format!(
                "If this machine was told to trust it, take that back with:\n  {}\n\
                 and if {NODE_EXTRA_CA_CERTS} names it or a file holding it:\n  {}\n",
                untrust_command(trust.os(), &sha1),
                unapps_command(trust.os())
            ));
        }
    }
    std::fs::remove_dir_all(dir)
        .map_err(|failed| format!("could not remove {}: {failed}", dir.display()))?;
    said.push_str(&format!(
        "Removed {}. A deployment it signed for keeps that certificate until `meridian up` or \
         `meridian upgrade-deployment --https` issues one from a new authority.\n",
        dir.display()
    ));
    Ok(said)
}

#[cfg(test)]
#[path = "authority/tests.rs"]
mod tests;

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
//! which asks for the person's password itself, so this never handles one;
//! elsewhere the one command to run is printed. Nothing here does either
//! without `Trust`, which a test replaces.

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
/// or, where this cannot run the step itself, once the command was printed.
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
    let said = path.display().to_string();
    if said
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-~".contains(c))
    {
        said
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

/// What an agent running on Node, Claude Code among them, is pointed at the
/// root with. Whether Claude Code reads the macOS keychain without it is not
/// documented; the task's spike decides what to recommend.
pub fn node_line(root: &Path) -> String {
    format!("NODE_EXTRA_CA_CERTS={}", shell(root))
}

/// What the root is for, said before anything asks to trust it.
pub fn purpose(dir: &Path) -> String {
    format!(
        "This machine has its own certificate authority for Open Meridian:\n\
         \x20 {root}\n\
         It signs the HTTPS certificates of deployments on this machine, at names under \
         .localhost and nothing else, so a browser and an agent such as Claude Code reach them \
         over HTTPS without a warning. Its key stays in {dir}, readable by you alone. \
         `meridian authority remove` takes it away again.",
        root = root_path(dir).display(),
        dir = dir.display(),
    )
}

/// What the person's machine is asked, so a test can be the machine.
pub trait Trust {
    /// `macos`, `linux`, `windows`, as `std::env::consts::OS` says.
    fn os(&self) -> &str;
    /// Add the root to the login keychain, which asks for the person's
    /// password itself. Only on macOS, and only after the person said yes.
    fn add(&self, root: &Path) -> Result<(), String>;
    /// Take it out of the login keychain, with its trust settings.
    fn remove(&self, sha1: &str) -> Result<(), String>;
}

/// The person's own machine: `security`, run where they can answer it.
pub struct ThisMachine;

fn login_keychain() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join("Library/Keychains/login.keychain-db"))
        .ok_or_else(|| "there is no home directory, so no login keychain".into())
}

fn security(arguments: &[&std::ffi::OsStr]) -> Result<(), String> {
    // Inherited, all three: macOS asks for the password in its own dialog,
    // and says why it refused here.
    let status = std::process::Command::new("security")
        .args(arguments)
        .status()
        .map_err(|failed| format!("security could not be run: {failed}"))?;
    match status.success() {
        true => Ok(()),
        false => Err(format!("security said no ({status})")),
    }
}

impl Trust for ThisMachine {
    fn os(&self) -> &str {
        std::env::consts::OS
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
}

/// The authority, ready to sign.
pub struct Prepared {
    pub authority: Authority,
    pub dir: PathBuf,
}

/// The authority in `dir`, made if there is none; and the machine asked to
/// trust it, once, having been told what it is for. On macOS that is the
/// login keychain, when the person says yes to `ask`; anywhere else, or when
/// they say no, the command that does it is said instead.
pub fn prepare(
    dir: &Path,
    machine: &str,
    now_s: u64,
    trust: &dyn Trust,
    ask: &dyn Fn(&str) -> bool,
    say: &mut dyn FnMut(&str),
) -> Result<Prepared, String> {
    let authority = match Authority::read(dir)? {
        Some(held) => held,
        None => {
            let made = Authority::make(machine, now_s)?;
            made.write(dir)?;
            made
        }
    };
    if !dir.join(ASKED).exists() {
        let root = root_path(dir);
        let command = trust_command(trust.os(), &root);
        say(&purpose(dir));
        let asked = if trust.os() == "macos" {
            if ask("Add it to your login keychain now? macOS asks for your password itself") {
                match trust.add(&root) {
                    Ok(()) => {
                        say("Added to your login keychain, trusted for HTTPS.");
                        true
                    }
                    Err(refused) => {
                        say(&format!(
                            "It was not added: {refused}. To add it later:\n  {command}"
                        ));
                        false
                    }
                }
            } else {
                say(&format!("Not added. To add it later:\n  {command}"));
                false
            }
        } else {
            say(&format!("To trust it, run:\n  {command}"));
            true
        };
        if asked {
            let _ = std::fs::write(dir.join(ASKED), "");
        }
    }
    Ok(Prepared {
        authority,
        dir: dir.to_path_buf(),
    })
}

/// The authority taken away: out of the login keychain on macOS, its files
/// removed. What was done, to say.
pub fn remove(dir: &Path, trust: &dyn Trust) -> Result<String, String> {
    let root = std::fs::read_to_string(root_path(dir)).ok();
    if root.is_none() && !dir.exists() {
        return Ok(format!(
            "There is no certificate authority at {}; nothing to remove.",
            dir.display()
        ));
    }
    let mut said = String::new();
    if let Some(root_pem) = root {
        let sha1 = Authority {
            root_pem,
            key_pem: String::new(),
        }
        .sha1();
        if trust.os() == "macos" {
            match trust.remove(&sha1) {
                Ok(()) => said.push_str("Removed it from your login keychain.\n"),
                Err(refused) => said.push_str(&format!(
                    "It was not in your login keychain, or could not be taken out ({refused}). \
                     If Keychain Access lists it, `{}` removes it.\n",
                    untrust_command("macos", &sha1)
                )),
            }
        } else {
            said.push_str(&format!(
                "If this machine was told to trust it, take that back with:\n  {}\n",
                untrust_command(trust.os(), &sha1)
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

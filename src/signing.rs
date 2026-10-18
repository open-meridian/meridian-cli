//! `meridian plugin key setup` (ruling 256): the key a publisher signs
//! release tags with, made ready in one step, and offered by `plugin new` on
//! first use.
//!
//! - **The key** is git's configured SSH signing key where there is one
//!   (`gpg.format ssh` and `user.signingkey`); otherwise `ssh-keygen` makes
//!   one: `-t ed25519`, a desktop key, by default, or `-t ed25519-sk` with
//!   `--hardware`, a key held on a FIDO authenticator (ruling 255: both are
//!   supported; the desktop key is the documented default). ssh-keygen asks
//!   for the passphrase itself, at the terminal: this never reads one, and
//!   never reads the private half.
//! - **git** is set to sign with it: `gpg.format ssh` and `user.signingkey`,
//!   globally, or in the plugin's repository alone where git signs with a GPG
//!   key globally, so nothing else this person signs changes.
//! - **The platform**: its public half registered on the organisation
//!   through the existing row (RegisterSigningKey, as `meridian plugin key
//!   register` sends it), where it is not registered already. Only an Owner
//!   or Admin registers a key; anybody else is told to ask one, with the
//!   public half to give them.

use std::path::{Path, PathBuf};

use crate::registry;

/// This machine, as setup reaches it: git's configuration, files, and
/// ssh-keygen, which a test stands in for.
pub trait Workstation: Sync {
    /// `git config --get <key>`, as git reads it in `dir` (its repository's
    /// own, then the global).
    fn git_config(&self, dir: &Path, key: &str) -> Option<String>;
    /// `git config [--global|--local] <key> <value>`.
    fn set_git_config(
        &self,
        dir: &Path,
        global: bool,
        key: &str,
        value: &str,
    ) -> Result<(), String>;
    /// Whether `dir` is in a git repository.
    fn in_repository(&self, dir: &Path) -> bool;
    fn read(&self, path: &Path) -> Option<String>;
    fn exists(&self, path: &Path) -> bool;
    /// `ssh-keygen -t <kind> -f <file> -C <comment>`, at the terminal: it asks
    /// for the passphrase itself.
    fn keygen(&self, kind: &str, file: &Path, comment: &str) -> Result<(), String>;
    fn home(&self) -> Option<PathBuf>;
}

pub struct ThisWorkstation;

impl Workstation for ThisWorkstation {
    fn git_config(&self, dir: &Path, key: &str) -> Option<String> {
        let shown = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", "--get", key])
            .output()
            .ok()?;
        shown
            .status
            .success()
            .then(|| String::from_utf8_lossy(&shown.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn set_git_config(
        &self,
        dir: &Path,
        global: bool,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "config",
                if global { "--global" } else { "--local" },
                key,
                value,
            ])
            .output()
            .map_err(|failed| format!("git could not be run: {failed}"))?;
        if done.status.success() {
            Ok(())
        } else {
            Err(format!(
                "`git config {key}` failed: {}",
                String::from_utf8_lossy(&done.stderr).trim()
            ))
        }
    }

    fn in_repository(&self, dir: &Path) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["rev-parse", "--git-dir"])
            .output()
            .is_ok_and(|shown| shown.status.success())
    }

    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn keygen(&self, kind: &str, file: &Path, comment: &str) -> Result<(), String> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|failed| format!("{}: {failed}", dir.display()))?;
        }
        // Its own terminal: ssh-keygen asks for the passphrase, and for a
        // touch of the authenticator with -sk; none of it passes through here.
        let status = std::process::Command::new("ssh-keygen")
            .args(["-t", kind, "-f"])
            .arg(file)
            .args(["-C", comment])
            .status()
            .map_err(|failed| match failed.kind() {
                std::io::ErrorKind::NotFound => {
                    "ssh-keygen is not installed here: it comes with OpenSSH".to_string()
                }
                _ => format!("ssh-keygen could not be run: {failed}"),
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "ssh-keygen did not make the key ({status}){}",
                if kind.ends_with("-sk") {
                    ": a hardware key needs a FIDO authenticator plugged in and touched when it \
                     blinks"
                } else {
                    ""
                }
            ))
        }
    }

    fn home(&self) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
    }
}

/// What `plugin key setup` was asked.
pub struct Setup {
    pub dir: PathBuf,
    pub hardware: bool,
}

/// The key made ready: its public line, the file it is in, and what was done
/// to make it so, each said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub line: String,
    pub file: Option<PathBuf>,
    pub did: Vec<String>,
}

/// Whether git signs with an SSH key already: `gpg.format ssh` and a
/// `user.signingkey`.
pub fn has_ssh_key(machine: &dyn Workstation, dir: &Path) -> bool {
    machine.git_config(dir, "gpg.format").as_deref() == Some("ssh")
        && machine.git_config(dir, "user.signingkey").is_some()
}

fn expanded(machine: &dyn Workstation, path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), machine.home()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

/// The public line of git's configured SSH signing key: `key::<line>`, or a
/// path to the public half, or to the private half with its `.pub` beside.
fn configured_line(
    machine: &dyn Workstation,
    configured: &str,
) -> Result<(String, Option<PathBuf>), String> {
    if let Some(line) = configured.strip_prefix("key::") {
        return Ok((registry::public_key(line)?, None));
    }
    let path = expanded(machine, configured);
    let public = if path.extension().is_some_and(|x| x == "pub") {
        path
    } else {
        PathBuf::from(format!("{}.pub", path.display()))
    };
    let text = machine.read(&public).ok_or_else(|| {
        format!(
            "git signs with {configured}, and its public half, {}, is not here: put it back, or \
             point user.signingkey at the key's .pub",
            public.display()
        )
    })?;
    Ok((registry::public_key(&text)?, Some(public)))
}

/// The file setup makes a new key in: `~/.ssh/id_ed25519_meridian`, or
/// `~/.ssh/id_ed25519_sk_meridian` for a hardware key.
pub fn new_key_file(machine: &dyn Workstation, hardware: bool) -> Result<PathBuf, String> {
    let home = machine
        .home()
        .ok_or("there is no home directory to keep a key in")?;
    Ok(home.join(".ssh").join(if hardware {
        "id_ed25519_sk_meridian"
    } else {
        "id_ed25519_meridian"
    }))
}

/// git's SSH signing key, or a new one made and git set to sign with it.
pub fn prepare(machine: &dyn Workstation, asked: &Setup) -> Result<Ready, String> {
    let dir = &asked.dir;
    let format = machine.git_config(dir, "gpg.format");
    let configured = machine.git_config(dir, "user.signingkey");
    if let (Some("ssh"), Some(configured)) = (format.as_deref(), configured.as_deref()) {
        let (line, file) = configured_line(machine, configured)?;
        if asked.hardware && !line.starts_with("sk-") {
            return Err(format!(
                "git signs with a desktop key already ({configured}). To sign with a hardware key \
                 instead, `git config --global --unset user.signingkey`, then `meridian plugin key \
                 setup --hardware`; revoke the old key at the platform once nothing needs it"
            ));
        }
        return Ok(Ready {
            line,
            file,
            did: vec![format!(
                "git signs with {configured} already; it is the key used"
            )],
        });
    }
    // git signs with a GPG key globally: an SSH key for the plugin's
    // repository alone, so nothing else this person signs changes.
    let gpg = configured.is_some() && format.as_deref().unwrap_or("openpgp") != "ssh";
    let global = !gpg;
    if gpg && !machine.in_repository(dir) {
        return Err(format!(
            "git signs with a GPG key ({}), which the platform does not take for tags. Run this \
             in the plugin's repository, where an SSH key is set for it alone, or --dir names it",
            configured.unwrap_or_default()
        ));
    }
    let file = new_key_file(machine, asked.hardware)?;
    let public = PathBuf::from(format!("{}.pub", file.display()));
    let mut did = Vec::new();
    if machine.exists(&public) {
        did.push(format!(
            "{} is here already; it is the key used",
            public.display()
        ));
    } else if machine.exists(&file) {
        return Err(format!(
            "{} is here without its public half: `ssh-keygen -y -f {} > {}` writes it, then run \
             this again",
            file.display(),
            file.display(),
            public.display()
        ));
    } else {
        let kind = if asked.hardware {
            "ed25519-sk"
        } else {
            "ed25519"
        };
        let comment = match machine.git_config(dir, "user.email") {
            Some(email) => format!("{email} tag signing"),
            None => "meridian tag signing".to_string(),
        };
        eprintln!(
            "Making {} with ssh-keygen ({kind}). It asks for a passphrase{}; this never sees it.",
            file.display(),
            if asked.hardware {
                ", and for a touch of your authenticator"
            } else {
                ""
            }
        );
        machine.keygen(kind, &file, &comment)?;
        did.push(format!("made {} with ssh-keygen ({kind})", file.display()));
    }
    let text = machine
        .read(&public)
        .ok_or_else(|| format!("{} was not written", public.display()))?;
    let line = registry::public_key(&text)?;
    let path = public.display().to_string();
    machine.set_git_config(dir, global, "gpg.format", "ssh")?;
    machine.set_git_config(dir, global, "user.signingkey", &path)?;
    did.push(format!(
        "set git {}to sign with it: gpg.format ssh, user.signingkey {path}",
        if global { "" } else { "in this repository " }
    ));
    Ok(Ready {
        line,
        file: Some(public),
        did,
    })
}

/// An SSH public key's type and body, which name it whatever its comment.
fn named_by(line: &str) -> String {
    line.split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

/// What registering came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    /// It was registered on the organisation already, active: its key ID.
    Already(String),
    /// Registered now: the key's ID and fingerprint.
    Made { key_id: String, fingerprint: String },
    /// The person is no Owner or Admin: what to tell them.
    Ask(String),
}

/// The key registered on the organisation, unless it is already.
pub async fn register(
    signed_in: &registry::SignedIn,
    organisation: &str,
    ready: &Ready,
) -> Result<Registered, String> {
    let listed = registry::list_keys(signed_in, organisation).await?;
    let wanted = named_by(&ready.line);
    if let Some(key) = listed["keys"].as_array().into_iter().flatten().find(|key| {
        key["public_key"].as_str().map(named_by).as_deref() == Some(wanted.as_str())
            && key["revoked_at_ns"].as_u64().unwrap_or(0) == 0
    }) {
        return Ok(Registered::Already(
            key["key_id"].as_str().unwrap_or_default().to_string(),
        ));
    }
    let name = registry::comment_of(&ready.line).unwrap_or_else(|| "tag signing".into());
    match registry::register_key(signed_in, organisation, &name, &ready.line).await {
        Ok(key) => Ok(Registered::Made {
            key_id: key["key_id"].as_str().unwrap_or_default().to_string(),
            fingerprint: key["fingerprint"].as_str().unwrap_or_default().to_string(),
        }),
        Err(refused) if refused.contains("Owner or Admin") => {
            let file = ready
                .file
                .as_ref()
                .map(|file| file.display().to_string())
                .unwrap_or_else(|| "<the key's .pub>".into());
            Ok(Registered::Ask(format!(
                "Only an Owner or Admin of {organisation} registers a signing key. Ask one of \
                 them to register its public half -- on the organisation's Signing keys page at \
                 {platform}, or with `meridian plugin key register {file} --organisation \
                 {organisation}` -- giving them this line:\n\n  {line}\n",
                platform = signed_in.platform(),
                line = ready.line
            )))
        }
        Err(refused) => Err(refused),
    }
}

/// What setup did, said.
pub fn said(ready: &Ready) -> String {
    let mut out = String::new();
    for step in &ready.did {
        out.push_str(&format!("  {step}\n"));
    }
    out
}

#[cfg(test)]
mod tests;

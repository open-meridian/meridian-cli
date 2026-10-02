//! The sessions file: one per deployment, readable only by the person whose
//! it is, holding the address and the one credential for it and nothing
//! else (spec/the-cli, requirement 14): a delegation's pair -- the client
//! this computer registered as, an access token and a refresh token, and
//! when each lapses (decisions/029) -- or, written by a CLI from before
//! delegations, a terminal session, honoured until it lapses.
//!
//! It is called the sessions file and never the configuration, because
//! `--config` would then mean two things in one tool. Not the system
//! keychain: four platforms with four keychains is four implementations of
//! the same file, and the file is the one every target has.

use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Held {
    pub address: String,
    pub subject: String,
    /// When the credential lapses, RFC 3339, as the deployment said it.
    pub expires_at: String,
    /// A terminal session from before delegations; empty for a delegation.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session: String,
    /// The client this computer registered as.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub access_token: String,
    /// When the access token lapses, in seconds since the epoch.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub access_expires_at_s: u64,
    /// Single use: refreshing spends it and writes the next one here.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub refresh_token: String,
    /// When the delegation lapses, in seconds since the epoch.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub expires_at_s: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Held {
    /// A delegation's pair, as against an older CLI's terminal session.
    pub fn is_delegation(&self) -> bool {
        !self.refresh_token.is_empty()
    }
}

/// Where the sessions files live: `$XDG_CONFIG_HOME/meridian/sessions`, or
/// `~/.config/meridian/sessions`, or `%APPDATA%\meridian\sessions`.
pub fn directory() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|held| !held.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA")
                .filter(|held| !held.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|held| !held.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .ok_or("there is no home directory to keep a session in")?;
    Ok(base.join("meridian").join("sessions"))
}

/// A deployment's address as a file name: its host and port, and nothing a
/// file system could read as a path.
fn file_name(address: &str) -> String {
    let bare = address
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let named: String = bare
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{named}.json")
}

pub fn read(within: &Path, address: &str) -> Option<Held> {
    let text = std::fs::read_to_string(within.join(file_name(address))).ok()?;
    serde_json::from_str(&text).ok()
}

/// Every session held, for `sign-out` with no address.
pub fn all(within: &Path) -> Vec<Held> {
    let Ok(entries) = std::fs::read_dir(within) else {
        return Vec::new();
    };
    let mut held: Vec<Held> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect();
    held.sort_by(|a, b| a.address.cmp(&b.address));
    held
}

/// Write a session, readable by its owner alone from the moment it exists:
/// written to a file made with those permissions and then renamed into
/// place, so there is no instant when it is anybody else's to read.
pub fn write(within: &Path, held: &Held) -> Result<(), String> {
    create_private_directory(within)?;
    let path = within.join(file_name(&held.address));
    let partial = within.join(format!(".{}.partial", file_name(&held.address)));
    let _ = std::fs::remove_file(&partial);
    let text = serde_json::to_string_pretty(held).map_err(|failed| failed.to_string())?;
    let mut file = private_file(&partial)
        .map_err(|failed| format!("could not write {}: {failed}", partial.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|failed| format!("could not write {}: {failed}", partial.display()))?;
    std::fs::rename(&partial, &path)
        .map_err(|failed| format!("could not write {}: {failed}", path.display()))
}

/// Hold a deployment's file exclusively until the returned file is dropped:
/// what refreshing takes, so two commands at once never present one refresh
/// token twice -- which would revoke the delegation (spec/clients-act-on-a-persons-delegation,
/// requirement 21). A file of its own beside the sessions file, since that
/// one is replaced by a rename and a lock on it would be a lock on a file
/// nobody reads any more.
pub fn lock(within: &Path, address: &str) -> Result<std::fs::File, String> {
    create_private_directory(within)?;
    let path = within.join(format!(".{}.lock", file_name(address)));
    let file = open_lock(&path)
        .map_err(|failed| format!("could not open {}: {failed}", path.display()))?;
    file.lock()
        .map_err(|failed| format!("could not lock {}: {failed}", path.display()))?;
    Ok(file)
}

#[cfg(unix)]
fn open_lock(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_lock(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

pub fn forget(within: &Path, address: &str) -> Result<(), String> {
    match std::fs::remove_file(within.join(file_name(address))) {
        Ok(()) => Ok(()),
        Err(failed) if failed.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(failed) => Err(failed.to_string()),
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|failed| format!("could not make {}: {failed}", path.display()))?;
    // And made private if it was there already, somebody else's to list.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|failed| format!("could not make {} private: {failed}", path.display()))
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> Result<(), String> {
    // Under %APPDATA%, which is the person's own profile.
    std::fs::create_dir_all(path)
        .map_err(|failed| format!("could not make {}: {failed}", path.display()))
}

#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("meridian-sessions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn held(address: &str) -> Held {
        Held {
            address: address.into(),
            subject: "local|ada".into(),
            expires_at: "2026-12-25T00:00:00Z".into(),
            client_id: "mdc_test".into(),
            access_token: "mda_test".into(),
            access_expires_at_s: 1_790_381_400,
            refresh_token: "mdr_test".into(),
            expires_at_s: 1_798_156_800,
            ..Held::default()
        }
    }

    #[test]
    fn a_file_an_older_cli_wrote_is_read_as_its_terminal_session() {
        let older: Held = serde_json::from_str(
            r#"{"address":"https://dash.firm.example","session":"s3cr3t","subject":"local|ada","expires_at":"2026-09-26T12:00:00Z"}"#,
        )
        .unwrap();
        assert!(!older.is_delegation());
        assert_eq!(older.session, "s3cr3t");
        assert!(held("x").is_delegation());
        let written = serde_json::to_string(&held("x")).unwrap();
        assert!(!written.contains("\"session\""), "{written}");
    }

    #[test]
    fn the_lock_is_exclusive_across_handles() {
        let within = scratch("lock");
        let first = lock(&within, "https://dash.firm.example").unwrap();
        let second = std::fs::OpenOptions::new()
            .write(true)
            .open(within.join(".dash.firm.example.json.lock"))
            .unwrap();
        assert!(second.try_lock().is_err(), "held by the first");
        drop(first);
        assert!(second.try_lock().is_ok(), "free once the first lets go");
    }

    #[test]
    fn a_session_is_kept_per_deployment_and_read_back() {
        let within = scratch("per-deployment");
        write(&within, &held("https://dash.firm.example")).unwrap();
        write(&within, &held("http://127.0.0.1:8443")).unwrap();
        assert_eq!(
            read(&within, "https://dash.firm.example"),
            Some(held("https://dash.firm.example"))
        );
        assert_eq!(all(&within).len(), 2);
        forget(&within, "https://dash.firm.example").unwrap();
        assert_eq!(read(&within, "https://dash.firm.example"), None);
        forget(&within, "https://dash.firm.example").unwrap();
        assert_eq!(all(&within), vec![held("http://127.0.0.1:8443")]);
    }

    #[test]
    fn an_address_cannot_name_a_file_outside_the_directory() {
        assert_eq!(
            file_name("https://dash.firm.example:8443"),
            "dash.firm.example_8443.json"
        );
        assert!(!file_name("https://../../etc/passwd").contains('/'));
    }

    #[cfg(unix)]
    #[test]
    fn nobody_but_its_owner_can_read_a_session() {
        use std::os::unix::fs::PermissionsExt as _;
        let within = scratch("private");
        std::fs::create_dir_all(&within).unwrap();
        std::fs::set_permissions(&within, std::fs::Permissions::from_mode(0o755)).unwrap();
        write(&within, &held("https://dash.firm.example")).unwrap();
        let file = within.join(file_name("https://dash.firm.example"));
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&within).unwrap().permissions().mode() & 0o777,
            0o700,
            "a directory that was there already is made private too"
        );
    }
}

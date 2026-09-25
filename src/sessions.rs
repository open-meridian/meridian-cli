//! The sessions file: one per deployment, readable only by the person whose
//! it is, holding the address and the session and nothing else
//! (spec/the-cli, requirement 14).
//!
//! It is called the sessions file and never the configuration, because
//! `--config` would then mean two things in one tool. Not the system
//! keychain: four platforms with four keychains is four implementations of
//! the same file, and the file is the one every target has.

use std::io::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Held {
    pub address: String,
    pub session: String,
    pub subject: String,
    pub expires_at: String,
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
            session: "s3cr3t".into(),
            subject: "local|ada".into(),
            expires_at: "2026-09-26T12:00:00Z".into(),
        }
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

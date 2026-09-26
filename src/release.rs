//! Released binaries, and this one replaced by another (spec/the-cli, ruling
//! 8): `meridian --version`, `upgrade` and `uninstall`, and the same
//! download-and-check the install script does.
//!
//! Releases are GitHub's, one binary per target with its `.sha256` beside it
//! (ruling 2). The latest is found by following `/releases/latest`, which
//! GitHub answers with a redirect to its tag -- a page, not the API, so no
//! rate limit a firm's shared address could run out of. Nothing here is
//! asked unless a person asks for it: the CLI never looks for a newer
//! release on its own.
//!
//! The checksum catches a broken download, not a compromised release: it is
//! published beside the binary it describes. Signing is the gap ruling 8
//! names.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest as _, Sha256};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CORE_REV: &str = env!("MERIDIAN_CORE_REV");
pub const TARGET: &str = env!("MERIDIAN_TARGET");

const RELEASES: &str = "https://github.com/open-meridian/meridian-cli/releases";

/// What `meridian --version` says.
pub fn version_line() -> String {
    let core: String = CORE_REV.chars().take(12).collect();
    format!("meridian {VERSION} ({TARGET}; meridian-core {core})")
}

/// Where releases are: GitHub's, or `MERIDIAN_RELEASES` -- a mirror, or a
/// stand-in in a test -- which must be HTTPS unless it is this machine.
pub fn releases() -> Result<String, String> {
    releases_from(std::env::var("MERIDIAN_RELEASES").ok().as_deref())
}

fn releases_from(given: Option<&str>) -> Result<String, String> {
    let Some(given) = given.filter(|held| !held.is_empty()) else {
        return Ok(RELEASES.into());
    };
    let base = given.trim_end_matches('/').to_string();
    let local = ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|at| {
            base == *at
                || base.starts_with(&format!("{at}:"))
                || base.starts_with(&format!("{at}/"))
        });
    if base.starts_with("https://") || local {
        Ok(base)
    } else {
        Err(format!(
            "MERIDIAN_RELEASES is {given}: releases come over https://, or plain http from this machine alone"
        ))
    }
}

/// The release binary for a target. Linux's are built static against musl,
/// so a glibc build is replaced by the musl binary for its architecture.
pub fn asset(target: &str) -> Option<String> {
    let arch = target.split('-').next()?;
    let named = if target.contains("-linux-") {
        format!("{arch}-unknown-linux-musl")
    } else {
        target.to_string()
    };
    [
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
    ]
    .contains(&named.as_str())
    .then(|| format!("meridian-{named}"))
}

/// The tag a `/releases/latest` redirect names: `.../releases/tag/v0.2.0`.
pub fn tag_of(location: &str) -> Option<String> {
    let tag = location.split("/releases/tag/").nth(1)?;
    let tag = tag.split(['?', '#', '/']).next()?;
    (!tag.is_empty()).then(|| tag.to_string())
}

/// `v0.2.0` or `0.2.0` as a version to compare: numbers, dot by dot.
fn numbers(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map_while(|part| part.parse().ok())
        .collect()
}

pub fn compare(a: &str, b: &str) -> Ordering {
    numbers(a).cmp(&numbers(b))
}

/// Whether `bytes` are what a `.sha256` file says: its first field, which is
/// all there is to compare whatever file name `shasum` wrote after it.
pub fn checksum_matches(bytes: &[u8], sha256_file: &str) -> bool {
    let expected = sha256_file.split_whitespace().next().unwrap_or_default();
    expected.len() == 64 && expected.eq_ignore_ascii_case(&format!("{:x}", Sha256::digest(bytes)))
}

/// The binary running, with any link to it followed: what is replaced.
pub fn this_binary() -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe().map_err(|failed| format!("where this binary is: {failed}"))?;
    std::fs::canonicalize(&exe).map_err(|failed| format!("{}: {failed}", exe.display()))
}

/// Whether a file can be written into `dir`, found out by writing one: a
/// root-owned directory, or one a package manager keeps, says no before
/// anything else is done.
pub fn writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(format!(".meridian-probe-{}", std::process::id()));
    std::fs::write(&probe, b"").map_err(|failed| {
        format!(
            "{} cannot be written ({failed}), and this binary lives there; \
             run this as whoever owns it, or reinstall with the install script",
            dir.display()
        )
    })?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// Put `bytes` where `exe` is, whole or not at all: written beside it and
/// renamed over it, so an interrupted upgrade leaves the old binary.
pub fn replace(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = exe.parent().ok_or("this binary is in no directory")?;
    let beside = dir.join(format!(".meridian-upgrade-{}", std::process::id()));
    let written = (|| {
        std::fs::write(&beside, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&beside, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&beside, exe)
    })();
    if let Err(failed) = written {
        let _ = std::fs::remove_file(&beside);
        return Err(format!("{} was left as it was: {failed}", exe.display()));
    }
    Ok(())
}

fn client(redirects: bool) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .redirect(if redirects {
            reqwest::redirect::Policy::limited(5)
        } else {
            reqwest::redirect::Policy::none()
        })
        .user_agent(format!("meridian/{VERSION}"))
        .build()
        .map_err(|failed| failed.to_string())
}

/// The latest release's tag.
pub async fn latest(base: &str) -> Result<String, String> {
    let answer = client(false)?
        .get(format!("{base}/latest"))
        .send()
        .await
        .map_err(|failed| format!("could not reach {base}: {failed}"))?;
    answer
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .and_then(tag_of)
        .ok_or_else(|| format!("{base} has no release published yet"))
}

/// A release's binary for this machine, checked against its checksum.
pub async fn download(base: &str, tag: &str) -> Result<Vec<u8>, String> {
    let name = asset(TARGET).ok_or(format!("no release is built for {TARGET}"))?;
    let http = client(true)?;
    let fetch = |url: String| {
        let http = http.clone();
        async move {
            let answer = http
                .get(&url)
                .send()
                .await
                .map_err(|failed| format!("could not reach {url}: {failed}"))?;
            if !answer.status().is_success() {
                return Err(format!("{url}: {}", answer.status()));
            }
            answer
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|failed| format!("{url}: {failed}"))
        }
    };
    let binary = fetch(format!("{base}/download/{tag}/{name}")).await?;
    let sum = fetch(format!("{base}/download/{tag}/{name}.sha256")).await?;
    if !checksum_matches(&binary, &String::from_utf8_lossy(&sum)) {
        return Err(format!(
            "{name} from {tag} is not what its checksum says; nothing was changed"
        ));
    }
    Ok(binary)
}

#[cfg(test)]
mod tests;

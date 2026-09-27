//! The live loop from this side: `plugin dev`, `logs`, `events` and `open`
//! (spec/live-plugin-development, requirements 10 to 14; W8.5, W8.6, W6.15).
//!
//! Everything goes to the dashboard on the person's terminal session, as the
//! catalogue commands do, and the dashboard relays it to the instance's
//! sidecar. Nothing here holds a credential for the cluster.
//!
//! A directory is watched by scanning it: a plugin's source is a handful of
//! files, so reading their modification times four times a second costs
//! nothing and needs nothing from any operating system's notifications.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

/// The most one change may carry, before encoding: the sidecar takes 16 MiB
/// of JSON, and base64 is four bytes for every three.
pub const MOST: u64 = 11 << 20;
/// Between looks at the directory.
pub const SCAN: Duration = Duration::from_millis(250);
/// Between asks for events.
pub const POLL: Duration = Duration::from_millis(500);

/// Why a command failed, which is also how it exits: a session that is gone
/// is said apart from everything else, so an agent knows to ask the person to
/// connect again rather than retry.
#[derive(Debug, PartialEq)]
pub enum Failed {
    Session(String),
    Refused(String),
}

impl Failed {
    pub fn code(&self) -> i32 {
        match self {
            Failed::Session(_) => 3,
            Failed::Refused(_) => 1,
        }
    }

    pub fn said(&self) -> &str {
        match self {
            Failed::Session(said) | Failed::Refused(said) => said,
        }
    }
}

impl From<String> for Failed {
    fn from(said: String) -> Failed {
        Failed::Refused(said)
    }
}

/// A connected deployment: its address and the terminal session held for it.
pub struct Deployment<'a> {
    pub address: &'a str,
    pub session: &'a str,
}

fn client() -> Result<reqwest::Client, Failed> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(crate::release::naming_this_version())
        .build()
        .map_err(|failed| Failed::Refused(failed.to_string()))
}

/// What a refusal said, and which kind it is.
pub fn refusal(address: &str, status: reqwest::StatusCode, body: &str) -> Failed {
    if let Some(refused) = crate::release::version_refused(status, body) {
        return Failed::Refused(refused);
    }
    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(String::from))
        .unwrap_or_else(|| body.trim().to_string());
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Failed::Session(format!(
            "{reason}: `meridian connect {address}` to sign in again"
        ));
    }
    Failed::Refused(format!("{status}: {reason}"))
}

impl Deployment<'_> {
    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, Failed> {
        let mut asked = client()?
            .request(method, format!("{}{path}", self.address))
            .bearer_auth(self.session);
        if let Some(body) = body {
            asked = asked
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let answer = asked.send().await.map_err(|failed| {
            Failed::Refused(format!("could not reach {}: {failed}", self.address))
        })?;
        let status = answer.status();
        let text = answer.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(refusal(self.address, status, &text));
        }
        serde_json::from_str(&text).map_err(|failed| Failed::Refused(failed.to_string()))
    }

    /// W8.5: the files that changed; the revision they are.
    pub async fn send(&self, instance: &str, change: &Change) -> Result<u64, Failed> {
        let said = self
            .call(
                reqwest::Method::PUT,
                &format!("/terminal/plugins/{instance}/dev/files"),
                Some(serde_json::json!({ "files": change.files, "deleted": change.deleted })),
            )
            .await?;
        said["revision"]
            .as_u64()
            .ok_or_else(|| Failed::Refused(format!("the change was not given a revision: {said}")))
    }

    /// W8.6: what the plugin printed after a revision, or all of it kept.
    pub async fn output(
        &self,
        instance: &str,
        since: Option<u64>,
    ) -> Result<serde_json::Value, Failed> {
        self.call(
            reqwest::Method::GET,
            &format!(
                "/terminal/plugins/{instance}/dev/output{}",
                since_query(since)
            ),
            None,
        )
        .await
    }

    /// W8.6: the runner's events and the sidecar's, after a revision.
    pub async fn events(
        &self,
        instance: &str,
        since: Option<u64>,
    ) -> Result<serde_json::Value, Failed> {
        self.call(
            reqwest::Method::GET,
            &format!(
                "/terminal/plugins/{instance}/dev/events{}",
                since_query(since)
            ),
            None,
        )
        .await
    }

    /// W6.15: a link to the plugin's host that one browser opens, once.
    pub async fn open(&self, instance: &str) -> Result<String, Failed> {
        let said = self
            .call(
                reqwest::Method::POST,
                &format!("/terminal/plugins/{instance}/open"),
                None,
            )
            .await?;
        said["url"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| Failed::Refused(format!("no link came back: {said}")))
    }

    /// W6.15: the page at a path on the plugin's host, as the person is
    /// served it.
    pub async fn page(&self, instance: &str, path: &str) -> Result<serde_json::Value, Failed> {
        self.call(
            reqwest::Method::GET,
            &format!(
                "/terminal/plugins/{instance}/page?path={}",
                query_escaped(path)
            ),
            None,
        )
        .await
    }
}

fn since_query(since: Option<u64>) -> String {
    since
        .map(|since| format!("?since={since}"))
        .unwrap_or_default()
}

/// A path as a query value: everything but what a URL leaves alone, escaped.
pub fn query_escaped(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

// ── The directory ────────────────────────────────────────────────────────

/// What is never sent: what a build leaves behind, what a checkout keeps, and
/// what the plugin's own `.dockerignore` keeps out of its image.
pub struct Ignored {
    names: Vec<String>,
    suffixes: Vec<String>,
    paths: Vec<String>,
}

const NEVER: [&str; 11] = [
    ".git",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "build",
    "dist",
    ".meridian",
    ".DS_Store",
];

impl Ignored {
    /// The fixed list, and the plugin's `.dockerignore`: a name anywhere
    /// (`__pycache__`), a path from the top (`tests/fixtures`), or a suffix
    /// (`*.egg-info`). A negation is not read, and a pattern it cannot read
    /// is sent rather than guessed at.
    pub fn from_dockerignore(dockerignore: &str) -> Ignored {
        let mut ignored = Ignored {
            names: NEVER.iter().map(|name| name.to_string()).collect(),
            suffixes: vec![".egg-info".into(), ".pyc".into()],
            paths: Vec::new(),
        };
        for line in dockerignore.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
                continue;
            }
            let line = line
                .trim_start_matches("**/")
                .trim_start_matches('/')
                .trim_end_matches('/');
            if let Some(suffix) = line.strip_prefix('*') {
                if !suffix.contains(['*', '?', '[', '/']) && !suffix.is_empty() {
                    ignored.suffixes.push(suffix.to_string());
                }
            } else if line.contains(['*', '?', '[']) {
                continue;
            } else if line.contains('/') {
                ignored.paths.push(line.to_string());
            } else {
                ignored.names.push(line.to_string());
            }
        }
        ignored
    }

    pub fn of(dir: &Path) -> Ignored {
        Ignored::from_dockerignore(
            &std::fs::read_to_string(dir.join(".dockerignore")).unwrap_or_default(),
        )
    }

    /// `path` relative to the plugin, `/` between its parts.
    pub fn holds(&self, path: &str) -> bool {
        let name = path.rsplit('/').next().unwrap_or(path);
        self.names.iter().any(|ignored| ignored == name)
            || self
                .suffixes
                .iter()
                .any(|suffix| name.ends_with(suffix.as_str()))
            || self
                .paths
                .iter()
                .any(|ignored| path == ignored || path.starts_with(&format!("{ignored}/")))
    }
}

/// Every file sent, by its path relative to the plugin, with what says it
/// changed.
pub type Snapshot = BTreeMap<String, (SystemTime, u64)>;

/// The plugin's files as they are now. A symbolic link is not followed, and
/// not sent: the live folder holds files.
pub fn scan(dir: &Path, ignored: &Ignored) -> Snapshot {
    let mut held = Snapshot::new();
    let mut waiting = vec![(dir.to_path_buf(), String::new())];
    while let Some((at, relative)) = waiting.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            if ignored.holds(&path) {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                waiting.push((entry.path(), path));
            } else if kind.is_file() {
                if let Ok(metadata) = entry.metadata() {
                    let changed = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    held.insert(path, (changed, metadata.len()));
                }
            }
        }
    }
    held
}

/// What goes to the sidecar: each file whole, base64, and each deletion.
#[derive(Debug, Default, PartialEq)]
pub struct Change {
    pub files: BTreeMap<String, String>,
    pub deleted: Vec<String>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.deleted.is_empty()
    }
}

/// What differs between two scans, read now. A file that vanished between the
/// scan and the read is sent as deleted, which is what it is by then.
pub fn change(dir: &Path, before: &Snapshot, now: &Snapshot) -> Result<Change, String> {
    let mut change = Change::default();
    let mut size = 0u64;
    for (path, held) in now {
        if before.get(path) == Some(held) {
            continue;
        }
        match std::fs::read(dir.join(path)) {
            Ok(bytes) => {
                size += bytes.len() as u64;
                change.files.insert(path.clone(), STANDARD.encode(bytes));
            }
            Err(gone) if gone.kind() == std::io::ErrorKind::NotFound => {
                change.deleted.push(path.clone());
            }
            Err(failed) => return Err(format!("{path} could not be read: {failed}")),
        }
    }
    for path in before.keys() {
        if !now.contains_key(path) {
            change.deleted.push(path.clone());
        }
    }
    if size > MOST {
        return Err(format!(
            "this change is {size} bytes, and one may be at most {MOST}: a plugin's source, \
             not its dependencies or its data. What should not be sent belongs in .dockerignore"
        ));
    }
    Ok(change)
}

// ── Events ───────────────────────────────────────────────────────────────

/// Each event once, however many times a poll returns it.
#[derive(Default)]
pub struct Seen(HashSet<String>);

impl Seen {
    /// The events in `said` not seen before, in the order they happened.
    pub fn new_in(&mut self, said: &serde_json::Value) -> Vec<serde_json::Value> {
        said["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|event| self.0.insert(event.to_string()))
            .cloned()
            .collect()
    }
}

/// An event as a person reads it: its revision first, since that is what
/// says which save it is about.
pub fn line(event: &serde_json::Value) -> String {
    let revision = event["revision"].as_u64().unwrap_or(0);
    let name = event["event"].as_str().unwrap_or("?");
    let detail = match name {
        "synced" => format!(
            " ({} sent, {} deleted)",
            event["files"].as_u64().unwrap_or(0),
            event["deleted"].as_u64().unwrap_or(0)
        ),
        "refused" => format!(": {}", event["reason"].as_str().unwrap_or_default()),
        "crashed" => {
            let exit = event["exit"]
                .as_i64()
                .map(|code| format!(", exit {code}"))
                .unwrap_or_default();
            let traceback = event["traceback"].as_str().unwrap_or_default().trim_end();
            if traceback.is_empty() {
                exit
            } else {
                format!("{exit}\n{}", indented(traceback))
            }
        }
        "exited" => ", exit 0".into(),
        _ => String::new(),
    };
    format!("r{revision} {name}{detail}")
}

fn indented(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests;

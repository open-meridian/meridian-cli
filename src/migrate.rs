//! `meridian plugin migrate`: a plugin moved to a newer release of its SDK
//! (decisions/025; sdk-contract/a-contract-change-ships-its-migration).
//!
//! Every release of an SDK that changes what a plugin calls carries a
//! migration from the release before it, and every other release one that
//! only moves the pins. The migrations are the SDK's, released inside it: for
//! the Python SDK, `meridian.migrations` in the `open-meridian` package, each
//! a record (`migration.toml`) and the rewrite code beside it. So they are in
//! every `plugin-python:<version>` image, and the newest release carries every
//! step there is.
//!
//! This finds the plugin's pins, moves them, and runs the steps between in
//! that image: made once into an image of its own with the SDK's `migrate`
//! extra (libcst, which no plugin's runtime carries), and run with no network
//! and nothing mounted. The plugin's files go in on stdin and what the steps
//! made of them comes back on stdout; only this process writes the plugin's
//! directory, and only once every step has run. Nothing needs a Python on
//! this machine, and a bind mount's stale read (Rancher Desktop gives one)
//! can never be written back over the plugin.
//!
//! Then `meridian plugin check`, and a report of what changed and what is
//! left by hand, each with its rule, file and line.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::check;
use crate::release;

#[cfg(test)]
mod tests;

/// An SDK a plugin is written on: its package, the image a plugin is built
/// on, where its releases are listed, and what runs its migrations inside
/// that image. The one there is; another SDK is another of these.
pub struct Sdk {
    pub package: &'static str,
    pub image: &'static str,
    pub releases: &'static str,
    pub runner: &'static [&'static str],
    /// What the migrations are given: files with these extensions, and these
    /// files by name, from the plugin's directory.
    pub extensions: &'static [&'static str],
    pub named: &'static [&'static str],
}

pub const PYTHON: Sdk = Sdk {
    package: "open-meridian",
    image: "ghcr.io/open-meridian/plugin-python",
    releases: "https://pypi.org/pypi/open-meridian/json",
    runner: &["python", "-m", "meridian.migrations"],
    extensions: &["py"],
    named: &["pyproject.toml"],
};

/// What the migrations run in: the SDK's image with its `migrate` extra. It
/// refuses an SDK image that carries no migrations, saying so.
fn migrations_image(image: &str) -> String {
    format!(
        "FROM {image}\n\
         RUN python -c \"import meridian.migrations\" \\\n \
         && v=\"$(python -c \"import importlib.metadata as m; print(m.version('open-meridian'))\")\" \\\n \
         && pip install --no-cache-dir \"open-meridian[migrate]==$v\"\n"
    )
}

/// The local tag the migrations' image is kept under, one per SDK image.
fn tag_for(image: &str) -> String {
    let named: String = image
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let named = named.trim_start_matches(['.', '-']);
    format!("meridian-migrate:{}", &named[..named.len().min(120)])
}

/// A program this ran, to its end.
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Ran {
    fn succeeded(&self) -> bool {
        self.code == Some(0)
    }

    /// The last lines of what it said, for a message.
    fn said(&self) -> String {
        let all = format!("{}\n{}", self.stdout, self.stderr);
        let lines: Vec<&str> = all.lines().filter(|l| !l.trim().is_empty()).collect();
        lines[lines.len().saturating_sub(8)..].join("\n")
    }
}

/// The machine, as migrate reaches it: programs (git and docker) and one GET.
#[async_trait::async_trait]
pub trait Host: Sync {
    async fn run(&self, program: &str, arguments: &[String], stdin: &[u8]) -> Result<Ran, String>;
    async fn fetch(&self, url: &str) -> Result<(u16, String), String>;
}

pub struct ThisHost;

#[async_trait::async_trait]
impl Host for ThisHost {
    async fn run(&self, program: &str, arguments: &[String], stdin: &[u8]) -> Result<Ran, String> {
        use std::process::Stdio;
        use tokio::io::AsyncWriteExt as _;

        let mut child = tokio::process::Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|failed| match failed.kind() {
                std::io::ErrorKind::NotFound => format!("{program} is not installed here"),
                _ => format!("{program}: {failed}"),
            })?;
        let mut input = child.stdin.take().ok_or("no stdin")?;
        let given = stdin.to_vec();
        // Written beside the wait, so a program that answers before it has
        // read everything cannot stall on a full pipe.
        let writing = tokio::spawn(async move {
            let _ = input.write_all(&given).await;
        });
        let output = child
            .wait_with_output()
            .await
            .map_err(|failed| format!("{program}: {failed}"))?;
        let _ = writing.await;
        Ok(Ran {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    async fn fetch(&self, url: &str) -> Result<(u16, String), String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|failed| failed.to_string())?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|failed| failed.to_string())?;
        let status = response.status().as_u16();
        Ok((status, response.text().await.unwrap_or_default()))
    }
}

pub struct Asked {
    pub dir: PathBuf,
    pub to: Option<String>,
    pub image: Option<String>,
    pub force: bool,
    pub run_tests: bool,
}

// ── The pins ─────────────────────────────────────────────────────────────

/// A pin moved: where it was, and from and to what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub file: String,
    pub line: usize,
    pub was: String,
    pub now: String,
}

/// The plugin's two pins: the SDK in pyproject.toml, pinned exactly, and the
/// base image in its Dockerfile. `meridian plugin check` holds them equal.
#[derive(Debug)]
pub struct Pins {
    pub version: String,
    pyproject: String,
    dockerfile: String,
}

fn line_at(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

fn is_version(text: &str) -> bool {
    !text.is_empty()
        && text
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// `open-meridian==X` among pyproject.toml's dependencies.
fn sdk_pin(sdk: &Sdk, pyproject: &str) -> Result<String, String> {
    let table = pyproject
        .parse::<toml::Table>()
        .map_err(|failed| format!("pyproject.toml does not read: {}", failed.message()))?;
    let dependencies = table
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
        .into_iter()
        .flatten()
        .filter_map(|d| d.as_str());
    for dependency in dependencies {
        let named = dependency
            .trim_start()
            .replace('_', "-")
            .to_ascii_lowercase();
        let Some(rest) = named.strip_prefix(sdk.package) else {
            continue;
        };
        if rest.starts_with(|c: char| c.is_alphanumeric() || c == '-') {
            continue; // another package whose name begins the same
        }
        return match dependency.split_once("==") {
            Some((_, version)) if is_version(version.trim()) => Ok(version.trim().to_string()),
            _ => Err(format!(
                "pyproject.toml's `{dependency}` does not pin {} exactly; a migration starts \
                 from one release: `{}==<version>`",
                sdk.package, sdk.package
            )),
        };
    }
    Err(format!(
        "pyproject.toml's dependencies do not name {}; this is not a plugin on it",
        sdk.package
    ))
}

/// `plugin-python:X` in the Dockerfile, every place it is said.
fn base_pins(sdk: &Sdk, dockerfile: &str) -> Vec<(usize, String)> {
    let prefix = format!("{}:", sdk.image);
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = dockerfile[from..].find(&prefix) {
        let start = from + at + prefix.len();
        let version: String = dockerfile[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            .collect();
        found.push((line_at(dockerfile, start), version));
        from = start;
    }
    found
}

pub fn pins(sdk: &Sdk, dir: &Path) -> Result<Pins, String> {
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .map_err(|failed| format!("{}: {failed}", dir.join(name).display()))
    };
    let pyproject = read("pyproject.toml")?;
    let dockerfile = read("Dockerfile")?;
    let version = sdk_pin(sdk, &pyproject)?;
    let bases = base_pins(sdk, &dockerfile);
    if bases.is_empty() {
        return Err(format!(
            "the Dockerfile is not built on {}:<version>; a plugin's base is its SDK's release",
            sdk.image
        ));
    }
    if let Some((line, other)) = bases.iter().find(|(_, base)| *base != version) {
        return Err(format!(
            "the pins disagree: pyproject.toml pins {}=={version}, and Dockerfile:{line} names \
             {}:{other}. Move them together to the release the plugin is on, then migrate",
            sdk.package, sdk.image
        ));
    }
    Ok(Pins {
        version,
        pyproject,
        dockerfile,
    })
}

/// Both pins moved to `to`, in the text, and where each was.
fn moved(sdk: &Sdk, pins: &Pins, to: &str) -> (String, String, Vec<Moved>) {
    let mut said = Vec::new();
    let mut replace = |file: &str, text: &str, old: &str, new: &str| {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        let mut offset = 0;
        while let Some(at) = rest.find(old) {
            // Only the whole pin: `==0.6.1` is not the start of `==0.6.10`.
            let after = rest[at + old.len()..].chars().next();
            let whole = !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '.');
            out.push_str(&rest[..at]);
            out.push_str(if whole { new } else { old });
            if whole {
                said.push(Moved {
                    file: file.to_string(),
                    line: line_at(text, offset + at),
                    was: old.to_string(),
                    now: new.to_string(),
                });
            }
            offset += at + old.len();
            rest = &rest[at + old.len()..];
        }
        out.push_str(rest);
        out
    };
    let from = &pins.version;
    let pyproject = replace(
        "pyproject.toml",
        &pins.pyproject,
        &format!("{}=={from}", sdk.package),
        &format!("{}=={to}", sdk.package),
    );
    let dockerfile = replace(
        "Dockerfile",
        &pins.dockerfile,
        &format!("{}:{from}", sdk.image),
        &format!("{}:{to}", sdk.image),
    );
    (pyproject, dockerfile, said)
}

// ── The plugin's files ───────────────────────────────────────────────────

/// Every regular file in the plugin, text or not, as `/`-separated paths,
/// never following a link or entering what is not the plugin.
fn walk(root: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut waiting = vec![root.to_path_buf()];
    while let Some(dir) = waiting.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if kind.is_dir() {
                if !check::NOT_THE_PLUGIN.contains(&name.as_str()) && !name.ends_with(".egg-info") {
                    waiting.push(path);
                }
            } else if kind.is_file() && kind.len() <= check::LARGEST {
                let Ok(relative) = path.strip_prefix(root) else {
                    continue;
                };
                let relative = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                found.push((relative, path));
            }
        }
    }
    found.sort();
    found
}

// ── What the migrations say ──────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Rewrote {
    pub file: String,
    pub rule: String,
    pub what: String,
    pub places: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub from: String,
    pub to: String,
    pub summary: String,
    pub breaking: bool,
    pub rewrote: Vec<Rewrote>,
}

/// A place left by hand: the migration's (from, to), or none for this
/// command's own.
#[derive(Debug, Clone, Deserialize)]
pub struct ByHand {
    pub rule: String,
    pub file: String,
    pub line: Option<usize>,
    pub found: String,
    pub instead: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Said {
    steps: Vec<Step>,
    files: std::collections::BTreeMap<String, String>,
    by_hand: Vec<ByHand>,
}

pub struct Report {
    pub dir: String,
    pub package: &'static str,
    pub from: String,
    pub to: String,
    /// What the steps ran in; none when the plugin was on its target already.
    pub image: Option<String>,
    pub pins: Vec<Moved>,
    pub steps: Vec<Step>,
    pub wrote: Vec<String>,
    pub by_hand: Vec<ByHand>,
    pub check: check::Report,
}

impl Report {
    /// Nothing left: no place by hand, and every rule holds.
    pub fn done(&self) -> bool {
        self.by_hand.is_empty() && self.check.passed()
    }
}

pub enum Migrated {
    /// Refused before anything changed: asked wrongly, or it would not be
    /// safe. Exit 2.
    Refused(String),
    /// Could not run, and nothing changed. Exit 1.
    Failed(String),
    Done(Box<Report>),
}

/// The releases the index lists, and the latest.
fn releases(said: &str) -> Result<(String, Vec<String>), String> {
    let value: serde_json::Value =
        serde_json::from_str(said).map_err(|failed| format!("not JSON: {failed}"))?;
    let latest = value
        .pointer("/info/version")
        .and_then(|v| v.as_str())
        .ok_or("names no latest version")?
        .to_string();
    let listed = value
        .get("releases")
        .and_then(|r| r.as_object())
        .map(|r| r.keys().cloned().collect())
        .unwrap_or_default();
    Ok((latest, listed))
}

pub async fn migrate(host: &dyn Host, sdk: &Sdk, asked: &Asked) -> Migrated {
    use Migrated::{Failed, Refused};
    let dir = &asked.dir;
    if !dir.is_dir() {
        return Refused(format!("{} is not a directory", dir.display()));
    }
    let pins = match pins(sdk, dir) {
        Ok(pins) => pins,
        Err(refusal) => return Refused(refusal),
    };
    let from = pins.version.clone();

    // A migration rewrites files, and git is how a person sees what it did
    // and takes it back.
    if !asked.force {
        let status = [
            "-C",
            &dir.display().to_string(),
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            ".",
        ]
        .map(String::from);
        match host.run("git", &status, b"").await {
            Ok(ran) if ran.succeeded() && ran.stdout.trim().is_empty() => {}
            Ok(ran) if ran.succeeded() => {
                let changed: Vec<&str> = ran.stdout.lines().take(5).collect();
                return Refused(format!(
                    "{} has changes git does not hold yet:\n  {}\nA migration rewrites files: \
                     commit or stash these first, so what it changes is all that git shows, or \
                     --force",
                    dir.display(),
                    changed.join("\n  ")
                ));
            }
            _ => {
                return Refused(format!(
                    "{} is not in a git repository, or git is not installed: a migration rewrites \
                     files, and git is how you see and undo what it did. --force to migrate \
                     anyway",
                    dir.display()
                ))
            }
        }
    }

    if let Some(to) = asked.to.as_deref().filter(|to| !is_version(to)) {
        return Refused(format!("--to {to} is not a release: numbers, dot by dot"));
    }
    // The index is asked only for what was not given: the latest, for the
    // target or the image the steps run in, and whether --to is released.
    let listed = if asked.to.is_none() || asked.image.is_none() {
        let answered = match host.fetch(sdk.releases).await {
            Ok((200, body)) => releases(&body),
            Ok((status, _)) => Err(format!("answered {status}")),
            Err(failed) => Err(failed),
        };
        match answered {
            Ok(listed) => Some(listed),
            Err(failed) => {
                return Failed(format!(
                    "could not ask {} which releases of {} there are: {failed}. Name them: --to \
                     <version> --image <an SDK image carrying the migrations>",
                    sdk.releases, sdk.package
                ))
            }
        }
    } else {
        None
    };
    let to = asked
        .to
        .clone()
        .or_else(|| listed.as_ref().map(|(latest, _)| latest.clone()))
        .unwrap_or_default();

    match release::compare(&to, &from) {
        std::cmp::Ordering::Less => {
            return Refused(format!(
                "the plugin is on {} {from}, after {to}: a migration never goes backwards",
                sdk.package
            ))
        }
        std::cmp::Ordering::Equal => return finish(sdk, asked, from, to, Did::default()),
        std::cmp::Ordering::Greater => {}
    }
    if let (Some((_, releases)), None) = (&listed, &asked.image) {
        if !releases.contains(&to) {
            return Refused(format!(
                "{} {to} is not released. To migrate to one built here, name its image: --image",
                sdk.package
            ));
        }
    }
    let image = asked.image.clone().unwrap_or_else(|| {
        let latest = listed
            .as_ref()
            .map(|(latest, _)| latest.as_str())
            .unwrap_or(&to);
        format!("{}:{latest}", sdk.image)
    });

    // What the steps are given: the plugin's code, and pyproject.toml with
    // its pin moved, as it will be written.
    let (pyproject, dockerfile, pins_moved) = moved(sdk, &pins, &to);
    let mut sent = std::collections::BTreeMap::new();
    let mut by_hand = Vec::new();
    for (relative, path) in walk(dir) {
        let extension = relative
            .rsplit_once('.')
            .map(|(_, e)| e)
            .unwrap_or_default();
        if !sdk.named.contains(&relative.as_str()) && !sdk.extensions.contains(&extension) {
            continue;
        }
        if relative == "pyproject.toml" {
            sent.insert(relative, pyproject.clone());
            continue;
        }
        match std::fs::read(&path).map(String::from_utf8) {
            Ok(Ok(text)) => {
                sent.insert(relative, text);
            }
            _ => by_hand.push(ByHand {
                rule: "unreadable".into(),
                file: relative,
                line: None,
                found: "not UTF-8 text, so no migration read it".into(),
                instead: "apply each step's rewrites to it by hand".into(),
                from: None,
                to: None,
            }),
        }
    }

    // The image the steps run in, made once and kept.
    let tag = tag_for(&image);
    // Plain, so a step that fails says why in what comes back.
    let build = ["build", "--progress=plain", "-t", &tag, "-"].map(String::from);
    match host
        .run("docker", &build, migrations_image(&image).as_bytes())
        .await
    {
        Ok(ran) if ran.succeeded() => {}
        Ok(ran) if ran.stderr.contains("No module named 'meridian.migrations'") => {
            return Failed(format!(
                "{image} carries no migrations: its release of {} came before them. Name an SDK \
                 image that does, with --image",
                sdk.package
            ))
        }
        Ok(ran) => {
            return Failed(format!(
                "could not make the image the migrations run in, from {image}:\n{}",
                ran.said()
            ))
        }
        Err(failed) => return Failed(format!("could not run docker: {failed}")),
    }

    let mut run = [
        "run",
        "--rm",
        "-i",
        "--network",
        "none",
        "--read-only",
        &tag,
    ]
    .map(String::from)
    .to_vec();
    run.extend(sdk.runner.iter().map(|part| part.to_string()));
    run.extend(["--from", &from, "--to", &to].map(String::from));
    let given = serde_json::json!({ "files": sent }).to_string();
    let said: Said = match host.run("docker", &run, given.as_bytes()).await {
        Ok(ran) if ran.succeeded() => match serde_json::from_str(&ran.stdout) {
            Ok(said) => said,
            Err(failed) => {
                return Failed(format!(
                    "the migrations answered what does not read: {failed}"
                ))
            }
        },
        // Their refusal: no recorded steps between the two.
        Ok(ran) if ran.code == Some(2) => {
            let reason = ran
                .stderr
                .trim()
                .trim_start_matches("python -m meridian.migrations: ");
            return Refused(format!(
                "{image} cannot migrate from {from} to {to}: {reason}"
            ));
        }
        Ok(ran) => return Failed(format!("the migrations failed:\n{}", ran.said())),
        Err(failed) => return Failed(format!("could not run docker: {failed}")),
    };
    // Only what was given comes back.
    if let Some(stranger) = said.files.keys().find(|path| !sent.contains_key(*path)) {
        return Failed(format!(
            "the migrations answered with {stranger}, which they were not given; nothing is \
             written"
        ));
    }

    // Written together, once every step has run.
    let mut writes: Vec<(String, String)> = said.files.into_iter().collect();
    if !writes.iter().any(|(path, _)| path == "pyproject.toml") {
        writes.push(("pyproject.toml".into(), pyproject));
    }
    writes.push(("Dockerfile".into(), dockerfile));
    let mut wrote = Vec::new();
    for (path, text) in writes {
        let target = dir.join(&path);
        if std::fs::read_to_string(&target).ok().as_deref() == Some(text.as_str()) {
            continue;
        }
        if let Err(failed) = std::fs::write(&target, &text) {
            return Failed(format!(
                "{}: {failed}. Written before it: {}; `git checkout -- .` puts them back",
                target.display(),
                if wrote.is_empty() {
                    "nothing".to_string()
                } else {
                    wrote.join(", ")
                }
            ));
        }
        wrote.push(path);
    }
    wrote.sort();

    by_hand.extend(said.by_hand);
    by_hand.extend(pins_elsewhere(sdk, dir, &from, &to));
    let did = Did {
        image: Some(image),
        pins: pins_moved,
        steps: said.steps,
        wrote,
        by_hand,
    };
    finish(sdk, asked, from, to, did)
}

/// The old release named anywhere else: a Makefile's base, a workflow, a
/// document. Said, not moved, since a changelog naming it is history.
fn pins_elsewhere(sdk: &Sdk, dir: &Path, from: &str, to: &str) -> Vec<ByHand> {
    let pins = [
        format!("{}=={from}", sdk.package),
        format!("{}:{from}", sdk.image),
    ];
    let mut found = Vec::new();
    for (relative, path) in walk(dir) {
        if relative == "pyproject.toml" || relative == "Dockerfile" {
            continue;
        }
        let Ok(Ok(text)) = std::fs::read(&path).map(String::from_utf8) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            let named = pins.iter().any(|pin| {
                line.match_indices(pin.as_str()).any(|(at, pin)| {
                    !line[at + pin.len()..]
                        .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '.')
                })
            });
            if named {
                found.push(ByHand {
                    rule: "pin-elsewhere".into(),
                    file: relative.clone(),
                    line: Some(index + 1),
                    found: line.trim().to_string(),
                    instead: format!(
                        "it names {} {from} still: where that is a pin, move it to {to} with \
                         pyproject.toml's and the Dockerfile's; where it is history, leave it",
                        sdk.package
                    ),
                    from: None,
                    to: None,
                });
            }
        }
    }
    found
}

/// What a migration did, before the check.
#[derive(Default)]
struct Did {
    image: Option<String>,
    pins: Vec<Moved>,
    steps: Vec<Step>,
    wrote: Vec<String>,
    by_hand: Vec<ByHand>,
}

/// The check, over what the migration left, and the report of both.
fn finish(sdk: &Sdk, asked: &Asked, from: String, to: String, did: Did) -> Migrated {
    match check::check(&asked.dir, asked.run_tests) {
        Ok(checked) => Migrated::Done(Box::new(Report {
            dir: asked.dir.display().to_string(),
            package: sdk.package,
            from,
            to,
            image: did.image,
            pins: did.pins,
            steps: did.steps,
            wrote: did.wrote,
            by_hand: did.by_hand,
            check: checked,
        })),
        Err(failed) => Migrated::Failed(failed),
    }
}

// ── Saying it ────────────────────────────────────────────────────────────

/// A summary's first sentence, on one line.
fn first_sentence(summary: &str) -> String {
    let joined = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    match joined.find(". ") {
        Some(end) => joined[..=end].to_string(),
        None => joined,
    }
}

fn located(file: &str, line: Option<usize>) -> String {
    match line {
        Some(line) => format!("{file}:{line}"),
        None => file.to_string(),
    }
}

pub fn text(report: &Report) -> String {
    let mut out = format!(
        "meridian plugin migrate: {}, {} {} to {}",
        report.dir, report.package, report.from, report.to
    );
    match &report.image {
        None => out.push_str(": already there, so nothing moved\n"),
        Some(image) => out.push_str(&format!(", the steps run in {image}\n")),
    }
    if !report.pins.is_empty() {
        out.push('\n');
        for (index, pin) in report.pins.iter().enumerate() {
            let label = if index == 0 { "pins" } else { "" };
            out.push_str(&format!(
                "  {label:6}{:22}{} -> {}\n",
                located(&pin.file, Some(pin.line)),
                pin.was,
                pin.now
            ));
        }
    }
    for step in &report.steps {
        out.push_str(&format!(
            "\n  {} to {}  {}\n",
            step.from,
            step.to,
            first_sentence(&step.summary)
        ));
        if step.rewrote.is_empty() {
            out.push_str("        nothing to rewrite: only the pins move\n");
        }
        let mut files: Vec<&str> = step.rewrote.iter().map(|r| r.file.as_str()).collect();
        files.dedup();
        let width = files.iter().map(|f| f.len()).max().unwrap_or(0);
        for file in files {
            let rules: Vec<String> = step
                .rewrote
                .iter()
                .filter(|r| r.file == file)
                .map(|r| {
                    if r.places == 1 {
                        r.rule.clone()
                    } else {
                        format!("{} ({})", r.rule, r.places)
                    }
                })
                .collect();
            out.push_str(&format!("        {file:width$}  {}\n", rules.join(", ")));
        }
    }
    if !report.wrote.is_empty() {
        out.push_str(&format!("\nWrote {}.\n", report.wrote.join(", ")));
    }
    if report.by_hand.is_empty() {
        out.push_str("\nNothing is left by hand.\n");
    } else {
        out.push_str(&format!(
            "\nLeft by hand, {} place(s): do each as it says.\n",
            report.by_hand.len()
        ));
        let width = report
            .by_hand
            .iter()
            .map(|b| b.rule.len())
            .max()
            .unwrap_or(0);
        for left in &report.by_hand {
            out.push_str(&format!(
                "  {:width$}  {}: {}\n        instead: {}\n",
                left.rule,
                located(&left.file, left.line),
                left.found,
                left.instead
            ));
        }
    }
    out.push('\n');
    out.push_str(&check::text(&report.check));
    out
}

pub fn json(report: &Report) -> serde_json::Value {
    serde_json::json!({
        "dir": report.dir,
        "meridian": release::VERSION,
        "sdk": report.package,
        "from": report.from,
        "to": report.to,
        "image": report.image,
        "done": report.done(),
        "pins": report.pins.iter().map(|pin| serde_json::json!({
            "file": pin.file, "line": pin.line, "from": pin.was, "to": pin.now,
        })).collect::<Vec<_>>(),
        "steps": report.steps.iter().map(|step| serde_json::json!({
            "from": step.from,
            "to": step.to,
            "summary": step.summary,
            "breaking": step.breaking,
            "rewrote": step.rewrote.iter().map(|r| serde_json::json!({
                "file": r.file, "rule": r.rule, "what": r.what, "places": r.places,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "wrote": report.wrote,
        "by_hand": report.by_hand.iter().map(|left| serde_json::json!({
            "rule": left.rule,
            "file": left.file,
            "line": left.line,
            "found": left.found,
            "instead": left.instead,
            "from": left.from,
            "to": left.to,
        })).collect::<Vec<_>>(),
        "check": check::json(&report.check),
    })
}

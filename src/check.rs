//! `meridian plugin check`: a plugin, in its directory, held to the
//! framework's rules (decisions/025; sdk-contract/a-plugin-is-checked-
//! against-the-framework).
//!
//! Agents write most plugins, and an agent keeps the rules it is checked
//! against more reliably than the rules it is told. So each failure names the
//! rule, the file and the line, and what to write instead, and an agent fixes
//! it without asking anybody.
//!
//! The rules are all in `rules.rs`, and released with this binary: the check
//! a plugin meets is the one of the `meridian` that runs it. They read the
//! plugin's files as text, so the page rules hold for a plugin in any
//! language; the rules about its code read Python, the one SDK there is. What
//! the check cannot decide -- layout, naming, which component fits -- stays
//! advice in AGENTS.md, and the CLI's documentation lists it.
//!
//! It changes nothing, reaches nothing but the directory, and with
//! `--run-tests` runs the plugin's own tests there.

use std::path::{Path, PathBuf};

mod rules;
#[cfg(test)]
mod tests;

pub use rules::{EDGE_ROLES, RULES};

/// One of the plugin's files, as text. `path` is relative to the plugin's
/// directory, with `/` between its parts on every system.
pub struct Source {
    pub path: String,
    pub text: String,
}

/// The plugin as the rules read it.
pub struct Plugin {
    pub root: PathBuf,
    pub files: Vec<Source>,
}

impl Plugin {
    pub fn file(&self, path: &str) -> Option<&Source> {
        self.files.iter().find(|source| source.path == path)
    }

    pub fn has(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }
}

/// A rule the plugin does not keep, where, and what to write instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub rule: &'static str,
    pub file: String,
    /// From 1; 0 when the failure is the file as a whole, or its absence.
    pub line: usize,
    pub found: String,
    pub instead: String,
}

/// One rule: what it holds, and how it is checked.
pub struct Rule {
    pub id: &'static str,
    pub holds: &'static str,
    pub check: fn(&Plugin) -> Vec<Failure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    /// Not run: the tests, without `--run-tests`.
    Skipped,
}

impl Outcome {
    fn word(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Skipped => "skipped",
        }
    }
}

pub struct Report {
    pub dir: String,
    pub outcomes: Vec<(&'static str, &'static str, Outcome)>,
    pub failures: Vec<Failure>,
}

impl Report {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Never read: what a checkout, a build, a virtual environment or an agent
/// leaves beside the plugin, none of which is the plugin. `.e2e` is where the
/// plugin's e2e copies the runtime's plugin harness: core's code, not the
/// plugin's, and it reads its own environment.
pub(crate) const NOT_THE_PLUGIN: [&str; 19] = [
    ".git",
    ".venv",
    "venv",
    "node_modules",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
    ".nox",
    "build",
    "dist",
    "target",
    ".meridian",
    ".claude",
    ".idea",
    ".vscode",
    ".sdk-scratch",
    ".e2e",
];

/// Read as text: what a page, its code or its declarations are written in.
const TEXT: [&str; 18] = [
    "py", "pyi", "html", "htm", "css", "svg", "js", "mjs", "cjs", "jsx", "ts", "tsx", "vue",
    "svelte", "jinja", "jinja2", "j2", "toml",
];

/// Anything larger is generated, not written.
pub(crate) const LARGEST: u64 = 1 << 20;

/// The plugin's files, read once for every rule.
pub fn read(root: &Path) -> Result<Plugin, String> {
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let mut files = Vec::new();
    let mut waiting = vec![root.to_path_buf()];
    while let Some(dir) = waiting.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|failed| format!("{}: {failed}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            // Never followed: a link out of the plugin is not the plugin.
            let Ok(kind) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if kind.is_dir() {
                if !NOT_THE_PLUGIN.contains(&name.as_str()) && !name.ends_with(".egg-info") {
                    waiting.push(path);
                }
                continue;
            }
            let extension = name.rsplit_once('.').map(|(_, e)| e).unwrap_or_default();
            let named = matches!(
                name.as_str(),
                "Dockerfile" | ".dockerignore" | "AGENTS.md" | "CLAUDE.md"
            );
            if !kind.is_file() || kind.len() > LARGEST || !(named || TEXT.contains(&extension)) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let relative = path
                .strip_prefix(root)
                .map_err(|failed| failed.to_string())?
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            files.push(Source {
                path: relative,
                text: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Plugin {
        root: root.to_path_buf(),
        files,
    })
}

/// Every rule over the plugin in `dir`; its tests run only when asked.
pub fn check(dir: &Path, run_tests: bool) -> Result<Report, String> {
    let plugin = read(dir)?;
    let mut report = Report {
        dir: dir.display().to_string(),
        outcomes: Vec::new(),
        failures: Vec::new(),
    };
    for rule in RULES.iter() {
        let failures = (rule.check)(&plugin);
        let outcome = if failures.is_empty() {
            Outcome::Passed
        } else {
            Outcome::Failed
        };
        report.outcomes.push((rule.id, rule.holds, outcome));
        report.failures.extend(failures);
    }
    let pass = &rules::TESTS_PASS;
    if run_tests {
        let failures = (pass.check)(&plugin);
        let outcome = if failures.is_empty() {
            Outcome::Passed
        } else {
            Outcome::Failed
        };
        report.outcomes.push((pass.id, pass.holds, outcome));
        report.failures.extend(failures);
    } else {
        report
            .outcomes
            .push((pass.id, pass.holds, Outcome::Skipped));
    }
    Ok(report)
}

fn located(failure: &Failure) -> String {
    if failure.line == 0 {
        failure.file.clone()
    } else {
        format!("{}:{}", failure.file, failure.line)
    }
}

/// For a person, or an agent reading a terminal.
pub fn text(report: &Report) -> String {
    let mut out = format!(
        "meridian plugin check: {}, by the rules of meridian {}\n\n",
        report.dir,
        crate::release::VERSION
    );
    let width = report
        .outcomes
        .iter()
        .map(|(id, _, _)| id.len())
        .max()
        .unwrap_or(0);
    for (id, holds, outcome) in &report.outcomes {
        let mark = match outcome {
            Outcome::Passed => "ok  ",
            Outcome::Failed => "FAIL",
            Outcome::Skipped => "--  ",
        };
        let holds = match outcome {
            Outcome::Skipped => format!("{holds}: --run-tests runs them"),
            _ => holds.to_string(),
        };
        out.push_str(&format!("  {mark}  {id:width$}  {holds}\n"));
        let mut said_instead = "";
        for failure in report.failures.iter().filter(|failure| failure.rule == *id) {
            out.push_str(&format!(
                "        {}: {}\n",
                located(failure),
                failure.found.replace('\n', "\n          ")
            ));
            // Said once for a run of failures with the same fix.
            if failure.instead != said_instead {
                out.push_str(&format!("          instead: {}\n", failure.instead));
                said_instead = &failure.instead;
            }
        }
    }
    let failed = report
        .outcomes
        .iter()
        .filter(|(_, _, outcome)| *outcome == Outcome::Failed)
        .count();
    if failed == 0 {
        out.push_str("\nEvery rule it was checked against holds.\n");
    } else {
        out.push_str(&format!(
            "\n{failed} of {} rules failed, {} place(s) in all. Fix each as it says, then check again.\n",
            report.outcomes.len(),
            report.failures.len()
        ));
    }
    out
}

/// For a script or an agent: one object.
pub fn json(report: &Report) -> serde_json::Value {
    serde_json::json!({
        "dir": report.dir,
        "meridian": crate::release::VERSION,
        "passed": report.passed(),
        "rules": report.outcomes.iter().map(|(id, holds, outcome)| serde_json::json!({
            "rule": id, "holds": holds, "outcome": outcome.word(),
        })).collect::<Vec<_>>(),
        "failures": report.failures.iter().map(|failure| serde_json::json!({
            "rule": failure.rule,
            "file": failure.file,
            "line": if failure.line == 0 { serde_json::Value::Null } else { failure.line.into() },
            "found": failure.found,
            "instead": failure.instead,
        })).collect::<Vec<_>>(),
    })
}

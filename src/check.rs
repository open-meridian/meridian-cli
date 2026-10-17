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

pub use rules::{BUNDLE_BYTES, EDGE_ROLES, ROLES, RULES, SECTIONS, SECTION_BYTES};

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
    /// Checked as a verified plugin is (`--verified`): a changing route may
    /// not be kept from agents (spec/a-deployment-serves-its-mcp,
    /// requirement 23), and a role held with a suite runs it in its tests
    /// (spec/vendor-differences-have-a-place-in-the-contract, requirement 18).
    pub verified: bool,
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
    /// Not run: the tests, without `--run-tests`; a rule only `--verified`
    /// holds, without it.
    Skipped,
    /// Found what a warning rule warns of (`rules::WARNED`): said, and
    /// failing nothing, whether or not the plugin is checked as verified.
    Warned,
}

impl Outcome {
    fn word(self) -> &'static str {
        match self {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
            Outcome::Skipped => "skipped",
            Outcome::Warned => "warned",
        }
    }
}

pub struct Report {
    pub dir: String,
    pub outcomes: Vec<(&'static str, &'static str, Outcome)>,
    pub failures: Vec<Failure>,
    /// What a rule only `--verified` holds found on a plugin not checked as
    /// verified (`rules::ADVISED`), and what a rule advises beside what it
    /// fails (`rules::ADVISING`): said as advice, failing nothing.
    pub advice: Vec<Failure>,
    /// What a warning rule found (`rules::WARNED`): failing nothing, in
    /// every mode, since what it warns of the deployment accepts.
    pub warnings: Vec<Failure>,
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
        verified: false,
    })
}

/// Every rule over the plugin in `dir`; its tests run only when asked.
pub fn check(dir: &Path, run_tests: bool) -> Result<Report, String> {
    check_as(dir, run_tests, false)
}

/// The same, held as a verified plugin is when `verified`: no changing
/// route kept from agents, each role held with a suite run in its tests, and
/// each role's common part kept (`rules::ONLY_VERIFIED`, skipped otherwise,
/// the common part's findings then said as advice).
pub fn check_as(dir: &Path, run_tests: bool, verified: bool) -> Result<Report, String> {
    let mut plugin = read(dir)?;
    plugin.verified = verified;
    let mut report = Report {
        dir: dir.display().to_string(),
        outcomes: Vec::new(),
        failures: Vec::new(),
        advice: Vec::new(),
        warnings: Vec::new(),
    };
    for rule in RULES.iter() {
        if !verified && rules::ONLY_VERIFIED.contains(&rule.id) {
            if rules::ADVISED.contains(&rule.id) {
                report.advice.extend((rule.check)(&plugin));
            }
            report
                .outcomes
                .push((rule.id, rule.holds, Outcome::Skipped));
            continue;
        }
        let failures = (rule.check)(&plugin);
        if let Some((_, advising)) = rules::ADVISING.iter().find(|(id, _)| *id == rule.id) {
            report.advice.extend(advising(&plugin));
        }
        let outcome = match (failures.is_empty(), rules::WARNED.contains(&rule.id)) {
            (true, _) => Outcome::Passed,
            (false, true) => Outcome::Warned,
            (false, false) => Outcome::Failed,
        };
        report.outcomes.push((rule.id, rule.holds, outcome));
        if outcome == Outcome::Warned {
            report.warnings.extend(failures);
        } else {
            report.failures.extend(failures);
        }
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

/// What a person adds to be checked against a rule this run skipped.
fn skipped_unless(id: &str) -> &'static str {
    if rules::ONLY_VERIFIED.contains(&id) {
        "--verified holds it"
    } else {
        "--run-tests runs them"
    }
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
            Outcome::Warned => "warn",
        };
        let holds = match outcome {
            Outcome::Skipped => format!("{holds}: {}", skipped_unless(id)),
            _ => holds.to_string(),
        };
        out.push_str(&format!("  {mark}  {id:width$}  {holds}\n"));
        let mut said_instead = "";
        // A skipped rule's advice, and a warning rule's warnings, are said
        // where its failures would be.
        let found = report
            .failures
            .iter()
            .chain(&report.advice)
            .chain(&report.warnings);
        for failure in found.filter(|failure| failure.rule == *id) {
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
    if !report.advice.is_empty() {
        let held_verified = report
            .advice
            .iter()
            .any(|advice| rules::ADVISED.contains(&advice.rule));
        out.push_str(&format!(
            "\nAdvice, {} place(s), failing nothing{}.\n",
            report.advice.len(),
            if held_verified {
                ": a verified plugin is held to some of it (--verified)"
            } else {
                ""
            }
        ));
    }
    if !report.warnings.is_empty() {
        out.push_str(&format!(
            "\nWarnings, {} place(s), failing nothing: what the deployment accepts, and the \
             plugin should still change.\n",
            report.warnings.len()
        ));
    }
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
        "advice": report.advice.iter().map(|advice| serde_json::json!({
            "rule": advice.rule,
            "file": advice.file,
            "line": if advice.line == 0 { serde_json::Value::Null } else { advice.line.into() },
            "found": advice.found,
            "instead": advice.instead,
        })).collect::<Vec<_>>(),
        "warnings": report.warnings.iter().map(|warning| serde_json::json!({
            "rule": warning.rule,
            "file": warning.file,
            "line": if warning.line == 0 { serde_json::Value::Null } else { warning.line.into() },
            "found": warning.found,
            "instead": warning.instead,
        })).collect::<Vec<_>>(),
    })
}

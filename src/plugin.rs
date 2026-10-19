//! `meridian plugin new`: a plugin to start from.
//!
//! What it writes is meridian-python's template -- the reference plugin, kept
//! beside the SDK it is written against and tested there against a real
//! sidecar -- copied into this repository at one SDK revision (`SDK_REV` in the
//! Makefile) and compiled into the binary. So scaffolding needs no network and
//! gives the same plugin every time, and `check-vendored-template` fails when
//! this copy stops being the SDK's. With `--role` and `connectivity`,
//! `custody`, `data`, `operations`, `pricing` or `reporting` it writes that
//! role's template instead, from the
//! SDK's `templates/<role>/`, vendored the same way: a whole plugin holding the
//! role, on the role's common part (spec/plugin-pages-follow-their-role), its
//! tests running the role's suite where the template has one.
//!
//! The only thing changed on the way out is the name: the templates call
//! themselves `reference-plugin`, importable as `reference_plugin`, and both
//! become whatever the plugin is called.

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// The template, file by file, as it sits in `plugin-template/`. A test fails
/// if a file there is missing from this list, so a new one cannot be dropped.
const TEMPLATE: [(&str, &str); 22] = [
    ("AGENTS.md", include_str!("../plugin-template/AGENTS.md")),
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-template/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-template/.dockerignore"),
    ),
    ("CLAUDE.md", include_str!("../plugin-template/CLAUDE.md")),
    (".gitignore", include_str!("../plugin-template/.gitignore")),
    ("Dockerfile", include_str!("../plugin-template/Dockerfile")),
    ("README.md", include_str!("../plugin-template/README.md")),
    (
        "pyproject.toml",
        include_str!("../plugin-template/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-template/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-template/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-template/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/templates/accounts.html",
        include_str!("../plugin-template/src/reference_plugin/templates/accounts.html"),
    ),
    (
        "src/reference_plugin/templates/setup.html",
        include_str!("../plugin-template/src/reference_plugin/templates/setup.html"),
    ),
    (
        "tests/test_page.py",
        include_str!("../plugin-template/tests/test_page.py"),
    ),
    (
        "docs/overview.md",
        include_str!("../plugin-template/docs/overview.md"),
    ),
    (
        "docs/setup.md",
        include_str!("../plugin-template/docs/setup.md"),
    ),
    (
        "docs/troubleshooting.md",
        include_str!("../plugin-template/docs/troubleshooting.md"),
    ),
    (
        "docs/terms.md",
        include_str!("../plugin-template/docs/terms.md"),
    ),
    (
        "docs/pages.md",
        include_str!("../plugin-template/docs/pages.md"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-template/src/reference_plugin/declaration.py"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-template/.github/workflows/check.yaml"),
    ),
    (
        "hooks/pre-push",
        include_str!("../plugin-template/hooks/pre-push"),
    ),
];

/// The `data` template, as it sits in `plugin-templates/data/`: a vendor's
/// prices put into the lake, its catalogue declared, the data role's common
/// pages, Connection and Datasets, with their read tools, and its tests
/// running the suite.
const DATA: [(&str, &str); 25] = [
    (
        "AGENTS.md",
        include_str!("../plugin-templates/data/AGENTS.md"),
    ),
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-templates/data/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-templates/data/.dockerignore"),
    ),
    (
        "CLAUDE.md",
        include_str!("../plugin-templates/data/CLAUDE.md"),
    ),
    (
        ".gitignore",
        include_str!("../plugin-templates/data/.gitignore"),
    ),
    (
        "Dockerfile",
        include_str!("../plugin-templates/data/Dockerfile"),
    ),
    (
        "README.md",
        include_str!("../plugin-templates/data/README.md"),
    ),
    (
        "pyproject.toml",
        include_str!("../plugin-templates/data/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-templates/data/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-templates/data/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/convert.py",
        include_str!("../plugin-templates/data/src/reference_plugin/convert.py"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-templates/data/src/reference_plugin/declaration.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-templates/data/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/templates/connection.html",
        include_str!("../plugin-templates/data/src/reference_plugin/templates/connection.html"),
    ),
    (
        "src/reference_plugin/templates/datasets.html",
        include_str!("../plugin-templates/data/src/reference_plugin/templates/datasets.html"),
    ),
    (
        "src/reference_plugin/vendor.py",
        include_str!("../plugin-templates/data/src/reference_plugin/vendor.py"),
    ),
    (
        "tests/test_page.py",
        include_str!("../plugin-templates/data/tests/test_page.py"),
    ),
    (
        "tests/test_suite.py",
        include_str!("../plugin-templates/data/tests/test_suite.py"),
    ),
    (
        "docs/overview.md",
        include_str!("../plugin-templates/data/docs/overview.md"),
    ),
    (
        "docs/setup.md",
        include_str!("../plugin-templates/data/docs/setup.md"),
    ),
    (
        "docs/troubleshooting.md",
        include_str!("../plugin-templates/data/docs/troubleshooting.md"),
    ),
    (
        "docs/terms.md",
        include_str!("../plugin-templates/data/docs/terms.md"),
    ),
    (
        "docs/pages.md",
        include_str!("../plugin-templates/data/docs/pages.md"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/data/.github/workflows/check.yaml"),
    ),
    (
        "hooks/pre-push",
        include_str!("../plugin-templates/data/hooks/pre-push"),
    ),
];

/// The `custody` template, as it sits in `plugin-templates/custody/`: a
/// custodian's accounts and statements brought in, on the custody role's
/// common pages with their tools, and its tests running the suite.
const CUSTODY: [(&str, &str); 27] = [
    (
        "AGENTS.md",
        include_str!("../plugin-templates/custody/AGENTS.md"),
    ),
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-templates/custody/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-templates/custody/.dockerignore"),
    ),
    (
        "CLAUDE.md",
        include_str!("../plugin-templates/custody/CLAUDE.md"),
    ),
    (
        ".gitignore",
        include_str!("../plugin-templates/custody/.gitignore"),
    ),
    (
        "Dockerfile",
        include_str!("../plugin-templates/custody/Dockerfile"),
    ),
    (
        "README.md",
        include_str!("../plugin-templates/custody/README.md"),
    ),
    (
        "pyproject.toml",
        include_str!("../plugin-templates/custody/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-templates/custody/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-templates/custody/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/convert.py",
        include_str!("../plugin-templates/custody/src/reference_plugin/convert.py"),
    ),
    (
        "src/reference_plugin/custodian.py",
        include_str!("../plugin-templates/custody/src/reference_plugin/custodian.py"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-templates/custody/src/reference_plugin/declaration.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-templates/custody/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/templates/account_links.html",
        include_str!(
            "../plugin-templates/custody/src/reference_plugin/templates/account_links.html"
        ),
    ),
    (
        "src/reference_plugin/templates/connections.html",
        include_str!("../plugin-templates/custody/src/reference_plugin/templates/connections.html"),
    ),
    (
        "src/reference_plugin/templates/history.html",
        include_str!("../plugin-templates/custody/src/reference_plugin/templates/history.html"),
    ),
    (
        "src/reference_plugin/templates/statements.html",
        include_str!("../plugin-templates/custody/src/reference_plugin/templates/statements.html"),
    ),
    (
        "tests/test_page.py",
        include_str!("../plugin-templates/custody/tests/test_page.py"),
    ),
    (
        "tests/test_suite.py",
        include_str!("../plugin-templates/custody/tests/test_suite.py"),
    ),
    (
        "docs/overview.md",
        include_str!("../plugin-templates/custody/docs/overview.md"),
    ),
    (
        "docs/setup.md",
        include_str!("../plugin-templates/custody/docs/setup.md"),
    ),
    (
        "docs/troubleshooting.md",
        include_str!("../plugin-templates/custody/docs/troubleshooting.md"),
    ),
    (
        "docs/terms.md",
        include_str!("../plugin-templates/custody/docs/terms.md"),
    ),
    (
        "docs/pages.md",
        include_str!("../plugin-templates/custody/docs/pages.md"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/custody/.github/workflows/check.yaml"),
    ),
    (
        "hooks/pre-push",
        include_str!("../plugin-templates/custody/hooks/pre-push"),
    ),
];

/// The `operations` template, as it sits in `plugin-templates/operations/`:
/// the operations role's common pages, Breaks, Reconciliations and Opening
/// balances, with their tools, an opening balance's confirmation checked so
/// an agent may prepare it.
const OPERATIONS: [(&str, &str); 23] = [
    (
        "AGENTS.md",
        include_str!("../plugin-templates/operations/AGENTS.md"),
    ),
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-templates/operations/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-templates/operations/.dockerignore"),
    ),
    (
        "CLAUDE.md",
        include_str!("../plugin-templates/operations/CLAUDE.md"),
    ),
    (
        ".gitignore",
        include_str!("../plugin-templates/operations/.gitignore"),
    ),
    (
        "Dockerfile",
        include_str!("../plugin-templates/operations/Dockerfile"),
    ),
    (
        "README.md",
        include_str!("../plugin-templates/operations/README.md"),
    ),
    (
        "pyproject.toml",
        include_str!("../plugin-templates/operations/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-templates/operations/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-templates/operations/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-templates/operations/src/reference_plugin/declaration.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-templates/operations/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/templates/breaks.html",
        include_str!("../plugin-templates/operations/src/reference_plugin/templates/breaks.html"),
    ),
    (
        "src/reference_plugin/templates/opening_balances.html",
        include_str!(
            "../plugin-templates/operations/src/reference_plugin/templates/opening_balances.html"
        ),
    ),
    (
        "src/reference_plugin/templates/reconciliations.html",
        include_str!(
            "../plugin-templates/operations/src/reference_plugin/templates/reconciliations.html"
        ),
    ),
    (
        "tests/test_page.py",
        include_str!("../plugin-templates/operations/tests/test_page.py"),
    ),
    (
        "docs/overview.md",
        include_str!("../plugin-templates/operations/docs/overview.md"),
    ),
    (
        "docs/setup.md",
        include_str!("../plugin-templates/operations/docs/setup.md"),
    ),
    (
        "docs/troubleshooting.md",
        include_str!("../plugin-templates/operations/docs/troubleshooting.md"),
    ),
    (
        "docs/terms.md",
        include_str!("../plugin-templates/operations/docs/terms.md"),
    ),
    (
        "docs/pages.md",
        include_str!("../plugin-templates/operations/docs/pages.md"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/operations/.github/workflows/check.yaml"),
    ),
    (
        "hooks/pre-push",
        include_str!("../plugin-templates/operations/hooks/pre-push"),
    ),
];

/// The `reporting` template, as it sits in `plugin-templates/reporting/`: the
/// book's positions valued at the close from the lake, its Datasets page, its
/// tests running the suite.
const REPORTING: [(&str, &str); 24] = [
    (
        "AGENTS.md",
        include_str!("../plugin-templates/reporting/AGENTS.md"),
    ),
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-templates/reporting/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-templates/reporting/.dockerignore"),
    ),
    (
        "CLAUDE.md",
        include_str!("../plugin-templates/reporting/CLAUDE.md"),
    ),
    (
        ".gitignore",
        include_str!("../plugin-templates/reporting/.gitignore"),
    ),
    (
        "Dockerfile",
        include_str!("../plugin-templates/reporting/Dockerfile"),
    ),
    (
        "README.md",
        include_str!("../plugin-templates/reporting/README.md"),
    ),
    (
        "pyproject.toml",
        include_str!("../plugin-templates/reporting/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-templates/reporting/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-templates/reporting/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-templates/reporting/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/report.py",
        include_str!("../plugin-templates/reporting/src/reference_plugin/report.py"),
    ),
    (
        "src/reference_plugin/templates/closes.html",
        include_str!("../plugin-templates/reporting/src/reference_plugin/templates/closes.html"),
    ),
    (
        "src/reference_plugin/templates/datasets.html",
        include_str!("../plugin-templates/reporting/src/reference_plugin/templates/datasets.html"),
    ),
    (
        "tests/test_page.py",
        include_str!("../plugin-templates/reporting/tests/test_page.py"),
    ),
    (
        "tests/test_suite.py",
        include_str!("../plugin-templates/reporting/tests/test_suite.py"),
    ),
    (
        "docs/overview.md",
        include_str!("../plugin-templates/reporting/docs/overview.md"),
    ),
    (
        "docs/setup.md",
        include_str!("../plugin-templates/reporting/docs/setup.md"),
    ),
    (
        "docs/troubleshooting.md",
        include_str!("../plugin-templates/reporting/docs/troubleshooting.md"),
    ),
    (
        "docs/terms.md",
        include_str!("../plugin-templates/reporting/docs/terms.md"),
    ),
    (
        "docs/pages.md",
        include_str!("../plugin-templates/reporting/docs/pages.md"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-templates/reporting/src/reference_plugin/declaration.py"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/reporting/.github/workflows/check.yaml"),
    ),
    (
        "hooks/pre-push",
        include_str!("../plugin-templates/reporting/hooks/pre-push"),
    ),
];

/// The connectivity template (SDK 0.30.0): a venue placed at through the
/// SDK's `Trader`, its stand-in venue in memory, the Trading page and its
/// tool, and storage for its trading notes; from SDK 0.31.0 its tests run
/// the connectivity suite.
const CONNECTIVITY: [(&str, &str); 22] = [
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-templates/connectivity/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-templates/connectivity/.dockerignore"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/connectivity/.github/workflows/check.yaml"),
    ),
    (
        ".gitignore",
        include_str!("../plugin-templates/connectivity/.gitignore"),
    ),
    (
        "AGENTS.md",
        include_str!("../plugin-templates/connectivity/AGENTS.md"),
    ),
    (
        "CLAUDE.md",
        include_str!("../plugin-templates/connectivity/CLAUDE.md"),
    ),
    (
        "Dockerfile",
        include_str!("../plugin-templates/connectivity/Dockerfile"),
    ),
    (
        "README.md",
        include_str!("../plugin-templates/connectivity/README.md"),
    ),
    (
        "docs/overview.md",
        include_str!("../plugin-templates/connectivity/docs/overview.md"),
    ),
    (
        "docs/pages.md",
        include_str!("../plugin-templates/connectivity/docs/pages.md"),
    ),
    (
        "docs/setup.md",
        include_str!("../plugin-templates/connectivity/docs/setup.md"),
    ),
    (
        "docs/terms.md",
        include_str!("../plugin-templates/connectivity/docs/terms.md"),
    ),
    (
        "docs/troubleshooting.md",
        include_str!("../plugin-templates/connectivity/docs/troubleshooting.md"),
    ),
    (
        "hooks/pre-push",
        include_str!("../plugin-templates/connectivity/hooks/pre-push"),
    ),
    (
        "pyproject.toml",
        include_str!("../plugin-templates/connectivity/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-templates/connectivity/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-templates/connectivity/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-templates/connectivity/src/reference_plugin/declaration.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-templates/connectivity/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/venue.py",
        include_str!("../plugin-templates/connectivity/src/reference_plugin/venue.py"),
    ),
    (
        "tests/test_trading.py",
        include_str!("../plugin-templates/connectivity/tests/test_trading.py"),
    ),
    (
        "tests/test_suite.py",
        include_str!("../plugin-templates/connectivity/tests/test_suite.py"),
    ),
];

/// The pricing template (SDK 0.31.0, contract v26): the analytics
/// component's batches answered through the SDK's `Pricer` over its own
/// models, its Batches page, and its tests running the pricing suite; from
/// SDK 0.31.1 it asks for no storage, keeping its last batch in memory.
const PRICING: [(&str, &str); 23] = [
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-templates/pricing/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-templates/pricing/.dockerignore"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/pricing/.github/workflows/check.yaml"),
    ),
    (
        ".gitignore",
        include_str!("../plugin-templates/pricing/.gitignore"),
    ),
    (
        "AGENTS.md",
        include_str!("../plugin-templates/pricing/AGENTS.md"),
    ),
    (
        "CLAUDE.md",
        include_str!("../plugin-templates/pricing/CLAUDE.md"),
    ),
    (
        "Dockerfile",
        include_str!("../plugin-templates/pricing/Dockerfile"),
    ),
    (
        "README.md",
        include_str!("../plugin-templates/pricing/README.md"),
    ),
    (
        "docs/overview.md",
        include_str!("../plugin-templates/pricing/docs/overview.md"),
    ),
    (
        "docs/pages.md",
        include_str!("../plugin-templates/pricing/docs/pages.md"),
    ),
    (
        "docs/setup.md",
        include_str!("../plugin-templates/pricing/docs/setup.md"),
    ),
    (
        "docs/terms.md",
        include_str!("../plugin-templates/pricing/docs/terms.md"),
    ),
    (
        "docs/troubleshooting.md",
        include_str!("../plugin-templates/pricing/docs/troubleshooting.md"),
    ),
    (
        "hooks/pre-push",
        include_str!("../plugin-templates/pricing/hooks/pre-push"),
    ),
    (
        "pyproject.toml",
        include_str!("../plugin-templates/pricing/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-templates/pricing/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-templates/pricing/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-templates/pricing/src/reference_plugin/declaration.py"),
    ),
    (
        "src/reference_plugin/model.py",
        include_str!("../plugin-templates/pricing/src/reference_plugin/model.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-templates/pricing/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/templates/batches.html",
        include_str!("../plugin-templates/pricing/src/reference_plugin/templates/batches.html"),
    ),
    (
        "tests/test_page.py",
        include_str!("../plugin-templates/pricing/tests/test_page.py"),
    ),
    (
        "tests/test_suite.py",
        include_str!("../plugin-templates/pricing/tests/test_suite.py"),
    ),
];

/// The roles `plugin new --role` has a template for, each with its files.
pub const ROLE_TEMPLATES: [(&str, &[(&str, &str)]); 6] = [
    ("connectivity", &CONNECTIVITY),
    ("custody", &CUSTODY),
    ("data", &DATA),
    ("operations", &OPERATIONS),
    ("pricing", &PRICING),
    ("reporting", &REPORTING),
];

/// The files to write: the reference plugin's, or the template of `role`.
/// A role from the fixed list with no template of its own is refused, naming
/// those that have one, rather than given the reference plugin under it.
fn template(role: Option<&str>) -> Result<&'static [(&'static str, &'static str)], String> {
    let Some(role) = role else {
        return Ok(&TEMPLATE);
    };
    if let Some((_, files)) = ROLE_TEMPLATES.iter().find(|(held, _)| *held == role) {
        return Ok(files);
    }
    let mut with = ROLE_TEMPLATES
        .iter()
        .map(|(held, _)| format!("`--role {held}`"))
        .collect::<Vec<_>>();
    let last = with.pop().unwrap_or_default();
    let with = if with.is_empty() {
        last
    } else {
        format!("{} or {last}", with.join(", "))
    };
    if crate::check::ROLES.contains(&role) {
        Err(format!(
            "there is no template for `{role}` yet: `plugin new` writes the reference plugin, \
             whose roles you declare in pyproject.toml, or {with}"
        ))
    } else {
        Err(format!(
            "{role:?} is not a role; the roles are {}",
            crate::check::ROLES.join(", ")
        ))
    }
}

/// The files a template holds executable, written so: the pre-push hook git
/// runs once the plugin sets `core.hooksPath hooks` (CI-first, from SDK
/// 0.28.0), which git skips, with only a hint, when it is not.
pub const EXECUTABLE: [&str; 1] = ["hooks/pre-push"];

/// What the template calls itself.
const DISTRIBUTION: &str = "reference-plugin";
const MODULE: &str = "reference_plugin";

/// A plugin's name: what its package is called, what its image is called, and
/// with hyphens as underscores, what it imports as.
///
/// Lowercase letters, digits and single hyphens, starting with a letter. That
/// is a valid Python distribution name, a valid module name once its hyphens
/// become underscores, and a valid image name, which is every place it goes.
pub fn check_name(name: &str) -> Result<(), String> {
    let valid = name.len() <= 50
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !valid {
        return Err(format!(
            "{name:?} is not a plugin name: lowercase letters, digits and single hyphens, \
             starting with a letter, at most 50"
        ));
    }
    // It would import as `meridian`, which is the SDK, and hide it.
    if name == "meridian" {
        return Err(
            "\"meridian\" is the SDK's own name; a plugin called that would hide it".into(),
        );
    }
    Ok(())
}

/// Write a new plugin called `name` into `into`, which must not exist yet:
/// the reference plugin, or `role`'s template. Returns what it wrote.
pub fn scaffold(name: &str, into: &Path, role: Option<&str>) -> Result<Vec<PathBuf>, String> {
    check_name(name)?;
    let files = template(role)?;
    // Never into something already there: a scaffold that overwrote a plugin
    // somebody had started would be the worst thing this command could do.
    if into.exists() {
        return Err(format!(
            "{} already exists; a new plugin goes somewhere new",
            into.display()
        ));
    }
    let module = name.replace('-', "_");

    let mut wrote = Vec::new();
    for (held, contents) in files {
        let path = into.join(renamed(held, name, &module));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|failed| format!("{}: {failed}", parent.display()))?;
        }
        let mut written = renamed(contents, name, &module);
        if *held == CHECK_WORKFLOW {
            written = tracking_this_cli(&written);
        }
        std::fs::write(&path, written).map_err(|failed| format!("{}: {failed}", path.display()))?;
        if EXECUTABLE.contains(held) {
            executable(&path)?;
        }
        wrote.push(path);
    }
    Ok(wrote)
}

/// The plugin's CI workflow, whose `MERIDIAN_VERSION` names the meridian it
/// is checked with.
const CHECK_WORKFLOW: &str = ".github/workflows/check.yaml";

/// The workflow with `MERIDIAN_VERSION` set to this meridian's version: the
/// check the plugin's CI runs is the one of the meridian that wrote it, and
/// moves when its author moves it (the v25 plan, step 6: it tracks the CLI).
pub fn tracking_this_cli(workflow: &str) -> String {
    let mut out = String::with_capacity(workflow.len());
    for line in workflow.split_inclusive('\n') {
        let indent = &line[..line.len() - line.trim_start().len()];
        if line.trim_start().starts_with("MERIDIAN_VERSION:") {
            let end = if line.ends_with('\n') { "\n" } else { "" };
            out.push_str(&format!(
                "{indent}MERIDIAN_VERSION: {}{end}",
                crate::release::VERSION
            ));
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Readable by all and executable, as git checks the template's copy out
/// (0755). The CLI's targets are macOS and Linux; elsewhere there is no bit.
fn executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|failed| format!("{}: {failed}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// The template's name for itself, replaced by the plugin's. The distribution
/// first, because it is the longer match and contains no underscore.
fn renamed(text: &str, name: &str, module: &str) -> String {
    text.replace(DISTRIBUTION, name).replace(MODULE, module)
}

/// What to do next, said once the files are written.
pub fn next_steps(name: &str, into: &Path, role: Option<&str>) -> String {
    let made = match role {
        Some(role) if role.starts_with(['a', 'e', 'i', 'o', 'u']) => {
            format!("an `{role}` plugin, ")
        }
        Some(role) => format!("a `{role}` plugin, "),
        None => String::new(),
    };
    // A role's template that runs the role's suite in its tests says so: a
    // plugin holding the role is verified for it only by passing every case.
    let with_suite = template(role)
        .is_ok_and(|files| files.iter().any(|(path, _)| *path == "tests/test_suite.py"));
    let suite = match role.filter(|_| with_suite) {
        Some(role) => format!(
            "\nIts tests run the {role} suite, each case through its own code; hold it to\n\
             every case, as a verified plugin is held:\n\
             \n\
             \x20 pip install -e . pytest\n\
             \x20 meridian plugin check --verified --run-tests\n"
        ),
        None => String::new(),
    };
    format!(
        "Made {name} in {into}, {made}on the Python SDK (open-meridian).\n\
         \n\
         \x20 cd {into}\n\
         \x20 meridian plugin upload\n\
         \x20 meridian plugin launch {name} 0.1.0 --instance {name}\n\
         \n\
         Upload builds its image here and puts it in the catalogue of the deployment\n\
         `meridian connect` signed you in to; launch shows the roles its\n\
         pyproject.toml asks for, and runs it once you approve them. It reaches its\n\
         sidecar and nothing else.\n\
         \n\
         On a deployment installed for development, run it as you write it instead:\n\
         \n\
         \x20 meridian plugin dev --instance {name}\n\
         {suite}\
         \n\
         Its docs/ is what a deployment admin reads on its Documentation tab before\n\
         launching it: replace each section's prompt with the plugin's own words.\n\
         `meridian plugin check` requires overview.md and setup.md.\n\
         \n\
         AGENTS.md teaches your coding agent that loop, whichever agent it is;\n\
         CLAUDE.md and the develop-live skill lead Claude Code to it. Commit them\n\
         with the plugin, so whoever works on it next has them too.\n",
        into = into.display()
    )
}

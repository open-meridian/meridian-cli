//! `meridian plugin new`: a plugin to start from.
//!
//! What it writes is meridian-python's template -- the reference plugin, kept
//! beside the SDK it is written against and tested there against a real
//! sidecar -- copied into this repository at one SDK revision (`SDK_REV` in the
//! Makefile) and compiled into the binary. So scaffolding needs no network and
//! gives the same plugin every time, and `check-vendored-template` fails when
//! this copy stops being the SDK's. With `--role dgm` or `--role reporting` it
//! writes that role's template instead, from the SDK's `templates/<role>/`,
//! vendored the same way: a whole plugin holding the role, its tests running
//! the role's suite.
//!
//! The only thing changed on the way out is the name: the templates call
//! themselves `reference-plugin`, importable as `reference_plugin`, and both
//! become whatever the plugin is called.

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// The template, file by file, as it sits in `plugin-template/`. A test fails
/// if a file there is missing from this list, so a new one cannot be dropped.
const TEMPLATE: [(&str, &str); 15] = [
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
        ".github/workflows/check.yaml",
        include_str!("../plugin-template/.github/workflows/check.yaml"),
    ),
];

/// The `dgm` template, as it sits in `plugin-templates/dgm/`: a vendor's
/// prices put into the lake, its catalogue declared, its Connection and
/// Datasets pages with their read tools, and its tests running the suite.
const DGM: [(&str, &str); 19] = [
    (
        "AGENTS.md",
        include_str!("../plugin-templates/dgm/AGENTS.md"),
    ),
    (
        ".claude/skills/develop-live/SKILL.md",
        include_str!("../plugin-templates/dgm/.claude/skills/develop-live/SKILL.md"),
    ),
    (
        ".dockerignore",
        include_str!("../plugin-templates/dgm/.dockerignore"),
    ),
    (
        "CLAUDE.md",
        include_str!("../plugin-templates/dgm/CLAUDE.md"),
    ),
    (
        ".gitignore",
        include_str!("../plugin-templates/dgm/.gitignore"),
    ),
    (
        "Dockerfile",
        include_str!("../plugin-templates/dgm/Dockerfile"),
    ),
    (
        "README.md",
        include_str!("../plugin-templates/dgm/README.md"),
    ),
    (
        "pyproject.toml",
        include_str!("../plugin-templates/dgm/pyproject.toml"),
    ),
    (
        "src/reference_plugin/__init__.py",
        include_str!("../plugin-templates/dgm/src/reference_plugin/__init__.py"),
    ),
    (
        "src/reference_plugin/__main__.py",
        include_str!("../plugin-templates/dgm/src/reference_plugin/__main__.py"),
    ),
    (
        "src/reference_plugin/convert.py",
        include_str!("../plugin-templates/dgm/src/reference_plugin/convert.py"),
    ),
    (
        "src/reference_plugin/declaration.py",
        include_str!("../plugin-templates/dgm/src/reference_plugin/declaration.py"),
    ),
    (
        "src/reference_plugin/page.py",
        include_str!("../plugin-templates/dgm/src/reference_plugin/page.py"),
    ),
    (
        "src/reference_plugin/vendor.py",
        include_str!("../plugin-templates/dgm/src/reference_plugin/vendor.py"),
    ),
    (
        "src/reference_plugin/templates/connection.html",
        include_str!("../plugin-templates/dgm/src/reference_plugin/templates/connection.html"),
    ),
    (
        "src/reference_plugin/templates/datasets.html",
        include_str!("../plugin-templates/dgm/src/reference_plugin/templates/datasets.html"),
    ),
    (
        "tests/test_page.py",
        include_str!("../plugin-templates/dgm/tests/test_page.py"),
    ),
    (
        "tests/test_suite.py",
        include_str!("../plugin-templates/dgm/tests/test_suite.py"),
    ),
    (
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/dgm/.github/workflows/check.yaml"),
    ),
];

/// The `reporting` template, as it sits in `plugin-templates/reporting/`: the
/// book's positions valued at the close from the lake, its tests running the
/// suite.
const REPORTING: [(&str, &str); 17] = [
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
        ".github/workflows/check.yaml",
        include_str!("../plugin-templates/reporting/.github/workflows/check.yaml"),
    ),
];

/// The roles `plugin new --role` has a template for, each with its files.
pub const ROLE_TEMPLATES: [(&str, &[(&str, &str)]); 2] = [("dgm", &DGM), ("reporting", &REPORTING)];

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
    let with = ROLE_TEMPLATES
        .iter()
        .map(|(held, _)| format!("`--role {held}`"))
        .collect::<Vec<_>>()
        .join(" or ");
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
    for (path, contents) in files {
        let path = into.join(renamed(path, name, &module));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|failed| format!("{}: {failed}", parent.display()))?;
        }
        std::fs::write(&path, renamed(contents, name, &module))
            .map_err(|failed| format!("{}: {failed}", path.display()))?;
        wrote.push(path);
    }
    Ok(wrote)
}

/// The template's name for itself, replaced by the plugin's. The distribution
/// first, because it is the longer match and contains no underscore.
fn renamed(text: &str, name: &str, module: &str) -> String {
    text.replace(DISTRIBUTION, name).replace(MODULE, module)
}

/// What to do next, said once the files are written.
pub fn next_steps(name: &str, into: &Path, role: Option<&str>) -> String {
    let made = match role {
        Some(role) => format!("a `{role}` plugin, "),
        None => String::new(),
    };
    // A role's template runs the role's suite in its tests, and a plugin
    // holding the role is verified for it only by passing every case.
    let suite = match role {
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
         AGENTS.md teaches your coding agent that loop, whichever agent it is;\n\
         CLAUDE.md and the develop-live skill lead Claude Code to it. Commit them\n\
         with the plugin, so whoever works on it next has them too.\n",
        into = into.display()
    )
}

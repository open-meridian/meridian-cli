//! `meridian plugin new`: a plugin to start from.
//!
//! What it writes is meridian-python's template -- the reference plugin, kept
//! beside the SDK it is written against and tested there against a real
//! sidecar -- copied into this repository at one SDK revision (`SDK_REV` in the
//! Makefile) and compiled into the binary. So scaffolding needs no network and
//! gives the same plugin every time, and `check-vendored-template` fails when
//! this copy stops being the SDK's.
//!
//! The only thing changed on the way out is the name: the template calls
//! itself `reference-plugin`, importable as `reference_plugin`, and both become
//! whatever the plugin is called.

use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// The template, file by file, as it sits in `plugin-template/`. A test fails
/// if a file there is missing from this list, so a new one cannot be dropped.
const TEMPLATE: [(&str, &str); 7] = [
    (
        ".dockerignore",
        include_str!("../plugin-template/.dockerignore"),
    ),
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
];

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

/// Write a new plugin called `name` into `into`, which must not exist yet.
/// Returns what it wrote.
pub fn scaffold(name: &str, into: &Path) -> Result<Vec<PathBuf>, String> {
    check_name(name)?;
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
    for (path, contents) in TEMPLATE {
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
pub fn next_steps(name: &str, into: &Path) -> String {
    format!(
        "Made {name} in {into}, on the Python SDK (open-meridian).\n\
         \n\
         \x20 cd {into}\n\
         \x20 meridian plugin upload\n\
         \x20 meridian plugin launch {name} 0.1.0 --instance {name}\n\
         \n\
         Upload builds its image here and puts it in the catalogue of the deployment\n\
         `meridian connect` signed you in to; launch shows the roles and tags its\n\
         pyproject.toml asks for, and runs it once you approve them. It reaches its\n\
         sidecar and nothing else.\n",
        into = into.display()
    )
}

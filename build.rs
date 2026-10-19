//! What `meridian --version` and `meridian upgrade` need to know about the
//! build: the target it was built for, which names the release binary that
//! replaces it, and the meridian-core revision it links. And what `plugin
//! check`'s role-pages rule reads of the SDK: every file in sdk-roles/, the
//! SDK's role modules declaring pages at SDK_REV (`make vendor-template`),
//! compiled in by its path there, so a file the vendoring adds is read
//! without a list kept by hand.

use std::path::Path;

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for a build script");
    println!("cargo:rustc-env=MERIDIAN_TARGET={target}");
    // Cargo.lock names the pinned revision in the git source of every
    // meridian-core crate: `...meridian-core.git?rev=<rev>#<rev>`.
    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let core = lock
        .split("meridian-core.git?rev=")
        .nth(1)
        .and_then(|rest| rest.split(['#', '"']).next())
        .unwrap_or("unknown");
    println!("cargo:rustc-env=MERIDIAN_CORE_REV={core}");
    println!("cargo:rerun-if-changed=Cargo.lock");
    sdk_roles();
}

/// `$OUT_DIR/sdk_roles.rs`: `SDK_ROLES`, each file under sdk-roles/ as its
/// path there (`/` between its parts) and its text.
fn sdk_roles() {
    fn walk(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|entry| entry.expect("a directory entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|e| e == "py") {
                found.push(path);
            }
        }
    }
    let root =
        Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it")).join("sdk-roles");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut out = String::from("pub(crate) const SDK_ROLES: &[(&str, &str)] = &[\n");
    for file in &files {
        let at = file
            .strip_prefix(&root)
            .expect("under sdk-roles")
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        out.push_str(&format!(
            "    ({at:?}, include_str!({:?})),\n",
            file.display().to_string()
        ));
    }
    out.push_str("];\n");
    let dest =
        Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("sdk_roles.rs");
    std::fs::write(dest, out).expect("OUT_DIR is writable");
}

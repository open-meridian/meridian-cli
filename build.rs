//! What `meridian --version` and `meridian upgrade` need to know about the
//! build: the target it was built for, which names the release binary that
//! replaces it, and the meridian-core revision it links.

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
}

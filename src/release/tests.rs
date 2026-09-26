use super::*;

#[test]
fn each_target_is_replaced_by_its_own_release_binary() {
    for (target, named) in [
        (
            "x86_64-unknown-linux-musl",
            "meridian-x86_64-unknown-linux-musl",
        ),
        (
            "x86_64-unknown-linux-gnu",
            "meridian-x86_64-unknown-linux-musl",
        ),
        (
            "aarch64-unknown-linux-gnu",
            "meridian-aarch64-unknown-linux-musl",
        ),
        ("aarch64-apple-darwin", "meridian-aarch64-apple-darwin"),
        ("x86_64-apple-darwin", "meridian-x86_64-apple-darwin"),
    ] {
        assert_eq!(asset(target).as_deref(), Some(named), "{target}");
    }
    assert_eq!(asset("x86_64-pc-windows-msvc"), None);
    assert_eq!(asset("riscv64gc-unknown-linux-gnu"), None);
}

#[test]
fn the_latest_is_the_tag_its_redirect_names() {
    assert_eq!(
        tag_of("https://github.com/open-meridian/meridian-cli/releases/tag/v0.2.0").as_deref(),
        Some("v0.2.0")
    );
    assert_eq!(
        tag_of("/releases/tag/v1.0.0?x=1").as_deref(),
        Some("v1.0.0")
    );
    // No release yet: GitHub sends /releases/latest to the releases page.
    assert_eq!(
        tag_of("https://github.com/open-meridian/meridian-cli/releases"),
        None
    );
}

#[test]
fn versions_compare_by_their_numbers() {
    assert_eq!(compare("v0.2.0", "0.2.0"), Ordering::Equal);
    assert_eq!(compare("0.10.0", "0.9.9"), Ordering::Greater);
    assert_eq!(compare("v0.1.0", "0.2.0"), Ordering::Less);
    assert_eq!(compare("1.0.0", "0.99.0"), Ordering::Greater);
}

#[test]
fn a_checksum_is_its_first_field_whatever_file_name_follows() {
    let bytes = b"a binary";
    let sum = format!("{:x}", Sha256::digest(bytes));
    assert!(checksum_matches(
        bytes,
        &format!("{sum}  dist/meridian-aarch64-apple-darwin\n")
    ));
    assert!(checksum_matches(bytes, &sum.to_uppercase()));
    assert!(!checksum_matches(b"another", &format!("{sum}  meridian")));
    assert!(!checksum_matches(bytes, ""));
    assert!(!checksum_matches(bytes, "abc  meridian"));
}

#[test]
fn a_replacement_is_whole_or_nothing() {
    let dir = std::env::temp_dir().join(format!("meridian-replace-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("meridian");
    std::fs::write(&exe, b"old").unwrap();
    replace(&exe, b"new").unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"new");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
    // Nothing is left beside it.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    // Into a directory that is not there: refused, and said.
    let refused = replace(&dir.join("gone").join("meridian"), b"x").unwrap_err();
    assert!(refused.contains("was left as it was"), "{refused}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_version_line_names_the_target_and_the_core_it_links() {
    let line = version_line();
    assert!(line.starts_with(&format!("meridian {VERSION} (")), "{line}");
    assert!(
        line.contains(TARGET) && line.contains("meridian-core "),
        "{line}"
    );
    assert_ne!(CORE_REV, "unknown", "build.rs found the pinned revision");
}

#[test]
fn releases_come_over_https_or_from_this_machine() {
    assert_eq!(releases_from(None).unwrap(), RELEASES);
    assert_eq!(releases_from(Some("")).unwrap(), RELEASES);
    assert_eq!(
        releases_from(Some("https://mirror.firm.example/meridian/")).unwrap(),
        "https://mirror.firm.example/meridian"
    );
    assert_eq!(
        releases_from(Some("http://127.0.0.1:8765/releases")).unwrap(),
        "http://127.0.0.1:8765/releases"
    );
    for refused in [
        "http://mirror.firm.example",
        "http://127.0.0.1.evil.example",
        "ftp://x",
    ] {
        assert!(releases_from(Some(refused)).is_err(), "{refused}");
    }
}

#[test]
fn a_deployment_refusing_this_version_is_said_as_what_to_do() {
    let body = r#"{"error":"cli_version","reason":"meridian 0.0.9 is older than this deployment serves; it serves meridian 0.1.0 or later","serves":"0.1.0"}"#;
    let said = version_refused(reqwest::StatusCode::BAD_REQUEST, body).unwrap();
    assert!(said.contains("serves meridian 0.1.0 or later"), "{said}");
    assert!(said.contains("meridian upgrade --to"), "{said}");
    // Anything else is somebody else's refusal.
    assert_eq!(
        version_refused(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_request"}"#
        ),
        None
    );
    assert_eq!(
        version_refused(reqwest::StatusCode::UNAUTHORIZED, body),
        None
    );
    assert_eq!(
        version_refused(reqwest::StatusCode::BAD_REQUEST, "not json"),
        None
    );
}

#[test]
fn every_request_to_a_deployment_names_this_version() {
    let headers = naming_this_version();
    assert_eq!(headers[VERSION_HEADER], VERSION);
}

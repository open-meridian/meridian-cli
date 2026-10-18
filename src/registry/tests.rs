use std::sync::{Arc, Mutex};

use super::*;
use crate::fake_http::{serve, Asked, Reply, Scratch};

// ── Forms ────────────────────────────────────────────────────────────────

#[test]
fn a_release_tag_is_v_and_a_semver_version() {
    for (tag, version) in [
        ("v0.18.0", Some("0.18.0")),
        ("v1.4.0-rc.1", Some("1.4.0-rc.1")),
        ("v1.4.0+build.7", Some("1.4.0+build.7")),
        ("v1.4.0-rc.1+b", Some("1.4.0-rc.1+b")),
        ("0.18.0", None),
        ("v0.18", None),
        ("v01.2.3", None),
        ("v1.2.3-", None),
        ("v1.2.3-rc..1", None),
        ("v1.2.3.4", None),
        ("vx.y.z", None),
    ] {
        assert_eq!(semver_of_tag(tag), version, "{tag}");
    }
}

#[test]
fn a_slug_is_a_lower_case_dns_label() {
    assert!(is_slug("open-meridian"));
    assert!(is_slug("acme"));
    assert!(!is_slug("Acme"));
    assert!(!is_slug("-acme"));
    assert!(!is_slug("acme_co"));
    assert!(!is_slug(&"a".repeat(40)));
}

#[test]
fn origin_names_the_github_repository() {
    for origin in [
        "git@github.com:acme/plugin-venue.git",
        "ssh://git@github.com/acme/plugin-venue",
        "https://github.com/acme/plugin-venue.git",
        "https://github.com/acme/plugin-venue/",
    ] {
        assert_eq!(
            github_repository(origin).unwrap(),
            "https://github.com/acme/plugin-venue",
            "{origin}"
        );
    }
    assert!(github_repository("https://gitlab.com/acme/venue")
        .unwrap_err()
        .contains("not a GitHub repository"));
    assert!(github_repository("https://github.com/acme").is_err());
}

#[test]
fn a_private_key_is_refused_without_being_echoed() {
    let private = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
    let refused = public_key(private).unwrap_err();
    assert!(refused.contains("private key"));
    assert!(!refused.contains("b3BlbnNzaC1rZXktdjEAAAAA"));
    let line = public_key(
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFr2Kd8Wm4yT0sQ6nB1cJ9vX3gL7pE5hU2aR8oZ0iN4t release key\n",
    )
    .unwrap();
    assert_eq!(comment_of(&line).as_deref(), Some("release key"));
    assert!(public_key("not a key").is_err());
}

#[test]
fn a_moment_reads_back_as_the_nanoseconds_it_was() {
    let at = 1_792_540_800_000_000_000;
    assert_eq!(moment(at), "2026-10-21T00:00:00Z");
    assert_eq!(ns_of("2026-10-21T00:00:00Z"), Some(at));
    assert_eq!(ns_of("2026-10-21T00:00Z"), Some(at));
    assert_eq!(ns_of("2026-10-21"), Some(at));
    assert_eq!(ns_of("2026-13-01"), None);
    assert_eq!(ns_of("yesterday"), None);
}

// ── The pre-flight ───────────────────────────────────────────────────────

struct Repo {
    tag: Result<String, String>,
    origin: String,
}

impl Repository for Repo {
    fn tag_object(&self, _: &Path, _: &str) -> Result<String, String> {
        self.tag.clone()
    }
    fn origin(&self, _: &Path) -> Result<String, String> {
        Ok(self.origin.clone())
    }
}

const SIGNED: &str = "object 4e1d9c27b85a03f6e2c1d0b9a8f7e6d5c4b3a291\ntype commit\ntag v0.1.0\n\
    tagger Ada <ada@acme.example> 1792108800 +0000\n\nv0.1.0\n-----BEGIN SSH SIGNATURE-----\n\
    U1NIU0lH\n-----END SSH SIGNATURE-----\n";

fn plugin(scratch: &Scratch) -> PathBuf {
    let dir = scratch.0.join("venue-desk");
    crate::plugin::scaffold("venue-desk", &dir, None).expect("scaffolded");
    // Its test needs no SDK, so the check needs nothing installed.
    std::fs::write(
        dir.join("tests/test_page.py"),
        "def test_nothing() -> None:\n    assert True\n",
    )
    .unwrap();
    dir
}

fn asked(dir: &Path, registry: &str, tag: &str) -> Publish {
    Publish {
        dir: dir.to_path_buf(),
        registry: registry.into(),
        tag: tag.into(),
        organisation: "acme".into(),
        follow_for: Duration::from_secs(5),
        follow_every: Duration::from_millis(10),
        shown: Shown::default(),
    }
}

fn repo() -> Repo {
    Repo {
        tag: Ok(SIGNED.into()),
        origin: "git@github.com:acme/plugin-venue-desk.git".into(),
    }
}

#[test]
fn the_preflight_finds_the_plugin_its_version_and_its_repository() {
    let scratch = Scratch::new("preflight");
    let dir = plugin(&scratch);
    assert_eq!(
        preflight(&asked(&dir, "community", "v0.1.0"), &repo()).unwrap(),
        Ready {
            plugin: "venue-desk".into(),
            version: "0.1.0".into(),
            repository: "https://github.com/acme/plugin-venue-desk".into(),
        }
    );
}

#[test]
fn the_preflight_refuses_locally_what_the_platform_would() {
    let scratch = Scratch::new("preflight-refusals");
    let dir = plugin(&scratch);
    let refused =
        |publish: Publish, repository: &Repo| preflight(&publish, repository).unwrap_err();
    assert!(refused(asked(&dir, "private", "v0.1.0"), &repo()).contains("community or verified"));
    assert!(refused(asked(&dir, "community", "0.1.0"), &repo()).contains("not a release tag"));
    assert!(refused(asked(&dir, "community", "v0.2.0"), &repo())
        .contains("v0.2.0 names 0.2.0, and pyproject.toml's version is 0.1.0"));
    let unsigned = Repo {
        tag: Ok(SIGNED.replace("-----BEGIN SSH SIGNATURE-----", "")),
        ..repo()
    };
    assert!(refused(asked(&dir, "community", "v0.1.0"), &unsigned).contains("not signed"));
    let no_tag = Repo {
        tag: Err("v0.1.0 is not an annotated tag".into()),
        ..repo()
    };
    assert!(refused(asked(&dir, "community", "v0.1.0"), &no_tag).contains("annotated"));
    // A missing section fails plugin check, which the builder runs too.
    std::fs::remove_file(dir.join("docs/overview.md")).unwrap();
    let said = refused(asked(&dir, "community", "v0.1.0"), &repo());
    assert!(
        said.contains("does not pass") && said.contains("there is no docs/overview.md"),
        "{said}"
    );
}

// ── The sign-in and a publish, against a stand-in platform ───────────────

/// A stand-in platform: its authorization redirects straight back with a
/// code, as a person consenting would; its token endpoint answers as
/// `tokens` says; its publishing API builds and lists.
#[derive(Default)]
struct Platform {
    narrows_on_refresh: bool,
    refused_refresh: bool,
    /// The narrowed tokens' life, in seconds: short, to be renewed at once.
    narrowed_expires_in: Option<u64>,
    /// Whether the narrowed browser sign-in answers no refresh token.
    narrowed_without_refresh: bool,
}

fn open_in_a_browser() -> Box<Opener> {
    Box::new(|url: &str| {
        let url = url.to_string();
        tokio::spawn(async move {
            // Follows the platform's redirect to the loopback address.
            let _ = reqwest::get(url).await;
        });
    })
}

async fn platform(given: Platform) -> (String, crate::fake_http::Kept) {
    let builds = Arc::new(Mutex::new(0u32));
    serve(move |asked: &Asked| {
        let path = asked.path.split('?').next().unwrap_or_default().to_string();
        match (asked.method.as_str(), path.as_str()) {
            ("GET", "/oauth/authorize") => {
                let redirect = asked.field("redirect_uri").unwrap_or_default();
                let state = asked.field("state").unwrap_or_default();
                let scope = asked.field("scope").unwrap_or_default();
                let code = if scope == "publish" { "C-sign-in" } else { "C-narrowed" };
                Reply::redirect(&format!("{redirect}?code={code}&state={state}"))
            }
            ("POST", "/oauth/token") => match asked.field("grant_type").as_deref() {
                Some("authorization_code") => {
                    let narrowed = asked.field("code").as_deref() == Some("C-narrowed");
                    let refresh = match (narrowed, given.narrowed_without_refresh) {
                        (true, true) => "",
                        (true, false) => "R-narrowed",
                        _ => "R-1",
                    };
                    Reply::json(
                        200,
                        serde_json::json!({
                            "token_type": "Bearer",
                            "expires_in": if narrowed { given.narrowed_expires_in.unwrap_or(600) } else { 600 },
                            "access_token": if narrowed { "A-push" } else { "A-1" },
                            "refresh_token": refresh,
                            "scope": if narrowed { "publish:acme/venue-desk" } else { "publish" },
                        }),
                    )
                }
                // A narrowed sign-in's own refresh keeps its narrow scope.
                Some("refresh_token") if asked.field("refresh_token").as_deref() == Some("R-narrowed") => {
                    Reply::json(
                        200,
                        serde_json::json!({
                            "token_type": "Bearer", "expires_in": 600,
                            "access_token": "A-push-2", "refresh_token": "R-narrowed-2",
                            "scope": "publish:acme/venue-desk",
                        }),
                    )
                }
                Some("refresh_token") if given.refused_refresh => Reply::json(
                    400,
                    serde_json::json!({"error": "invalid_grant",
                                       "error_description": "the refresh token was already used; sign in again"}),
                ),
                Some("refresh_token") => {
                    let asked_scope = asked.field("scope").unwrap_or_default();
                    let scope = if given.narrows_on_refresh && !asked_scope.is_empty() {
                        asked_scope
                    } else {
                        "publish".into()
                    };
                    let push = scope != "publish";
                    Reply::json(
                        200,
                        serde_json::json!({
                            "token_type": "Bearer",
                            "expires_in": if push { given.narrowed_expires_in.unwrap_or(600) } else { 600 },
                            "access_token": if push { "A-push" } else { "A-2" },
                            "refresh_token": "R-2", "scope": scope,
                        }),
                    )
                }
                _ => Reply::json(400, serde_json::json!({"error": "unsupported_grant_type"})),
            },
            ("POST", "/api/v1/registry/organisations/acme/plugins/venue-desk/versions") => {
                if asked.header("authorization") != Some("Bearer A-push") {
                    return Reply::json(401, serde_json::json!({"error": "invalid_token"}));
                }
                Reply::json(
                    202,
                    serde_json::json!({
                        "build_id": "BLD-1", "listing": "acme/venue-desk", "version": "0.1.0",
                        "registry": "community", "state": "PLUGIN_BUILD_STATE_QUEUED",
                        "log_url": "https://github.com/open-meridian/plugin-builder/actions/runs/1",
                    }),
                )
            }
            ("GET", "/api/v1/registry/organisations/acme/plugins/venue-desk/builds/BLD-1") => {
                let mut seen = builds.lock().unwrap();
                *seen += 1;
                let state = if *seen < 2 {
                    "PLUGIN_BUILD_STATE_BUILDING"
                } else {
                    "PLUGIN_BUILD_STATE_LISTED"
                };
                Reply::json(
                    200,
                    serde_json::json!({
                        "build_id": "BLD-1", "listing": "acme/venue-desk", "version": "0.1.0",
                        "registry": "community", "state": state,
                        "image_digest": "sha256:7b3e",
                        "findings": [
                            {"check": "tag signature", "severity": "BUILD_FINDING_SEVERITY_PASSED",
                             "detail": "signed by a registered, active key of acme"},
                            {"check": "vulnerabilities", "severity": "BUILD_FINDING_SEVERITY_WARNING",
                             "detail": "one medium finding, no fix released"},
                            {"check": "attestations", "severity": "BUILD_FINDING_SEVERITY_PASSED",
                             "detail": "provenance and plugin metadata read from the image's referrers"},
                        ],
                    }),
                )
            }
            ("GET", "/api/v1/registry/organisations/acme/signing-keys") => Reply::json(
                200,
                serde_json::json!({"keys": [{
                    "key_id": "SGK-1", "organisation": "acme", "name": "release key",
                    "fingerprint": "SHA256:Zc3v", "registered_at_ns": 1792368000000000000u64,
                }]}),
            ),
            ("POST", "/api/v1/registry/organisations/acme/signing-keys") => {
                let body = asked.json();
                Reply::json(
                    201,
                    serde_json::json!({"key_id": "SGK-2", "name": body["name"],
                                       "fingerprint": "SHA256:Lm7p", "public_key": body["public_key"]}),
                )
            }
            ("DELETE", "/api/v1/registry/organisations/acme/signing-keys/SGK-1") => Reply::json(
                200,
                serde_json::json!({"key_id": "SGK-1", "fingerprint": "SHA256:Zc3v",
                                   "revoked_at_ns": 1792627200000000000u64,
                                   "not_trusted_after_ns": asked.json()["not_trusted_after_ns"]}),
            ),
            _ => Reply::json(404, serde_json::json!({"error": "no such row"})),
        }
    })
    .await
}

#[tokio::test]
async fn a_publish_signs_in_narrows_its_token_and_follows_the_build_until_it_is_listed() {
    let (address, kept) = platform(Platform {
        narrows_on_refresh: true,
        ..Platform::default()
    })
    .await;
    let scratch = Scratch::new("publish");
    let dir = plugin(&scratch);
    let within = scratch.0.join("platform");
    let signed_in = SignedIn::within(within.clone(), &address, open_in_a_browser())
        .await
        .unwrap();
    let build = publish(&asked(&dir, "community", "v0.1.0"), &repo(), &signed_in)
        .await
        .unwrap();
    assert_eq!(build["state"], "PLUGIN_BUILD_STATE_LISTED");
    let said = said(&build, &address, &Shown::default());
    assert!(
        said.contains("acme/venue-desk 0.1.0 (community): listed"),
        "{said}"
    );
    assert!(
        said.contains("warn  vulnerabilities: one medium finding"),
        "{said}"
    );
    assert!(
        said.contains(&format!("listing: {address}/plugins/acme/venue-desk")),
        "{said}"
    );

    let asked: Vec<Asked> = kept.lock().unwrap().clone();
    let authorize = asked
        .iter()
        .find(|a| a.path.starts_with("/oauth/authorize"))
        .unwrap();
    assert_eq!(authorize.field("client_id").as_deref(), Some(CLIENT_ID));
    assert_eq!(authorize.field("scope").as_deref(), Some("publish"));
    assert_eq!(
        authorize.field("code_challenge_method").as_deref(),
        Some("S256")
    );
    assert_eq!(
        authorize.field("resource"),
        Some(format!("{address}/api/v1/registry"))
    );
    assert!(authorize
        .field("redirect_uri")
        .unwrap()
        .starts_with("http://127.0.0.1:"));
    let tokens: Vec<&Asked> = asked.iter().filter(|a| a.path == "/oauth/token").collect();
    assert_eq!(
        tokens[0].field("grant_type").as_deref(),
        Some("authorization_code")
    );
    assert!(tokens[0]
        .field("code_verifier")
        .is_some_and(|v| v.len() >= 43));
    // The push token: a refresh naming the plugin's scope, once, and held
    // while the build is followed, never asked again on each poll.
    assert_eq!(
        tokens[1].field("grant_type").as_deref(),
        Some("refresh_token")
    );
    assert_eq!(
        tokens[1].field("scope").as_deref(),
        Some("publish:acme/venue-desk")
    );
    assert_eq!(tokens.len(), 2, "narrowed once for the whole publish");
    let polls = asked
        .iter()
        .filter(|a| a.method == "GET" && a.path.contains("/builds/"))
        .count();
    assert!(polls >= 2, "{polls}");
    let published = asked
        .iter()
        .find(|a| a.method == "POST" && a.path.ends_with("/versions"))
        .unwrap();
    assert_eq!(
        published.json(),
        serde_json::json!({
            "organisation": "acme", "plugin": "venue-desk", "registry": "community",
            "repository": "https://github.com/acme/plugin-venue-desk", "tag": "v0.1.0",
        })
    );

    // The sign-in is kept, its refresh token the newest, and never the push
    // token, which was for this publish alone.
    let held = sessions::read(&within, &address).expect("kept");
    assert_eq!(held.client_id, CLIENT_ID);
    assert_ne!(held.access_token, "A-push");
    assert!(held.expires_at_s >= crate::credential::now_s() + SIGN_IN_S - 60);
    assert!(held.refresh_token.starts_with("R-"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let file = std::fs::read_dir(&within)
            .unwrap()
            .flatten()
            .find(|e| e.path().extension().is_some_and(|x| x == "json"))
            .unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[tokio::test]
async fn a_platform_that_does_not_narrow_on_refresh_is_signed_in_to_for_the_plugin_alone() {
    let (address, kept) = platform(Platform::default()).await;
    let scratch = Scratch::new("narrowed");
    let signed_in = SignedIn::within(scratch.0.join("platform"), &address, open_in_a_browser())
        .await
        .unwrap();
    assert_eq!(
        signed_in.push_token("acme", "venue-desk").await.unwrap(),
        "A-push"
    );
    let scopes: Vec<String> = kept
        .lock()
        .unwrap()
        .iter()
        .filter(|a| a.path.starts_with("/oauth/authorize"))
        .filter_map(|a| a.field("scope"))
        .collect();
    assert_eq!(scopes, ["publish", "publish:acme/venue-desk"]);
}

#[tokio::test]
async fn a_push_token_is_held_for_its_life_and_narrowed_again_only_near_its_end() {
    // Ten minutes: held, so a second ask sends nothing.
    let (address, kept) = platform(Platform {
        narrows_on_refresh: true,
        ..Platform::default()
    })
    .await;
    let scratch = Scratch::new("held-push");
    let signed_in = SignedIn::within(scratch.0.join("platform"), &address, open_in_a_browser())
        .await
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            signed_in.push_token("acme", "venue-desk").await.unwrap(),
            "A-push"
        );
    }
    let refreshes = |kept: &crate::fake_http::Kept| {
        kept.lock()
            .unwrap()
            .iter()
            .filter(|a| a.field("grant_type").as_deref() == Some("refresh_token"))
            .count()
    };
    assert_eq!(refreshes(&kept), 1);

    // Thirty seconds: within a minute of its end, so narrowed again each time.
    let (address, kept) = platform(Platform {
        narrows_on_refresh: true,
        narrowed_expires_in: Some(30),
        ..Platform::default()
    })
    .await;
    let scratch = Scratch::new("near-its-end");
    let signed_in = SignedIn::within(scratch.0.join("platform"), &address, open_in_a_browser())
        .await
        .unwrap();
    signed_in.push_token("acme", "venue-desk").await.unwrap();
    signed_in.push_token("acme", "venue-desk").await.unwrap();
    assert_eq!(refreshes(&kept), 2);
}

#[tokio::test]
async fn the_browser_is_opened_for_a_narrowed_sign_in_once_a_publish_at_most() {
    let authorized = |kept: &crate::fake_http::Kept| -> Vec<String> {
        kept.lock()
            .unwrap()
            .iter()
            .filter(|a| a.path.starts_with("/oauth/authorize"))
            .filter_map(|a| a.field("scope"))
            .collect()
    };
    // The narrowed sign-in's own refresh token renews it: no second browser.
    let (address, kept) = platform(Platform {
        narrowed_expires_in: Some(30),
        ..Platform::default()
    })
    .await;
    let scratch = Scratch::new("browser-once");
    let signed_in = SignedIn::within(scratch.0.join("platform"), &address, open_in_a_browser())
        .await
        .unwrap();
    assert_eq!(
        signed_in.push_token("acme", "venue-desk").await.unwrap(),
        "A-push"
    );
    assert_eq!(
        signed_in.push_token("acme", "venue-desk").await.unwrap(),
        "A-push-2"
    );
    assert_eq!(authorized(&kept), ["publish", "publish:acme/venue-desk"]);

    // Without one, a second browser is refused rather than opened.
    let (address, kept) = platform(Platform {
        narrowed_expires_in: Some(30),
        narrowed_without_refresh: true,
        ..Platform::default()
    })
    .await;
    let scratch = Scratch::new("browser-refused");
    let signed_in = SignedIn::within(scratch.0.join("platform"), &address, open_in_a_browser())
        .await
        .unwrap();
    signed_in.push_token("acme", "venue-desk").await.unwrap();
    let refused = signed_in
        .push_token("acme", "venue-desk")
        .await
        .unwrap_err();
    assert!(
        refused.contains("signed in for it once already"),
        "{refused}"
    );
    assert_eq!(authorized(&kept), ["publish", "publish:acme/venue-desk"]);
}

#[test]
fn a_build_report_is_one_line_a_check_and_every_full_text_with_details() {
    let build = serde_json::json!({
        "listing": "acme/venue-desk", "version": "0.2.0", "registry": "community",
        "state": "PLUGIN_BUILD_STATE_REFUSED",
        "findings": [
            {"check": "licences", "severity": "BUILD_FINDING_SEVERITY_BLOCKED",
             "detail": "requests 2.31 is GPL-3.0, the plugin Apache-2.0\nrequests 2.31.0 (GPL-3.0-only) conflicts with the plugin's declared licence, Apache-2.0.\n\nReplace it, or relicense the plugin."},
            {"check": "vulnerabilities", "severity": "BUILD_FINDING_SEVERITY_WARNING",
             "detail": "an older finding with no newline, which says a great deal more than one terminal line holds at forty"},
            {"check": "tag signature", "severity": "BUILD_FINDING_SEVERITY_PASSED",
             "detail": "signed"},
        ],
        "log_url": "https://github.com/open-meridian/plugin-builder/actions/runs/9",
    });
    let narrow = Shown {
        width: 70,
        details: false,
    };
    let said = said(&build, "https://open-meridian.com", &narrow);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines[0], "acme/venue-desk 0.2.0 (community): refused");
    assert_eq!(
        lines[1],
        "  FAIL  licences: requests 2.31 is GPL-3.0, the plugin Apache-2.0"
    );
    assert!(lines[2].starts_with("  warn  vulnerabilities: an older finding"));
    assert!(
        lines[2].ends_with('…') && lines[2].chars().count() == 70,
        "{}",
        lines[2]
    );
    assert_eq!(lines[3], "  ok    tag signature: signed");
    assert_eq!(lines[4], "  (--details prints each check's full text)");
    assert!(!said.contains("Replace it"), "{said}");
    assert!(!said.contains("listing:"), "only once listed: {said}");

    let detailed = said_with_details(&build);
    assert!(detailed.contains(
        "  FAIL  licences: requests 2.31 is GPL-3.0, the plugin Apache-2.0\n          requests 2.31.0 (GPL-3.0-only) conflicts"
    ), "{detailed}");
    assert!(
        detailed.contains("\n\n          Replace it, or relicense the plugin.\n"),
        "{detailed}"
    );
    assert!(
        detailed.contains("at forty\n"),
        "the older finding whole: {detailed}"
    );
    assert!(!detailed.contains("--details"), "{detailed}");
    assert_eq!(detail_parts("one line"), ("one line", "one line"));
    assert_eq!(fitted("short", 10), "short");
}

fn said_with_details(build: &serde_json::Value) -> String {
    said(
        build,
        "https://open-meridian.com",
        &Shown {
            width: 40,
            details: true,
        },
    )
}

#[tokio::test]
async fn a_refused_refresh_ends_the_sign_in_and_says_to_sign_in_again() {
    let (address, _) = platform(Platform {
        narrows_on_refresh: true,
        refused_refresh: true,
        ..Platform::default()
    })
    .await;
    let scratch = Scratch::new("refused-refresh");
    let within = scratch.0.join("platform");
    let signed_in = SignedIn::within(within.clone(), &address, open_in_a_browser())
        .await
        .unwrap();
    let refused = signed_in
        .push_token("acme", "venue-desk")
        .await
        .unwrap_err();
    assert!(
        refused.contains("has ended") && refused.contains("sign in"),
        "{refused}"
    );
    assert!(sessions::read(&within, &address).is_none(), "forgotten");
}

#[tokio::test]
async fn the_keys_are_registered_revoked_and_listed_as_the_rows_say() {
    let (address, kept) = platform(Platform {
        narrows_on_refresh: true,
        ..Platform::default()
    })
    .await;
    let scratch = Scratch::new("keys");
    let signed_in = SignedIn::within(scratch.0.join("platform"), &address, open_in_a_browser())
        .await
        .unwrap();
    let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFr2Kd8Wm4yT0sQ6nB1cJ9vX3gL7pE5hU2aR8oZ0iN4t";
    let made = register_key(&signed_in, "acme", "laptop", line)
        .await
        .unwrap();
    assert_eq!(made["key_id"], "SGK-2");
    let revoked = revoke_key(
        &signed_in,
        "acme",
        "SGK-1",
        1_792_540_800_000_000_000,
        "copied",
    )
    .await
    .unwrap();
    assert_eq!(
        revoked["not_trusted_after_ns"],
        1_792_540_800_000_000_000u64
    );
    let listed = list_keys(&signed_in, "acme").await.unwrap();
    assert!(
        keys_said(&listed).starts_with("SGK-1  SHA256:Zc3v  release key  active since 2026-10-")
    );
    let asked: Vec<Asked> = kept.lock().unwrap().clone();
    let register = asked
        .iter()
        .find(|a| a.method == "POST" && a.path.ends_with("/signing-keys"))
        .unwrap();
    assert_eq!(
        register.json(),
        serde_json::json!({"organisation": "acme", "name": "laptop", "public_key": line})
    );
    assert_eq!(register.header("authorization"), Some("Bearer A-1"));
    let revoke = asked.iter().find(|a| a.method == "DELETE").unwrap();
    assert_eq!(revoke.json()["reason"], "copied");
}

#[test]
fn the_organisation_defaults_to_the_one_the_git_remote_names() {
    // Item 154: `--organisation` approved, defaulting to the organisation the
    // remote is linked to; refused, naming the flag, where none is known.
    let here = Path::new(".");
    assert_eq!(
        organisation(Some("open-meridian"), here, &repo()).unwrap(),
        "open-meridian"
    );
    assert_eq!(organisation(None, here, &repo()).unwrap(), "acme");
    let mixed = Repo {
        origin: "https://github.com/Acme-Co/plugin-venue".into(),
        ..repo()
    };
    assert_eq!(organisation(None, here, &mixed).unwrap(), "acme-co");
    let elsewhere = Repo {
        origin: "https://gitlab.com/acme/plugin-venue".into(),
        ..repo()
    };
    let refused = organisation(None, here, &elsewhere).unwrap_err();
    assert!(refused.contains("--organisation"), "{refused}");
    assert!(organisation(Some("Not A Slug"), here, &repo())
        .unwrap_err()
        .contains("not an organisation's namespace"));
}

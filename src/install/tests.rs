use std::sync::Arc;

use super::*;
use crate::fake_http::{serve, Asked, Reply};

// ── Names ────────────────────────────────────────────────────────────────

#[test]
fn a_listing_is_organisation_and_plugin_with_a_version_after_the_at() {
    assert_eq!(
        listing_named("open-meridian/coinbase@1.2.0").unwrap(),
        (
            "open-meridian/coinbase".to_string(),
            Some("1.2.0".to_string())
        )
    );
    assert_eq!(
        listing_named("open-meridian/coinbase@v1.2.0-rc.1")
            .unwrap()
            .1,
        Some("1.2.0-rc.1".to_string())
    );
    assert_eq!(
        listing_named("open-meridian/coinbase").unwrap(),
        ("open-meridian/coinbase".to_string(), None)
    );
    for wrong in [
        "coinbase",
        "open-meridian/coinbase@1.2",
        "open-meridian/Coinbase@1.2.0",
    ] {
        assert!(listing_named(wrong).is_err(), "{wrong}");
    }
}

#[test]
fn versions_compare_by_semver_precedence() {
    use std::cmp::Ordering::*;
    assert_eq!(compare("1.10.0", "1.9.0"), Some(Greater));
    assert_eq!(compare("1.0.0", "1.0.0-rc.1"), Some(Greater));
    assert_eq!(compare("1.0.0-rc.2", "1.0.0-rc.10"), Some(Less));
    assert_eq!(compare("1.0.0-alpha", "1.0.0-alpha.1"), Some(Less));
    assert_eq!(compare("1.0.0+build.1", "1.0.0"), Some(Equal));
    assert_eq!(compare("1.0", "1.0.0"), None);
}

// ── A synthetic catalogue, as PullCatalogueReply answers ─────────────────

fn version(listing: &str, version: &str, registry: &str) -> serde_json::Value {
    serde_json::json!({
        "listing": listing, "version": version, "registry": registry,
        "image_digest": "sha256:00", "withdrawn_at_ns": 0, "sequence": 1,
    })
}

fn withdrawn(mut v: serde_json::Value) -> serde_json::Value {
    v["withdrawn_at_ns"] = serde_json::json!("1792368000000000000");
    v
}

fn pulled(mut v: serde_json::Value, restored: bool) -> serde_json::Value {
    v["pull"] = serde_json::json!({
        "kind": "PULL_REASON_KIND_TERMS_BREACH", "reason": "terms",
        "pulled_at_ns": "1792368000000000000",
        "restored_at_ns": if restored { "1792454400000000000" } else { "0" },
    });
    v
}

/// The Crypto pack's members' listings: one whose newest is withdrawn, one
/// in both registries, one pulled, one withdrawn whole.
fn catalogue() -> serde_json::Value {
    let data = "open-meridian/plugin-cryptodata";
    let feed = "open-meridian/plugin-public-feed";
    let coinbase = "open-meridian/plugin-coinbase";
    let kraken = "open-meridian/plugin-kraken";
    let gone = "open-meridian/plugin-gone";
    serde_json::json!({
        "sequence": 40,
        "listings": [
            {"listing": data, "versions": [
                version(data, "0.4.0", "community"),
                version(data, "0.5.0-rc.1", "community"),
                withdrawn(version(data, "0.4.1", "community")),
            ]},
            {"listing": feed, "versions": [
                version(feed, "1.0.0", "community"),
                version(feed, "1.1.0", "community"),
            ]},
            {"listing": coinbase, "versions": [
                version(coinbase, "1.2.0", "verified"),
                version(coinbase, "1.3.0", "community"),
            ]},
            {"listing": kraken, "versions": [
                pulled(version(kraken, "0.9.0", "community"), false),
                pulled(version(kraken, "0.8.0", "community"), true),
            ]},
            {"listing": gone, "versions": [
                withdrawn(version(gone, "0.2.0", "community")),
            ]},
        ],
    })
}

fn nothing_installed() -> serde_json::Value {
    serde_json::json!({"versions": [], "launches": []})
}

#[test]
fn browses_latest_is_verified_first_then_community() {
    let catalogue = catalogue();
    assert_eq!(
        latest(&catalogue, "open-meridian/plugin-coinbase", None),
        Some(("1.2.0".into(), "verified".into()))
    );
    assert_eq!(
        latest(
            &catalogue,
            "open-meridian/plugin-coinbase",
            Some("community")
        ),
        Some(("1.3.0".into(), "community".into()))
    );
    assert_eq!(latest(&catalogue, "open-meridian/plugin-gone", None), None);
    assert_eq!(latest(&catalogue, "acme/unknown", None), None);
}

// ── Installing, against a stand-in deployment ────────────────────────────

/// A deployment's terminal routes: its catalogue, and its install, which
/// refuses `failing` as the conductor refuses a version it cannot verify.
async fn deployment(
    installed: serde_json::Value,
    failing: &'static str,
) -> (String, crate::fake_http::Kept) {
    let installed = Arc::new(installed);
    serve(
        move |asked: &Asked| match (asked.method.as_str(), asked.path.as_str()) {
            ("GET", "/terminal/plugins") => Reply::json(200, (*installed).clone()),
            ("POST", "/terminal/plugins/install") => {
                let body = asked.json();
                let listing = body["listing"].as_str().unwrap_or_default().to_string();
                if listing == failing {
                    return Reply::json(
                        409,
                        serde_json::json!({"error": "the listing statement does not verify"}),
                    );
                }
                let plugin = listing.split_once('/').map(|(_, p)| p).unwrap_or_default();
                Reply::json(
                    201,
                    serde_json::json!({
                        "name": plugin, "version": body["version"], "listing": listing,
                        "source_registry": body["registry"],
                    }),
                )
            }
            _ => Reply::json(404, serde_json::json!({"error": "no such route"})),
        },
    )
    .await
}

fn installs(kept: &crate::fake_http::Kept) -> Vec<serde_json::Value> {
    kept.lock()
        .unwrap()
        .iter()
        .filter(|a| a.method == "POST" && a.path == "/terminal/plugins/install")
        .map(Asked::json)
        .collect()
}

#[tokio::test]
async fn one_listed_version_is_installed_through_the_deployments_install() {
    let (address, kept) = deployment(nothing_installed(), "").await;
    let session = Credential::terminal_session(&address, "a-session");
    let recorded = install_one(
        &address,
        &session,
        "open-meridian/plugin-coinbase",
        "1.2.0",
        "verified",
        "trading desk",
    )
    .await
    .unwrap();
    assert_eq!(
        recorded,
        ("plugin-coinbase".to_string(), "1.2.0".to_string())
    );
    assert_eq!(
        checklist(&recorded.0, &recorded.1),
        "Installed; nothing is launched. Launch each, approving the roles it asks for:\n  \
         meridian plugin launch plugin-coinbase 1.2.0 --instance plugin-coinbase\n"
    );
    assert_eq!(
        installs(&kept),
        [
            serde_json::json!({"listing": "open-meridian/plugin-coinbase", "version": "1.2.0",
                            "registry": "verified", "note": "trading desk"})
        ]
    );
    let (address, _) = deployment(nothing_installed(), "open-meridian/plugin-coinbase").await;
    let session = Credential::terminal_session(&address, "a-session");
    let failed = install_one(
        &address,
        &session,
        "open-meridian/plugin-coinbase",
        "1.2.0",
        "",
        "n",
    )
    .await
    .unwrap_err();
    assert!(failed
        .said()
        .contains("the listing statement does not verify"));
}

#[tokio::test]
async fn the_catalogue_is_read_with_the_persons_bearer_and_an_older_platform_said_plainly() {
    let (platform, kept) = serve(|asked: &Asked| match asked.header("authorization") {
        Some("Bearer A-1") => Reply::json(200, catalogue()),
        _ => Reply::json(401, serde_json::json!({"error": "not authenticated"})),
    })
    .await;
    let read = catalogue_at(&platform, "A-1").await.unwrap();
    assert_eq!(
        read["listings"][0]["listing"],
        "open-meridian/plugin-cryptodata"
    );
    assert_eq!(kept.lock().unwrap()[0].path, CATALOGUE_PATH);
    // A platform taking only a deployment's assertion there.
    assert_eq!(
        catalogue_at(&platform, "A-old").await.unwrap_err(),
        format!("{platform}: this platform does not yet let the CLI read its catalogue")
    );
}

use std::sync::{Arc, Mutex};

use super::*;
use crate::fake_http::{serve, Asked, Reply};

// ── Names ────────────────────────────────────────────────────────────────

#[test]
fn a_bare_pack_name_is_open_meridians() {
    assert_eq!(pack_name("crypto").unwrap(), "open-meridian/crypto");
    assert_eq!(pack_name("acme/fx-desk").unwrap(), "acme/fx-desk");
    for wrong in ["", "Crypto", "acme/", "/crypto", "a/b/c", "acme/fx_desk"] {
        assert!(pack_name(wrong).is_err(), "{wrong}");
    }
}

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

fn member(
    listing: &str,
    registry: &str,
    selection: &str,
    group: &str,
    minimum: &str,
) -> serde_json::Value {
    serde_json::json!({
        "listing": listing, "registry": registry,
        "selection": format!("PACK_SELECTION_{selection}"), "group": group,
        "minimum_version": minimum,
    })
}

/// The Crypto pack and its members' listings: data preselected, venues to
/// choose, one data member whose newest is withdrawn, one venue pulled.
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
        "packs": [
            {
                "pack": "open-meridian/crypto", "curator": "Open Meridian", "sequence": 39,
                "withdrawn_at_ns": 0,
                "revision": {
                    "revision": 2, "title": "Crypto", "description": "Crypto data and venues",
                    "asset_classes": ["crypto"], "note": "first", "published_at_ns": 1,
                    "members": [
                        member(data, "community", "PRESELECTED", "Data", "0.3.0"),
                        member(feed, "community", "PRESELECTED", "Data", "1.0.0"),
                        member(coinbase, "verified", "CHOOSE", "Venues", "1.0.0"),
                        member(kraken, "community", "CHOOSE", "Venues", "0.8.0"),
                        member(gone, "community", "CHOOSE", "Venues", "0.1.0"),
                    ],
                },
            },
            {
                "pack": "open-meridian/old", "curator": "Open Meridian", "sequence": 12,
                "withdrawn_at_ns": "1792368000000000000",
                "revision": {"revision": 1, "title": "Old", "members": []},
            },
            {
                "pack": "open-meridian/needs-curation", "curator": "Open Meridian",
                "sequence": 13, "withdrawn_at_ns": 0,
                "revision": {"revision": 1, "title": "Needs curation", "members": [
                    member(gone, "community", "REQUIRED", "Data", "0.1.0"),
                ]},
            },
        ],
    })
}

fn crypto() -> Pack {
    pack_of(&catalogue(), "open-meridian/crypto").unwrap()
}

fn nothing_installed() -> serde_json::Value {
    serde_json::json!({"versions": [], "launches": []})
}

fn picks<'a>(with: &'a [String], without: &'a [String]) -> Picks<'a> {
    Picks {
        with,
        without,
        ask: None,
    }
}

#[test]
fn a_member_resolves_to_its_latest_offered_release_at_or_above_its_minimum() {
    let catalogue = catalogue();
    let pack = crypto();
    let resolved: Vec<Resolved> = pack
        .members
        .iter()
        .map(|m| resolve(&catalogue, m))
        .collect();
    assert_eq!(
        resolved,
        [
            // 0.4.1 withdrawn, and the pre-release never chosen over a release.
            Resolved::Version("0.4.0".into()),
            Resolved::Version("1.1.0".into()),
            // Its own registry: verified's 1.2.0, not community's 1.3.0.
            Resolved::Version("1.2.0".into()),
            // 0.9.0's pull is open; 0.8.0's was restored.
            Resolved::Version("0.8.0".into()),
            Resolved::Unavailable("withdrawn"),
        ]
    );
    // A minimum above everything listed: none listed at or above it.
    let mut high = pack.members[1].clone();
    high.minimum_version = "2.0.0".into();
    assert_eq!(
        resolve(&catalogue, &high),
        Resolved::Unavailable("none listed")
    );
    // Every version at or above it pulled.
    let mut floor = pack.members[3].clone();
    floor.minimum_version = "0.9.0".into();
    assert_eq!(resolve(&catalogue, &floor), Resolved::Unavailable("pulled"));
    // A pre-release only where no release is.
    let mut only_pre = pack.members[0].clone();
    only_pre.minimum_version = "0.5.0-rc.1".into();
    assert_eq!(
        resolve(&catalogue, &only_pre),
        Resolved::Version("0.5.0-rc.1".into())
    );
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

#[test]
fn a_withdrawn_pack_or_one_not_in_the_catalogue_is_refused() {
    let catalogue = catalogue();
    assert!(pack_of(&catalogue, "open-meridian/old")
        .unwrap_err()
        .contains("withdrawn by its curator"));
    assert!(pack_of(&catalogue, "open-meridian/equities")
        .unwrap_err()
        .contains("holds no starter pack open-meridian/equities"));
    let pack = crypto();
    assert_eq!(pack.revision, 2);
    assert_eq!(pack.curator, "Open Meridian");
    assert_eq!(pack.members[2].selection, Selection::Choose);
}

#[test]
fn preselected_members_are_ticked_and_choose_members_only_when_named() {
    let catalogue = catalogue();
    let pack = crypto();
    let rows = plan(&pack, &catalogue, &nothing_installed(), &picks(&[], &[])).unwrap();
    assert_eq!(
        to_install(&rows),
        [
            (
                "open-meridian/plugin-cryptodata".into(),
                "0.4.0".into(),
                "community".into()
            ),
            (
                "open-meridian/plugin-public-feed".into(),
                "1.1.0".into(),
                "community".into()
            ),
        ]
    );
    let text = plan_text(&pack, &rows, "https://meridian.localhost");
    assert!(
        text.starts_with(
            "open-meridian/crypto (revision 2, curated by Open Meridian): Crypto, into \
             https://meridian.localhost\n  Data\n    install      \
             open-meridian/plugin-cryptodata 0.4.0 (community)\n"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "    not chosen   open-meridian/plugin-coinbase 1.2.0 (verified); --with \
             open-meridian/plugin-coinbase installs it\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("    unavailable  open-meridian/plugin-gone: Unavailable (withdrawn)\n"),
        "{text}"
    );

    // --with by listing or by plugin part; --without a preselected one.
    let with = ["plugin-coinbase".to_string()];
    let without = ["open-meridian/plugin-public-feed".to_string()];
    let rows = plan(
        &pack,
        &catalogue,
        &nothing_installed(),
        &picks(&with, &without),
    )
    .unwrap();
    assert_eq!(
        to_install(&rows)
            .into_iter()
            .map(|(listing, version, _)| format!("{listing} {version}"))
            .collect::<Vec<_>>(),
        [
            "open-meridian/plugin-cryptodata 0.4.0",
            "open-meridian/plugin-coinbase 1.2.0"
        ]
    );

    // Asked at a terminal: each unnamed, available choose member.
    let asked = Mutex::new(Vec::new());
    let ask = |member: &Member, version: &str| {
        asked
            .lock()
            .unwrap()
            .push(format!("{} {version}", member.listing));
        member.listing.ends_with("kraken")
    };
    let rows = plan(
        &pack,
        &catalogue,
        &nothing_installed(),
        &Picks {
            with: &with,
            without: &[],
            ask: Some(&ask),
        },
    )
    .unwrap();
    assert_eq!(
        *asked.lock().unwrap(),
        ["open-meridian/plugin-kraken 0.8.0"],
        "neither the named one nor the unavailable one is asked about"
    );
    assert_eq!(to_install(&rows).len(), 4);
}

#[test]
fn a_name_that_is_no_member_is_refused_before_anything_moves() {
    let catalogue = catalogue();
    let pack = crypto();
    let refused = |with: &[String], without: &[String]| {
        plan(
            &pack,
            &catalogue,
            &nothing_installed(),
            &picks(with, without),
        )
        .unwrap_err()
    };
    assert!(refused(&["plugin-tradier".into()], &[]).starts_with(
        "plugin-tradier is not a member of open-meridian/crypto: its members are \
         open-meridian/plugin-cryptodata, "
    ));
    assert!(refused(&[], &["acme/plugin-coinbase".into()]).contains("is not a member"));
    assert!(refused(&["plugin-gone".into()], &[])
        .contains("open-meridian/plugin-gone is unavailable (withdrawn)"));
    assert!(
        refused(&["plugin-coinbase".into()], &["plugin-coinbase".into()])
            .contains("named by both --with and --without")
    );

    // A required member: never left out, and a pack missing one not offered.
    let needs = pack_of(&catalogue, "open-meridian/needs-curation").unwrap();
    assert!(
        plan(&needs, &catalogue, &nothing_installed(), &picks(&[], &[]))
            .unwrap_err()
            .contains(
                "is not offered: its required member open-meridian/plugin-gone is unavailable"
            )
    );
    let mut required = crypto();
    required.members[0].selection = Selection::Required;
    assert!(plan(
        &required,
        &catalogue,
        &nothing_installed(),
        &picks(&[], &["plugin-cryptodata".into()])
    )
    .unwrap_err()
    .contains("is required in open-meridian/crypto"));
}

#[test]
fn a_member_already_installed_is_never_moved() {
    let catalogue = catalogue();
    let pack = crypto();
    let installed = serde_json::json!({"versions": [
        {"name": "plugin-cryptodata", "version": "0.3.0",
         "listing": "open-meridian/plugin-cryptodata", "source_registry": "community"},
        {"name": "plugin-public-feed", "version": "1.1.0",
         "listing": "open-meridian/plugin-public-feed", "source_registry": "community"},
        // A local version of the same name is not the listing's.
        {"name": "plugin-coinbase", "version": "9.0.0", "listing": ""},
    ], "launches": []});
    let with = ["plugin-coinbase".to_string()];
    let rows = plan(&pack, &catalogue, &installed, &picks(&with, &[])).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.outcome.clone()).collect::<Vec<_>>(),
        [
            Outcome::Below {
                held: "0.3.0".into(),
                offered: "0.4.0".into()
            },
            Outcome::Installed {
                held: "1.1.0".into()
            },
            Outcome::Install("1.2.0".into()),
            Outcome::NotChosen("0.8.0".into()),
            Outcome::Unavailable("withdrawn"),
        ]
    );
    let text = plan_text(&pack, &rows, "https://d");
    assert!(
        text.contains(
            "    kept         open-meridian/plugin-cryptodata: Installed 0.3.0; 0.4.0 offered\n"
        ),
        "{text}"
    );
    assert!(text.contains("    installed    open-meridian/plugin-public-feed 1.1.0\n"));
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
async fn a_pack_installs_each_ticked_member_and_goes_on_past_a_failure() {
    let (address, kept) = deployment(nothing_installed(), "open-meridian/plugin-public-feed").await;
    let session = Credential::terminal_session(&address, "a-session");
    let with = ["plugin-coinbase".to_string()];
    let asked = Mutex::new(Vec::new());
    let approve = |question: &str| {
        asked.lock().unwrap().push(question.to_string());
        true
    };
    let mut said = Vec::new();
    let done = install_pack(
        &address,
        &session,
        &catalogue(),
        &PackAsked {
            pack: "open-meridian/crypto",
            picks: picks(&with, &[]),
            note: "the crypto desk",
        },
        &approve,
        &mut |line| said.push(line.to_string()),
    )
    .await
    .unwrap();
    assert_eq!(*asked.lock().unwrap(), ["Install these 3 members?"]);
    assert_eq!(
        installs(&kept),
        [
            serde_json::json!({"listing": "open-meridian/plugin-cryptodata", "version": "0.4.0",
                               "registry": "community", "note": "the crypto desk"}),
            serde_json::json!({"listing": "open-meridian/plugin-public-feed", "version": "1.1.0",
                               "registry": "community", "note": "the crypto desk"}),
            serde_json::json!({"listing": "open-meridian/plugin-coinbase", "version": "1.2.0",
                               "registry": "verified", "note": "the crypto desk"}),
        ]
    );
    // One line a member, the failure said and the rest installed.
    let lines: Vec<&String> = said
        .iter()
        .filter(|l| l.starts_with("  open-meridian/"))
        .collect();
    assert_eq!(
        lines,
        [
            "  open-meridian/plugin-cryptodata 0.4.0: installed",
            "  open-meridian/plugin-public-feed 1.1.0: failed: 409 Conflict: the listing \
             statement does not verify",
            "  open-meridian/plugin-coinbase 1.2.0: installed",
        ]
    );
    assert_eq!(
        done.failed,
        [(
            "open-meridian/plugin-public-feed".to_string(),
            "409 Conflict: the listing statement does not verify".to_string()
        )]
    );
    let checklist = checklist(&done);
    assert_eq!(
        checklist,
        "Installed; nothing is launched. Launch each, approving the roles it asks for:\n  \
         meridian plugin launch plugin-cryptodata 0.4.0 --instance plugin-cryptodata\n  \
         meridian plugin launch plugin-coinbase 1.2.0 --instance plugin-coinbase\n\
         Not installed:\n  open-meridian/plugin-public-feed: 409 Conflict: the listing \
         statement does not verify\n"
    );
}

#[tokio::test]
async fn nothing_is_installed_unless_approved() {
    let (address, kept) = deployment(nothing_installed(), "").await;
    let session = Credential::terminal_session(&address, "a-session");
    let failed = install_pack(
        &address,
        &session,
        &catalogue(),
        &PackAsked {
            pack: "open-meridian/crypto",
            picks: picks(&[], &[]),
            note: "why",
        },
        &|_: &str| false,
        &mut |_| {},
    )
    .await
    .unwrap_err();
    assert!(failed
        .said()
        .starts_with("not approved, so nothing was installed"));
    assert!(installs(&kept).is_empty());

    // A non-member refused before anything is asked or installed.
    let asked = Mutex::new(0);
    let failed = install_pack(
        &address,
        &session,
        &catalogue(),
        &PackAsked {
            pack: "open-meridian/crypto",
            picks: picks(&["plugin-tradier".into()], &[]),
            note: "why",
        },
        &|_: &str| {
            *asked.lock().unwrap() += 1;
            true
        },
        &mut |_| {},
    )
    .await
    .unwrap_err();
    assert!(failed
        .said()
        .contains("is not a member of open-meridian/crypto"));
    assert_eq!(*asked.lock().unwrap(), 0);
    assert!(installs(&kept).is_empty());
}

#[tokio::test]
async fn a_pack_held_whole_installs_nothing_and_lists_each_launch() {
    let installed = serde_json::json!({"versions": [
        {"name": "plugin-cryptodata", "version": "0.4.0",
         "listing": "open-meridian/plugin-cryptodata"},
        {"name": "plugin-public-feed", "version": "1.2.0",
         "listing": "open-meridian/plugin-public-feed"},
    ], "launches": []});
    let (address, kept) = deployment(installed, "").await;
    let session = Credential::terminal_session(&address, "a-session");
    let mut said = Vec::new();
    let done = install_pack(
        &address,
        &session,
        &catalogue(),
        &PackAsked {
            pack: "open-meridian/crypto",
            picks: picks(&[], &[]),
            note: "why",
        },
        &|_: &str| panic!("nothing to approve"),
        &mut |line| said.push(line.to_string()),
    )
    .await
    .unwrap();
    assert!(
        said.contains(&"Nothing to install.".to_string()),
        "{said:?}"
    );
    assert!(installs(&kept).is_empty());
    assert_eq!(
        done.ready,
        [
            ("plugin-cryptodata".to_string(), "0.4.0".to_string()),
            ("plugin-public-feed".to_string(), "1.2.0".to_string()),
        ]
    );
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

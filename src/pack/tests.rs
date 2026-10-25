use std::sync::{Arc, Mutex};

use super::*;
use crate::fake_http::{serve, Asked, Reply};

const DATA: &str = "open-meridian/plugin-cryptodata";
const FEED: &str = "open-meridian/plugin-public-feed";
const COINBASE: &str = "open-meridian/plugin-coinbase";
const KRAKEN: &str = "open-meridian/plugin-kraken";
const GONE: &str = "open-meridian/plugin-gone";
const NEWCOMER: &str = "open-meridian/plugin-newcomer";

// ── Names ────────────────────────────────────────────────────────────────

#[test]
fn a_bare_pack_name_is_open_meridians() {
    assert_eq!(pack_name("crypto").unwrap(), "open-meridian/crypto");
    assert_eq!(pack_name("acme/fx-desk").unwrap(), "acme/fx-desk");
    for wrong in ["", "Crypto", "acme/", "/crypto", "a/b/c", "acme/fx_desk"] {
        assert!(pack_name(wrong).is_err(), "{wrong}");
    }
}

// ── Core's answers, as its terminal routes write them ───────────────────

fn listed(listing: &str, registry: &str, selection: &str, group: &str) -> serde_json::Value {
    serde_json::json!({
        "listing": listing, "registry": registry, "selection": selection, "group": group,
        "minimum_version": "0.1.0",
    })
}

/// `GET /terminal/packs`: the Crypto pack (data preselected, venues to
/// choose), one withdrawn, and one whose required member is unavailable.
fn packs() -> serde_json::Value {
    serde_json::json!({
        "packs": [
            {
                "pack": "open-meridian/crypto", "curator": "Open Meridian", "revision": 2,
                "title": "Crypto", "description": "Crypto data and venues",
                "asset_classes": ["crypto"],
                "members": [
                    listed(DATA, "community", "preselected", "Data"),
                    listed(FEED, "community", "preselected", "Data"),
                    listed(COINBASE, "verified", "choose", "Venues"),
                    listed(KRAKEN, "community", "choose", "Venues"),
                    listed(GONE, "community", "choose", "Venues"),
                ],
                "withdrawn_at_ns": 0, "state": "not_installed", "said": "Not installed",
                "installed_revision": 0,
            },
            {
                "pack": "open-meridian/old", "curator": "Open Meridian", "revision": 1,
                "title": "Old", "description": "", "asset_classes": [], "members": [],
                "withdrawn_at_ns": 1792368000000000000_u64, "state": "installed",
                "said": "Installed: 0 of 0 members", "installed_revision": 1,
            },
            {
                "pack": "open-meridian/needs-curation", "curator": "Open Meridian",
                "revision": 1, "title": "Needs curation", "description": "",
                "asset_classes": ["equity"],
                "members": [listed(GONE, "community", "required", "Data")],
                "withdrawn_at_ns": 0, "state": "not_installed", "said": "Not installed",
                "installed_revision": 0,
            },
        ],
        "catalogue_pulled_at_ns": 1792368000000000000_u64,
    })
}

fn member(
    listing: &str,
    group: &str,
    state: &str,
    installed: &str,
    latest: &str,
    reason: &str,
) -> serde_json::Value {
    serde_json::json!({
        "listing": listing, "group": group, "installed_version": installed,
        "latest_version": latest, "state": state, "reason": reason, "instance_ids": [],
        "setup": if state == "installed" { "Launch to connect" } else { "" },
        "added_by_revision": 0, "removed_by_revision": 0,
    })
}

/// `GET /terminal/packs/open-meridian/crypto` before any install: each
/// member as the install sheet shows it.
fn crypto_here() -> serde_json::Value {
    serde_json::json!({
        "pack": "open-meridian/crypto", "installed_revision": 0, "latest_revision": 2,
        "withdrawn": false, "state": "Not installed",
        "members": [
            member(DATA, "Data", "not_installed", "", "0.4.0", ""),
            member(FEED, "Data", "not_installed", "", "1.1.0", ""),
            member(COINBASE, "Venues", "not_installed", "", "1.2.0", ""),
            member(KRAKEN, "Venues", "not_permitted", "", "0.8.0",
                   "its floor is contract v29, and this deployment is at v28"),
            member(GONE, "Venues", "unavailable", "", "", "withdrawn"),
        ],
    })
}

fn crypto() -> Offered {
    offered(&packs(), "open-meridian/crypto").unwrap()
}

fn picks<'a>(with: &'a [String], without: &'a [String]) -> Picks<'a> {
    Picks {
        with,
        without,
        ask: None,
    }
}

// ── The pack and the plan ────────────────────────────────────────────────

#[test]
fn a_withdrawn_pack_or_one_not_in_the_catalogue_is_refused() {
    assert!(offered(&packs(), "open-meridian/old")
        .unwrap_err()
        .contains("withdrawn by its curator"));
    assert_eq!(
        offered(&packs(), "open-meridian/equities").unwrap_err(),
        "no starter pack open-meridian/equities is offered in the catalogue"
    );
    let mut odd = packs();
    odd["packs"][0]["members"][0]["selection"] = serde_json::json!("unspecified");
    assert!(offered(&odd, "open-meridian/crypto")
        .unwrap_err()
        .contains("a selection this CLI does not know (unspecified)"));
    let pack = crypto();
    assert_eq!(pack.revision, 2);
    assert_eq!(pack.members[2].selection, Selection::Choose);
}

#[test]
fn preselected_members_are_ticked_and_choose_members_only_when_named_or_asked() {
    let pack = crypto();
    let rows = plan(&pack, &crypto_here(), &picks(&[], &[])).unwrap();
    let outcomes: Vec<&Outcome> = rows.iter().map(|r| &r.outcome).collect();
    assert_eq!(
        outcomes,
        [
            &Outcome::Install("0.4.0".into()),
            &Outcome::Install("1.1.0".into()),
            &Outcome::NotChosen("1.2.0".into()),
            &Outcome::NotPermitted(
                "its floor is contract v29, and this deployment is at v28".into()
            ),
            &Outcome::Unavailable("withdrawn".into()),
        ]
    );
    // --with by plugin part, --without a preselected one.
    let rows = plan(
        &pack,
        &crypto_here(),
        &picks(&["plugin-coinbase".into()], &[FEED.into()]),
    )
    .unwrap();
    assert_eq!(rows[1].outcome, Outcome::NotChosen("1.1.0".into()));
    assert_eq!(rows[2].outcome, Outcome::Install("1.2.0".into()));
    // Asked at a terminal: only offered, unnamed choose members are asked.
    let asked = Mutex::new(Vec::new());
    let ask = |m: &Member, version: &str| {
        asked
            .lock()
            .unwrap()
            .push(format!("{} {version}", m.listing));
        true
    };
    let rows = plan(
        &pack,
        &crypto_here(),
        &Picks {
            with: &[],
            without: &[],
            ask: Some(&ask),
        },
    )
    .unwrap();
    assert_eq!(*asked.lock().unwrap(), [format!("{COINBASE} 1.2.0")]);
    assert_eq!(rows[2].outcome, Outcome::Install("1.2.0".into()));
    assert_eq!(
        plan_text(&pack, &rows, "https://m.test"),
        format!(
            "open-meridian/crypto (revision 2, curated by Open Meridian): Crypto, into \
             https://m.test\n  Data\n    install        {DATA} 0.4.0 (community)\n    \
             install        {FEED} 1.1.0 (community)\n  Venues\n    install        \
             {COINBASE} 1.2.0 (verified)\n    not permitted  {KRAKEN}: Not permitted here \
             (its floor is contract v29, and this deployment is at v28)\n    unavailable    \
             {GONE}: Unavailable (withdrawn)\n"
        )
    );
}

#[test]
fn what_cannot_be_installed_is_refused_before_anything_moves() {
    let pack = crypto();
    let here = crypto_here();
    let refused = |with: &[String], without: &[String]| {
        plan(&pack, &here, &picks(with, without)).unwrap_err()
    };
    assert!(refused(&["plugin-tradier".into()], &[])
        .starts_with("plugin-tradier is not a member of open-meridian/crypto: its members are"));
    assert_eq!(
        refused(&[KRAKEN.into()], &[]),
        format!(
            "{KRAKEN} is not permitted here (its floor is contract v29, and this deployment is \
             at v28), so it cannot be installed"
        )
    );
    assert_eq!(
        refused(&[GONE.into()], &[]),
        format!("{GONE} is unavailable (withdrawn), so it cannot be installed")
    );
    assert_eq!(
        refused(&[COINBASE.into()], &[COINBASE.into()]),
        format!("{COINBASE} is named by both --with and --without")
    );
    // A required member is never left out, nor installed without.
    let needs = offered(&packs(), "open-meridian/needs-curation").unwrap();
    let here =
        serde_json::json!({"members": [member(GONE, "Data", "unavailable", "", "", "withdrawn")]});
    assert_eq!(
        plan(&needs, &here, &picks(&[], &[])).unwrap_err(),
        format!(
            "open-meridian/needs-curation is not installed: its required member {GONE} is \
             unavailable (withdrawn)"
        )
    );
    assert!(plan(&needs, &here, &picks(&[], &[GONE.into()]))
        .unwrap_err()
        .contains("is required in open-meridian/needs-curation"));
}

#[test]
fn the_picks_go_to_core_as_with_and_without() {
    // A preselected member not to be had here goes as `without`, so core
    // never ticks it; a choose member answered yes goes as `with`.
    let mut here = crypto_here();
    here["members"][1] = member(FEED, "Data", "unavailable", "", "", "withdrawn");
    let rows = plan(&crypto(), &here, &picks(&[COINBASE.into()], &[DATA.into()])).unwrap();
    assert_eq!(
        with_and_without(&rows),
        (
            vec![COINBASE.to_string()],
            vec![DATA.to_string(), FEED.to_string()]
        )
    );
}

#[test]
fn a_member_installed_already_is_never_moved() {
    let mut here = crypto_here();
    here["members"][0] = member(
        DATA,
        "Data",
        "launched",
        "0.3.0",
        "0.4.0",
        "Installed 0.3.0; 0.4.0 offered",
    );
    here["members"][1] = member(FEED, "Data", "installed", "1.2.0", "1.1.0", "");
    let rows = plan(&crypto(), &here, &picks(&[], &[])).unwrap();
    assert_eq!(
        rows[0].outcome,
        Outcome::Below {
            held: "0.3.0".into(),
            offered: "0.4.0".into()
        }
    );
    assert_eq!(
        rows[1].outcome,
        Outcome::Installed {
            held: "1.2.0".into()
        }
    );
    let text = plan_text(&crypto(), &rows, "https://m.test");
    assert!(text.contains(&format!(
        "    kept           {DATA}: Installed 0.3.0; 0.4.0 offered\n"
    )));
    assert!(text.contains(&format!("    installed      {FEED} 1.2.0\n")));
}

// ── Against a stand-in deployment ────────────────────────────────────────

/// A deployment's pack routes, as core answers them: the install a job
/// that answers at once, each queued member "installing", and that settles
/// over the next two reads of the pack's page, `failing` failed with why.
struct Stand {
    installed: Option<Vec<String>>,
    reads: u32,
}

async fn deployment(failing: &'static str) -> (String, crate::fake_http::Kept) {
    let stand = Arc::new(Mutex::new(Stand {
        installed: None,
        reads: 0,
    }));
    serve(move |asked: &Asked| {
        let mut stand = stand.lock().unwrap();
        match (asked.method.as_str(), asked.path.as_str()) {
            ("GET", "/terminal/packs") => Reply::json(200, packs()),
            ("GET", "/terminal/packs/open-meridian/crypto") => {
                let Some(ticked) = stand.installed.clone() else {
                    return Reply::json(200, crypto_here());
                };
                stand.reads += 1;
                let mut here = crypto_here();
                here["installed_revision"] = serde_json::json!(2);
                for (n, held) in here["members"].as_array_mut().unwrap().iter_mut().enumerate() {
                    let listing = held["listing"].as_str().unwrap().to_string();
                    if !ticked.contains(&listing) {
                        continue;
                    }
                    let latest = held["latest_version"].as_str().unwrap().to_string();
                    // One member settles a read: the first at once.
                    *held = if n as u32 >= stand.reads {
                        member(&listing, "", "installing", "", &latest, "")
                    } else if listing == failing {
                        member(&listing, "", "failed", "", &latest, "verification failed: the listing statement does not verify")
                    } else {
                        member(&listing, "", "installed", &latest, &latest, "")
                    };
                }
                Reply::json(200, here)
            }
            ("GET", "/terminal/packs/open-meridian/missing") => Reply::json(
                404,
                serde_json::json!({"error": "no starter pack open-meridian/missing is in the catalogue this deployment holds"}),
            ),
            ("POST", "/terminal/packs/install") => {
                // Core's ticking: the preselected, less `without`, with `with`.
                let body = asked.json();
                let named = |key: &str| -> Vec<String> {
                    body[key]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|m| m.as_str().unwrap().to_string())
                        .collect()
                };
                let (with, without) = (named("with"), named("without"));
                let ticked: Vec<String> = [DATA, FEED, COINBASE, KRAKEN, GONE]
                    .iter()
                    .map(|l| l.to_string())
                    .filter(|l| {
                        ([DATA, FEED].contains(&l.as_str()) && !without.contains(l))
                            || with.contains(l)
                    })
                    .collect();
                stand.installed = Some(ticked.clone());
                Reply::json(
                    201,
                    serde_json::json!({
                        "pack": body["pack"], "revision": body["revision"],
                        "members": ticked.iter().map(|l| member(l, "", "installing", "", "", "")).collect::<Vec<_>>(),
                    }),
                )
            }
            ("POST", "/terminal/packs/update") | ("POST", "/terminal/packs/remove") => {
                let mut here = crypto_here();
                here["installed_revision"] = serde_json::json!(2);
                here["state"] = serde_json::json!("Installed: 0 of 3 members");
                Reply::json(200, here)
            }
            ("GET", "/terminal/plugins") => Reply::json(
                200,
                serde_json::json!({"versions": [
                    {"name": "cryptodata", "version": "0.4.0", "listing": DATA},
                ], "launches": []}),
            ),
            _ => Reply::bytes(404, "text/plain", Vec::new()),
        }
    })
    .await
}

fn posted(kept: &crate::fake_http::Kept, path: &str) -> Vec<serde_json::Value> {
    kept.lock()
        .unwrap()
        .iter()
        .filter(|a| a.method == "POST" && a.path == path)
        .map(Asked::json)
        .collect()
}

const QUICK: Following = Following {
    every: Duration::from_millis(1),
    within: Duration::from_secs(10),
};

#[tokio::test]
async fn a_pack_is_one_install_through_core_followed_member_by_member() {
    let (address, kept) = deployment(FEED).await;
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
        &PackAsked {
            pack: "open-meridian/crypto",
            picks: picks(&with, &[]),
            note: "the crypto desk",
        },
        &approve,
        &mut |line| said.push(line.to_string()),
        &QUICK,
    )
    .await
    .unwrap();
    assert_eq!(*asked.lock().unwrap(), ["Install these 3 members?"]);
    // One install, the picks as core takes them, at the revision read.
    assert_eq!(
        posted(&kept, "/terminal/packs/install"),
        [serde_json::json!({
            "pack": "open-meridian/crypto", "with": [COINBASE], "without": [],
            "revision": 2, "note": "the crypto desk",
        })]
    );
    assert!(posted(&kept, "/terminal/plugins/install").is_empty());
    // One line a member, as each ends.
    let lines: Vec<&String> = said
        .iter()
        .filter(|l| l.starts_with("  open-meridian/"))
        .collect();
    assert_eq!(
        lines,
        [
            &format!("  {DATA} 0.4.0: installed"),
            &format!(
                "  {FEED}: failed: verification failed: the listing statement does not verify"
            ),
            &format!("  {COINBASE} 1.2.0: installed"),
        ]
    );
    assert!(done.failed);
    assert_eq!(
        done.checklist,
        format!(
            "Installed; nothing is launched. Launch each, approving the roles it asks for:\n  \
             meridian plugin launch cryptodata 0.4.0 --instance cryptodata\n  \
             meridian plugin launch plugin-coinbase 1.2.0 --instance plugin-coinbase\n\
             Not installed:\n  {FEED}: verification failed: the listing statement does not \
             verify\n"
        )
    );
}

#[tokio::test]
async fn nothing_is_installed_unless_approved_or_when_a_name_is_no_member() {
    let (address, kept) = deployment("").await;
    let session = Credential::terminal_session(&address, "a-session");
    let failed = install_pack(
        &address,
        &session,
        &PackAsked {
            pack: "open-meridian/crypto",
            picks: picks(&[], &[]),
            note: "why",
        },
        &|_: &str| false,
        &mut |_| {},
        &QUICK,
    )
    .await
    .unwrap_err();
    assert!(failed
        .said()
        .starts_with("not approved, so nothing was installed"));
    let asked = Mutex::new(0);
    let failed = install_pack(
        &address,
        &session,
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
        &QUICK,
    )
    .await
    .unwrap_err();
    assert!(failed
        .said()
        .contains("is not a member of open-meridian/crypto"));
    assert_eq!(*asked.lock().unwrap(), 0);
    assert!(posted(&kept, "/terminal/packs/install").is_empty());
}

#[tokio::test]
async fn an_older_core_is_said_plainly_and_a_pack_it_lacks_by_its_words() {
    let (address, _) = serve(|_: &Asked| Reply::bytes(404, "text/plain", Vec::new())).await;
    let session = Credential::terminal_session(&address, "a-session");
    assert_eq!(
        list(&address, &session).await.unwrap_err(),
        Failed::Refused(format!("{address} {NOT_SERVED}"))
    );
    let (address, _) = deployment("").await;
    let session = Credential::terminal_session(&address, "a-session");
    assert_eq!(
        read(&address, &session, "open-meridian/missing")
            .await
            .unwrap_err(),
        Failed::Refused(
            "404 Not Found: no starter pack open-meridian/missing is in the catalogue this \
             deployment holds"
                .into()
        )
    );
}

#[tokio::test]
async fn an_install_still_running_is_left_running_and_said() {
    let (address, _) = serve(|_: &Asked| {
        Reply::json(
            200,
            serde_json::json!({"members": [member(COINBASE, "", "installing", "", "", "")]}),
        )
    })
    .await;
    let session = Credential::terminal_session(&address, "a-session");
    let failed = follow(
        &address,
        &session,
        "open-meridian/crypto",
        &serde_json::json!({"members": [member(COINBASE, "", "installing", "", "", "")]}),
        Duration::from_millis(1),
        Duration::ZERO,
        &mut |_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(
        failed.said(),
        format!(
            "open-meridian/crypto is still installing ({COINBASE} to go) after 0 minutes; it \
             goes on, and `meridian plugin pack show open-meridian/crypto` shows where it is"
        )
    );
}

// ── Reading, updating and removing ───────────────────────────────────────

#[test]
fn pack_list_is_one_line_a_pack() {
    assert_eq!(
        list_text(&packs()),
        "open-meridian/crypto  Crypto  crypto  5 members  Not installed\n\
         open-meridian/old  Old  0 members  Installed: 0 of 0 members  Withdrawn by its curator\n\
         open-meridian/needs-curation  Needs curation  equity  1 member  Not installed\n"
    );
    assert_eq!(
        list_text(&serde_json::json!({"packs": []})),
        "No starter pack is offered in the catalogue.\n"
    );
}

#[test]
fn pack_show_is_the_checklist() {
    let mut here = crypto_here();
    here["installed_revision"] = serde_json::json!(1);
    here["state"] = serde_json::json!("Installed: 2 of 5 members");
    here["members"][0] = member(DATA, "Data", "installed", "0.4.0", "0.4.0", "");
    here["members"][1] = member(
        FEED,
        "Data",
        "launched",
        "1.0.0",
        "1.1.0",
        "Installed 1.0.0; 1.1.0 offered",
    );
    here["members"][1]["setup"] = serde_json::json!("Set up");
    here["members"][1]["setup_href"] = serde_json::json!("/admin/plugins/i/feed");
    here["members"][2] = member(COINBASE, "Venues", "failed", "", "1.2.0", "timeout");
    here["members"][2]["added_by_revision"] = serde_json::json!(2);
    here["members"].as_array_mut().unwrap().push({
        let mut m = member(NEWCOMER, "Venues", "not_installed", "", "0.2.0", "");
        m["added_by_revision"] = serde_json::json!(2);
        m
    });
    let installed = serde_json::json!({"versions": [
        {"name": "cryptodata", "version": "0.4.0", "listing": DATA},
    ]});
    assert_eq!(
        show_text(&here, &installed),
        format!(
            "open-meridian/crypto: Installed: 2 of 5 members\n\
             Installed at revision 1; revision 2 is in the catalogue.\n  \
             Data\n    \
             {DATA}  0.4.0  Installed  Launch to connect\n    \
             {FEED}  1.0.0 · 1.1.0  Launched: Installed 1.0.0; 1.1.0 offered  Set up (/admin/plugins/i/feed)\n  \
             Venues\n    \
             {COINBASE}  1.2.0  Failed: timeout\n    \
             {KRAKEN}  0.8.0  Not permitted here: its floor is contract v29, and this deployment is at v28\n    \
             {GONE}  Unavailable: withdrawn\n    \
             {NEWCOMER}  0.2.0  Not installed\n\
             Added to this pack: {NEWCOMER}\n\
             Installed; nothing is launched. Launch each, approving the roles it asks for:\n  \
             meridian plugin launch cryptodata 0.4.0 --instance cryptodata\n"
        )
    );
    assert_eq!(
        to_update(&here),
        [format!("  {FEED}: Installed 1.0.0; 1.1.0 offered")]
    );
    assert_eq!(
        staying(&here),
        [
            format!("  {DATA} · Installed"),
            format!("  {FEED} · Launched")
        ]
    );
}

#[tokio::test]
async fn update_and_remove_send_the_pack_and_the_note_and_answer_cores_json() {
    let (address, kept) = deployment("").await;
    let session = Credential::terminal_session(&address, "a-session");
    let updated = update(&address, &session, "open-meridian/crypto", "monthly")
        .await
        .unwrap();
    let removed = remove(&address, &session, "open-meridian/crypto", "not used")
        .await
        .unwrap();
    assert_eq!(
        posted(&kept, "/terminal/packs/update"),
        [serde_json::json!({"pack": "open-meridian/crypto", "note": "monthly"})]
    );
    assert_eq!(
        posted(&kept, "/terminal/packs/remove"),
        [serde_json::json!({"pack": "open-meridian/crypto", "note": "not used"})]
    );
    // --json is core's answer as it came: the pack's state, each member's.
    assert_eq!(updated["state"], "Installed: 0 of 3 members");
    assert_eq!(removed["members"][0]["state"], "not_installed");
    let listed = list(&address, &session).await.unwrap();
    assert_eq!(listed["packs"][0]["said"], "Not installed");
    assert_eq!(listed["packs"][0]["state"], "not_installed");
}

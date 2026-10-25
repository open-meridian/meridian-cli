//! Starter packs, through the deployment (spec/starter-packs, requirements
//! 28 to 30; rulings 400, 401, 411, 613 and 616): `meridian plugin install
//! --pack` and `meridian plugin pack list|show|update|remove`, each through
//! core's terminal routes for packs (W8.18, W8.19), as the dashboard's
//! Browse and pack page act, for the deployment admin, under their
//! delegation:
//!
//! - `GET /terminal/packs`: each pack the deployment's catalogue holds, its
//!   members and their selection, and this deployment's state of it;
//! - `GET /terminal/packs/{organisation}/{pack}`: the pack's page, the setup
//!   checklist -- each member's versions, its state and its connection; for
//!   a pack not installed, its members as the install sheet shows them;
//! - `POST /terminal/packs/install`: `InstallPack`, a job, member by member;
//! - `POST /terminal/packs/update` and `.../remove`: "Update all" and
//!   "Remove pack".
//!
//! **Install** reads the pack and its members' states here first, so the
//! plan is shown version by version and asked about before anything moves:
//! `required` and `preselected` members are ticked and `choose` members are
//! not; `--with` ticks one and `--without` unticks a preselected one, each
//! repeatable, and a name that is no member is refused before anything
//! moves. At a terminal without `--yes` each unnamed `choose` member is asked
//! about; with `--yes`, or with no terminal, it is not installed. A member
//! that is Unavailable or "Not permitted here" is never ticked (requirements
//! 10 and 26), and a pack whose required member is either is not installed. A
//! member installed already is never moved (requirement 25). The picks go
//! to core as one install, as the fixture writes it: `with` each `choose`
//! member ticked, `without` each `preselected` one not (left out, or not to
//! be had here), at the revision read. Core ticks the same members, and the
//! install leaves one `pack_installed` record naming the revision, the
//! members and each outcome, and each member's own install record naming the
//! pack (requirement 18). The job is then followed,
//! one line a member as each ends, and the checklist printed: each member's
//! `meridian plugin launch` command, then each that failed, the command
//! exiting 1 naming them (requirement 24). Install launches nothing (405).
//!
//! `--json` answers with core's own JSON, the keys the tools answer with
//! (616): a pack's `state` and `said`, each member's `state`, and its
//! `setup_href` where core gives one.

use std::time::Duration;

use crate::credential::Credential;
use crate::live::Failed;

/// The organisation a bare pack name belongs to (401), as Docker Hub reads
/// `library/`.
pub const FIRST_PARTY: &str = "open-meridian";

/// Where core serves packs to a terminal.
pub const PACKS_PATH: &str = "/terminal/packs";

/// What a deployment whose core has no pack routes yet (contract v27's and
/// earlier) is said to be.
pub const NOT_SERVED: &str = "does not install or keep starter packs yet: its core is older than \
    contract v28's, and `meridian upgrade-deployment` moves it";

/// Between reads of a pack while its install runs.
pub const EVERY: Duration = Duration::from_secs(2);

/// How long an install is followed before the command leaves it running.
pub const WITHIN: Duration = Duration::from_secs(30 * 60);

// ── Names ────────────────────────────────────────────────────────────────

/// A pack's name as core and the platform key it: `organisation/pack`, a
/// bare name read as `open-meridian/<name>` (401).
pub fn pack_name(given: &str) -> Result<String, String> {
    let given = given.trim();
    let (organisation, pack) = given.split_once('/').unwrap_or((FIRST_PARTY, given));
    if !crate::registry::is_slug(organisation) || !crate::catalogue::is_name(pack) {
        return Err(format!(
            "`{given}` is not a pack's name: organisation/pack, or a bare pack name for \
             {FIRST_PARTY}'s"
        ));
    }
    Ok(format!("{organisation}/{pack}"))
}

fn plugin_part(listing: &str) -> &str {
    listing
        .split_once('/')
        .map_or(listing, |(_, plugin)| plugin)
}

// ── Core's routes ────────────────────────────────────────────────────────

/// A pack route, with the session: its answer as JSON. A 404 that says
/// nothing of why is a core without the route, said as [`NOT_SERVED`]; core's
/// own 404 for a pack it does not hold says why.
async fn call(
    address: &str,
    session: &Credential,
    method: reqwest::Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, Failed> {
    let http = crate::catalogue::http()?;
    let body = body.map(|body| body.to_string());
    let (status, text, _) = session
        .send(|bearer| {
            let request = http
                .request(method.clone(), format!("{address}{path}"))
                .bearer_auth(bearer);
            match &body {
                Some(body) => request
                    .header("content-type", "application/json")
                    .body(body.clone()),
                None => request,
            }
        })
        .await?;
    let says_why = serde_json::from_str::<serde_json::Value>(&text)
        .is_ok_and(|said| said["error"].is_string());
    if status == reqwest::StatusCode::NOT_FOUND && !says_why {
        return Err(Failed::Refused(format!("{address} {NOT_SERVED}")));
    }
    if !status.is_success() {
        return Err(crate::live::refusal(address, status, &text));
    }
    serde_json::from_str(&text).map_err(|failed| {
        Failed::Refused(format!("{address} answered what does not read: {failed}"))
    })
}

/// ListPacksFromTerminal: every pack, each with this deployment's state.
pub async fn list(address: &str, session: &Credential) -> Result<serde_json::Value, Failed> {
    call(address, session, reqwest::Method::GET, PACKS_PATH, None).await
}

/// ReadPackFromTerminal: the pack's page, the setup checklist.
pub async fn read(
    address: &str,
    session: &Credential,
    pack: &str,
) -> Result<serde_json::Value, Failed> {
    call(
        address,
        session,
        reqwest::Method::GET,
        &format!("{PACKS_PATH}/{pack}"),
        None,
    )
    .await
}

/// UpdatePackFromTerminal: "Update all".
pub async fn update(
    address: &str,
    session: &Credential,
    pack: &str,
    note: &str,
) -> Result<serde_json::Value, Failed> {
    call(
        address,
        session,
        reqwest::Method::POST,
        &format!("{PACKS_PATH}/update"),
        Some(serde_json::json!({ "pack": pack, "note": note })),
    )
    .await
}

/// RemovePackFromTerminal: "Remove pack"; every member stays.
pub async fn remove(
    address: &str,
    session: &Credential,
    pack: &str,
    note: &str,
) -> Result<serde_json::Value, Failed> {
    call(
        address,
        session,
        reqwest::Method::POST,
        &format!("{PACKS_PATH}/remove"),
        Some(serde_json::json!({ "pack": pack, "note": note })),
    )
    .await
}

/// A number as core writes it, or as the proto's JSON mapping writes a
/// 64-bit one: a string.
fn number(value: &serde_json::Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

fn text(value: &serde_json::Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn members(value: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    value["members"].as_array().into_iter().flatten()
}

// ── The pack, as the deployment offers it ───────────────────────────────

/// A member's selection, as core answers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Required,
    Preselected,
    Choose,
}

/// A pack's member, as its revision lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub listing: String,
    pub registry: String,
    pub selection: Selection,
    pub group: String,
}

/// A pack at the revision the deployment's catalogue holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub pack: String,
    pub curator: String,
    pub revision: u64,
    pub title: String,
    pub members: Vec<Member>,
}

/// The pack named, from core's list; or why it cannot be installed: not in
/// the deployment's catalogue, or withdrawn by its curator.
pub fn offered(list: &serde_json::Value, name: &str) -> Result<Offered, String> {
    let found = list["packs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|pack| pack["pack"] == name)
        .ok_or_else(|| format!("no starter pack {name} is offered in the catalogue"))?;
    if number(&found["withdrawn_at_ns"]) != 0 {
        return Err(format!(
            "{name} was withdrawn by its curator, and is not installed"
        ));
    }
    let members = members(found)
        .map(|member| {
            let selection = match text(&member["selection"]) {
                "required" => Selection::Required,
                "preselected" => Selection::Preselected,
                "choose" => Selection::Choose,
                other => {
                    return Err(format!(
                        "{name}'s member {} has a selection this CLI does not know ({}): \
                         upgrade the CLI (`meridian upgrade`)",
                        text(&member["listing"]),
                        if other.is_empty() { "none" } else { other }
                    ))
                }
            };
            Ok(Member {
                listing: text(&member["listing"]).to_string(),
                registry: text(&member["registry"]).to_string(),
                selection,
                group: text(&member["group"]).to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Offered {
        pack: name.to_string(),
        curator: text(&found["curator"]).to_string(),
        revision: number(&found["revision"]),
        title: text(&found["title"]).to_string(),
        members,
    })
}

/// A member's state here, from the pack's page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Here {
    state: String,
    installed: String,
    latest: String,
    reason: String,
}

fn here_of(status: &serde_json::Value, listing: &str) -> Here {
    members(status)
        .find(|member| member["listing"] == listing)
        .map(|member| Here {
            state: text(&member["state"]).to_string(),
            installed: text(&member["installed_version"]).to_string(),
            latest: text(&member["latest_version"]).to_string(),
            reason: text(&member["reason"]).to_string(),
        })
        .unwrap_or_default()
}

/// A member's state as the pack page says it (requirement 19).
pub fn state_said(state: &str) -> &str {
    match state {
        "not_installed" => "Not installed",
        "installing" => "Installing",
        "failed" => "Failed",
        "installed" => "Installed",
        "launched" => "Launched",
        "unavailable" => "Unavailable",
        "not_permitted" => "Not permitted here",
        other => other,
    }
}

// ── The plan ─────────────────────────────────────────────────────────────

/// What becomes of one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Ticked, and installed at this version.
    Install(String),
    /// Held already at or above the version offered: counted as installed.
    Installed { held: String },
    /// Held below the version offered, and never moved by a pack install.
    Below { held: String, offered: String },
    /// Not ticked.
    NotChosen(String),
    /// Requirement 10: none of its versions offered, with why.
    Unavailable(String),
    /// Requirement 26: this deployment won't take it, with why.
    NotPermitted(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub member: Member,
    pub outcome: Outcome,
}

impl Row {
    /// Whether the member is ticked: installed by the job, or held.
    fn ticked(&self) -> bool {
        matches!(
            self.outcome,
            Outcome::Install(_) | Outcome::Installed { .. } | Outcome::Below { .. }
        )
    }
}

/// The picks as core's install takes them (W8.18's fixture): each `choose`
/// member ticked, and each `preselected` member not.
pub fn with_and_without(rows: &[Row]) -> (Vec<String>, Vec<String>) {
    let mut with = Vec::new();
    let mut without = Vec::new();
    for row in rows {
        match (row.member.selection, row.ticked()) {
            (Selection::Choose, true) => with.push(row.member.listing.clone()),
            (Selection::Preselected, false) => without.push(row.member.listing.clone()),
            _ => {}
        }
    }
    (with, without)
}

/// The question asked of an unnamed `choose` member: the member and the
/// version it is offered at; true to tick it.
pub type Ask<'a> = &'a dyn Fn(&Member, &str) -> bool;

/// How the person picked: `--with`, `--without`, and whether an unnamed
/// `choose` member may be asked about.
pub struct Picks<'a> {
    pub with: &'a [String],
    pub without: &'a [String],
    /// Asked of each unnamed, offered `choose` member at a terminal without
    /// `--yes`; None where nothing prompts.
    pub ask: Option<Ask<'a>>,
}

/// The member a `--with` or `--without` name means: its listing, or its
/// plugin part where only one member has it.
fn member_named<'a>(pack: &'a Offered, name: &str) -> Result<&'a Member, String> {
    if let Some(member) = pack.members.iter().find(|m| m.listing == name) {
        return Ok(member);
    }
    let by_plugin: Vec<&Member> = pack
        .members
        .iter()
        .filter(|m| plugin_part(&m.listing) == name)
        .collect();
    match by_plugin.as_slice() {
        [one] => Ok(one),
        _ => Err(format!(
            "{name} is not a member of {}: its members are {}",
            pack.pack,
            pack.members
                .iter()
                .map(|m| m.listing.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Every member's outcome, in the curator's order, from the pack and its
/// page here; or what refuses the install before anything moves: a name that
/// is no member, a required member left out, Unavailable or not permitted,
/// or a member named that cannot be installed.
pub fn plan(pack: &Offered, status: &serde_json::Value, picks: &Picks) -> Result<Vec<Row>, String> {
    let mut with = Vec::new();
    for name in picks.with {
        with.push(member_named(pack, name)?.listing.clone());
    }
    let mut without = Vec::new();
    for name in picks.without {
        let member = member_named(pack, name)?;
        if member.selection == Selection::Required {
            return Err(format!(
                "{} is required in {}, and is installed with it",
                member.listing, pack.pack
            ));
        }
        if with.contains(&member.listing) {
            return Err(format!(
                "{} is named by both --with and --without",
                member.listing
            ));
        }
        without.push(member.listing.clone());
    }
    let mut rows = Vec::new();
    for member in &pack.members {
        let here = here_of(status, &member.listing);
        let barred = match here.state.as_str() {
            "unavailable" => Some(("unavailable", Outcome::Unavailable(here.reason.clone()))),
            "not_permitted" => Some((
                "not permitted here",
                Outcome::NotPermitted(here.reason.clone()),
            )),
            _ => None,
        };
        if let Some((said, outcome)) = barred {
            let why = if here.reason.is_empty() {
                String::new()
            } else {
                format!(" ({})", here.reason)
            };
            if member.selection == Selection::Required {
                return Err(format!(
                    "{} is not installed: its required member {} is {said}{why}",
                    pack.pack, member.listing
                ));
            }
            if with.contains(&member.listing) {
                return Err(format!(
                    "{} is {said}{why}, so it cannot be installed",
                    member.listing
                ));
            }
            rows.push(Row {
                member: member.clone(),
                outcome,
            });
            continue;
        }
        let offered = here.latest.clone();
        let ticked = match member.selection {
            Selection::Required => true,
            Selection::Preselected => !without.contains(&member.listing),
            Selection::Choose => {
                with.contains(&member.listing) || picks.ask.is_some_and(|ask| ask(member, &offered))
            }
        };
        let held = matches!(here.state.as_str(), "installed" | "launched");
        let outcome = match (ticked, held) {
            (false, _) => Outcome::NotChosen(offered),
            (true, true) => match crate::install::compare(&here.installed, &offered) {
                Some(std::cmp::Ordering::Less) => Outcome::Below {
                    held: here.installed,
                    offered,
                },
                _ => Outcome::Installed {
                    held: here.installed,
                },
            },
            (true, false) => Outcome::Install(offered),
        };
        rows.push(Row {
            member: member.clone(),
            outcome,
        });
    }
    Ok(rows)
}

/// The groups of a list of members, in the curator's order.
fn groups<'a>(of: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen: Vec<&str> = Vec::new();
    for group in of {
        if !seen.contains(&group) {
            seen.push(group);
        }
    }
    seen
}

fn group_said(group: &str) -> &str {
    if group.is_empty() {
        "Members"
    } else {
        group
    }
}

/// The plan, for a person to read: the pack, then each group in the
/// curator's order, one line a member, version by version.
pub fn plan_text(pack: &Offered, rows: &[Row], address: &str) -> String {
    let mut out = format!(
        "{} (revision {}{}): {}, into {address}\n",
        pack.pack,
        pack.revision,
        if pack.curator.is_empty() {
            String::new()
        } else {
            format!(", curated by {}", pack.curator)
        },
        pack.title
    );
    for group in groups(rows.iter().map(|row| row.member.group.as_str())) {
        out.push_str(&format!("  {}\n", group_said(group)));
        for row in rows.iter().filter(|row| row.member.group == group) {
            let listing = &row.member.listing;
            let registry = &row.member.registry;
            out.push_str(&match &row.outcome {
                Outcome::Install(version) => {
                    format!("    install        {listing} {version} ({registry})\n")
                }
                Outcome::Installed { held } => {
                    format!("    installed      {listing} {held}\n")
                }
                Outcome::Below { held, offered } => {
                    format!("    kept           {listing}: Installed {held}; {offered} offered\n")
                }
                Outcome::NotChosen(version) => format!(
                    "    not chosen     {listing} {version} ({registry}); --with {listing} \
                     installs it\n"
                ),
                Outcome::Unavailable(why) => {
                    format!(
                        "    unavailable    {listing}: Unavailable{}\n",
                        because(why)
                    )
                }
                Outcome::NotPermitted(why) => format!(
                    "    not permitted  {listing}: Not permitted here{}\n",
                    because(why)
                ),
            });
        }
    }
    out
}

fn because(why: &str) -> String {
    if why.is_empty() {
        String::new()
    } else {
        format!(" ({why})")
    }
}

// ── Following the job ────────────────────────────────────────────────────

/// The line said as a member's install ends.
fn ended(member: &serde_json::Value) -> String {
    let listing = text(&member["listing"]);
    let reason = text(&member["reason"]);
    match text(&member["state"]) {
        "installed" | "launched" => format!(
            "  {listing} {}: installed",
            text(&member["installed_version"])
        ),
        "failed" => format!(
            "  {listing}: failed: {}",
            if reason.is_empty() {
                "no reason given"
            } else {
                reason
            }
        ),
        other => format!("  {listing}: {}{}", state_said(other), because(reason)),
    }
}

/// The pack's install followed (requirement 30): its page read every
/// `every` until no member the install queued is "installing", one line a
/// member said as each ends. The page as it ended; or, after `within`, the
/// job left running and said so.
pub async fn follow(
    address: &str,
    session: &Credential,
    pack: &str,
    answer: &serde_json::Value,
    every: Duration,
    within: Duration,
    say: &mut dyn FnMut(&str),
) -> Result<serde_json::Value, Failed> {
    let mut queued: Vec<String> = members(answer)
        .filter(|member| member["state"] == "installing")
        .map(|member| text(&member["listing"]).to_string())
        .collect();
    let started = tokio::time::Instant::now();
    loop {
        let status = read(address, session, pack).await?;
        for member in members(&status) {
            let listing = text(&member["listing"]);
            if member["state"] != "installing" && queued.iter().any(|q| q == listing) {
                say(&ended(member));
                queued.retain(|q| q != listing);
            }
        }
        if queued.is_empty() {
            return Ok(status);
        }
        if started.elapsed() >= within {
            return Err(Failed::Refused(format!(
                "{pack} is still installing ({} to go) after {} minutes; it goes on, and \
                 `meridian plugin pack show {pack}` shows where it is",
                queued.join(", "),
                within.as_secs() / 60
            )));
        }
        tokio::time::sleep(every).await;
    }
}

// ── The checklist ────────────────────────────────────────────────────────

/// The name a listed version launches as, from the deployment's catalogue
/// (`GET /terminal/plugins`), or its plugin part.
fn launch_name(installed: &serde_json::Value, listing: &str, version: &str) -> String {
    installed["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["listing"] == listing && v["version"] == version)
        .and_then(|v| v["name"].as_str())
        .unwrap_or_else(|| plugin_part(listing))
        .to_string()
}

/// The checklist (requirement 30) of the members named, from the pack's
/// page: each installed and not launched, with its `meridian plugin launch`
/// command; then each not installed, with why. And whether any is not.
pub fn checklist(
    status: &serde_json::Value,
    named: &[String],
    installed: &serde_json::Value,
) -> (String, bool) {
    let mut ready = Vec::new();
    let mut failed = Vec::new();
    for member in members(status) {
        let listing = text(&member["listing"]);
        if !named.iter().any(|n| n == listing) {
            continue;
        }
        let version = text(&member["installed_version"]);
        match text(&member["state"]) {
            "installed" => {
                let name = launch_name(installed, listing, version);
                ready.push(format!(
                    "  meridian plugin launch {name} {version} --instance {name}\n"
                ));
            }
            "launched" => {}
            state => {
                let reason = text(&member["reason"]);
                failed.push(format!(
                    "  {listing}: {}\n",
                    if reason.is_empty() {
                        state_said(state)
                    } else {
                        reason
                    }
                ));
            }
        }
    }
    let mut out = String::new();
    if !ready.is_empty() {
        out.push_str(
            "Installed; nothing is launched. Launch each, approving the roles it asks for:\n",
        );
        out.extend(ready);
    }
    let any_failed = !failed.is_empty();
    if any_failed {
        out.push_str("Not installed:\n");
        out.extend(failed);
    }
    (out, any_failed)
}

// ── Installing a pack ────────────────────────────────────────────────────

/// What `install --pack` was asked: the pack's name, the picks, and the
/// note kept with the install.
pub struct PackAsked<'a> {
    pub pack: &'a str,
    pub picks: Picks<'a>,
    pub note: &'a str,
}

/// How the install is followed.
pub struct Following {
    pub every: Duration,
    pub within: Duration,
}

/// What a pack install came to: the checklist, and whether a member failed.
#[derive(Debug, PartialEq, Eq)]
pub struct Done {
    pub checklist: String,
    pub failed: bool,
}

/// `install --pack`: the pack and its page read, the plan made and shown,
/// approved (`approve` is the question asked, or `--yes`), the ticked
/// members installed as one pack install, and the job followed. What it came
/// to, or what refused it before anything moved.
pub async fn install_pack(
    address: &str,
    session: &Credential,
    asked: &PackAsked<'_>,
    approve: &dyn Fn(&str) -> bool,
    say: &mut dyn FnMut(&str),
    following: &Following,
) -> Result<Done, Failed> {
    let pack = offered(&list(address, session).await?, asked.pack)?;
    let status = read(address, session, asked.pack).await?;
    let rows = plan(&pack, &status, &asked.picks)?;
    say(plan_text(&pack, &rows, address).trim_end());
    let ticked: Vec<String> = rows
        .iter()
        .filter(|row| row.ticked())
        .map(|row| row.member.listing.clone())
        .collect();
    let to_install = rows
        .iter()
        .filter(|row| matches!(row.outcome, Outcome::Install(_)))
        .count();
    // Every member held already is still the pack installed here: the
    // install is sent, core leaves each held member alone, and its
    // `pack_installed` record is written (decisions/031).
    let question = match to_install {
        0 => {
            say("Nothing to install.");
            "Record this pack as installed?".to_string()
        }
        1 => "Install this member?".to_string(),
        n => format!("Install these {n} members?"),
    };
    let settled = if ticked.is_empty() {
        status
    } else {
        if !approve(&question) {
            return Err(Failed::Refused(
                "not approved, so nothing was installed. Where there is no terminal to ask at, \
                 --yes approves"
                    .into(),
            ));
        }
        let (with, without) = with_and_without(&rows);
        let answer = call(
            address,
            session,
            reqwest::Method::POST,
            &format!("{PACKS_PATH}/install"),
            Some(serde_json::json!({
                "pack": pack.pack, "with": with, "without": without,
                "revision": pack.revision, "note": asked.note,
            })),
        )
        .await?;
        follow(
            address,
            session,
            &pack.pack,
            &answer,
            following.every,
            following.within,
            say,
        )
        .await?
    };
    let installed = crate::catalogue::catalogue(address, session).await?;
    let (checklist, failed) = checklist(&settled, &ticked, &installed);
    Ok(Done { checklist, failed })
}

// ── Reading packs ────────────────────────────────────────────────────────

/// `pack list`, for a person to read: one line a pack -- its name, title,
/// asset classes, members and this deployment's state (requirement 16).
pub fn list_text(answer: &serde_json::Value) -> String {
    let packs: Vec<&serde_json::Value> = answer["packs"].as_array().into_iter().flatten().collect();
    if packs.is_empty() {
        return "No starter pack is offered in the catalogue.\n".into();
    }
    let mut out = String::new();
    for pack in packs {
        let classes: Vec<&str> = pack["asset_classes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|class| class.as_str())
            .collect();
        let count = members(pack).count();
        let parts = [
            text(&pack["pack"]).to_string(),
            text(&pack["title"]).to_string(),
            classes.join(", "),
            format!("{count} member{}", if count == 1 { "" } else { "s" }),
            text(&pack["said"]).to_string(),
            if number(&pack["withdrawn_at_ns"]) != 0 {
                "Withdrawn by its curator".to_string()
            } else {
                String::new()
            },
        ];
        let parts: Vec<&str> = parts
            .iter()
            .map(String::as_str)
            .filter(|part| !part.is_empty())
            .collect();
        out.push_str(&parts.join("  "));
        out.push('\n');
    }
    out
}

/// A member's versions as the pack page shows them: installed, then the
/// latest offered where it differs.
fn versions(member: &serde_json::Value) -> String {
    match (
        text(&member["installed_version"]),
        text(&member["latest_version"]),
    ) {
        ("", latest) => latest.to_string(),
        (installed, latest) if latest.is_empty() || installed == latest => installed.to_string(),
        (installed, latest) => format!("{installed} · {latest}"),
    }
}

/// One member's line on the checklist: its versions, its state (with why),
/// and its connection.
fn member_line(member: &serde_json::Value) -> String {
    let state = text(&member["state"]);
    let reason = text(&member["reason"]);
    let mut said = if state == "failed" && !reason.is_empty() {
        format!("Failed: {reason}")
    } else {
        state_said(state).to_string()
    };
    if number(&member["removed_by_revision"]) > 0 {
        said = format!("No longer in this pack · {said}");
    }
    if state != "failed" && !reason.is_empty() {
        said = format!("{said}: {reason}");
    }
    let versions = versions(member);
    let mut line = if versions.is_empty() {
        format!("    {}  {said}", text(&member["listing"]))
    } else {
        format!("    {}  {versions}  {said}", text(&member["listing"]))
    };
    let setup = text(&member["setup"]);
    if !setup.is_empty() {
        line.push_str(&format!("  {setup}"));
    }
    if let Some(href) = member["setup_href"].as_str().filter(|h| !h.is_empty()) {
        line.push_str(&format!(" ({href})"));
    }
    line
}

/// `pack show`, for a person to read: the pack and this deployment's state
/// of it, each group's members one line each, what a revision added, and
/// the checklist's launch commands (requirements 19, 21 and 27).
pub fn show_text(status: &serde_json::Value, installed: &serde_json::Value) -> String {
    let pack = text(&status["pack"]);
    let mut out = format!("{pack}: {}\n", text(&status["state"]));
    let at = number(&status["installed_revision"]);
    let latest = number(&status["latest_revision"]);
    if at > 0 && latest > at {
        out.push_str(&format!(
            "Installed at revision {at}; revision {latest} is in the catalogue.\n"
        ));
    }
    let withdrawn = status["withdrawn"].as_bool().unwrap_or(false);
    if withdrawn {
        out.push_str("Withdrawn by its curator.\n");
    }
    for group in groups(members(status).map(|m| text(&m["group"]))) {
        out.push_str(&format!("  {}\n", group_said(group)));
        for member in members(status).filter(|m| m["group"] == group) {
            out.push_str(&member_line(member));
            out.push('\n');
        }
    }
    let added: Vec<&str> = members(status)
        .filter(|m| number(&m["added_by_revision"]) > 0 && m["state"] == "not_installed")
        .map(|m| text(&m["listing"]))
        .collect();
    if at > 0 && !added.is_empty() && !withdrawn {
        out.push_str(&format!("Added to this pack: {}\n", added.join(", ")));
    }
    if members(status).any(|m| m["state"] == "installing") {
        out.push_str("Installing, one member at a time.\n");
    }
    let named: Vec<String> = members(status)
        .filter(|m| m["state"] == "installed")
        .map(|m| text(&m["listing"]).to_string())
        .collect();
    out.push_str(&checklist(status, &named, installed).0);
    out
}

/// The members "Update all" would move, as the pack page's sheet lists
/// them: each installed or launched with a newer version offered, and why.
pub fn to_update(status: &serde_json::Value) -> Vec<String> {
    members(status)
        .filter(|m| matches!(text(&m["state"]), "installed" | "launched"))
        .filter(|m| !text(&m["reason"]).is_empty())
        .map(|m| format!("  {}: {}", text(&m["listing"]), text(&m["reason"])))
        .collect()
}

/// The members that stay installed when the pack's record is removed, as
/// the "Remove pack" sheet lists them.
pub fn staying(status: &serde_json::Value) -> Vec<String> {
    members(status)
        .filter(|m| matches!(text(&m["state"]), "installed" | "launched"))
        .map(|m| {
            format!(
                "  {} · {}",
                text(&m["listing"]),
                state_said(text(&m["state"]))
            )
        })
        .collect()
}

#[cfg(test)]
mod tests;

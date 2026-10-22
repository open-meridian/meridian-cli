//! `meridian plugin install` (spec/starter-packs, requirements 28 to 30;
//! rulings 400, 401 and 411): a listed version, or a starter pack's members,
//! installed into the connected deployment through its existing install,
//! `POST /terminal/plugins/install` (InstallPluginFromTerminal, contract v26,
//! W8.10), once a member. Install launches nothing; the checklist it prints
//! says how each is launched.
//!
//! **One listing**, `organisation/plugin@version`, is installed as named,
//! from `--registry` where given (the deployment takes community otherwise).
//! Without `@version` it is the version Browse shows (core's `latest()`):
//! the highest offered release, verified first, read from the platform's
//! catalogue.
//!
//! **A pack**, `--pack <name>`, a bare name being `open-meridian/<name>`
//! (401), is read from the platform's catalogue (`PullCatalogueReply.packs`,
//! its latest published revision), and each member is resolved against the
//! catalogue's listings here, by the platform's and core's rule (400): the
//! highest offered release in the member's registry at or above its minimum,
//! a pre-release only where no release is. A member whose versions are all
//! withdrawn or pulled, or that lists none, is Unavailable and never ticked;
//! a pack whose required member is Unavailable is not offered. `required`
//! and `preselected` members are ticked, `choose` members are not: `--with`
//! ticks one and `--without` unticks a preselected one, each repeatable, and
//! a name that is no member is refused before anything moves. At a terminal
//! without `--yes`, each unnamed `choose` member is asked about; with
//! `--yes`, or with no terminal, it is not installed. A member the
//! deployment holds already is never moved (requirement 25): at or above the
//! resolved version it counts as installed, below it the line says
//! "Installed 0.3.0; 0.5.1 offered". What will be installed is shown version
//! by version and asked about first unless `--yes`; each member's install is
//! its own act with the note, and a failure is said and the rest go on
//! (requirement 24), the command then exiting 1 and naming the failed.
//!
//! The deployment's install takes no pack name from a terminal, so no
//! `pack_installed` record is written from here: each member leaves its own
//! install record, with the note.

use crate::credential::Credential;
use crate::live::Failed;

/// The organisation a bare pack name belongs to (401), as Docker Hub reads
/// `library/`.
pub const FIRST_PARTY: &str = "open-meridian";

/// Whether the platform answers its catalogue (`GET
/// /api/v1/registry/catalogue`) for a person's sign-in. `--pack` and a
/// listing named without `@version` read it; where it is not answered they
/// refuse with [`CATALOGUE_NOT_YET`], and the rest of this module stays.
pub const CATALOGUE_FOR_A_PERSON: bool = true;

/// What `--pack` and a listing without a version say where the platform's
/// catalogue cannot be read by a person yet.
pub const CATALOGUE_NOT_YET: &str =
    "needs the platform's catalogue read, coming in the next platform release";

/// Where the platform's catalogue is read.
pub const CATALOGUE_PATH: &str = "/api/v1/registry/catalogue";

// ── Names ────────────────────────────────────────────────────────────────

fn is_label(part: &str) -> bool {
    crate::catalogue::is_name(part)
}

/// A pack's name as the platform keys it: `organisation/pack`, a bare name
/// read as `open-meridian/<name>` (401).
pub fn pack_name(given: &str) -> Result<String, String> {
    let given = given.trim();
    let (organisation, pack) = given.split_once('/').unwrap_or((FIRST_PARTY, given));
    if !crate::registry::is_slug(organisation) || !is_label(pack) {
        return Err(format!(
            "`{given}` is not a pack's name: organisation/pack, or a bare pack name for \
             {FIRST_PARTY}'s"
        ));
    }
    Ok(format!("{organisation}/{pack}"))
}

/// A listing as `install` is given it: `organisation/plugin`, and the
/// version after `@` where one is named.
pub fn listing_named(given: &str) -> Result<(String, Option<String>), String> {
    let (listing, version) = match given.split_once('@') {
        Some((listing, version)) => (listing, Some(version.trim_start_matches('v'))),
        None => (given, None),
    };
    let well_formed = listing
        .split_once('/')
        .is_some_and(|(organisation, plugin)| {
            crate::registry::is_slug(organisation) && is_label(plugin)
        });
    if !well_formed {
        return Err(format!(
            "`{given}` is not a listing: organisation/plugin, with @version for one version"
        ));
    }
    if let Some(version) = version {
        if parse(version).is_none() {
            return Err(format!(
                "`{version}` is not a version: a SemVer 2.0 version, as 1.4.0"
            ));
        }
    }
    Ok((listing.to_string(), version.map(String::from)))
}

// ── Versions ─────────────────────────────────────────────────────────────

/// A SemVer 2.0 version's precedence parts: major, minor, patch and its
/// pre-release identifiers (build metadata ignored).
fn parse(version: &str) -> Option<(u64, u64, u64, Vec<String>)> {
    let core = version.split('+').next()?;
    let (numbers, pre) = match core.split_once('-') {
        Some((numbers, pre)) => (numbers, Some(pre)),
        None => (core, None),
    };
    let mut parts = numbers.split('.');
    let mut number = || -> Option<u64> {
        let part = parts.next()?;
        (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok())?
    };
    let (major, minor, patch) = (number()?, number()?, number()?);
    if parts.next().is_some() {
        return None;
    }
    let pre: Vec<String> = match pre {
        Some(pre) if pre.split('.').all(|p| !p.is_empty()) => {
            pre.split('.').map(String::from).collect()
        }
        Some(_) => return None,
        None => Vec::new(),
    };
    Some((major, minor, patch, pre))
}

/// SemVer 2.0 precedence; None where either is no version.
pub fn compare(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    let (a, b) = (parse(a)?, parse(b)?);
    let order = (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2));
    if order != Equal {
        return Some(order);
    }
    Some(match (a.3.is_empty(), b.3.is_empty()) {
        (true, true) => Equal,
        (true, false) => Greater,
        (false, true) => Less,
        (false, false) => {
            for (x, y) in a.3.iter().zip(&b.3) {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Less,
                    (Err(_), Ok(_)) => Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != Equal {
                    return Some(order);
                }
            }
            a.3.len().cmp(&b.3.len())
        }
    })
}

fn is_prerelease(version: &str) -> bool {
    parse(version).is_some_and(|(_, _, _, pre)| !pre.is_empty())
}

// ── The catalogue ────────────────────────────────────────────────────────

/// A listed version as resolution reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    version: String,
    registry: String,
    withdrawn: bool,
    pulled: bool,
}

fn versions_of(catalogue: &serde_json::Value, listing: &str) -> Option<Vec<Listed>> {
    let found = catalogue["listings"]
        .as_array()?
        .iter()
        .find(|item| item["listing"] == listing)?;
    Some(
        found["versions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| Listed {
                version: v["version"].as_str().unwrap_or_default().to_string(),
                registry: v["registry"].as_str().unwrap_or_default().to_string(),
                withdrawn: number(&v["withdrawn_at_ns"]) != 0,
                // A pull is open until it is restored.
                pulled: v["pull"].is_object() && number(&v["pull"]["restored_at_ns"]) == 0,
            })
            .filter(|v| parse(&v.version).is_some())
            .collect(),
    )
}

/// A number as the proto's JSON mapping writes it: a number, or a string
/// for a 64-bit one.
fn number(value: &serde_json::Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(0)
}

fn offered(version: &Listed) -> bool {
    !version.withdrawn && !version.pulled
}

/// The highest of `versions` by precedence, a release first.
fn highest(versions: &[&Listed]) -> Option<Listed> {
    let best = |of: Vec<&Listed>| {
        of.into_iter()
            .max_by(|a, b| compare(&a.version, &b.version).unwrap_or(std::cmp::Ordering::Equal))
            .cloned()
    };
    let releases: Vec<&Listed> = versions
        .iter()
        .copied()
        .filter(|v| !is_prerelease(&v.version))
        .collect();
    best(releases).or_else(|| best(versions.to_vec()))
}

/// The version Browse shows for a listing (core's `latest()`): the highest
/// offered release, verified first, or failing that the highest offered
/// pre-release; within `only` where a registry is named. Its version and
/// registry.
pub fn latest(
    catalogue: &serde_json::Value,
    listing: &str,
    only: Option<&str>,
) -> Option<(String, String)> {
    let versions = versions_of(catalogue, listing)?;
    for registry in ["verified", "community"]
        .into_iter()
        .filter(|registry| only.is_none_or(|only| only == *registry))
    {
        let live: Vec<&Listed> = versions
            .iter()
            .filter(|v| v.registry == registry && offered(v))
            .collect();
        if let Some(best) = highest(&live) {
            return Some((best.version, best.registry));
        }
    }
    None
}

/// A member's selection (`PACK_SELECTION_*`).
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
    pub minimum_version: String,
}

/// A pack at its latest published revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub pack: String,
    pub curator: String,
    pub revision: u64,
    pub title: String,
    pub members: Vec<Member>,
}

/// The pack named in the platform's catalogue, or why it cannot be
/// installed: not there, withdrawn by its curator, or never published.
pub fn pack_of(catalogue: &serde_json::Value, name: &str) -> Result<Pack, String> {
    let found = catalogue["packs"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|pack| pack["pack"] == name)
        .ok_or_else(|| format!("the platform's catalogue holds no starter pack {name}"))?;
    if number(&found["withdrawn_at_ns"]) != 0 {
        return Err(format!(
            "{name} was withdrawn by its curator, and is not installed"
        ));
    }
    let revision = &found["revision"];
    if !revision.is_object() {
        return Err(format!("{name} has no published revision"));
    }
    let text = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_string();
    let members = revision["members"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|member| {
            let selection = match member["selection"].as_str() {
                Some("PACK_SELECTION_REQUIRED") => Selection::Required,
                Some("PACK_SELECTION_PRESELECTED") => Selection::Preselected,
                Some("PACK_SELECTION_CHOOSE") => Selection::Choose,
                other => {
                    return Err(format!(
                        "{name}'s member {} has a selection this CLI does not know ({}): \
                         upgrade the CLI (`meridian upgrade`)",
                        text(&member["listing"]),
                        other.unwrap_or("none")
                    ))
                }
            };
            Ok(Member {
                listing: text(&member["listing"]),
                registry: text(&member["registry"]),
                selection,
                group: text(&member["group"]),
                minimum_version: text(&member["minimum_version"]),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Pack {
        pack: name.to_string(),
        curator: text(&found["curator"]),
        revision: number(&revision["revision"]),
        title: text(&revision["title"]),
        members,
    })
}

/// A member resolved (400): its version, or why it is Unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Version(String),
    Unavailable(&'static str),
}

/// Requirement 8 and 10: the highest offered release in the member's
/// registry at or above its minimum, a pre-release only where no release is;
/// or Unavailable, with its reason's kind.
pub fn resolve(catalogue: &serde_json::Value, member: &Member) -> Resolved {
    let Some(versions) = versions_of(catalogue, &member.listing) else {
        return Resolved::Unavailable("none listed");
    };
    let above: Vec<&Listed> = versions
        .iter()
        .filter(|v| v.registry == member.registry || member.registry.is_empty())
        .filter(|v| {
            member.minimum_version.is_empty()
                || compare(&v.version, &member.minimum_version)
                    .is_some_and(|order| order != std::cmp::Ordering::Less)
        })
        .collect();
    let live: Vec<&Listed> = above.iter().copied().filter(|v| offered(v)).collect();
    match highest(&live) {
        Some(best) => Resolved::Version(best.version),
        None if above.is_empty() => Resolved::Unavailable("none listed"),
        None if above.iter().all(|v| v.withdrawn) => Resolved::Unavailable("withdrawn"),
        None => Resolved::Unavailable("pulled"),
    }
}

// ── The plan ─────────────────────────────────────────────────────────────

/// What becomes of one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Ticked, and installed at this version.
    Install(String),
    /// Held already at or above the resolved version: counted as installed.
    Installed {
        held: String,
    },
    /// Held below the resolved version, and never moved by a pack install.
    Below {
        held: String,
        offered: String,
    },
    /// Not ticked.
    NotChosen(String),
    Unavailable(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub member: Member,
    pub outcome: Outcome,
}

/// The question asked of an unnamed `choose` member: the member and the
/// version it resolved to; true to tick it.
pub type Ask<'a> = &'a dyn Fn(&Member, &str) -> bool;

/// How the person picked: `--with`, `--without`, and whether an unnamed
/// `choose` member may be asked about.
pub struct Picks<'a> {
    pub with: &'a [String],
    pub without: &'a [String],
    /// Asked of each unnamed, available `choose` member at a terminal
    /// without `--yes`; None where nothing prompts.
    pub ask: Option<Ask<'a>>,
}

/// The member a `--with` or `--without` name means: its listing, or its
/// plugin part where only one member has it.
fn member_named<'a>(pack: &'a Pack, name: &str) -> Result<&'a Member, String> {
    if let Some(member) = pack.members.iter().find(|m| m.listing == name) {
        return Ok(member);
    }
    let by_plugin: Vec<&Member> = pack
        .members
        .iter()
        .filter(|m| m.listing.split_once('/').map(|(_, p)| p) == Some(name))
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

/// The deployment's versions of a listing, from its catalogue
/// (`GET /terminal/plugins`).
fn held_versions(installed: &serde_json::Value, listing: &str) -> Vec<String> {
    installed["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["listing"] == listing)
        .filter_map(|v| v["version"].as_str().map(String::from))
        .collect()
}

/// Every member's outcome, in the curator's order; or what refuses the
/// install before anything moves: a name that is no member, a required
/// member left out or Unavailable, an Unavailable member named.
pub fn plan(
    pack: &Pack,
    catalogue: &serde_json::Value,
    installed: &serde_json::Value,
    picks: &Picks,
) -> Result<Vec<Row>, String> {
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
        let version = match resolve(catalogue, member) {
            Resolved::Version(version) => version,
            Resolved::Unavailable(why) => {
                if member.selection == Selection::Required {
                    return Err(format!(
                        "{} is not offered: its required member {} is unavailable ({why})",
                        pack.pack, member.listing
                    ));
                }
                if with.contains(&member.listing) {
                    return Err(format!(
                        "{} is unavailable ({why}), so it cannot be installed",
                        member.listing
                    ));
                }
                rows.push(Row {
                    member: member.clone(),
                    outcome: Outcome::Unavailable(why),
                });
                continue;
            }
        };
        let ticked = match member.selection {
            Selection::Required => true,
            Selection::Preselected => !without.contains(&member.listing),
            Selection::Choose => {
                with.contains(&member.listing) || picks.ask.is_some_and(|ask| ask(member, &version))
            }
        };
        let held = held_versions(installed, &member.listing);
        let highest_held = held
            .iter()
            .max_by(|a, b| compare(a, b).unwrap_or(std::cmp::Ordering::Equal))
            .cloned();
        let outcome = match (ticked, highest_held) {
            (false, _) => Outcome::NotChosen(version),
            (true, Some(held)) => match compare(&held, &version) {
                Some(std::cmp::Ordering::Less) => Outcome::Below {
                    held,
                    offered: version,
                },
                _ => Outcome::Installed { held },
            },
            (true, None) => Outcome::Install(version),
        };
        rows.push(Row {
            member: member.clone(),
            outcome,
        });
    }
    Ok(rows)
}

/// The plan, for a person to read: the pack, then each group in the
/// curator's order, one line a member, version by version.
pub fn plan_text(pack: &Pack, rows: &[Row], address: &str) -> String {
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
    let mut groups: Vec<&str> = Vec::new();
    for row in rows {
        if !groups.contains(&row.member.group.as_str()) {
            groups.push(&row.member.group);
        }
    }
    for group in groups {
        out.push_str(&format!(
            "  {}\n",
            if group.is_empty() { "Members" } else { group }
        ));
        for row in rows.iter().filter(|row| row.member.group == group) {
            let listing = &row.member.listing;
            let registry = &row.member.registry;
            out.push_str(&match &row.outcome {
                Outcome::Install(version) => {
                    format!("    install      {listing} {version} ({registry})\n")
                }
                Outcome::Installed { held } => {
                    format!("    installed    {listing} {held}\n")
                }
                Outcome::Below { held, offered } => {
                    format!("    kept         {listing}: Installed {held}; {offered} offered\n")
                }
                Outcome::NotChosen(version) => format!(
                    "    not chosen   {listing} {version} ({registry}); --with {listing} \
                     installs it\n"
                ),
                Outcome::Unavailable(why) => {
                    format!("    unavailable  {listing}: Unavailable ({why})\n")
                }
            });
        }
    }
    out
}

/// The members to install, in the curator's order: listing, version,
/// registry.
pub fn to_install(rows: &[Row]) -> Vec<(String, String, String)> {
    rows.iter()
        .filter_map(|row| match &row.outcome {
            Outcome::Install(version) => Some((
                row.member.listing.clone(),
                version.clone(),
                row.member.registry.clone(),
            )),
            _ => None,
        })
        .collect()
}

// ── Acting ───────────────────────────────────────────────────────────────

/// The platform's catalogue (`PullCatalogueReply`), read with the person's
/// sign-in.
pub async fn read_catalogue(
    signed_in: &crate::registry::SignedIn,
) -> Result<serde_json::Value, String> {
    let platform = signed_in.platform().to_string();
    let bearer = signed_in.bearer().await?;
    crate::registry::call(
        &platform,
        reqwest::Method::GET,
        CATALOGUE_PATH,
        &bearer,
        None,
    )
    .await
}

/// One listed version installed through the deployment's own install (W8.10):
/// the name and version the deployment recorded it as.
pub async fn install_one(
    address: &str,
    session: &Credential,
    listing: &str,
    version: &str,
    registry: &str,
    note: &str,
) -> Result<(String, String), Failed> {
    let answer = crate::catalogue::post(
        address,
        session,
        "/terminal/plugins/install",
        serde_json::json!({
            "listing": listing, "version": version, "registry": registry, "note": note,
        }),
    )
    .await?;
    let name = answer["name"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(String::from)
        .unwrap_or_else(|| plugin_part(listing).to_string());
    let version = answer["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(version)
        .to_string();
    Ok((name, version))
}

fn plugin_part(listing: &str) -> &str {
    listing
        .split_once('/')
        .map_or(listing, |(_, plugin)| plugin)
}

/// What a pack install came to: each member installed or held, by the name
/// and version it launches as, and each that failed, with why.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Done {
    pub ready: Vec<(String, String)>,
    pub failed: Vec<(String, String)>,
}

/// Each ticked member installed in the curator's order, a line said as each
/// ends, going on past a failure (requirement 24).
pub async fn install_members(
    address: &str,
    session: &Credential,
    rows: &[Row],
    installed: &serde_json::Value,
    note: &str,
    say: &mut dyn FnMut(&str),
) -> Done {
    let mut done = Done::default();
    for row in rows {
        let listing = &row.member.listing;
        match &row.outcome {
            Outcome::Install(version) => {
                match install_one(
                    address,
                    session,
                    listing,
                    version,
                    &row.member.registry,
                    note,
                )
                .await
                {
                    Ok(recorded) => {
                        say(&format!("  {listing} {version}: installed"));
                        done.ready.push(recorded);
                    }
                    Err(failed) => {
                        say(&format!("  {listing} {version}: failed: {}", failed.said()));
                        done.failed
                            .push((listing.clone(), failed.said().to_string()));
                    }
                }
            }
            Outcome::Installed { held } | Outcome::Below { held, .. } => {
                let name = installed["versions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|v| v["listing"] == listing.as_str() && v["version"] == held.as_str())
                    .and_then(|v| v["name"].as_str())
                    .unwrap_or_else(|| plugin_part(listing))
                    .to_string();
                done.ready.push((name, held.clone()));
            }
            Outcome::NotChosen(_) | Outcome::Unavailable(_) => {}
        }
    }
    done
}

/// The checklist (requirement 30): each member ready to launch, and how;
/// then each that failed. Install launches nothing (405).
pub fn checklist(done: &Done) -> String {
    let mut out = String::new();
    if !done.ready.is_empty() {
        out.push_str(
            "Installed; nothing is launched. Launch each, approving the roles it asks for:\n",
        );
        for (name, version) in &done.ready {
            out.push_str(&format!(
                "  meridian plugin launch {name} {version} --instance {name}\n"
            ));
        }
    }
    if !done.failed.is_empty() {
        out.push_str("Not installed:\n");
        for (listing, why) in &done.failed {
            out.push_str(&format!("  {listing}: {why}\n"));
        }
    }
    out
}

/// What `install --pack` was asked: the pack's name, the picks, and the
/// note kept with each install.
pub struct PackAsked<'a> {
    pub pack: &'a str,
    pub picks: Picks<'a>,
    pub note: &'a str,
}

/// `install --pack`, once the catalogue is read: the pack and the
/// deployment's catalogue read, the plan made and shown, approved (`approve`
/// is the question asked, or `--yes`), and each ticked member installed. What
/// it came to, or what refused it before anything moved.
pub async fn install_pack(
    address: &str,
    session: &Credential,
    catalogue: &serde_json::Value,
    asked: &PackAsked<'_>,
    approve: &dyn Fn(&str) -> bool,
    say: &mut dyn FnMut(&str),
) -> Result<Done, Failed> {
    let (picks, note) = (&asked.picks, asked.note);
    let pack = pack_of(catalogue, asked.pack)?;
    let installed = crate::catalogue::catalogue(address, session).await?;
    let rows = plan(&pack, catalogue, &installed, picks)?;
    say(plan_text(&pack, &rows, address).trim_end());
    let members = to_install(&rows);
    if members.is_empty() {
        say("Nothing to install.");
    } else {
        let question = match members.len() {
            1 => "Install this member?".to_string(),
            n => format!("Install these {n} members?"),
        };
        if !approve(&question) {
            return Err(Failed::Refused(
                "not approved, so nothing was installed. Where there is no terminal to ask at, \
                 --yes approves"
                    .into(),
            ));
        }
    }
    Ok(install_members(address, session, &rows, &installed, note, say).await)
}

#[cfg(test)]
mod tests;

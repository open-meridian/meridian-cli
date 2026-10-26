//! `meridian plugin install <organisation>/<plugin>[@<version>]` (spec/
//! starter-packs, requirement 28; rulings 400, 401 and 411): a listed version
//! installed into the connected deployment through its existing install,
//! `POST /terminal/plugins/install` (InstallPluginFromTerminal, contract v26,
//! W8.10). Install launches nothing; the checklist it prints says how the
//! version is launched.
//!
//! The listing is installed as named, from `--registry` where given (the
//! deployment takes community otherwise). Without `@version` it is the
//! version Browse shows (core's `latest()`): the highest offered release,
//! verified first, read from the platform's catalogue.
//!
//! A starter pack (`--pack`) is core's to install, from CLI 0.1.53: see
//! [`crate::pack`].

use crate::credential::Credential;
use crate::live::Failed;

/// Whether the platform answers its catalogue (`GET
/// /api/v1/registry/catalogue`) for a person's sign-in, as ruling 525 has it
/// do. A listing named without `@version` reads it; switched off, it refuses
/// with [`CATALOGUE_NOT_YET`], and the rest of this module stays.
pub const CATALOGUE_FOR_A_PERSON: bool = true;

/// What a listing without a version says where the platform's catalogue
/// cannot be read by a person yet.
pub const CATALOGUE_NOT_YET: &str =
    "needs the platform's catalogue read, coming in the next platform release";

/// Where the platform's catalogue is read.
pub const CATALOGUE_PATH: &str = "/api/v1/registry/catalogue";

// ── Names ────────────────────────────────────────────────────────────────

fn is_label(part: &str) -> bool {
    crate::catalogue::is_name(part)
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

// ── Acting ───────────────────────────────────────────────────────────────

/// What a platform that takes only a deployment's assertion at its
/// catalogue says to the CLI, before ruling 525's platform release.
pub const CATALOGUE_REFUSED: &str = "this platform does not yet let the CLI read its catalogue";

/// The platform's catalogue (`PullCatalogueReply`), read with the person's
/// sign-in (ruling 525): the bearer is fresh, so a 401 or 403 is a platform
/// that does not take it at its catalogue yet, said in one line.
pub async fn read_catalogue(
    signed_in: &crate::registry::SignedIn,
) -> Result<serde_json::Value, String> {
    let platform = signed_in.platform().to_string();
    let bearer = signed_in.bearer().await?;
    catalogue_at(&platform, &bearer).await
}

/// The catalogue at `platform`, read with `bearer`.
pub async fn catalogue_at(platform: &str, bearer: &str) -> Result<serde_json::Value, String> {
    let (status, text) =
        crate::registry::send(platform, reqwest::Method::GET, CATALOGUE_PATH, bearer, None).await?;
    match status.as_u16() {
        401 | 403 => Err(format!("{platform}: {CATALOGUE_REFUSED}")),
        _ if !status.is_success() => Err(format!("{platform} refused ({status}): {}", text.trim())),
        _ => serde_json::from_str(&text)
            .map_err(|failed| format!("{platform}'s catalogue is unreadable: {failed}")),
    }
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

/// The checklist (requirement 30) of one version installed: how it is
/// launched. Install launches nothing (405).
pub fn checklist(name: &str, version: &str) -> String {
    format!(
        "Installed; nothing is launched. Launch each, approving the roles it asks for:\n  \
         meridian plugin launch {name} {version} --instance {name}\n"
    )
}

#[cfg(test)]
mod tests;

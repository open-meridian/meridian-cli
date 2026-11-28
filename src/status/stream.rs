//! What `meridian status` says of the lake's write-ahead stream: each
//! member's health and the words it said it with, as ReadResilience answers
//! them (`members[].health`, `members[].detail`) through the dashboard's
//! terminal read, on the session `meridian connect` keeps. The dashboard's
//! standing notice for a stuck stream sends a person here ("meridian status
//! shows what it holds").
//!
//! A member whose words say its stream is stuck full or at its rate limit,
//! or that the broker refuses its writes or does not answer them, is a line:
//! its state, since when, what the stream holds against its bound and, at
//! its rate limit, the rate its bounds allow, where its words say them. Nothing is said while none does. `--json` lists every member as the
//! deployment answered it. Read-only.

use serde_json::{json, Value};

use super::Account;
use crate::credential::Credential;

/// Every member as ReadResilience answers it, or why it was not read.
pub type Members = Result<Vec<Value>, String>;

/// The words a member's detail opens with while its stream is stuck full,
/// as the lake's writer says them.
const STUCK_FULL: &str = "the lake's write-ahead stream is stuck full";

/// The words a member's detail opens with while its stream is at its rate
/// limit: the lake keeps up, but writes arrive faster than the stream's
/// bounds allow. Never said with [`STUCK_FULL`].
const RATE_LIMITED: &str = "the lake's write-ahead stream is at its rate limit";

/// The words a member's detail holds while the broker refuses its writes or
/// does not answer them, first or after the stuck-full words.
const REFUSED: &str = "the lake's write-ahead writes are refused or not answered by the broker";

/// How the words say since when, after the state and for how long.
const SINCE: &str = ", since ";

/// Every member as ReadResilience answers it, or why it was not read.
pub async fn read(session: &Credential) -> Members {
    let answer = crate::moves::resilience(session.address(), session)
        .await
        .map_err(|failed| failed.said())?;
    Ok(answer["members"].as_array().cloned().unwrap_or_default())
}

/// Book health and every member, read on the one session.
pub async fn with_book_health(session: &Credential) -> (Result<Vec<Account>, String>, Members) {
    (
        super::book_health(session.address(), session).await,
        read(session).await,
    )
}

/// Neither read, and why: `refusal`, or where it is `not_connected`, what
/// connects.
pub fn not_read(refusal: String, not_connected: &str) -> (Result<Vec<Account>, String>, Members) {
    let why = match refusal == not_connected {
        true => super::NOT_CONNECTED.to_string(),
        false => refusal,
    };
    (Err(why.clone()), Err(why))
}

/// The time the words right after a state say it began, as they say it
/// (` for 30s, since 2026-11-24T10:00:00Z`): none where they say none that
/// reads.
fn since(after: &str) -> Option<&str> {
    let (seconds, rest) = after.strip_prefix(" for ")?.split_once('s')?;
    seconds.parse::<u64>().ok()?;
    let at = rest.strip_prefix(SINCE)?.get(..20)?;
    crate::registry::ns_of(at)
        .filter(|_| at.ends_with('Z'))
        .map(|_| at)
}

/// A member's line, where its words say its stream is stuck full or
/// at its rate limit, or refused; none otherwise.
pub fn line(member: &Value) -> Option<String> {
    let detail = member["detail"].as_str().unwrap_or_default();
    let mut said = Vec::new();
    for (opening, state) in [
        (STUCK_FULL, "write-ahead stream stuck full"),
        (RATE_LIMITED, "write-ahead stream at its rate limit"),
    ] {
        let Some(words) = detail.strip_prefix(opening) else {
            continue;
        };
        // What it holds, in its own words up to the next clause: `holds
        // 256.0 MiB of its 256.0 MiB bound`.
        let holds = words
            .split_once(": holds ")
            .map(|(_, rest)| rest.split([',', ';']).next().unwrap_or_default().trim());
        // At its rate limit, the rate its bounds allow: `74 KB/s` in
        // `(256.0 MiB every 3600s, 74 KB/s)`.
        let rate = words
            .split_once("bounds allow (")
            .and_then(|(_, rest)| rest.split_once(')'))
            .and_then(|(allowed, _)| allowed.rsplit_once(", "))
            .map(|(_, rate)| rate.trim());
        let mut state = state.to_string();
        if let Some(at) = since(words) {
            state.push_str(&format!(" since {at}"));
        }
        for (lead, figure) in [("holds ", holds), ("", rate)] {
            if let Some(figure) = figure.filter(|figure| !figure.is_empty()) {
                state.push_str(&format!(", {lead}{figure}"));
            }
        }
        said.push(state);
    }
    if let Some((_, refused)) = detail.split_once(REFUSED) {
        let mut state = "write-ahead writes refused or not answered by the broker".to_string();
        if let Some(at) = since(refused) {
            state.push_str(&format!(" since {at}"));
        }
        said.push(state);
    }
    if said.is_empty() {
        return None;
    }
    Some(format!(
        "  {}/{}: {}\n",
        member["component"].as_str().unwrap_or("?"),
        member["member"].as_str().unwrap_or("?"),
        said.join("; ")
    ))
}

/// A line each member whose stream is stuck full, at its rate limit or
/// refused; nothing where none is, or where the members were not read.
pub fn said(members: &Members) -> String {
    members.iter().flatten().filter_map(line).collect()
}

/// As `--json` adds it: every member as the deployment answered it, or why
/// they were not read.
pub fn json_of(members: &Members) -> Value {
    match members {
        Ok(members) => json!(members),
        Err(why) => json!({ "not_read": why }),
    }
}

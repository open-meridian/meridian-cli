//! `meridian partition split`, `meridian partition move` and `meridian
//! account move` (step 3d; spec/a-deployment-scales-out-and-fails-over-by-
//! configuration sections 6.1 and 6.8): a deployment admin's three acts on
//! a store's partitions, sent through the dashboard's terminal on the
//! session `meridian connect` keeps, each answered as its tool for agents
//! answers it (`dashboard__split_partition`, `dashboard__move_partition`,
//! `dashboard__move_account`).
//!
//! - **`SplitPartition`** halves a lake range at its midpoint: one
//!   transaction of the range's writer, answered with the split's record.
//! - **`MovePartition`** moves a lake range or a book partition to another
//!   writer group. It is answered at once, begun; the move's own record,
//!   `PartitionMoved`, or `PartitionMoveAborted` with its cause (lag,
//!   timeout, digest), is read from the Resilience read's history, which
//!   this follows until one shows.
//! - **`MoveAccount`** continues an account in another book partition,
//!   answered with its record once both entries are journalled.
//!
//! Each takes a reason, kept with its record. A refusal is said in the
//! deployment's own words; one that names a wait (`OVERLOADED` with a
//! retry-after: a rollout in progress, the target lagging, a switch that
//! ran out of time) says nothing was changed and when to ask again.

use std::time::Duration;

use serde_json::{json, Value};

use crate::credential::Credential;

/// SplitPartition from the terminal.
pub const SPLIT_PATH: &str = "/terminal/partitions/split";
/// MovePartition from the terminal: the lake's or the book's, by the
/// partition named.
pub const MOVE_PARTITION_PATH: &str = "/terminal/partitions/move";
/// MoveAccount from the terminal.
pub const MOVE_ACCOUNT_PATH: &str = "/terminal/accounts/move";
/// ReadResilience from the terminal: members, partitions and history.
pub const RESILIENCE_PATH: &str = "/terminal/resilience";

/// The most a reason holds, as the contract bounds it.
pub const MOST_REASON: usize = 2_000;

/// Why an act or a read was not answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// No session, or one the deployment no longer takes: connect again.
    Session(String),
    /// The deployment does not serve the route: its core is older.
    NotServed(String),
    /// The deployment refused it, in its own words.
    Refused {
        /// The refusal's reason as the terminal names it (`overloaded`,
        /// `failed_precondition`, `invalid_argument`); empty where unsaid.
        reason: String,
        said: String,
        /// When to ask again, where the refusal names a wait.
        retry_after_seconds: Option<u64>,
    },
}

impl Failed {
    /// The exit code: 3 for a session, 1 for anything else.
    pub fn code(&self) -> i32 {
        match self {
            Failed::Session(_) => 3,
            _ => 1,
        }
    }

    /// As a person reads it: nothing was changed, and when to ask again
    /// where the deployment named a wait.
    pub fn said(&self) -> String {
        match self {
            Failed::Session(said) | Failed::NotServed(said) => said.clone(),
            Failed::Refused {
                said,
                retry_after_seconds: Some(seconds),
                ..
            } => format!(
                "{}. Nothing was changed; ask again in {seconds} s.",
                said.trim_end_matches('.')
            ),
            Failed::Refused { said, .. } => said.clone(),
        }
    }

    /// As `--json` prints it.
    pub fn json(&self) -> Value {
        match self {
            Failed::Session(said) => {
                json!({ "outcome": "refused", "reason": "session", "detail": said })
            }
            Failed::NotServed(said) => {
                json!({ "outcome": "refused", "reason": "not_served", "detail": said })
            }
            Failed::Refused {
                reason,
                said,
                retry_after_seconds,
            } => {
                let mut refused = json!({ "outcome": "refused", "reason": reason, "detail": said });
                if let Some(seconds) = retry_after_seconds {
                    refused["retry_after_seconds"] = json!(seconds);
                }
                refused
            }
        }
    }
}

/// A count as core writes it, or as the proto's JSON mapping writes a
/// 64-bit one: a string.
fn number(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|said| said.trim().parse().ok()))
}

/// What a refusal's body and headers say: the reason, the words, and the
/// wait, from `retry_after_seconds` or the `Retry-After` header.
fn refusal(
    address: &str,
    status: reqwest::StatusCode,
    body: &str,
    headers: &reqwest::header::HeaderMap,
) -> Failed {
    let said: Value = serde_json::from_str(body).unwrap_or_default();
    let retry_after_seconds = number(&said["retry_after_seconds"]).or_else(|| {
        headers
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok())
    });
    if retry_after_seconds.is_none() || status == reqwest::StatusCode::UNAUTHORIZED {
        // Every other refusal as the rest of the terminal's are read: a
        // session apart, a version refused, and the words after the status.
        return match crate::live::refusal(address, status, body) {
            crate::live::Failed::Session(said) => Failed::Session(said),
            crate::live::Failed::Refused(words) => Failed::Refused {
                reason: said["reason"].as_str().unwrap_or_default().to_string(),
                said: words,
                retry_after_seconds: None,
            },
        };
    }
    Failed::Refused {
        reason: said["reason"].as_str().unwrap_or("overloaded").to_string(),
        said: said["error"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| body.trim().to_string()),
        retry_after_seconds,
    }
}

async fn call(
    address: &str,
    session: &Credential,
    path: &str,
    body: Option<Value>,
) -> Result<Value, Failed> {
    let reach = |failed: crate::live::Failed| match failed {
        crate::live::Failed::Session(said) => Failed::Session(said),
        crate::live::Failed::Refused(said) => Failed::Refused {
            reason: String::new(),
            said,
            retry_after_seconds: None,
        },
    };
    let http = crate::catalogue::http().map_err(reach)?;
    let url = format!("{address}{path}");
    let body = body.map(|body| body.to_string());
    let (status, text, headers) = session
        .send(|bearer| match &body {
            Some(body) => http
                .post(url.clone())
                .bearer_auth(bearer)
                .header("content-type", "application/json")
                .body(body.clone()),
            None => http.get(url.clone()).bearer_auth(bearer),
        })
        .await
        .map_err(reach)?;
    let says_why = serde_json::from_str::<Value>(&text).is_ok_and(|said| said["error"].is_string());
    if status == reqwest::StatusCode::NOT_FOUND && !says_why {
        return Err(Failed::NotServed(format!(
            "{address} does not answer this from a terminal: its core is older than step 3d's \
             moves and splits. The Resilience page makes them where its core has them"
        )));
    }
    if !status.is_success() {
        return Err(refusal(address, status, &text, &headers));
    }
    serde_json::from_str(&text).map_err(|failed| Failed::Refused {
        reason: String::new(),
        said: format!("{address} answered what does not read: {failed}"),
        retry_after_seconds: None,
    })
}

/// A reason as every act takes one: given, and at most 2,000 characters.
pub fn reason(given: Option<&str>) -> Result<&str, String> {
    let given = given.map(str::trim).unwrap_or_default();
    if given.is_empty() {
        return Err(
            "--reason <why> is required: the deployment keeps it with the act's record".into(),
        );
    }
    if given.chars().count() > MOST_REASON {
        return Err("--reason is at most 2,000 characters".into());
    }
    Ok(given)
}

/// Whether `partition` names one of the book's (`P0` and those after it);
/// any other is the lake's.
pub fn is_book_partition(partition: &str) -> bool {
    partition
        .strip_prefix('P')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The store a partition is of, as its records name it.
pub fn component_of(partition: &str) -> &'static str {
    if is_book_partition(partition) {
        "bor"
    } else {
        "lake"
    }
}

/// SplitPartition: the split's record, `{"split": {...}}`.
pub async fn split(
    address: &str,
    session: &Credential,
    partition: &str,
    reason: &str,
) -> Result<Value, Failed> {
    call(
        address,
        session,
        SPLIT_PATH,
        Some(json!({ "partition": partition, "note": reason })),
    )
    .await
}

/// MovePartition: begun, `{"move": {partition, from_group, started_at_ns}}`.
pub async fn move_partition(
    address: &str,
    session: &Credential,
    partition: &str,
    target_group: u32,
    reason: &str,
) -> Result<Value, Failed> {
    call(
        address,
        session,
        MOVE_PARTITION_PATH,
        Some(json!({ "partition": partition, "target_group": target_group, "note": reason })),
    )
    .await
}

/// MoveAccount: the move's record, `{"moved": {...}}`.
pub async fn move_account(
    address: &str,
    session: &Credential,
    account: &str,
    target_partition: &str,
    reason: &str,
) -> Result<Value, Failed> {
    call(
        address,
        session,
        MOVE_ACCOUNT_PATH,
        Some(
            json!({ "account_id": account, "target_partition": target_partition, "note": reason }),
        ),
    )
    .await
}

/// ReadResilience: `{members, partitions, history, next_cursor}`.
pub async fn resilience(address: &str, session: &Credential) -> Result<Value, Failed> {
    call(address, session, RESILIENCE_PATH, None).await
}

/// A page of ReadResilience's history, `page_size` items from `cursor` (the
/// page before's `next_cursor`, empty for the first), as the tool takes them.
pub async fn resilience_page(
    address: &str,
    session: &Credential,
    page_size: u32,
    cursor: &str,
) -> Result<Value, Failed> {
    let asked = std::collections::BTreeMap::from([
        ("cursor".to_string(), cursor.to_string()),
        ("page_size".to_string(), page_size.to_string()),
    ]);
    let path = format!("{RESILIENCE_PATH}?{}", crate::up::form_encoded(&asked));
    call(address, session, &path, None).await
}

/// The history `meridian doctor` reads each shard's last takeover from
/// (step 3d, item 32), through the session `meridian connect` keeps.
pub struct SessionHistory {
    pub session: Credential,
}

#[async_trait::async_trait]
impl crate::doctor::shards::History for SessionHistory {
    async fn page(&self, page_size: u32, cursor: &str) -> Result<Value, String> {
        let address = self.session.address().to_string();
        resilience_page(&address, &self.session, page_size, cursor)
            .await
            .map_err(|failed| failed.said())
    }
}

/// A partition's move, begun.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moving {
    pub partition: String,
    pub from_group: u64,
    pub target_group: u32,
    /// By the store's clock; 0 where the answer did not say.
    pub started_at_ns: u64,
}

impl Moving {
    /// From MovePartition's answer.
    pub fn begun(partition: &str, target_group: u32, answer: &Value) -> Moving {
        Moving {
            partition: partition.to_string(),
            from_group: number(&answer["move"]["from_group"]).unwrap_or_default(),
            target_group,
            started_at_ns: number(&answer["move"]["started_at_ns"]).unwrap_or_default(),
        }
    }
}

/// Where a move begun stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stands {
    /// PartitionMoved is recorded: the target owns it.
    Moved(String),
    /// PartitionMoveAborted is recorded, with its cause: the source still
    /// owns it.
    Aborted(String),
    /// Neither yet.
    Moving,
}

/// Where the move stands, from a Resilience read: its own record in the
/// history, written since it began, named by its `component` and
/// `partition` (by its words' start, from a core whose history carries
/// neither); else the partition's owner, once that is the target group.
pub fn stands(resilience: &Value, moving: &Moving) -> Stands {
    let component = component_of(&moving.partition);
    let named = format!("{component} {}:", moving.partition);
    for item in resilience["history"].as_array().into_iter().flatten() {
        let words = item["words"].as_str().unwrap_or_default();
        let its_own = if item["component"].is_null() && item["partition"].is_null() {
            words.starts_with(&named)
        } else {
            item["component"] == component && item["partition"] == moving.partition.as_str()
        };
        if !its_own || number(&item["at_ns"]).unwrap_or_default() < moving.started_at_ns {
            continue;
        }
        match item["kind"].as_str() {
            Some("partition_moved") => return Stands::Moved(words.to_string()),
            Some("move_aborted") => return Stands::Aborted(words.to_string()),
            _ => {}
        }
    }
    let owned = resilience["partitions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|one| one["component"] == component && one["partition"] == moving.partition.as_str());
    match owned {
        Some(owned)
            if number(&owned["group"]) == Some(u64::from(moving.target_group))
                && moving.from_group != u64::from(moving.target_group) =>
        {
            Stands::Moved(format!(
                "{named} with writer group {} now, epoch {}",
                moving.target_group,
                number(&owned["epoch"]).unwrap_or_default()
            ))
        }
        _ => Stands::Moving,
    }
}

/// How a move is followed: how often it is read, and for how long.
#[derive(Debug, Clone, Copy)]
pub struct Following {
    pub every: Duration,
    pub within: Duration,
}

/// Read the Resilience read until the move's record shows, or the time
/// runs out: `Stands::Moving` then, which is no failure of the move.
pub async fn follow(
    address: &str,
    session: &Credential,
    moving: &Moving,
    following: Following,
) -> Result<Stands, Failed> {
    let started = tokio::time::Instant::now();
    loop {
        let stands = stands(&resilience(address, session).await?, moving);
        if stands != Stands::Moving || started.elapsed() >= following.within {
            return Ok(stands);
        }
        tokio::time::sleep(following.every).await;
    }
}

/// A time core says in nanoseconds since the epoch, as a person reads it.
fn moment(at: &Value) -> String {
    match number(at) {
        Some(ns) if ns > 0 => crate::registry::moment(ns),
        _ => "an unstated time".to_string(),
    }
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

/// A split, as `partition split` prints it.
pub fn split_text(answer: &Value) -> String {
    let split = &answer["split"];
    let children: Vec<&str> = split["children"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    format!(
        "Split: {} closed at sequence {} and continues as {}, each with its parent's writer \
         group.\n  by {}, {}\n  reason: {}\n",
        text(&split["parent"]),
        number(&split["closing_sequence"]).unwrap_or_default(),
        children.join(" and "),
        text(&split["split_by"]),
        moment(&split["split_at_ns"]),
        text(&split["reason"]),
    )
}

/// A partition's move begun, as `partition move` prints it.
pub fn begun_text(moving: &Moving) -> String {
    format!(
        "Begun: {} {} moves from writer group {} to {} (started {}). Writes to it are held \
         for a moment at the switch, never refused inside the hold.\n",
        component_of(&moving.partition),
        moving.partition,
        moving.from_group,
        moving.target_group,
        match moving.started_at_ns {
            0 => "at an unstated time".to_string(),
            ns => crate::registry::moment(ns),
        },
    )
}

/// Where the move stood when the following ended.
pub fn stands_text(stands: &Stands, waited: Duration) -> String {
    match stands {
        Stands::Moved(record) => format!("Moved: {record}.\n"),
        Stands::Aborted(record) => format!(
            "Not moved: {record}. Its source still owns it, and any writes held went to it. \
             The move may be asked again.\n"
        ),
        Stands::Moving => format!(
            "Still moving after {} s: a move across databases copies the partition first. Its \
             record, moved or not switched with its cause, shows in the Resilience page's \
             History; nothing more is needed here.\n",
            waited.as_secs()
        ),
    }
}

/// The move and where it stands, as `partition move --json` prints it.
pub fn move_json(answer: &Value, stands: Option<&Stands>) -> Value {
    let mut out = answer.clone();
    match stands {
        Some(Stands::Moved(record)) => {
            out["outcome"] = json!("moved");
            out["record"] = json!(record);
        }
        Some(Stands::Aborted(record)) => {
            out["outcome"] = json!("aborted");
            out["record"] = json!(record);
        }
        Some(Stands::Moving) | None => out["outcome"] = json!("moving"),
    }
    out
}

/// An account's move, as `account move` prints it.
pub fn account_text(answer: &Value) -> String {
    let moved = &answer["moved"];
    format!(
        "Moved: account {} continues in {}, from {}: closed at sequence {} there, opened at \
         {} here. Its positions, lots and orders read as before.\n  by {}, {}\n  reason: {}\n",
        text(&moved["account_id"]),
        text(&moved["to_partition"]),
        text(&moved["from_partition"]),
        number(&moved["closing_sequence"]).unwrap_or_default(),
        number(&moved["opening_sequence"]).unwrap_or_default(),
        text(&moved["moved_by"]),
        moment(&moved["moved_at_ns"]),
        text(&moved["reason"]),
    )
}

#[cfg(test)]
mod tests;

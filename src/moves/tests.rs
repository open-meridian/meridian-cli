use std::sync::{Arc, Mutex};

use serde_json::json;

use super::*;
use crate::fake_http::{serve, Asked, Kept, Reply};

const NS: u64 = 1_794_000_000_000_000_000;

/// A deployment whose terminal answers the three acts and the Resilience
/// read: `eq:bars` moves once its history has been read `reads` times, or
/// is not switched for `cause`.
async fn deployment(reads: u32, cause: Option<&'static str>) -> (String, Kept) {
    let seen = Arc::new(Mutex::new(0u32));
    serve(move |asked: &Asked| {
        let body = asked.json();
        match (asked.method.as_str(), asked.path.as_str()) {
            ("POST", SPLIT_PATH) => Reply::json(
                200,
                json!({ "split": {
                    "parent": body["partition"], "children": ["eq:bars/00000000-7fffffff", "eq:bars/80000000-ffffffff"],
                    "closing_sequence": 412, "split_by": "u-ada", "reason": body["note"],
                    "split_at_ns": NS.to_string(),
                } }),
            ),
            ("POST", MOVE_PARTITION_PATH) if body["partition"] == "fx:rates" => Reply::json(
                409,
                json!({ "error": "partition: a move of fx:rates is under way", "reason": "failed_precondition" }),
            ),
            ("POST", MOVE_PARTITION_PATH) => Reply::json(
                200,
                json!({ "move": { "partition": body["partition"], "from_group": 0,
                                  "started_at_ns": NS.to_string() } }),
            ),
            ("POST", MOVE_ACCOUNT_PATH) => match body["account_id"].as_str() {
                // A rollout in progress, the target lagging, a switch out of
                // time: OVERLOADED with a retry-after, in the body or the
                // header.
                Some("ACC-ROLLOUT") => Reply::json(
                    503,
                    json!({ "error": "the move of account ACC-ROLLOUT to P1 waits for a rollout in progress: 1 member behind; the account stays in P0; ask again",
                            "reason": "overloaded", "retry_after_seconds": 5 }),
                ),
                Some("ACC-LAG") => Reply::json(
                    503,
                    json!({ "error": "the move of account ACC-LAG to P1 was not switched: its target still lagged when the switch's time ran out." }),
                )
                .with("retry-after", "30"),
                Some("ACC-DIGEST") => Reply::json(
                    409,
                    json!({ "error": "account_id: the two digests differed", "reason": "failed_precondition" }),
                ),
                _ => Reply::json(
                    200,
                    json!({ "moved": {
                        "account_id": body["account_id"], "from_partition": "P0", "closing_sequence": 41,
                        "to_partition": body["target_partition"], "opening_sequence": 1,
                        "moved_by": "u-ada", "reason": body["note"], "moved_at_ns": NS.to_string(),
                    } }),
                ),
            },
            ("GET", RESILIENCE_PATH) => {
                let mut seen = seen.lock().unwrap();
                *seen += 1;
                // An older record of the same partition, from before this move.
                let mut history = vec![json!({
                    "at_ns": (NS - 1).to_string(), "kind": "move_aborted",
                    "component": "lake", "partition": "eq:bars",
                    "words": "lake eq:bars: to group 1 not switched, the two digests differed",
                })];
                if *seen >= reads {
                    history.insert(0, match cause {
                        Some(cause) => json!({ "at_ns": (NS + 7).to_string(), "kind": "move_aborted",
                            "component": "lake", "partition": "eq:bars",
                            "words": format!("lake eq:bars: to group 1 not switched, {cause}") }),
                        None => json!({ "at_ns": (NS + 7).to_string(), "kind": "partition_moved",
                            "component": "lake", "partition": "eq:bars",
                            "words": "lake eq:bars: group 0 to 1, epoch 9" }),
                    });
                }
                Reply::json(
                    200,
                    json!({ "members": [], "history": history, "next_cursor": "",
                            "partitions": [{ "component": "lake", "partition": "eq:bars", "group": 0,
                                             "writer": "om-lake-0", "epoch": 8 }] }),
                )
            }
            _ => Reply::bytes(404, "text/plain", Vec::new()),
        }
    })
    .await
}

fn posted(kept: &Kept, path: &str) -> Vec<serde_json::Value> {
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
async fn a_split_sends_the_partition_and_the_reason_and_says_its_record() {
    let (address, kept) = deployment(1, None).await;
    let session = Credential::terminal_session(&address, "a-session");
    let answer = split(
        &address,
        &session,
        "eq:bars",
        "it holds half the lake's writes",
    )
    .await
    .unwrap();
    assert_eq!(
        posted(&kept, SPLIT_PATH),
        [json!({ "partition": "eq:bars", "note": "it holds half the lake's writes" })]
    );
    assert_eq!(
        kept.lock().unwrap()[0].header("authorization"),
        Some("Bearer a-session")
    );
    let text = split_text(&answer);
    assert!(
        text.starts_with(
            "Split: eq:bars closed at sequence 412 and continues as eq:bars/00000000-7fffffff \
             and eq:bars/80000000-ffffffff"
        ),
        "{text}"
    );
    assert!(text.contains("by u-ada, 2026-"), "{text}");
    assert!(
        text.contains("reason: it holds half the lake's writes"),
        "{text}"
    );
}

#[tokio::test]
async fn a_partitions_move_is_begun_at_once_and_followed_to_its_record() {
    let (address, kept) = deployment(3, None).await;
    let session = Credential::terminal_session(&address, "a-session");
    let answer = move_partition(
        &address,
        &session,
        "eq:bars",
        1,
        "group 0 is at its ceiling",
    )
    .await
    .unwrap();
    assert_eq!(
        posted(&kept, MOVE_PARTITION_PATH),
        [json!({ "partition": "eq:bars", "target_group": 1, "note": "group 0 is at its ceiling" })]
    );
    let moving = Moving::begun("eq:bars", 1, &answer);
    assert_eq!(
        moving,
        Moving {
            partition: "eq:bars".into(),
            from_group: 0,
            target_group: 1,
            started_at_ns: NS
        }
    );
    assert!(
        begun_text(&moving).starts_with("Begun: lake eq:bars moves from writer group 0 to 1"),
        "{}",
        begun_text(&moving)
    );
    // Its record shows on the third read: the one from before it began is
    // never taken for it.
    let stands = follow(&address, &session, &moving, QUICK).await.unwrap();
    assert_eq!(
        stands,
        Stands::Moved("lake eq:bars: group 0 to 1, epoch 9".into())
    );
    let reads = kept
        .lock()
        .unwrap()
        .iter()
        .filter(|a| a.path == RESILIENCE_PATH)
        .count();
    assert_eq!(reads, 3);
    assert_eq!(
        stands_text(&stands, QUICK.within),
        "Moved: lake eq:bars: group 0 to 1, epoch 9.\n"
    );
    let printed = move_json(&answer, Some(&stands));
    assert_eq!(printed["outcome"], "moved");
    assert_eq!(printed["move"]["from_group"], 0);
}

#[tokio::test]
async fn a_move_that_did_not_switch_says_its_cause_and_that_the_source_owns_it() {
    let (address, _) = deployment(2, Some("its switch did not finish in time")).await;
    let session = Credential::terminal_session(&address, "a-session");
    let answer = move_partition(&address, &session, "eq:bars", 1, "rebalance")
        .await
        .unwrap();
    let moving = Moving::begun("eq:bars", 1, &answer);
    let stands = follow(&address, &session, &moving, QUICK).await.unwrap();
    let text = stands_text(&stands, QUICK.within);
    assert!(
        text.starts_with(
            "Not moved: lake eq:bars: to group 1 not switched, its switch did not finish in time."
        ),
        "{text}"
    );
    assert!(text.contains("Its source still owns it"), "{text}");
    assert_eq!(move_json(&answer, Some(&stands))["outcome"], "aborted");
}

#[tokio::test]
async fn a_move_still_under_way_when_the_time_runs_out_is_left_and_said() {
    let (address, _) = deployment(u32::MAX, None).await;
    let session = Credential::terminal_session(&address, "a-session");
    let answer = move_partition(&address, &session, "eq:bars", 1, "rebalance")
        .await
        .unwrap();
    let moving = Moving::begun("eq:bars", 1, &answer);
    let brief = Following {
        every: Duration::from_millis(1),
        within: Duration::from_millis(20),
    };
    let stands = follow(&address, &session, &moving, brief).await.unwrap();
    assert_eq!(stands, Stands::Moving);
    assert!(stands_text(&stands, Duration::from_secs(120)).starts_with("Still moving after 120 s"));
    assert_eq!(move_json(&answer, Some(&stands))["outcome"], "moving");
}

#[test]
fn a_move_with_no_record_yet_is_read_from_the_partitions_owner() {
    let moving = Moving {
        partition: "P1".into(),
        from_group: 0,
        target_group: 1,
        started_at_ns: NS,
    };
    let owned = |group: u64| {
        json!({ "history": [], "partitions": [
            { "component": "lake", "partition": "P1", "group": 1, "epoch": 2 },
            { "component": "bor", "partition": "P1", "group": group, "epoch": 5 },
        ] })
    };
    assert_eq!(stands(&owned(0), &moving), Stands::Moving);
    assert_eq!(
        stands(&owned(1), &moving),
        Stands::Moved("bor P1: with writer group 1 now, epoch 5".into())
    );
}

#[test]
fn a_moves_record_is_its_own_by_its_component_and_partition() {
    let moving = Moving {
        partition: "eq:bars".into(),
        from_group: 0,
        target_group: 1,
        started_at_ns: NS,
    };
    let history = |items: serde_json::Value| json!({ "history": items, "partitions": [] });
    // Another partition's record, the book's partition of a like name, an
    // account's aborted move and a split are never taken for it, whatever
    // their words.
    let others = history(json!([
        { "at_ns": (NS + 1).to_string(), "kind": "partition_moved", "component": "lake",
          "partition": "eq:bars/00000000-7fffffff", "words": "lake eq:bars: group 0 to 1, epoch 9" },
        { "at_ns": (NS + 2).to_string(), "kind": "partition_moved", "component": "bor",
          "partition": "eq:bars", "words": "lake eq:bars: group 0 to 1, epoch 9" },
        { "at_ns": (NS + 3).to_string(), "kind": "partition_split", "component": "lake",
          "partition": "eq:bars", "words": "lake eq:bars: into two" },
    ]));
    assert_eq!(stands(&others, &moving), Stands::Moving);
    let own = history(json!([
        { "at_ns": (NS + 4).to_string(), "kind": "partition_moved", "component": "lake",
          "partition": "eq:bars", "words": "moved, in words of any shape" },
    ]));
    assert_eq!(
        stands(&own, &moving),
        Stands::Moved("moved, in words of any shape".into())
    );
    // A core whose history carries neither field: by its words' start.
    let older = history(json!([
        { "at_ns": (NS + 4).to_string(), "kind": "move_aborted",
          "words": "lake eq:bars: to group 1 not switched, the two digests differed" },
        { "at_ns": (NS + 5).to_string(), "kind": "partition_moved",
          "words": "lake eq:bars/00000000-7fffffff: group 0 to 1, epoch 9" },
    ]));
    assert_eq!(
        stands(&older, &moving),
        Stands::Aborted("lake eq:bars: to group 1 not switched, the two digests differed".into())
    );
}

#[tokio::test]
async fn a_move_refused_while_another_is_under_way_is_said_in_the_deployments_words() {
    let (address, _) = deployment(1, None).await;
    let session = Credential::terminal_session(&address, "a-session");
    let refused = move_partition(&address, &session, "fx:rates", 1, "rebalance")
        .await
        .unwrap_err();
    assert_eq!(refused.code(), 1);
    assert!(
        refused
            .said()
            .contains("partition: a move of fx:rates is under way"),
        "{}",
        refused.said()
    );
    assert_eq!(refused.json()["reason"], "failed_precondition");
    assert!(refused.json().get("retry_after_seconds").is_none());
}

#[tokio::test]
async fn an_accounts_move_sends_the_account_and_its_target_and_says_its_record() {
    let (address, kept) = deployment(1, None).await;
    let session = Credential::terminal_session(&address, "a-session");
    let answer = move_account(
        &address,
        &session,
        "ACC-7",
        "P2",
        "the fund's accounts together",
    )
    .await
    .unwrap();
    assert_eq!(
        posted(&kept, MOVE_ACCOUNT_PATH),
        [json!({ "account_id": "ACC-7", "target_partition": "P2",
                 "note": "the fund's accounts together" })]
    );
    let text = account_text(&answer);
    assert!(
        text.starts_with(
            "Moved: account ACC-7 continues in P2, from P0: closed at sequence 41 there, \
             opened at 1 here."
        ),
        "{text}"
    );
    assert!(text.contains("by u-ada"), "{text}");
}

#[tokio::test]
async fn overloaded_says_nothing_changed_and_when_to_ask_again() {
    let (address, _) = deployment(1, None).await;
    let session = Credential::terminal_session(&address, "a-session");

    // A rollout in progress: the wait in the body.
    let rollout = move_account(&address, &session, "ACC-ROLLOUT", "P1", "rebalance")
        .await
        .unwrap_err();
    assert_eq!(
        rollout,
        Failed::Refused {
            reason: "overloaded".into(),
            said: "the move of account ACC-ROLLOUT to P1 waits for a rollout in progress: 1 \
                   member behind; the account stays in P0; ask again"
                .into(),
            retry_after_seconds: Some(5),
        }
    );
    assert!(
        rollout
            .said()
            .ends_with("ask again. Nothing was changed; ask again in 5 s."),
        "{}",
        rollout.said()
    );
    assert_eq!(rollout.json()["retry_after_seconds"], 5);
    assert_eq!(rollout.code(), 1);

    // Lag or a timeout: the wait in the Retry-After header alone.
    let lag = move_account(&address, &session, "ACC-LAG", "P1", "rebalance")
        .await
        .unwrap_err();
    assert!(
        lag.said().ends_with(
            "its target still lagged when the switch's time ran out. Nothing was changed; ask \
             again in 30 s."
        ),
        "{}",
        lag.said()
    );
    assert_eq!(lag.json()["reason"], "overloaded");

    // A digest: refused, with no wait named.
    let digest = move_account(&address, &session, "ACC-DIGEST", "P1", "rebalance")
        .await
        .unwrap_err();
    assert!(
        digest.said().contains("the two digests differed") && !digest.said().contains("ask again"),
        "{}",
        digest.said()
    );
}

#[tokio::test]
async fn an_older_core_and_a_lapsed_session_are_each_said_plainly() {
    let (address, _) = serve(|asked: &Asked| match asked.path.as_str() {
        SPLIT_PATH => Reply::bytes(404, "text/plain", Vec::new()),
        _ => Reply::json(401, json!({ "error": "invalid_token", "reason": "lapsed" })),
    })
    .await;
    let session = Credential::terminal_session(&address, "a-session");
    let older = split(&address, &session, "eq:bars", "why")
        .await
        .unwrap_err();
    assert!(
        matches!(&older, Failed::NotServed(said) if said.contains("older than step 3d")),
        "{older:?}"
    );
    let lapsed = resilience(&address, &session).await.unwrap_err();
    assert_eq!(lapsed.code(), 3);
    assert!(
        lapsed.said().contains("meridian connect"),
        "{}",
        lapsed.said()
    );
}

#[test]
fn a_reason_is_required_and_bounded() {
    assert!(reason(None)
        .unwrap_err()
        .contains("--reason <why> is required"));
    assert!(reason(Some("  ")).is_err());
    assert_eq!(reason(Some(" rebalance ")), Ok("rebalance"));
    assert!(reason(Some(&"x".repeat(2_001))).is_err());
    assert!(reason(Some(&"x".repeat(2_000))).is_ok());
}

#[test]
fn a_partition_is_the_books_by_its_name() {
    assert_eq!(component_of("P0"), "bor");
    assert_eq!(component_of("P12"), "bor");
    assert_eq!(component_of("P"), "lake");
    assert_eq!(component_of("eq:bars/00000000-7fffffff"), "lake");
}

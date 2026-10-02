//! Refreshing against a deployment stand-in: before a lapse, after a refusal
//! as expired, once for two processes at once, and said as the session when
//! the deployment will not.

use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("meridian-credential-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A deployment stand-in. `/oauth/token` answers the next pair, numbered,
/// after `pause`, or `refused` when one is given; `/terminal/probe` answers
/// 401 expired to any access token but the newest, so a request on a stale
/// one is refused as the dashboard refuses it.
struct Stand {
    address: String,
    tokens: Arc<AtomicUsize>,
    probes: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn stand(pause: std::time::Duration, refused: Option<&'static str>) -> Stand {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let tokens = Arc::new(AtomicUsize::new(0));
    let probes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (counted, seen) = (tokens.clone(), probes.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (counted, seen) = (counted.clone(), seen.clone());
            tokio::spawn(async move {
                let mut read = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    read.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&read).to_string();
                    let Some((head, rest)) = text.split_once("\r\n\r\n") else {
                        continue;
                    };
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if rest.len() < length {
                        continue;
                    }
                    let path = head.split(' ').nth(1).unwrap_or_default().to_string();
                    let bearer = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("authorization: bearer ")
                                .map(String::from)
                        })
                        .unwrap_or_default();
                    let (status, body) = if path == "/oauth/token" {
                        tokio::time::sleep(pause).await;
                        match refused {
                            Some(reason) => (
                                "400 Bad Request",
                                format!(
                                    r#"{{"error":"invalid_grant","error_description":"no","reason":"{reason}"}}"#
                                ),
                            ),
                            None => {
                                let n = counted.fetch_add(1, Ordering::SeqCst) + 1;
                                (
                                    "200 OK",
                                    format!(
                                        r#"{{"access_token":"mda_{n}","refresh_token":"mdr_{n}","expires_in":600,"subject":"local|ada","delegation_expires_at":"2026-12-25T00:00:00Z","delegation_expires_in":7776000}}"#
                                    ),
                                )
                            }
                        }
                    } else {
                        seen.lock().unwrap().push(bearer.clone());
                        let issued = counted.load(Ordering::SeqCst);
                        if issued > 0 && bearer == format!("mda_{issued}") {
                            ("200 OK", r#"{"ok":true}"#.to_string())
                        } else {
                            (
                                "401 Unauthorized",
                                r#"{"error":"invalid_token","reason":"expired"}"#.to_string(),
                            )
                        }
                    };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(reply.as_bytes()).await;
                    let _ = stream.shutdown().await;
                    return;
                }
            });
        }
    });
    Stand {
        address,
        tokens,
        probes,
    }
}

/// A delegation's pair kept for `address`, its access token with `left`
/// seconds to go.
fn kept(within: &std::path::Path, address: &str, left: i64) -> Held {
    let held = Held {
        address: address.to_string(),
        subject: "local|ada".into(),
        expires_at: "2026-12-25T00:00:00Z".into(),
        client_id: "mdc_laptop".into(),
        access_token: "mda_0".into(),
        access_expires_at_s: (now_s() as i64 + left) as u64,
        refresh_token: "mdr_0".into(),
        expires_at_s: now_s() + 30 * 24 * 60 * 60,
        ..Held::default()
    };
    sessions::write(within, &held).unwrap();
    held
}

#[tokio::test]
async fn a_fresh_access_token_is_presented_as_it_is() {
    let stand = stand(std::time::Duration::ZERO, None).await;
    let within = scratch("fresh");
    let held = kept(&within, &stand.address, 300);
    let credential = Credential::new(within.clone(), held);
    assert_eq!(credential.bearer().await.unwrap(), "mda_0");
    assert_eq!(stand.tokens.load(Ordering::SeqCst), 0, "nothing refreshed");
    let _ = std::fs::remove_dir_all(&within);
}

#[tokio::test]
async fn under_a_minute_left_is_refreshed_first_and_the_next_pair_kept() {
    let stand = stand(std::time::Duration::ZERO, None).await;
    let within = scratch("lapsing");
    let held = kept(&within, &stand.address, 30);
    let credential = Credential::new(within.clone(), held);
    assert_eq!(credential.bearer().await.unwrap(), "mda_1");
    let written = sessions::read(&within, &stand.address).unwrap();
    assert_eq!(written.access_token, "mda_1");
    assert_eq!(
        written.refresh_token, "mdr_1",
        "the spent one is never kept"
    );
    assert_eq!(written.client_id, "mdc_laptop");
    assert!(written.access_expires_at_s >= now_s() + 599);
    let _ = std::fs::remove_dir_all(&within);
}

#[tokio::test]
async fn a_request_refused_as_expired_is_refreshed_and_sent_again_once() {
    let stand = stand(std::time::Duration::ZERO, None).await;
    let within = scratch("expired");
    // The clock says five minutes left; the deployment says expired.
    let held = kept(&within, &stand.address, 300);
    let credential = Credential::new(within.clone(), held);
    let http = reqwest::Client::new();
    let url = format!("{}/terminal/probe", stand.address);
    let (status, _, _) = credential
        .send(|bearer| http.get(&url).bearer_auth(bearer))
        .await
        .unwrap();
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(*stand.probes.lock().unwrap(), vec!["mda_0", "mda_1"]);
    let _ = std::fs::remove_dir_all(&within);
}

#[tokio::test]
async fn two_processes_refreshing_at_once_spend_the_refresh_token_once() {
    // A slow deployment, so the second is waiting on the lock while the
    // first refreshes: it must find the first's pair, not spend mdr_0 again.
    let stand = stand(std::time::Duration::from_millis(300), None).await;
    let within = scratch("together");
    let held = kept(&within, &stand.address, 10);
    let first = Credential::new(within.clone(), held.clone());
    let second = Credential::new(within.clone(), held);
    let (a, b) = tokio::join!(first.bearer(), second.bearer());
    assert_eq!(
        stand.tokens.load(Ordering::SeqCst),
        1,
        "one refresh between them"
    );
    assert_eq!(a.unwrap(), "mda_1");
    assert_eq!(b.unwrap(), "mda_1");
    let _ = std::fs::remove_dir_all(&within);
}

#[tokio::test]
async fn a_revoked_delegation_is_said_as_the_session_with_the_command_to_connect() {
    let stand = stand(std::time::Duration::ZERO, Some("revoked")).await;
    let within = scratch("revoked");
    let held = kept(&within, &stand.address, 0);
    let credential = Credential::new(within.clone(), held);
    let failed = credential.bearer().await.unwrap_err();
    assert_eq!(failed.code(), 3, "{failed:?}");
    assert!(failed.said().contains("was revoked"), "{failed:?}");
    assert!(
        failed.said().ends_with(&format!(
            "`meridian connect {}` to connect again",
            stand.address
        )),
        "{failed:?}"
    );
    let _ = std::fs::remove_dir_all(&within);
}

#[tokio::test]
async fn signed_out_in_another_terminal_is_not_connected_any_more() {
    let stand = stand(std::time::Duration::ZERO, None).await;
    let within = scratch("gone");
    let held = kept(&within, &stand.address, 0);
    sessions::forget(&within, &stand.address).unwrap();
    let failed = Credential::new(within.clone(), held)
        .bearer()
        .await
        .unwrap_err();
    assert_eq!(failed.code(), 3);
    assert_eq!(stand.tokens.load(Ordering::SeqCst), 0, "nothing presented");
    let _ = std::fs::remove_dir_all(&within);
}

#[tokio::test]
async fn an_older_clis_terminal_session_is_presented_and_never_refreshed() {
    let credential = Credential::terminal_session("http://127.0.0.1:1", "s3cr3t");
    assert_eq!(credential.bearer().await.unwrap(), "s3cr3t");
    assert_eq!(credential.refreshed("s3cr3t").await.unwrap(), "s3cr3t");
}

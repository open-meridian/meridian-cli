//! `meridian connect` and `meridian sign-out`: W6.13 and W6.14 from the
//! terminal's side.
//!
//! The CLI never takes a password. It listens on a loopback port, opens the
//! deployment's terminal sign-in in a browser with a PKCE challenge
//! (RFC 7636) and a state, and waits for the browser to be sent back with a
//! one-time code (RFC 8252). The person signs in however that deployment
//! signs people in, afresh, and confirms. The code and the verifier only
//! this process holds become a session, which is kept in the sessions file
//! and presented as a bearer credential from then on.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rand::RngCore as _;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// How long `connect` waits for the person to sign in. Inside the ten
/// minutes the deployment holds a terminal's request for.
pub const WAIT: Duration = Duration::from_secs(5 * 60);

/// A verifier and the challenge that names it.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

fn random() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

impl Pkce {
    pub fn new() -> Pkce {
        let verifier = random();
        let challenge = challenge_for(&verifier);
        Pkce {
            verifier,
            challenge,
        }
    }
}

pub fn state() -> String {
    random()
}

/// The deployment's address as given, checked. HTTPS, or HTTP to this
/// machine only -- a local cluster reached through a port-forward. A
/// session sent in the clear to anywhere else is a session anybody on the
/// way holds.
pub fn address(given: &str) -> Result<String, String> {
    let trimmed = given.trim().trim_end_matches('/');
    let Some((scheme, rest)) = trimmed.split_once("://") else {
        return Err(format!(
            "{given} is not an address: give it as https://<host>, or http://127.0.0.1:<port> for a local one"
        ));
    };
    if rest.is_empty() || rest.contains('/') || rest.contains('?') || rest.contains('#') {
        return Err(format!(
            "{given} should be the dashboard's address alone, with no path"
        ));
    }
    let host = rest
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map(|(host, _)| host)
        .unwrap_or(rest);
    let local = matches!(host, "127.0.0.1" | "[::1]" | "localhost");
    match scheme {
        "https" => Ok(trimmed.to_string()),
        "http" if local => Ok(trimmed.to_string()),
        "http" => Err(format!(
            "{given} is plain HTTP to another machine, which would send your session in the clear; use https://"
        )),
        _ => Err(format!("{given} is neither https:// nor http://")),
    }
}

/// Percent-encoding for a query or a form, keeping only what needs none.
fn encoded(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{key}={}", encoded(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn authorize_url(address: &str, redirect_uri: &str, pkce: &Pkce, state: &str) -> String {
    format!(
        "{address}/terminal/authorize?{}",
        form(&[
            ("redirect_uri", redirect_uri),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
        ])
    )
}

/// Where the browser comes back to: the loopback interface, an ephemeral
/// port, and the path the deployment insists on.
pub async fn listen() -> Result<(TcpListener, String), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|failed| format!("could not listen on this machine's loopback: {failed}"))?;
    let port = listener
        .local_addr()
        .map_err(|failed| failed.to_string())?
        .port();
    Ok((listener, format!("http://127.0.0.1:{port}/callback")))
}

/// What came back to the loopback address.
#[derive(Debug, PartialEq, Eq)]
pub enum Returned {
    Code(String),
    Declined,
}

fn query_of(target: &str) -> Vec<(String, String)> {
    let Some((_, query)) = target.split_once('?') else {
        return Vec::new();
    };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

async fn answer(stream: &mut tokio::net::TcpStream, status: &str, sentence: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Meridian</title>\
         <p style=\"font-family:sans-serif\">{sentence}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\n\
         content-length: {}\r\ncache-control: no-store\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Wait for the browser to bring back this terminal's answer.
///
/// Anything else that reaches the port -- a favicon, a stray request, a
/// callback carrying some other state -- is answered and ignored, and the
/// wait goes on: only this terminal's state ends it. A callback with this
/// state and no code is the person declining, or the deployment refusing.
pub async fn returned(
    listener: &TcpListener,
    state: &str,
    wait: Duration,
) -> Result<Returned, String> {
    let waited = tokio::time::timeout(wait, async {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let mut buffer = vec![0u8; 8192];
            let mut read = 0;
            // The request line is all this needs; stop at the end of the
            // headers or when the buffer is full.
            while read < buffer.len() {
                let Ok(n) =
                    tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer[read..]))
                        .await
                        .unwrap_or(Ok(0))
                else {
                    break;
                };
                if n == 0 {
                    break;
                }
                read += n;
                if buffer[..read].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&buffer[..read]);
            let target = request
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("GET "))
                .and_then(|rest| rest.split(' ').next())
                .unwrap_or_default()
                .to_string();
            if !target.starts_with("/callback?") {
                answer(&mut stream, "404 Not Found", "Nothing here.").await;
                continue;
            }
            let query = query_of(&target);
            let find = |key: &str| {
                query
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.as_str())
            };
            if find("state") != Some(state) {
                answer(
                    &mut stream,
                    "400 Bad Request",
                    "This is not the sign-in this terminal started, so it was ignored.",
                )
                .await;
                continue;
            }
            if let Some(code) = find("code").filter(|code| !code.is_empty()) {
                answer(
                    &mut stream,
                    "200 OK",
                    "Connected. You can close this tab and go back to your terminal.",
                )
                .await;
                return Returned::Code(code.to_string());
            }
            answer(
                &mut stream,
                "200 OK",
                "Not connected. You can close this tab.",
            )
            .await;
            return Returned::Declined;
        }
    })
    .await;
    waited.map_err(|_| {
        format!(
            "nobody finished signing in within {} minutes",
            wait.as_secs() / 60
        )
    })
}

/// What the deployment issued.
#[derive(Debug, serde::Deserialize)]
pub struct Issued {
    pub session: String,
    pub subject: String,
    pub expires_at: String,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|failed| failed.to_string())
}

/// The code and the verifier, for a session.
pub async fn exchange(
    address: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Issued, String> {
    let response = client()?
        .post(format!("{address}/terminal/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form(&[
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ]))
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "{address} did not issue a session ({status}): {}",
            text.trim()
        ));
    }
    serde_json::from_str(&text).map_err(|failed| {
        format!("{address} answered with something that is not a session: {failed}")
    })
}

/// End a session at the deployment. Whether it was still live makes no
/// difference to what the CLI does next, so only reaching it matters.
pub async fn sign_out(address: &str, session: &str) -> Result<(), String> {
    client()?
        .post(format!("{address}/terminal/sign-out"))
        .bearer_auth(session)
        .send()
        .await
        .map(|_| ())
        .map_err(|failed| format!("could not reach {address}: {failed}"))
}

/// Open the sign-in in the person's browser, if there is one to open. The
/// address is printed either way, so a machine with no browser is not a
/// dead end.
pub fn open_browser(url: &str) {
    let opened = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        // cmd reads `&` as the end of a command, and every query has one.
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url.replace('&', "^&")])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    let _ = opened;
}

#[cfg(test)]
mod tests;

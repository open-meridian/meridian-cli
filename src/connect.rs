//! `meridian connect` and `meridian sign-out`: W6.13, W6.14 and W6.17 from
//! the terminal's side (decisions/029; spec/clients-act-on-a-persons-delegation).
//!
//! The CLI never takes a password. It registers this computer with the
//! deployment as a client (RFC 7591) if it has not, listens on a loopback
//! port, opens the deployment's authorisation in a browser with a PKCE
//! challenge (RFC 7636), a state and the resource it wants, and waits for
//! the browser to be sent back with a one-time code (RFC 8252). The person
//! signs in however that deployment signs people in, afresh, and consents.
//! The code and the verifier only this process holds become a delegation's
//! access token (ten minutes) and refresh token (single use), kept in the
//! sessions file; [`crate::credential`] presents and refreshes them.

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
    // A name under `.localhost` is this machine by definition (RFC 6761),
    // which is how a local deployment is reached through its Ingress.
    let local = matches!(host, "127.0.0.1" | "[::1]" | "localhost") || host.ends_with(".localhost");
    match scheme {
        "https" => Ok(trimmed.to_string()),
        "http" if local => Ok(trimmed.to_string()),
        "http" => Err(format!(
            "{given} is plain HTTP to another machine, which would send your session in the clear; use https://"
        )),
        _ => Err(format!("{given} is neither https:// nor http://")),
    }
}

/// The `meridian connect` to run for an address: bare for the local install,
/// which is what `connect` given no address signs in to.
pub fn command_for(address: &str) -> String {
    if address == crate::up::local_address() {
        "meridian connect".to_string()
    } else {
        format!("meridian connect {address}")
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

/// The resource a token for the CLI's surface names (RFC 8707).
pub fn terminal_resource(address: &str) -> String {
    format!("{address}/terminal")
}

/// What this CLI calls itself to a deployment it registers with, and says it
/// is: the consent page's defaults, and nothing else, rest on that.
pub const SOFTWARE_ID: &str = "meridian-cli";

/// This computer's name as the deployment lists it: "meridian on
/// ada-laptop", so each computer is told apart in Connected clients.
pub fn client_name() -> String {
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| "this computer".into());
    let host: String = host.chars().filter(|c| !c.is_control()).take(80).collect();
    format!("meridian on {host}")
}

pub fn authorize_url(
    address: &str,
    client_id: &str,
    redirect_uri: &str,
    pkce: &Pkce,
    state: &str,
) -> String {
    format!(
        "{address}/oauth/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("resource", &terminal_resource(address)),
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

/// What the deployment's token endpoint answered: a pair on a delegation,
/// and the delegation's end, which every answer carries.
#[derive(Debug, serde::Deserialize)]
pub struct Issued {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds the access token lives.
    pub expires_in: u64,
    pub subject: String,
    /// When the delegation lapses, RFC 3339, for a person to read.
    pub delegation_expires_at: String,
    /// And in seconds from now, for this to count down.
    pub delegation_expires_in: u64,
}

impl Issued {
    /// What the sessions file keeps of it, for `address` and `client_id`,
    /// counted from `now_s`.
    pub fn held(self, address: &str, client_id: &str, now_s: u64) -> crate::sessions::Held {
        crate::sessions::Held {
            address: address.to_string(),
            subject: self.subject,
            expires_at: self.delegation_expires_at,
            session: String::new(),
            client_id: client_id.to_string(),
            access_token: self.access_token,
            access_expires_at_s: now_s + self.expires_in,
            refresh_token: self.refresh_token,
            expires_at_s: now_s + self.delegation_expires_in,
        }
    }
}

/// Why the token endpoint said no: its `reason`, when it named one, and
/// what to tell the person.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused {
    /// `revoked`, `lapsed`, `groups`, `reused` or `unknown`; empty when the
    /// deployment could not be asked or did not say.
    pub reason: String,
    pub said: String,
}

fn client() -> Result<reqwest::Client, String> {
    crate::authority::trusted_here(reqwest::Client::builder())
        .timeout(Duration::from_secs(30))
        .default_headers(crate::release::naming_this_version())
        .build()
        .map_err(|failed| failed.to_string())
}

/// Register this computer as a client of the deployment: its id.
pub async fn register(address: &str, redirect_uri: &str) -> Result<String, String> {
    let answer = client()?
        .post(format!("{address}/oauth/register"))
        .header("content-type", "application/json")
        .body(
            serde_json::json!({
                "client_name": client_name(),
                "redirect_uris": [redirect_uri],
                "software_id": SOFTWARE_ID,
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            })
            .to_string(),
        )
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "{address} does not take delegations yet: its dashboard is older than this CLI. \
             Upgrade the deployment, or connect with meridian 0.1.24 \
             (`meridian upgrade --to 0.1.24`), the last to sign in the old way"
        ));
    }
    if !status.is_success() {
        return Err(format!(
            "{address} did not register this computer ({status}): {}",
            text.trim()
        ));
    }
    let said: serde_json::Value = serde_json::from_str(&text)
        .map_err(|failed| format!("{address} answered with something unreadable: {failed}"))?;
    said["client_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(String::from)
        .ok_or_else(|| format!("{address} registered this computer and gave it no id"))
}

async fn token(address: &str, fields: &[(&str, &str)]) -> Result<Issued, Refused> {
    let unreachable = |failed: String| Refused {
        reason: String::new(),
        said: failed,
    };
    let answer = client()
        .map_err(unreachable)?
        .post(format!("{address}/oauth/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form(fields))
        .send()
        .await
        .map_err(|failed| unreachable(format!("could not reach {address}: {failed}")))?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    if let Some(refused) = crate::release::version_refused(status, &text) {
        return Err(unreachable(refused));
    }
    if !status.is_success() {
        let said: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        return Err(Refused {
            reason: said["reason"].as_str().unwrap_or_default().to_string(),
            said: match said["error_description"].as_str() {
                Some(description) => format!("{description} ({status})"),
                None => format!("{status}: {}", text.trim()),
            },
        });
    }
    serde_json::from_str(&text).map_err(|failed| {
        unreachable(format!(
            "{address} answered with something that is not a token: {failed}"
        ))
    })
}

/// The code and the verifier, for a pair on the delegation just made.
pub async fn exchange(
    address: &str,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Issued, String> {
    token(
        address,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("resource", &terminal_resource(address)),
        ],
    )
    .await
    .map_err(|refused| format!("{address} did not issue a delegation: {}", refused.said))
}

/// A refresh token, spent, for the next pair.
pub async fn refresh(
    address: &str,
    client_id: &str,
    refresh_token: &str,
) -> Result<Issued, Refused> {
    token(
        address,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ],
    )
    .await
}

/// `meridian sign-out`: the delegation revoked at the deployment (RFC 7009),
/// by its refresh token. Whether it was still live makes no difference to
/// what the CLI does next, so reaching it matters, and being heard.
pub async fn revoke(address: &str, client_id: &str, refresh_token: &str) -> Result<(), String> {
    let answer = client()?
        .post(format!("{address}/oauth/revoke"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form(&[
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", client_id),
        ]))
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    if let Some(refused) = crate::release::version_refused(status, &text) {
        return Err(format!("{address} refused: {refused}"));
    }
    if !status.is_success() {
        return Err(format!(
            "{address} did not revoke it ({status}): {}",
            text.trim()
        ));
    }
    Ok(())
}

/// End a terminal session from before delegations, which a sessions file
/// written by an older CLI holds (honoured by the deployment for one
/// release).
pub async fn end_terminal_session(address: &str, session: &str) -> Result<(), String> {
    let answer = client()?
        .post(format!("{address}/terminal/sign-out"))
        .bearer_auth(session)
        .send()
        .await
        .map_err(|failed| format!("could not reach {address}: {failed}"))?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    match crate::release::version_refused(status, &text) {
        Some(refused) => Err(format!("{address} refused: {refused}")),
        None => Ok(()),
    }
}

/// Whatever a sessions file holds, ended at the deployment: the delegation
/// revoked, or an older terminal session ended.
pub async fn sign_out(held: &crate::sessions::Held) -> Result<(), String> {
    if held.is_delegation() {
        revoke(&held.address, &held.client_id, &held.refresh_token).await
    } else {
        end_terminal_session(&held.address, &held.session).await
    }
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

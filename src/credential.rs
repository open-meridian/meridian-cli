//! What a command acts with: the sessions file's credential, presented as a
//! bearer, and refreshed before it lapses (W6.18; decisions/029).
//!
//! An access token lives ten minutes. Every request asks [`Credential::bearer`]
//! for one with at least a minute left, which refreshes first when it has
//! less, and a request refused as expired refreshes once and is sent again
//! ([`Credential::send`]). So `meridian plugin dev` watches for hours on a
//! delegation that lasts up to 90 days, and nobody is asked for a browser.
//!
//! **Refreshing is serialised across this computer's processes.** A refresh
//! token is single use, and the deployment revokes the delegation when one is
//! presented twice, because the only way that happens is for somebody else to
//! hold it too. So refreshing takes an exclusive lock on the deployment's
//! sessions file and reads it again under the lock: a second process waiting
//! on the first finds the pair the first wrote, and uses that instead of
//! spending the token the first already spent. `plugin dev` in one terminal
//! and `plugin list` in another never revoke the person's CLI.
//!
//! A file an older CLI wrote holds a terminal session, which is presented as
//! it is and never refreshed. No deployment honours one since contract v15
//! retired them (W6.13), so it is refused, and the person connects again.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::live::Failed;
use crate::sessions::{self, Held};

/// An access token with less than this left is refreshed before it is sent.
pub const REFRESH_WITHIN_S: u64 = 60;
/// A delegation lapsing within this is said aloud on every command, once.
pub const NOTICE_S: u64 = 7 * 24 * 60 * 60;

pub fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// One deployment's credential, for the life of a command.
pub struct Credential {
    within: PathBuf,
    held: tokio::sync::Mutex<Held>,
    address: String,
    told: AtomicBool,
}

/// What a refused request said, when it said the access token is no good:
/// worth one refresh.
fn worth_refreshing(status: reqwest::StatusCode, body: &str) -> bool {
    if status != reqwest::StatusCode::UNAUTHORIZED {
        return false;
    }
    let said: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    matches!(said["reason"].as_str(), Some("expired") | Some("unknown"))
}

/// The person told what stopped a refresh, and what to do.
fn session_failed(address: &str, refused: &crate::connect::Refused) -> Failed {
    let command = crate::connect::command_for(address);
    let why = match refused.reason.as_str() {
        "revoked" => format!("your delegation to this computer at {address} was revoked"),
        "lapsed" => format!("your delegation to this computer at {address} lapsed"),
        "groups" => {
            format!("{address} needs you to sign in again before this computer acts for you")
        }
        "reused" => format!(
            "a refresh token for {address} was presented twice, so the delegation was revoked; \
             if you did not run two copies of an old meridian, tell your deployment admin"
        ),
        "unknown" => format!("{address} does not know this computer's delegation"),
        _ => return Failed::Refused(format!("could not refresh at {address}: {}", refused.said)),
    };
    Failed::Session(format!("{why}: `{command}` to connect again"))
}

impl Credential {
    pub fn new(within: PathBuf, held: Held) -> Credential {
        Credential {
            within,
            address: held.address.clone(),
            held: tokio::sync::Mutex::new(held),
            told: AtomicBool::new(false),
        }
    }

    /// An older CLI's terminal session, which is never refreshed: what the
    /// tests of everything that presents a credential present.
    #[cfg(test)]
    pub fn terminal_session(address: &str, session: &str) -> Credential {
        Credential::new(
            std::env::temp_dir().join("meridian-no-sessions-file"),
            Held {
                address: address.to_string(),
                session: session.to_string(),
                ..Held::default()
            },
        )
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    /// Within a week of the delegation's end, said once per command.
    fn notice(&self, held: &Held) {
        if held.is_delegation()
            && held.expires_at_s.saturating_sub(now_s()) <= NOTICE_S
            && !self.told.swap(true, Ordering::SeqCst)
        {
            eprintln!(
                "Your delegation to this computer at {} lapses at {}. `{}` renews it.",
                self.address,
                held.expires_at,
                crate::connect::command_for(&self.address)
            );
        }
    }

    /// The bearer for the next request: an access token with at least a
    /// minute left, refreshed first when it has less; or a terminal session
    /// an older CLI wrote.
    pub async fn bearer(&self) -> Result<String, Failed> {
        let mut held = self.held.lock().await;
        self.notice(&held);
        if !held.is_delegation() {
            return Ok(held.session.clone());
        }
        if held.access_expires_at_s > now_s() + REFRESH_WITHIN_S {
            return Ok(held.access_token.clone());
        }
        let stale = held.access_token.clone();
        *held = self.refresh(&stale).await?;
        Ok(held.access_token.clone())
    }

    /// After a request refused as expired: the next access token, whatever
    /// the clock says -- unless another process has refreshed already, whose
    /// pair is used.
    pub async fn refreshed(&self, stale: &str) -> Result<String, Failed> {
        let mut held = self.held.lock().await;
        if !held.is_delegation() {
            return Ok(held.session.clone());
        }
        if held.access_token != stale {
            return Ok(held.access_token.clone());
        }
        *held = self.refresh(stale).await?;
        Ok(held.access_token.clone())
    }

    /// Refresh under the file's lock: read again, and use what another
    /// process wrote if it refreshed past `stale`; otherwise spend the
    /// refresh token and write the next pair before letting go.
    async fn refresh(&self, stale: &str) -> Result<Held, Failed> {
        let (within, address) = (self.within.clone(), self.address.clone());
        let lock = tokio::task::spawn_blocking(move || sessions::lock(&within, &address))
            .await
            .map_err(|failed| Failed::Refused(failed.to_string()))?
            .map_err(Failed::Refused)?;
        let Some(current) = sessions::read(&self.within, &self.address) else {
            return Err(Failed::Session(format!(
                "not connected to {} any more: `{}` first",
                self.address,
                crate::connect::command_for(&self.address)
            )));
        };
        if current.is_delegation()
            && current.access_token != stale
            && current.access_expires_at_s > now_s() + REFRESH_WITHIN_S
        {
            drop(lock);
            return Ok(current);
        }
        if !current.is_delegation() {
            drop(lock);
            return Ok(current);
        }
        let issued =
            crate::connect::refresh(&self.address, &current.client_id, &current.refresh_token)
                .await
                .map_err(|refused| session_failed(&self.address, &refused))?;
        let next = issued.held(&self.address, &current.client_id, now_s());
        // Written before the lock is let go: the refresh token just spent
        // must never be read again by anybody.
        let written = sessions::write(&self.within, &next);
        drop(lock);
        written.map_err(|failed| {
            Failed::Refused(format!(
                "refreshed, and could not keep the next pair ({failed}); `{}` to connect again",
                crate::connect::command_for(&self.address)
            ))
        })?;
        Ok(next)
    }

    /// Send a request built around a bearer, and once more on the next one
    /// if it was refused as expired. `build` is called once per try.
    pub async fn send(
        &self,
        build: impl Fn(&str) -> reqwest::RequestBuilder,
    ) -> Result<(reqwest::StatusCode, String, reqwest::header::HeaderMap), Failed> {
        let bearer = self.bearer().await?;
        let (status, body, headers) = self.sent(build(&bearer)).await?;
        if !worth_refreshing(status, &body) || !self.held.lock().await.is_delegation() {
            return Ok((status, body, headers));
        }
        let next = self.refreshed(&bearer).await?;
        self.sent(build(&next)).await
    }

    async fn sent(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<(reqwest::StatusCode, String, reqwest::header::HeaderMap), Failed> {
        let answer = request.send().await.map_err(|failed| {
            Failed::Refused(format!("could not reach {}: {failed}", self.address))
        })?;
        let status = answer.status();
        let headers = answer.headers().clone();
        let body = answer.text().await.unwrap_or_default();
        Ok((status, body, headers))
    }
}

#[cfg(test)]
mod tests;

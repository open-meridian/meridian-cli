//! The registries, from the publisher's side (contract v25;
//! spec/plugins-are-shared-through-registries, requirements 4, 12, 18 and 44;
//! W8.8, W5.30): the platform sign-in, `meridian plugin publish`, and
//! `meridian plugin key register|revoke|list`. The bundle a deployment that
//! reaches nothing carries in is `bundle.rs`'s.
//!
//! **The platform sign-in** is the browser's, never a password: the
//! platform's own OAuth server, the authorization code flow with PKCE and a
//! loopback redirect (RFC 8252), as the public client `meridian-cli`, asking
//! for the scope `publish` and the publishing API as its resource. Its access
//! tokens live ten minutes and are accepted only at the publishing API
//! (`/api/v1/registry/...`); a sign-in lasts 30 days, each refresh token
//! single use (decisions/022, amended 2026-10-16, items 137 and 138). It is
//! kept beside the deployments' sessions, in `platform/`, readable by its
//! owner alone, and refreshed under the same lock, so two commands never
//! present one refresh token twice. A command that needs it and holds none,
//! or holds one that lapsed, signs in first.
//!
//! **A publish acts with a push token**, an access token narrowed to the one
//! plugin by the scope `publish:{organisation}/{plugin}` (item 137): asked of
//! the sign-in by a refresh naming the narrower scope (RFC 6749, section 6),
//! the refresh token keeping the sign-in's own; where the platform answers
//! anything wider, a sign-in narrowed to the plugin is made for that publish
//! alone, and kept nowhere.
//!
//! **Publishing** asks the platform to build a signed release tag in the
//! plugin's repository (requirement 12): the CLI runs `meridian plugin check`
//! as a pre-flight, refuses locally what would fail -- a tag that is not `v`
//! and the SemVer version pyproject.toml holds, one not signed, a section of
//! docs/ missing -- and then follows the build (ReadPluginBuild) until it is
//! listed, queued for review, or refused. It builds, signs and pushes
//! nothing: only the platform's builder pushes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::connect;
use crate::sessions::{self, Held};

/// The CLI's public OAuth client at the platform (item 140).
pub const CLIENT_ID: &str = "meridian-cli";
/// The scope a sign-in asks for (item 140).
pub const SCOPE: &str = "publish";
/// How long a sign-in lasts (item 138), the refresh token's life.
pub const SIGN_IN_S: u64 = 30 * 24 * 60 * 60;
/// An access token with less than this left is refreshed before it is sent.
const REFRESH_WITHIN_S: u64 = 60;
/// How often a build is asked after, and for how long by default.
pub const FOLLOW_EVERY: Duration = Duration::from_secs(5);
pub const FOLLOW_FOR: Duration = Duration::from_secs(30 * 60);

/// The publishing API, the resource the CLI's tokens are issued for.
pub fn resource(platform: &str) -> String {
    format!("{platform}/api/v1/registry")
}

/// The scope a push token for one plugin carries (item 137).
pub fn push_scope(organisation: &str, plugin: &str) -> String {
    format!("publish:{organisation}/{plugin}")
}

/// Where the platform sign-ins are kept: beside the deployments' sessions.
pub fn directory() -> Result<PathBuf, String> {
    Ok(sessions::configuration()?.join("platform"))
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(crate::release::naming_this_version())
        .build()
        .map_err(|failed| failed.to_string())
}

/// Percent-encoding for a form or a query, keeping only what needs none.
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

// ── The sign-in ──────────────────────────────────────────────────────────

/// The platform's authorization, for the browser.
pub fn authorize_url(
    platform: &str,
    redirect_uri: &str,
    pkce: &connect::Pkce,
    state: &str,
    scope: &str,
) -> String {
    format!(
        "{platform}/oauth/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", redirect_uri),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", state),
            ("resource", &resource(platform)),
            ("scope", scope),
        ])
    )
}

/// What the platform's token endpoint answers.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Issued {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    pub expires_in: u64,
    #[serde(default)]
    pub scope: String,
}

/// Why the token endpoint said no: its OAuth error, and what to tell the
/// person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub error: String,
    pub said: String,
}

async fn token(platform: &str, fields: &[(&str, &str)]) -> Result<Issued, Refused> {
    let unreachable = |said: String| Refused {
        error: String::new(),
        said,
    };
    let answer = client()
        .map_err(unreachable)?
        .post(format!("{platform}/oauth/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(form(fields))
        .send()
        .await
        .map_err(|failed| unreachable(format!("could not reach {platform}: {failed}")))?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    if !status.is_success() {
        let said: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        return Err(Refused {
            error: said["error"].as_str().unwrap_or_default().to_string(),
            said: match said["error_description"].as_str() {
                Some(description) => format!("{description} ({status})"),
                None => format!("{status}: {}", text.trim()),
            },
        });
    }
    serde_json::from_str(&text).map_err(|failed| {
        unreachable(format!(
            "{platform} answered with something that is not a token: {failed}"
        ))
    })
}

/// How the browser is opened, which a test stands in for.
pub type Opener = dyn Fn(&str) + Send + Sync;

/// The browser sign-in, narrowed by `scope`: a pair, or why not.
pub async fn sign_in(platform: &str, scope: &str, open: &Opener) -> Result<Issued, String> {
    let (listener, redirect_uri) = connect::listen().await?;
    let pkce = connect::Pkce::new();
    let state = connect::state();
    let url = authorize_url(platform, &redirect_uri, &pkce, &state, scope);
    eprintln!("Signing in to {platform} in your browser. If it does not open, open:\n  {url}");
    open(&url);
    let code = match connect::returned(&listener, &state, connect::WAIT).await? {
        connect::Returned::Code(code) => code,
        connect::Returned::Declined => {
            return Err(format!("not signed in: {platform} was told no"));
        }
    };
    token(
        platform,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &pkce.verifier),
            ("redirect_uri", &redirect_uri),
            ("client_id", CLIENT_ID),
            ("resource", &resource(platform)),
        ],
    )
    .await
    .map_err(|refused| format!("{platform} did not sign this computer in: {}", refused.said))
}

fn held(platform: &str, issued: &Issued, now_s: u64, started_s: u64) -> Held {
    Held {
        address: platform.to_string(),
        subject: String::new(),
        expires_at: String::new(),
        session: String::new(),
        client_id: CLIENT_ID.to_string(),
        access_token: issued.access_token.clone(),
        access_expires_at_s: now_s + issued.expires_in,
        refresh_token: issued.refresh_token.clone(),
        expires_at_s: started_s + SIGN_IN_S,
    }
}

/// The platform sign-in, for the life of a command.
pub struct SignedIn {
    within: PathBuf,
    held: tokio::sync::Mutex<Held>,
    platform: String,
    open: Box<Opener>,
}

impl SignedIn {
    /// The sign-in kept for `platform`, or a new one made in the browser.
    pub async fn for_platform(platform: &str, open: Box<Opener>) -> Result<SignedIn, String> {
        let within = directory()?;
        Self::within(within, platform, open).await
    }

    pub async fn within(
        within: PathBuf,
        platform: &str,
        open: Box<Opener>,
    ) -> Result<SignedIn, String> {
        let now = crate::credential::now_s();
        let kept = sessions::read(&within, platform)
            .filter(|held| held.is_delegation() && held.expires_at_s > now + REFRESH_WITHIN_S);
        let held = match kept {
            Some(held) => held,
            None => {
                let issued = sign_in(platform, SCOPE, open.as_ref()).await?;
                let made = held(platform, &issued, now, now);
                sessions::write(&within, &made)?;
                made
            }
        };
        Ok(SignedIn {
            within,
            held: tokio::sync::Mutex::new(held),
            platform: platform.to_string(),
            open,
        })
    }

    pub fn platform(&self) -> &str {
        &self.platform
    }

    /// A refresh, serialised across this computer's processes: under the
    /// lock the file is read again, and a pair another process wrote
    /// meanwhile is taken rather than spending a token already spent. With
    /// `scope`, the access token narrowed to it, the refresh token keeping
    /// the sign-in's own (RFC 6749, section 6). A refused refresh ends the
    /// sign-in here, and the next command signs in again.
    async fn refreshed(&self, scope: Option<&str>) -> Result<Issued, String> {
        let _lock = sessions::lock(&self.within, &self.platform)?;
        let mut held = self.held.lock().await;
        let now = crate::credential::now_s();
        if scope.is_none() {
            if let Some(newer) = sessions::read(&self.within, &self.platform) {
                if newer.refresh_token != held.refresh_token
                    && newer.access_expires_at_s > now + REFRESH_WITHIN_S
                {
                    *held = newer;
                    return Ok(Issued {
                        access_token: held.access_token.clone(),
                        refresh_token: held.refresh_token.clone(),
                        expires_in: held.access_expires_at_s.saturating_sub(now),
                        scope: SCOPE.into(),
                    });
                }
            }
        }
        let refresh_token = held.refresh_token.clone();
        let mut fields = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", CLIENT_ID),
        ];
        if let Some(scope) = scope {
            fields.push(("scope", scope));
        }
        match token(&self.platform, &fields).await {
            Ok(issued) => {
                let started = held.expires_at_s.saturating_sub(SIGN_IN_S);
                let next = Held {
                    refresh_token: if issued.refresh_token.is_empty() {
                        held.refresh_token.clone()
                    } else {
                        issued.refresh_token.clone()
                    },
                    ..held.clone()
                };
                // A narrowed access token is the publish's alone, never kept.
                let next = if scope.is_some() {
                    next
                } else {
                    let mut kept = self::held(&self.platform, &issued, now, started);
                    kept.refresh_token = next.refresh_token.clone();
                    kept
                };
                sessions::write(&self.within, &next)?;
                *held = next;
                Ok(issued)
            }
            Err(refused) if refused.error == "invalid_grant" => {
                let _ = sessions::forget(&self.within, &self.platform);
                Err(format!(
                    "your sign-in to {} has ended ({}): run the command again to sign in",
                    self.platform, refused.said
                ))
            }
            Err(refused) => Err(refused.said),
        }
    }

    /// An access token for the publishing API with at least a minute left.
    pub async fn bearer(&self) -> Result<String, String> {
        {
            let held = self.held.lock().await;
            if held.access_expires_at_s > crate::credential::now_s() + REFRESH_WITHIN_S {
                return Ok(held.access_token.clone());
            }
        }
        Ok(self.refreshed(None).await?.access_token)
    }

    /// A push token: an access token good for one plugin's rows alone.
    pub async fn push_token(&self, organisation: &str, plugin: &str) -> Result<String, String> {
        let wanted = push_scope(organisation, plugin);
        let issued = self.refreshed(Some(&wanted)).await?;
        if issued.scope == wanted {
            return Ok(issued.access_token);
        }
        // The platform answered a wider token: a sign-in narrowed to this
        // plugin, for this publish alone.
        eprintln!(
            "{} did not narrow the sign-in's token to {organisation}/{plugin}; signing in for \
             this publish alone.",
            self.platform
        );
        let narrowed = sign_in(&self.platform, &wanted, self.open.as_ref()).await?;
        if narrowed.scope != wanted {
            return Err(format!(
                "{} issued a token for `{}`, not `{wanted}`: a publish acts with a token for its \
                 plugin alone, so nothing was sent",
                self.platform, narrowed.scope
            ));
        }
        Ok(narrowed.access_token)
    }
}

// ── The publishing API ───────────────────────────────────────────────────

/// What the publishing API refused, said as it said it.
fn refusal(platform: &str, status: reqwest::StatusCode, body: &str) -> String {
    let said: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let words = said["error_description"]
        .as_str()
        .or_else(|| said["error"].as_str())
        .map(String::from)
        .unwrap_or_else(|| body.trim().to_string());
    match status.as_u16() {
        401 => format!("{platform} did not take the sign-in ({words}): run the command again"),
        _ => format!("{platform} refused ({status}): {words}"),
    }
}

/// One request to the publishing API with `bearer`, its answer as JSON.
pub async fn call(
    platform: &str,
    method: reqwest::Method,
    path: &str,
    bearer: &str,
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut request = client()?
        .request(method, format!("{platform}{path}"))
        .bearer_auth(bearer);
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let answer = request
        .send()
        .await
        .map_err(|failed| format!("could not reach {platform}: {failed}"))?;
    let status = answer.status();
    let text = answer.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(refusal(platform, status, &text));
    }
    serde_json::from_str(&text)
        .map_err(|failed| format!("{platform} answered with something unreadable: {failed}"))
}

/// A listing's organisation part: a slug, lower-case DNS label, at most 39
/// characters (requirement 6).
pub fn is_slug(slug: &str) -> bool {
    (1..=39).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

/// `vX.Y.Z[-pre][+build]`: `v` and a SemVer 2.0 version (requirement 8).
pub fn semver_of_tag(tag: &str) -> Option<&str> {
    let version = tag.strip_prefix('v')?;
    let (core, rest) = match version.find(['-', '+']) {
        Some(at) => (&version[..at], &version[at..]),
        None => (version, ""),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let numeric = |part: &&str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part.len() == 1 || !part.starts_with('0'))
    };
    if parts.len() != 3 || !parts.iter().all(numeric) {
        return None;
    }
    let identifiers = |text: &str| {
        !text.is_empty()
            && text.split('.').all(|id| {
                !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
    };
    let (pre, build) = match rest.strip_prefix('-') {
        Some(after) => match after.split_once('+') {
            Some((pre, build)) => (Some(pre), Some(build)),
            None => (Some(after), None),
        },
        None => (None, rest.strip_prefix('+')),
    };
    if !rest.is_empty() && !rest.starts_with(['-', '+']) {
        return None;
    }
    if pre.is_some_and(|pre| !identifiers(pre)) || build.is_some_and(|b| !identifiers(b)) {
        return None;
    }
    Some(version)
}

/// What the plugin's repository says, read with git: the tag's object and
/// whether it carries an SSH signature, and where `origin` is.
pub trait Repository {
    fn tag_object(&self, dir: &Path, tag: &str) -> Result<String, String>;
    fn origin(&self, dir: &Path) -> Result<String, String>;
}

pub struct Git;

impl Repository for Git {
    fn tag_object(&self, dir: &Path, tag: &str) -> Result<String, String> {
        let shown = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["cat-file", "tag", &format!("refs/tags/{tag}")])
            .output()
            .map_err(|failed| format!("git could not be run: {failed}"))?;
        if !shown.status.success() {
            return Err(format!(
                "{tag} is not an annotated tag in {}: make it with `git tag -s {tag}`, signed \
                 with an SSH key registered on the organisation",
                dir.display()
            ));
        }
        Ok(String::from_utf8_lossy(&shown.stdout).into_owned())
    }

    fn origin(&self, dir: &Path) -> Result<String, String> {
        let shown = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["remote", "get-url", "origin"])
            .output()
            .map_err(|failed| format!("git could not be run: {failed}"))?;
        if !shown.status.success() {
            return Err(format!(
                "{} has no remote `origin`: the platform builds from the plugin's GitHub \
                 repository, linked to the organisation through the GitHub App",
                dir.display()
            ));
        }
        Ok(String::from_utf8_lossy(&shown.stdout).trim().to_string())
    }
}

/// `origin` as the repository's address on GitHub: `https://github.com/o/r`,
/// from an SSH or HTTPS remote.
pub fn github_repository(origin: &str) -> Result<String, String> {
    let path = origin
        .strip_prefix("git@github.com:")
        .or_else(|| origin.strip_prefix("ssh://git@github.com/"))
        .or_else(|| origin.strip_prefix("https://github.com/"))
        .ok_or_else(|| {
            format!(
                "`origin` is {origin}, not a GitHub repository: the builder builds from one linked \
                 through the GitHub App `Open Meridian Registry`"
            )
        })?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    match path.split('/').collect::<Vec<_>>().as_slice() {
        [owner, repo] if !owner.is_empty() && !repo.is_empty() => {
            Ok(format!("https://github.com/{owner}/{repo}"))
        }
        _ => Err(format!(
            "`origin` is {origin}, which names no owner/repository"
        )),
    }
}

/// What `meridian plugin publish` was asked.
pub struct Publish {
    pub dir: PathBuf,
    pub registry: String,
    pub tag: String,
    pub organisation: String,
    pub follow_for: Duration,
    /// How often the build is asked after (FOLLOW_EVERY).
    pub follow_every: Duration,
}

/// What the pre-flight found to send: the plugin, its version and its
/// repository; or what it refused, locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ready {
    pub plugin: String,
    pub version: String,
    pub repository: String,
}

/// The pre-flight (requirement 12): what the platform would refuse, refused
/// here first, before anything is sent.
pub fn preflight(asked: &Publish, repository: &dyn Repository) -> Result<Ready, String> {
    if !matches!(asked.registry.as_str(), "community" | "verified") {
        return Err(format!(
            "--registry is community or verified, not `{}`",
            asked.registry
        ));
    }
    if !is_slug(&asked.organisation) {
        return Err(format!(
            "`{}` is not an organisation's slug: lower-case letters, digits and hyphens, at most \
             39",
            asked.organisation
        ));
    }
    let version = semver_of_tag(&asked.tag).ok_or_else(|| {
        format!(
            "{} is not a release tag: `v` and a SemVer 2.0 version, as v1.4.0",
            asked.tag
        )
    })?;
    let pyproject = std::fs::read_to_string(asked.dir.join("pyproject.toml"))
        .map_err(|failed| format!("{} has no pyproject.toml: {failed}", asked.dir.display()))?;
    let metadata = crate::catalogue::metadata(&pyproject)?;
    if metadata.version != version {
        return Err(format!(
            "{} names {version}, and pyproject.toml's version is {}: tag the version the \
             plugin says it is",
            asked.tag, metadata.version
        ));
    }
    let report = crate::check::check_as(&asked.dir, false, asked.registry == "verified")?;
    if !report.passed() {
        return Err(format!(
            "`meridian plugin check{}` does not pass, and the platform's builder runs the same \
             rules:\n\n{}",
            if asked.registry == "verified" {
                " --verified"
            } else {
                ""
            },
            crate::check::text(&report)
        ));
    }
    let tag = repository.tag_object(&asked.dir, &asked.tag)?;
    if !tag.contains("-----BEGIN SSH SIGNATURE-----") {
        return Err(format!(
            "{} is not signed with an SSH key: the builder refuses an unsigned tag. Sign it \
             (`git -c gpg.format=ssh tag -s {}`) with a key registered on {} (`meridian plugin \
             key register`)",
            asked.tag, asked.tag, asked.organisation
        ));
    }
    let repository = github_repository(&repository.origin(&asked.dir)?)?;
    Ok(Ready {
        plugin: metadata.name,
        version: version.to_string(),
        repository,
    })
}

fn build_path(organisation: &str, plugin: &str, build_id: &str) -> String {
    format!("/api/v1/registry/organisations/{organisation}/plugins/{plugin}/builds/{build_id}")
}

/// Where a build stands, said in a word.
fn state_word(state: &str) -> &str {
    match state {
        "PLUGIN_BUILD_STATE_QUEUED" => "queued",
        "PLUGIN_BUILD_STATE_BUILDING" => "building",
        "PLUGIN_BUILD_STATE_LISTED" => "listed",
        "PLUGIN_BUILD_STATE_IN_REVIEW" => "in review",
        "PLUGIN_BUILD_STATE_REFUSED" => "refused",
        other => other,
    }
}

/// Whether a build is done with: listed, in review, or refused.
pub fn settled(build: &serde_json::Value) -> bool {
    matches!(
        build["state"].as_str(),
        Some("PLUGIN_BUILD_STATE_LISTED")
            | Some("PLUGIN_BUILD_STATE_IN_REVIEW")
            | Some("PLUGIN_BUILD_STATE_REFUSED")
    )
}

/// A build, for a person to read: where it stands, each finding, and the
/// log where it is public.
pub fn said(build: &serde_json::Value) -> String {
    let mut out = format!(
        "{} {} ({}): {}\n",
        build["listing"].as_str().unwrap_or_default(),
        build["version"].as_str().unwrap_or_default(),
        build["registry"].as_str().unwrap_or_default(),
        state_word(build["state"].as_str().unwrap_or_default()),
    );
    if let Some(digest) = build["image_digest"].as_str().filter(|d| !d.is_empty()) {
        out.push_str(&format!("  image: {digest}\n"));
    }
    for finding in build["findings"].as_array().into_iter().flatten() {
        let mark = match finding["severity"].as_str() {
            Some("BUILD_FINDING_SEVERITY_PASSED") => "ok  ",
            Some("BUILD_FINDING_SEVERITY_WARNING") => "warn",
            Some("BUILD_FINDING_SEVERITY_BLOCKED") => "FAIL",
            _ => "--  ",
        };
        out.push_str(&format!(
            "  {mark}  {}: {}\n",
            finding["check"].as_str().unwrap_or_default(),
            finding["detail"].as_str().unwrap_or_default()
        ));
    }
    if let Some(log) = build["log_url"].as_str().filter(|l| !l.is_empty()) {
        out.push_str(&format!("  log: {log}\n"));
    }
    out
}

/// `meridian plugin publish`: the pre-flight, the request, and the build
/// followed until it settles. The build as it settled.
pub async fn publish(
    asked: &Publish,
    repository: &dyn Repository,
    signed_in: &SignedIn,
) -> Result<serde_json::Value, String> {
    let ready = preflight(asked, repository)?;
    let platform = signed_in.platform().to_string();
    let push = signed_in
        .push_token(&asked.organisation, &ready.plugin)
        .await?;
    let asked_for = serde_json::json!({
        "organisation": asked.organisation,
        "plugin": ready.plugin,
        "registry": asked.registry,
        "repository": ready.repository,
        "tag": asked.tag,
    });
    let path = format!(
        "/api/v1/registry/organisations/{}/plugins/{}/versions",
        asked.organisation, ready.plugin
    );
    let mut build = call(
        &platform,
        reqwest::Method::POST,
        &path,
        &push,
        Some(&asked_for),
    )
    .await?;
    let build_id = build["build_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| format!("{platform} answered the publish with no build"))?
        .to_string();
    eprint!("{}", said(&build));
    let started = tokio::time::Instant::now();
    while !settled(&build) {
        if started.elapsed() >= asked.follow_for {
            return Err(format!(
                "the build {build_id} has not settled within {} minutes; it goes on at the \
                 platform, and its page says how it ends",
                asked.follow_for.as_secs() / 60
            ));
        }
        tokio::time::sleep(asked.follow_every).await;
        // A narrowed token lives ten minutes; a long build asks again.
        let bearer = signed_in
            .push_token(&asked.organisation, &ready.plugin)
            .await?;
        let before = build["state"].clone();
        build = call(
            &platform,
            reqwest::Method::GET,
            &build_path(&asked.organisation, &ready.plugin, &build_id),
            &bearer,
            None,
        )
        .await?;
        if build["state"] != before {
            eprintln!(
                "  {}",
                state_word(build["state"].as_str().unwrap_or_default())
            );
        }
    }
    Ok(build)
}

// ── Signing keys (W5.30) ─────────────────────────────────────────────────

fn keys_path(organisation: &str) -> String {
    format!("/api/v1/registry/organisations/{organisation}/signing-keys")
}

/// An SSH public key's line, held to its form before it is sent: never a
/// private key, which is refused without being sent or echoed.
pub fn public_key(text: &str) -> Result<String, String> {
    if text.contains("PRIVATE KEY") {
        return Err(
            "that is a private key, and only a public half is ever sent: give the .pub file".into(),
        );
    }
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or("the file holds no public key")?;
    let mut parts = line.split_whitespace();
    let kind = parts.next().unwrap_or_default();
    let body = parts.next().unwrap_or_default();
    if !(kind.starts_with("ssh-") || kind.starts_with("ecdsa-sha2-") || kind.starts_with("sk-"))
        || body.is_empty()
    {
        return Err(
            "that is not an SSH public key (`ssh-ed25519 AAAA... name`), which git signs tags \
             with"
                .into(),
        );
    }
    Ok(line.to_string())
}

/// The key's own comment, as the name it is registered under where none is
/// given.
pub fn comment_of(line: &str) -> Option<String> {
    let rest: Vec<&str> = line.split_whitespace().skip(2).collect();
    (!rest.is_empty()).then(|| rest.join(" "))
}

pub async fn register_key(
    signed_in: &SignedIn,
    organisation: &str,
    name: &str,
    public_key: &str,
) -> Result<serde_json::Value, String> {
    let bearer = signed_in.bearer().await?;
    call(
        signed_in.platform(),
        reqwest::Method::POST,
        &keys_path(organisation),
        &bearer,
        Some(&serde_json::json!({
            "organisation": organisation, "name": name, "public_key": public_key,
        })),
    )
    .await
}

pub async fn revoke_key(
    signed_in: &SignedIn,
    organisation: &str,
    key_id: &str,
    not_trusted_after_ns: u64,
    reason: &str,
) -> Result<serde_json::Value, String> {
    let bearer = signed_in.bearer().await?;
    call(
        signed_in.platform(),
        reqwest::Method::DELETE,
        &format!("{}/{key_id}", keys_path(organisation)),
        &bearer,
        Some(&serde_json::json!({
            "organisation": organisation, "key_id": key_id,
            "not_trusted_after_ns": not_trusted_after_ns, "reason": reason,
        })),
    )
    .await
}

pub async fn list_keys(
    signed_in: &SignedIn,
    organisation: &str,
) -> Result<serde_json::Value, String> {
    let bearer = signed_in.bearer().await?;
    call(
        signed_in.platform(),
        reqwest::Method::GET,
        &keys_path(organisation),
        &bearer,
        None,
    )
    .await
}

/// RFC 3339, from nanoseconds since the epoch, for a person to read.
pub fn moment(ns: u64) -> String {
    let seconds = ns / 1_000_000_000;
    let days = seconds / 86_400;
    let clock = seconds % 86_400;
    // Days since the epoch to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        clock / 3_600,
        clock % 3_600 / 60,
        clock % 60
    )
}

/// `2026-10-16T12:00Z`, `2026-10-16T12:00:00Z` or `2026-10-16`, as
/// nanoseconds since the epoch, UTC.
pub fn ns_of(moment: &str) -> Option<u64> {
    let moment = moment.trim().trim_end_matches('Z');
    let (date, clock) = moment.split_once('T').unwrap_or((moment, "00:00:00"));
    let mut d = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (d.next()??, d.next()??, d.next()??);
    if d.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut c = clock.split(':').map(|part| part.parse::<i64>().ok());
    let (hours, minutes) = (c.next()??, c.next()??);
    let seconds = c.next().flatten().unwrap_or(0);
    if !(0..24).contains(&hours) || !(0..60).contains(&minutes) || !(0..61).contains(&seconds) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let total = days * 86_400 + hours * 3_600 + minutes * 60 + seconds;
    u64::try_from(total).ok().map(|s| s * 1_000_000_000)
}

/// The organisation's keys, for a person to read: active first.
pub fn keys_said(listed: &serde_json::Value) -> String {
    let keys = listed["keys"].as_array().cloned().unwrap_or_default();
    if keys.is_empty() {
        return "No signing keys registered.\n".into();
    }
    let mut out = String::new();
    for key in &keys {
        let revoked = key["revoked_at_ns"].as_u64().unwrap_or(0);
        out.push_str(&format!(
            "{}  {}  {}  {}\n",
            key["key_id"].as_str().unwrap_or_default(),
            key["fingerprint"].as_str().unwrap_or_default(),
            key["name"].as_str().unwrap_or_default(),
            if revoked == 0 {
                format!(
                    "active since {}",
                    moment(key["registered_at_ns"].as_u64().unwrap_or(0))
                )
            } else {
                format!(
                    "revoked {}, not trusted after {}",
                    moment(revoked),
                    moment(key["not_trusted_after_ns"].as_u64().unwrap_or(0))
                )
            }
        ));
    }
    out
}

#[cfg(test)]
mod tests;

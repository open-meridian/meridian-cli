//! `meridian plugin release <version>` (ruling 323, revising 256): a
//! plugin's release in one command, which checks, signs and publishes. It
//! makes no pull request and writes no file: the version bump and the
//! CHANGELOG entry went in with the change's own pull request.
//!
//! 1. **Checks**: the plugin's checkout is on its default branch, clean, and
//!    up to date with `origin`; the version is higher than the last release
//!    tag; `pyproject.toml`'s version is already that version; the CHANGELOG
//!    has its `## <version>` heading, undated (the date lives on the tag and
//!    the listing); and that branch's last CI run is green. Each refusal says
//!    what to do next.
//! 2. **The tag**: `v<version>` signed (git's SSH signing key, which `meridian
//!    plugin key setup` makes ready) on the branch's head, and pushed. Run
//!    again once the tag is on origin, it publishes that tag.
//! 3. **Publishing**: the existing path (`meridian plugin publish`), its build
//!    followed, and the listing's address printed. No package-visibility step:
//!    the builder pushes the image, and a package it pushes is created public.
//!
//! GitHub is asked through its REST API with the person's own GitHub sign-in:
//! `GH_TOKEN` or `GITHUB_TOKEN` where one is set, else the credential git
//! keeps for github.com (`git credential fill`, never prompting). Nothing new
//! is installed for it, and the token is never printed or kept.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::registry;

// ── The repository ───────────────────────────────────────────────────────

/// The plugin's checkout, as release reaches it through git.
pub trait Repository: Sync {
    /// The branch checked out, or None when detached.
    fn branch(&self) -> Result<Option<String>, String>;
    /// Whether the working tree holds no change git does not.
    fn clean(&self) -> Result<bool, String>;
    /// `git fetch origin --tags`.
    fn fetch(&self) -> Result<(), String>;
    /// A revision's commit, or None where it names none.
    fn commit_of(&self, revision: &str) -> Option<String>;
    /// Every tag the repository holds.
    fn tags(&self) -> Result<Vec<String>, String>;
    /// The commit a tag at `origin` names, or None where origin has none.
    fn remote_tag(&self, tag: &str) -> Result<Option<String>, String>;
    fn origin(&self) -> Result<String, String>;
    /// A file at `revision`, byte for byte (its final newline kept), or
    /// None where it has none.
    fn show(&self, revision: &str, path: &str) -> Option<String>;
    /// `git config --get <key>`.
    fn config(&self, key: &str) -> Option<String>;
    /// `tag` made at `commit`, annotated with `message` and signed (`-s`).
    fn sign_tag(&self, tag: &str, commit: &str, message: &str) -> Result<(), String>;
    fn push_tag(&self, tag: &str) -> Result<(), String>;
}

pub struct Checkout(pub PathBuf);

impl Checkout {
    fn git(&self, arguments: &[&str]) -> Result<String, String> {
        git_in(&self.0, arguments)
    }
}

fn git_in(dir: &Path, arguments: &[&str]) -> Result<String, String> {
    let shown = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(arguments)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|failed| format!("git could not be run: {failed}"))?;
    if shown.status.success() {
        Ok(String::from_utf8_lossy(&shown.stdout).trim().to_string())
    } else {
        Err(format!(
            "`git {}`: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&shown.stderr).trim()
        ))
    }
}

/// git with the terminal: what may ask for a key's passphrase or touch.
fn git_at_terminal(dir: &Path, arguments: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(arguments)
        .status()
        .map_err(|failed| format!("git could not be run: {failed}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`git {}` failed ({status})", arguments.join(" ")))
    }
}

impl Repository for Checkout {
    fn branch(&self) -> Result<Option<String>, String> {
        let named = self.git(&["rev-parse", "--abbrev-ref", "HEAD"])?;
        Ok((named != "HEAD").then_some(named))
    }

    fn clean(&self) -> Result<bool, String> {
        Ok(self
            .git(&["status", "--porcelain", "--untracked-files=normal"])?
            .is_empty())
    }

    fn fetch(&self) -> Result<(), String> {
        self.git(&["fetch", "--quiet", "--tags", "origin"])
            .map(|_| ())
    }

    fn commit_of(&self, revision: &str) -> Option<String> {
        self.git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{revision}^{{commit}}"),
        ])
        .ok()
        .filter(|sha| !sha.is_empty())
    }

    fn tags(&self) -> Result<Vec<String>, String> {
        Ok(self
            .git(&["tag", "--list"])?
            .lines()
            .map(str::to_string)
            .collect())
    }

    fn remote_tag(&self, tag: &str) -> Result<Option<String>, String> {
        let listed = self.git(&[
            "ls-remote",
            "--tags",
            "origin",
            &format!("refs/tags/{tag}"),
            &format!("refs/tags/{tag}^{{}}"),
        ])?;
        // The peeled line (`^{}`) names the commit an annotated tag points at.
        let mut found = None;
        for line in listed.lines() {
            let mut parts = line.split_whitespace();
            let (Some(sha), Some(name)) = (parts.next(), parts.next()) else {
                continue;
            };
            if name.ends_with("^{}") || found.is_none() {
                found = Some(sha.to_string());
            }
        }
        Ok(found)
    }

    fn origin(&self) -> Result<String, String> {
        self.git(&["remote", "get-url", "origin"])
    }

    fn show(&self, revision: &str, path: &str) -> Option<String> {
        // Not through `git`, which trims what git printed: a file is read
        // as git holds it, its final newline kept. (0.1.48's release PR
        // wrote CHANGELOG.md and pyproject.toml back from this trimmed, so
        // without their final newline.)
        let shown = std::process::Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(["show", &format!("{revision}:{path}")])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .ok()?;
        shown
            .status
            .success()
            .then(|| String::from_utf8_lossy(&shown.stdout).into_owned())
    }

    fn config(&self, key: &str) -> Option<String> {
        self.git(&["config", "--get", key])
            .ok()
            .filter(|value| !value.is_empty())
    }

    fn sign_tag(&self, tag: &str, commit: &str, message: &str) -> Result<(), String> {
        git_at_terminal(&self.0, &["tag", "-s", "-m", message, tag, commit])
    }

    fn push_tag(&self, tag: &str) -> Result<(), String> {
        git_at_terminal(
            &self.0,
            &["push", "--quiet", "origin", &format!("refs/tags/{tag}")],
        )
    }
}

// ── GitHub ───────────────────────────────────────────────────────────────

/// Where a commit's CI stands: its check runs and statuses together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ci {
    /// No run and no status on it.
    None,
    Pending,
    Green,
    /// Each that failed: its name and its page.
    Red(Vec<String>),
}

/// GitHub, as release asks it.
#[async_trait::async_trait]
pub trait GitHub: Sync {
    async fn default_branch(&self) -> Result<String, String>;
    async fn ci(&self, commit: &str) -> Result<Ci, String>;
}

/// The person's GitHub sign-in: `GH_TOKEN` or `GITHUB_TOKEN`, else the
/// credential git keeps for github.com, asked of git without a prompt.
pub fn github_token() -> Result<String, String> {
    for variable in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Some(token) = std::env::var(variable)
            .ok()
            .filter(|t| !t.trim().is_empty())
        {
            return Ok(token.trim().to_string());
        }
    }
    use std::io::Write as _;
    let mut child = std::process::Command::new("git")
        .args(["credential", "fill"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|failed| format!("git could not be run: {failed}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"protocol=https\nhost=github.com\n\n");
    }
    let said = child
        .wait_with_output()
        .map_err(|failed| format!("git could not be run: {failed}"))?;
    let password = String::from_utf8_lossy(&said.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("password=").map(str::to_string))
        .filter(|token| !token.is_empty());
    password.ok_or_else(|| {
        "main's CI is read through GitHub's API with your GitHub sign-in, and none was found: \
         not GH_TOKEN, not GITHUB_TOKEN, and git keeps no credential for github.com. `gh auth \
         setup-git` gives git one, or set GH_TOKEN to a token that may read checks on the \
         plugin's repository"
            .to_string()
    })
}

/// GitHub's REST API for one repository, at `api` (https://api.github.com).
pub struct Rest {
    pub api: String,
    pub owner: String,
    pub repository: String,
    pub token: String,
}

impl Rest {
    /// What every call to GitHub carries: this CLI's version, and a
    /// User-Agent, without which GitHub's REST API refuses the request (403,
    /// "Request forbidden by administrative rules").
    pub(crate) fn headers() -> reqwest::header::HeaderMap {
        let mut headers = crate::release::naming_this_version();
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_str(&format!(
                "meridian/{}",
                crate::release::VERSION
            ))
            .expect("the version is ASCII"),
        );
        headers
    }

    pub(crate) fn client() -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers(Self::headers())
            .build()
            .map_err(|failed| failed.to_string())
    }

    async fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let url = format!(
            "{}/repos/{}/{}{path}",
            self.api, self.owner, self.repository
        );
        let mut request = Self::client()?
            .request(method, &url)
            .bearer_auth(&self.token)
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28");
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let answer = request
            .send()
            .await
            .map_err(|failed| format!("could not reach GitHub: {failed}"))?;
        let status = answer.status();
        let text = answer.text().await.unwrap_or_default();
        if !status.is_success() {
            let said: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
            let message = said["message"].as_str().unwrap_or(text.trim());
            return Err(match status.as_u16() {
                401 => format!("GitHub did not take your sign-in ({message})"),
                403 | 404 => format!(
                    "GitHub refused ({status}: {message}): your sign-in may not reach \
                     {}/{}, or may not read its checks",
                    self.owner, self.repository
                ),
                _ => format!("GitHub refused ({status}): {message}"),
            });
        }
        serde_json::from_str(&text)
            .map_err(|failed| format!("GitHub answered unreadably: {failed}"))
    }
}

/// Check runs' and statuses' outcomes, together.
pub fn ci_of(runs: &serde_json::Value, statuses: &serde_json::Value) -> Ci {
    let mut failed = Vec::new();
    let mut pending = false;
    let mut seen = 0;
    for run in runs["check_runs"].as_array().into_iter().flatten() {
        seen += 1;
        if run["status"] != "completed" {
            pending = true;
            continue;
        }
        let conclusion = run["conclusion"].as_str().unwrap_or_default();
        if !matches!(conclusion, "success" | "neutral" | "skipped") {
            failed.push(format!(
                "{} ({conclusion}) {}",
                run["name"].as_str().unwrap_or_default(),
                run["html_url"].as_str().unwrap_or_default()
            ));
        }
    }
    for status in statuses["statuses"].as_array().into_iter().flatten() {
        seen += 1;
        match status["state"].as_str().unwrap_or_default() {
            "success" => {}
            "pending" => pending = true,
            other => failed.push(format!(
                "{} ({other}) {}",
                status["context"].as_str().unwrap_or_default(),
                status["target_url"].as_str().unwrap_or_default()
            )),
        }
    }
    if !failed.is_empty() {
        Ci::Red(failed)
    } else if pending {
        Ci::Pending
    } else if seen == 0 {
        Ci::None
    } else {
        Ci::Green
    }
}

#[async_trait::async_trait]
impl GitHub for Rest {
    async fn default_branch(&self) -> Result<String, String> {
        let said = self.call(reqwest::Method::GET, "", None).await?;
        said["default_branch"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| "GitHub named no default branch".into())
    }

    async fn ci(&self, commit: &str) -> Result<Ci, String> {
        let runs = self
            .call(
                reqwest::Method::GET,
                &format!("/commits/{commit}/check-runs?per_page=100"),
                None,
            )
            .await?;
        let statuses = self
            .call(
                reqwest::Method::GET,
                &format!("/commits/{commit}/status"),
                None,
            )
            .await?;
        Ok(ci_of(&runs, &statuses))
    }
}

/// `https://github.com/<owner>/<repository>` as its owner and name.
pub fn owner_and_name(repository: &str) -> Option<(String, String)> {
    let path = repository.strip_prefix("https://github.com/")?;
    let (owner, name) = path.split_once('/')?;
    Some((owner.to_string(), name.to_string()))
}

// ── The files ────────────────────────────────────────────────────────────

/// `[project]`'s version in pyproject.toml, or None where it names none.
pub fn version_of(pyproject: &str) -> Option<String> {
    let mut in_project = false;
    for line in pyproject.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_project = trimmed == "[project]";
            continue;
        }
        if !in_project {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("version") {
            if let Some(value) = rest.trim_start().strip_prefix('=') {
                let value = value.split('#').next().unwrap_or_default().trim();
                return Some(value.trim_matches(['"', '\'']).to_string());
            }
        }
    }
    None
}

/// Where the CHANGELOG stands for `version`: Ok when it has the heading
/// `## <version>` alone; else what to do, said.
pub fn changelog_entry(changelog: Option<&str>, version: &str) -> Result<(), String> {
    let wanted = format!("## {version}");
    let next = "add it, with this release's changes, in a pull request (with pyproject.toml's \
                version), merge it, then run this again";
    let Some(text) = changelog else {
        return Err(format!(
            "the plugin has no CHANGELOG.md in git, so no `{wanted}` entry: {next}"
        ));
    };
    // `## 0.4.0`, `## [0.4.0]`, `## 0.4.0 (2026-10-18)`, `## 0.4.0 (unreleased)`:
    // each names 0.4.0; `## 0.4.01` does not.
    let names_it = |line: &str| {
        line.strip_prefix("## ").is_some_and(|rest| {
            rest.split_whitespace()
                .next()
                .is_some_and(|first| first.trim_matches(['[', ']']) == version)
        })
    };
    match text.lines().map(str::trim_end).find(|line| names_it(line)) {
        Some(line) if line == wanted => Ok(()),
        Some(line) => Err(format!(
            "CHANGELOG.md's heading for {version} is `{line}`: make it `{wanted}` alone, with no \
             date and no \"(unreleased)\" (the date lives on the tag and the listing), in a pull \
             request, merge it, then run this again"
        )),
        None => Err(format!("CHANGELOG.md has no `{wanted}` heading: {next}")),
    }
}

/// The highest `vX.Y.Z` among the tags.
pub fn last_release(tags: &[String]) -> Option<String> {
    tags.iter()
        .filter_map(|tag| registry::semver_of_tag(tag))
        .max_by(|a, b| crate::release::compare(a, b))
        .map(String::from)
}

// ── The release ──────────────────────────────────────────────────────────

/// What `meridian plugin release` was asked.
pub struct Asked {
    pub dir: PathBuf,
    pub version: String,
}

/// The tag on origin, signed, at this commit: publish it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged {
    pub tag: String,
    pub commit: String,
}

/// The release, from its checks to its tag on origin. Each step said
/// through `say`.
pub async fn release(
    asked: &Asked,
    repo: &dyn Repository,
    github: &dyn GitHub,
    say: &mut (dyn FnMut(&str) + Send),
) -> Result<Tagged, String> {
    let version = asked.version.as_str();
    let tag = format!("v{version}");
    if registry::semver_of_tag(&tag).is_none() {
        return Err(format!(
            "{version} is not a version: a SemVer 2.0 version, as 1.4.0"
        ));
    }
    repo.fetch()?;
    let main = github.default_branch().await?;
    let pyproject_now = std::fs::read_to_string(asked.dir.join("pyproject.toml"))
        .map_err(|failed| format!("{} has no pyproject.toml: {failed}", asked.dir.display()))?;
    let name = crate::catalogue::metadata(&pyproject_now)?.name;

    // A tag on origin already: signed and pushed by an earlier run, whose
    // publish did not finish.
    if let Some(commit) = repo.remote_tag(&tag)? {
        say(&format!(
            "{tag} is on origin already, at {}; publishing it.",
            short(&commit)
        ));
        return Ok(Tagged { tag, commit });
    }

    // Checks, before anything is made.
    let on = repo.branch()?;
    if on.as_deref() != Some(main.as_str()) {
        return Err(format!(
            "the checkout is on {}, not {main}: a release is made from {main}",
            on.unwrap_or_else(|| "no branch".into())
        ));
    }
    if !repo.clean()? {
        return Err("the checkout holds changes git does not: commit or stash them first".into());
    }
    let head = repo.commit_of("HEAD").ok_or("HEAD names no commit")?;
    let upstream = repo
        .commit_of(&format!("origin/{main}"))
        .ok_or_else(|| format!("origin has no {main}"))?;
    if head != upstream {
        return Err(format!(
            "{main} here ({}) is not origin's ({}): pull or push first",
            short(&head),
            short(&upstream)
        ));
    }
    let tags = repo.tags()?;
    if let Some(last) = last_release(&tags) {
        if crate::release::compare(version, &last) != std::cmp::Ordering::Greater {
            return Err(format!(
                "{version} is not higher than the last release tag, v{last}"
            ));
        }
    }
    let pyproject = repo
        .show(&head, "pyproject.toml")
        .ok_or("pyproject.toml is not in git")?;
    match version_of(&pyproject) {
        Some(held) if held == version => {}
        Some(held) => {
            return Err(format!(
                "pyproject.toml's version is {held}, not {version}: raise it to {version}, with \
                 its `## {version}` CHANGELOG entry, in a pull request, merge it, then run this \
                 again"
            ))
        }
        None => return Err("pyproject.toml's [project] names no version".into()),
    }
    changelog_entry(repo.show(&head, "CHANGELOG.md").as_deref(), version)?;
    match github.ci(&head).await? {
        Ci::Green => {}
        Ci::None => {
            return Err(format!(
                "{main}'s head, {}, has no CI run: a release is made from a green {main}",
                short(&head)
            ))
        }
        Ci::Pending => {
            return Err(format!(
                "{main}'s CI is still running on {}: run this again once it is green",
                short(&head)
            ))
        }
        Ci::Red(failed) => {
            return Err(format!(
                "{main}'s last CI run on {} is not green:\n  {}",
                short(&head),
                failed.join("\n  ")
            ))
        }
    }
    say(&format!(
        "Checked: on {main}, clean, up to date with origin, {version} higher than the last tag, \
         pyproject.toml and CHANGELOG.md at {version}, CI green on {}.",
        short(&head)
    ));
    tagged(repo, &tag, &head, &name, version, say)
}

/// `tag` signed at `commit` and pushed: the person's own SSH signing key.
fn tagged(
    repo: &dyn Repository,
    tag: &str,
    commit: &str,
    name: &str,
    version: &str,
    say: &mut (dyn FnMut(&str) + Send),
) -> Result<Tagged, String> {
    if repo.config("gpg.format").as_deref() != Some("ssh")
        || repo.config("user.signingkey").is_none()
    {
        return Err(format!(
            "git has no SSH key to sign {tag} with: `meridian plugin key setup` makes one ready, \
             then run this again"
        ));
    }
    match repo.commit_of(&format!("refs/tags/{tag}")) {
        Some(held) if held == commit => {}
        Some(held) => {
            return Err(format!(
                "{tag} is here already, at {}, not at {}: delete it (`git tag -d {tag}`) if it is \
                 a mistake, then run this again",
                short(&held),
                short(commit)
            ))
        }
        None => {
            repo.sign_tag(tag, commit, &format!("{name} {version}"))?;
            say(&format!("Signed {tag} at {}.", short(commit)));
        }
    }
    repo.push_tag(tag)?;
    say(&format!("Pushed {tag}."));
    Ok(Tagged {
        tag: tag.to_string(),
        commit: commit.to_string(),
    })
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

#[cfg(test)]
mod tests;

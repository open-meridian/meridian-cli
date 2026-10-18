//! `meridian plugin release <version>` (ruling 256): a plugin's release in
//! one command, its merge a person's.
//!
//! 1. **Checks**: the plugin's checkout is on its default branch, clean, and
//!    up to date with `origin`; that branch's last CI run is green; and the
//!    version is higher than the last release tag.
//! 2. **The release PR**: `pyproject.toml`'s version and the CHANGELOG's
//!    heading for it, on a branch `release/v<version>` made in a worktree of
//!    its own (the person's checkout is never switched), pushed, and a pull
//!    request opened. Then its CI is waited for, and then its merge, which
//!    stays a person's. `--no-wait` stops once the PR is open; running the
//!    command again resumes from whatever has happened since: a PR open, one
//!    merged, a tag made.
//! 3. **The tag**: `v<version>` signed (git's SSH signing key, which `meridian
//!    plugin key setup` makes ready) on the merge commit, and pushed.
//! 4. **Publishing**: the existing path (`meridian plugin publish`), its build
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

/// The branch a release's PR is made on.
pub fn branch_of(version: &str) -> String {
    format!("release/v{version}")
}

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
    /// A file at `revision`, or None where it has none.
    fn show(&self, revision: &str, path: &str) -> Option<String>;
    /// `files` written over `base` and committed as `message` on `branch`,
    /// in a worktree of its own, and pushed to origin: the commit.
    fn push_branch(
        &self,
        base: &str,
        branch: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<String, String>;
    /// `git merge --ff-only origin/<branch>`, on a clean checkout of it.
    fn fast_forward(&self, branch: &str) -> Result<(), String>;
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
        self.git(&["show", &format!("{revision}:{path}")]).ok()
    }

    fn push_branch(
        &self,
        base: &str,
        branch: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<String, String> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();
        let worktree = std::env::temp_dir().join(format!("meridian-release-{unique}"));
        let at = worktree.display().to_string();
        self.git(&["worktree", "add", "--quiet", "--detach", &at, base])?;
        let made = (|| {
            for (path, text) in files {
                std::fs::write(worktree.join(path), text)
                    .map_err(|failed| format!("{path}: {failed}"))?;
                git_in(&worktree, &["add", "--", path])?;
            }
            git_at_terminal(&worktree, &["commit", "--quiet", "-m", message])?;
            let commit = git_in(&worktree, &["rev-parse", "HEAD"])?;
            git_at_terminal(
                &worktree,
                &[
                    "push",
                    "--quiet",
                    "origin",
                    &format!("HEAD:refs/heads/{branch}"),
                ],
            )?;
            Ok(commit)
        })();
        let _ = self.git(&["worktree", "remove", "--force", &at]);
        made
    }

    fn fast_forward(&self, branch: &str) -> Result<(), String> {
        self.git(&["merge", "--ff-only", "--quiet", &format!("origin/{branch}")])
            .map(|_| ())
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

/// A pull request, as release reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pull {
    pub number: u64,
    pub url: String,
    pub head: String,
    pub state: String,
    pub merged: bool,
    pub merge_commit: Option<String>,
}

/// GitHub, as release asks it.
#[async_trait::async_trait]
pub trait GitHub: Sync {
    async fn default_branch(&self) -> Result<String, String>;
    async fn ci(&self, commit: &str) -> Result<Ci, String>;
    /// The release PR from `branch`, the newest, open or closed.
    async fn pull_from(&self, branch: &str) -> Result<Option<Pull>, String>;
    async fn pull(&self, number: u64) -> Result<Pull, String>;
    async fn open_pull(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<Pull, String>;
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
        "the release PR is opened through GitHub's API with your GitHub sign-in, and none was \
         found: not GH_TOKEN, not GITHUB_TOKEN, and git keeps no credential for github.com. \
         `gh auth setup-git` gives git one, or set GH_TOKEN to a token that may write pull \
         requests and read checks on the plugin's repository"
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
    fn client() -> Result<reqwest::Client, String> {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers(crate::release::naming_this_version())
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
                     {}/{}, or may not write pull requests there",
                    self.owner, self.repository
                ),
                _ => format!("GitHub refused ({status}): {message}"),
            });
        }
        serde_json::from_str(&text)
            .map_err(|failed| format!("GitHub answered unreadably: {failed}"))
    }
}

fn pull_of(said: &serde_json::Value) -> Pull {
    Pull {
        number: said["number"].as_u64().unwrap_or(0),
        url: said["html_url"].as_str().unwrap_or_default().to_string(),
        head: said["head"]["sha"].as_str().unwrap_or_default().to_string(),
        state: said["state"].as_str().unwrap_or_default().to_string(),
        merged: said["merged_at"].as_str().is_some_and(|at| !at.is_empty())
            || said["merged"] == true,
        merge_commit: said["merge_commit_sha"]
            .as_str()
            .filter(|sha| !sha.is_empty())
            .map(String::from),
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

    async fn pull_from(&self, branch: &str) -> Result<Option<Pull>, String> {
        let said = self
            .call(
                reqwest::Method::GET,
                &format!(
                    "/pulls?state=all&head={}:{branch}&sort=created&direction=desc",
                    self.owner
                ),
                None,
            )
            .await?;
        Ok(said.as_array().and_then(|pulls| pulls.first()).map(pull_of))
    }

    async fn pull(&self, number: u64) -> Result<Pull, String> {
        let said = self
            .call(reqwest::Method::GET, &format!("/pulls/{number}"), None)
            .await?;
        Ok(pull_of(&said))
    }

    async fn open_pull(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<Pull, String> {
        let said = self
            .call(
                reqwest::Method::POST,
                "/pulls",
                Some(&serde_json::json!({
                    "title": title, "head": branch, "base": base, "body": body,
                })),
            )
            .await?;
        Ok(pull_of(&said))
    }
}

/// `https://github.com/<owner>/<repository>` as its owner and name.
pub fn owner_and_name(repository: &str) -> Option<(String, String)> {
    let path = repository.strip_prefix("https://github.com/")?;
    let (owner, name) = path.split_once('/')?;
    Some((owner.to_string(), name.to_string()))
}

// ── The files ────────────────────────────────────────────────────────────

/// pyproject.toml with `[project]`'s version set to `version`; None where
/// it names none.
pub fn with_version(pyproject: &str, version: &str) -> Option<String> {
    let mut out = String::with_capacity(pyproject.len());
    let mut in_project = false;
    let mut done = false;
    for line in pyproject.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_project = trimmed == "[project]";
        }
        if in_project && !done {
            if let Some(rest) = trimmed.strip_prefix("version") {
                if rest.trim_start().starts_with('=') {
                    let end = if line.ends_with('\n') { "\n" } else { "" };
                    let indent = &line[..line.len() - line.trim_start().len()];
                    out.push_str(&format!("{indent}version = \"{version}\"{end}"));
                    done = true;
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    done.then_some(out)
}

/// The CHANGELOG with `version`'s heading dated `date`: its own heading
/// (`## 0.4.0 (not released)`) or an `## Unreleased` one renamed, or a new
/// heading above the first release's. Without a CHANGELOG, one is begun.
pub fn with_heading(changelog: Option<&str>, version: &str, date: &str) -> String {
    let heading = format!("## {version} ({date})");
    let Some(text) = changelog else {
        return format!("# Changelog\n\n{heading}\n");
    };
    let is_versions = |line: &str| {
        let rest = line.trim_start_matches('#').trim();
        line.starts_with("## ")
            && (rest == version
                || rest.starts_with(&format!("{version} "))
                || rest == format!("[{version}]")
                || rest.starts_with(&format!("[{version}] ")))
    };
    let is_unreleased = |line: &str| {
        line.starts_with("## ")
            && line
                .trim_start_matches('#')
                .trim()
                .trim_matches(['[', ']'])
                .eq_ignore_ascii_case("unreleased")
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let at = lines
        .iter()
        .position(|line| is_versions(line))
        .or_else(|| lines.iter().position(|line| is_unreleased(line)));
    if let Some(at) = at {
        let mut out = String::new();
        for (n, line) in lines.iter().enumerate() {
            if n == at {
                out.push_str(&heading);
                out.push('\n');
            } else {
                out.push_str(line);
            }
        }
        return out;
    }
    match lines.iter().position(|line| line.starts_with("## ")) {
        Some(first) => {
            let mut out: String = lines[..first].concat();
            out.push_str(&format!("{heading}\n\n"));
            out.push_str(&lines[first..].concat());
            out
        }
        None => {
            let mut out = text.trim_end().to_string();
            out.push_str(&format!("\n\n{heading}\n"));
            out
        }
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
    pub wait: bool,
    pub every: Duration,
    pub timeout: Duration,
    /// Today, `YYYY-MM-DD`, the CHANGELOG heading's date.
    pub today: String,
}

/// Where a release stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reached {
    /// The PR is open (`--no-wait`), or a wait timed out: what to say.
    Stopped(String),
    /// The tag is on origin at this commit, signed: publish it.
    Tagged { tag: String, commit: String },
}

/// The release, from wherever it stands, to its tag on origin. Each step
/// said through `say`.
pub async fn release(
    asked: &Asked,
    repo: &dyn Repository,
    github: &dyn GitHub,
    say: &mut (dyn FnMut(&str) + Send),
) -> Result<Reached, String> {
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

    // A tag on origin already: the release is past its merge.
    if let Some(commit) = repo.remote_tag(&tag)? {
        say(&format!(
            "{tag} is on origin already, at {}; publishing it.",
            short(&commit)
        ));
        return Ok(Reached::Tagged { tag, commit });
    }

    let branch = branch_of(version);
    let pull = match github.pull_from(&branch).await? {
        Some(pull) if pull.merged => Some(pull),
        Some(pull) if pull.state == "open" => Some(pull),
        Some(pull) => {
            return Err(format!(
                "the release PR for {version}, #{} ({}), was closed without being merged: reopen \
                 it, or delete the branch {branch} at origin to begin again",
                pull.number, pull.url
            ))
        }
        None => None,
    };

    let pull = match pull {
        Some(pull) => {
            say(&format!("The release PR is #{}: {}", pull.number, pull.url));
            pull
        }
        None => {
            // Checks, before anything is made.
            let on = repo.branch()?;
            if on.as_deref() != Some(main.as_str()) {
                return Err(format!(
                    "the checkout is on {}, not {main}: a release is made from {main}",
                    on.unwrap_or_else(|| "no branch".into())
                ));
            }
            if !repo.clean()? {
                return Err(
                    "the checkout holds changes git does not: commit or stash them first".into(),
                );
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
            let tags = repo.tags()?;
            if let Some(last) = last_release(&tags) {
                if crate::release::compare(version, &last) != std::cmp::Ordering::Greater {
                    return Err(format!(
                        "{version} is not higher than the last release tag, v{last}"
                    ));
                }
            }
            say(&format!(
                "Checked: on {main}, clean, up to date with origin, CI green on {}, {version} \
                 higher than the last tag.",
                short(&head)
            ));

            // The release PR's files.
            let pyproject = repo
                .show(&head, "pyproject.toml")
                .ok_or("pyproject.toml is not in git")?;
            let bumped = with_version(&pyproject, version)
                .ok_or("pyproject.toml's [project] names no version")?;
            let changelog = repo.show(&head, "CHANGELOG.md");
            let dated = with_heading(changelog.as_deref(), version, &asked.today);
            let mut files = Vec::new();
            if bumped != pyproject {
                files.push(("pyproject.toml".to_string(), bumped));
            }
            if changelog.as_deref() != Some(dated.as_str()) {
                files.push(("CHANGELOG.md".to_string(), dated));
            }
            if files.is_empty() {
                // Nothing to change: the tag goes on the green head.
                say(&format!(
                    "pyproject.toml and CHANGELOG.md say {version} already: no release PR."
                ));
                return tagged(repo, &tag, &head, &name, version, say);
            }
            let commit = repo.push_branch(&head, &branch, &files, &format!("Release {version}"))?;
            let body = format!(
                "The release of {name} {version}: pyproject.toml's version and the CHANGELOG's \
                 heading.\n\nMerging it is yours. Once it is merged, `meridian plugin release \
                 {version}` signs `{tag}` on the merge commit, pushes it, and publishes it \
                 (it resumes where it stopped)."
            );
            let pull = github
                .open_pull(&branch, &main, &format!("Release {version}"), &body)
                .await?;
            say(&format!(
                "Opened the release PR #{}, at {}: {}",
                pull.number,
                short(&commit),
                pull.url
            ));
            pull
        }
    };

    if !pull.merged {
        if !asked.wait {
            return Ok(Reached::Stopped(format!(
                "The release PR is #{}: {}\nMerge it once its CI is green, then run `meridian \
                 plugin release {version}` again: it signs and pushes {tag} and publishes it.",
                pull.number, pull.url
            )));
        }
        // Its CI, then its merge, which is a person's.
        let started = tokio::time::Instant::now();
        let mut said_waiting = false;
        loop {
            let now = github.pull(pull.number).await?;
            if now.merged {
                break;
            }
            if now.state != "open" {
                return Err(format!(
                    "the release PR #{} was closed without being merged: {}",
                    now.number, now.url
                ));
            }
            match github.ci(&now.head).await? {
                Ci::Red(failed) => {
                    return Err(format!(
                        "the release PR's CI failed:\n  {}\nFix it on {branch}, then run this \
                         again: {}",
                        failed.join("\n  "),
                        now.url
                    ))
                }
                Ci::Green if !said_waiting => {
                    say(&format!(
                        "CI is green on #{}. Waiting for it to be merged, which is a person's: {}",
                        now.number, now.url
                    ));
                    said_waiting = true;
                }
                _ => {}
            }
            if started.elapsed() >= asked.timeout {
                return Ok(Reached::Stopped(format!(
                    "Stopped waiting after {} minutes; #{} is not merged yet: {}\nRun `meridian \
                     plugin release {version}` again to go on from here.",
                    asked.timeout.as_secs() / 60,
                    now.number,
                    now.url
                )));
            }
            tokio::time::sleep(asked.every).await;
        }
    }
    let merged = github.pull(pull.number).await?;
    let commit = merged
        .merge_commit
        .ok_or_else(|| format!("GitHub names no merge commit for #{}", merged.number))?;
    say(&format!(
        "#{} is merged, as {}.",
        merged.number,
        short(&commit)
    ));
    repo.fetch()?;
    // The checkout brought up to the merge, so the publish reads the
    // released pyproject.toml and holds the tag it signs.
    if repo.branch()?.as_deref() == Some(main.as_str()) && repo.clean()? {
        repo.fast_forward(&main)?;
    } else {
        return Err(format!(
            "#{} is merged; the checkout must be on {main} and clean to sign and publish it: \
             switch to {main}, then run this again",
            merged.number
        ));
    }
    tagged(repo, &tag, &commit, &name, version, say)
}

/// `tag` signed at `commit` and pushed: the person's own SSH signing key.
fn tagged(
    repo: &dyn Repository,
    tag: &str,
    commit: &str,
    name: &str,
    version: &str,
    say: &mut (dyn FnMut(&str) + Send),
) -> Result<Reached, String> {
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
    Ok(Reached::Tagged {
        tag: tag.to_string(),
        commit: commit.to_string(),
    })
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(7)]
}

#[cfg(test)]
mod tests;

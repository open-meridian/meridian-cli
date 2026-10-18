use std::sync::Mutex;

use super::*;
use crate::fake_http::{serve, Asked as Request, Reply, Scratch};

const HEAD: &str = "4e1d9c27b85a03f6e2c1d0b9a8f7e6d5c4b3a291";
const MERGE: &str = "9a8f7e6d5c4b3a2914e1d9c27b85a03f6e2c1d0b";
const PYPROJECT: &str = "[project]\nname = \"venue-desk\"\nversion = \"0.3.0\"\ndependencies = [\"open-meridian==0.30.0\"]\n\n[tool.meridian]\nroles = []\ninterface = true\n";
const CHANGELOG: &str = "# Changelog\n\n## 0.4.0 (not released)\n\n- Something.\n\n## 0.3.0 (2026-10-01)\n\n- Before.\n";

/// Each branch pushed, with its files.
type Pushed = Vec<(String, Vec<(String, String)>)>;

/// The checkout, answering as the test says, keeping what was done.
struct Repo {
    branch: Option<String>,
    clean: bool,
    head: String,
    upstream: String,
    tags: Vec<String>,
    remote_tag: Option<String>,
    local_tag: Mutex<Option<String>>,
    signing: bool,
    did: Mutex<Vec<String>>,
    pushed: Mutex<Pushed>,
}

impl Default for Repo {
    fn default() -> Repo {
        Repo {
            branch: Some("main".into()),
            clean: true,
            head: HEAD.into(),
            upstream: HEAD.into(),
            tags: vec!["v0.2.0".into(), "v0.3.0".into(), "not-a-release".into()],
            remote_tag: None,
            local_tag: Mutex::new(None),
            signing: true,
            did: Mutex::new(Vec::new()),
            pushed: Mutex::new(Vec::new()),
        }
    }
}

impl Repo {
    fn did(&self) -> Vec<String> {
        self.did.lock().unwrap().clone()
    }
}

impl Repository for Repo {
    fn branch(&self) -> Result<Option<String>, String> {
        Ok(self.branch.clone())
    }
    fn clean(&self) -> Result<bool, String> {
        Ok(self.clean)
    }
    fn fetch(&self) -> Result<(), String> {
        self.did.lock().unwrap().push("fetch".into());
        Ok(())
    }
    fn commit_of(&self, revision: &str) -> Option<String> {
        match revision {
            "HEAD" => Some(self.head.clone()),
            "origin/main" => Some(self.upstream.clone()),
            tag if tag.starts_with("refs/tags/") => self.local_tag.lock().unwrap().clone(),
            _ => None,
        }
    }
    fn tags(&self) -> Result<Vec<String>, String> {
        Ok(self.tags.clone())
    }
    fn remote_tag(&self, _: &str) -> Result<Option<String>, String> {
        Ok(self.remote_tag.clone())
    }
    fn origin(&self) -> Result<String, String> {
        Ok("git@github.com:acme/plugin-venue-desk.git".into())
    }
    fn show(&self, revision: &str, path: &str) -> Option<String> {
        assert_eq!(revision, HEAD);
        match path {
            "pyproject.toml" => Some(PYPROJECT.into()),
            "CHANGELOG.md" => Some(CHANGELOG.into()),
            _ => None,
        }
    }
    fn push_branch(
        &self,
        base: &str,
        branch: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<String, String> {
        assert_eq!((base, message), (HEAD, "Release 0.4.0"));
        self.pushed
            .lock()
            .unwrap()
            .push((branch.into(), files.to_vec()));
        Ok("5b4c3d2e1f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c".into())
    }
    fn fast_forward(&self, branch: &str) -> Result<(), String> {
        self.did
            .lock()
            .unwrap()
            .push(format!("fast-forward {branch}"));
        Ok(())
    }
    fn config(&self, key: &str) -> Option<String> {
        match (key, self.signing) {
            ("gpg.format", true) => Some("ssh".into()),
            ("user.signingkey", true) => Some("~/.ssh/id_ed25519_meridian.pub".into()),
            _ => None,
        }
    }
    fn sign_tag(&self, tag: &str, commit: &str, message: &str) -> Result<(), String> {
        self.did
            .lock()
            .unwrap()
            .push(format!("sign {tag} {commit} {message}"));
        *self.local_tag.lock().unwrap() = Some(commit.into());
        Ok(())
    }
    fn push_tag(&self, tag: &str) -> Result<(), String> {
        self.did.lock().unwrap().push(format!("push {tag}"));
        Ok(())
    }
}

/// GitHub, answering as the test says: main's CI, the PR's, and the PR as
/// it moves from open to merged after `merged_after` reads.
struct Hub {
    main_ci: Ci,
    pr_ci: Vec<Ci>,
    existing: Option<Pull>,
    merged_after: Option<usize>,
    reads: Mutex<usize>,
    ci_reads: Mutex<usize>,
    opened: Mutex<Vec<(String, String, String)>>,
}

impl Default for Hub {
    fn default() -> Hub {
        Hub {
            main_ci: Ci::Green,
            pr_ci: vec![Ci::None, Ci::Pending, Ci::Green],
            existing: None,
            merged_after: Some(4),
            reads: Mutex::new(0),
            ci_reads: Mutex::new(0),
            opened: Mutex::new(Vec::new()),
        }
    }
}

fn open_pull() -> Pull {
    Pull {
        number: 12,
        url: "https://github.com/acme/plugin-venue-desk/pull/12".into(),
        head: "5b4c3d2e".into(),
        state: "open".into(),
        merged: false,
        merge_commit: None,
    }
}

#[async_trait::async_trait]
impl GitHub for Hub {
    async fn default_branch(&self) -> Result<String, String> {
        Ok("main".into())
    }
    async fn ci(&self, commit: &str) -> Result<Ci, String> {
        if commit == HEAD {
            return Ok(self.main_ci.clone());
        }
        let mut read = self.ci_reads.lock().unwrap();
        let ci = self.pr_ci[(*read).min(self.pr_ci.len() - 1)].clone();
        *read += 1;
        Ok(ci)
    }
    async fn pull_from(&self, branch: &str) -> Result<Option<Pull>, String> {
        assert_eq!(branch, "release/v0.4.0");
        Ok(self.existing.clone())
    }
    async fn pull(&self, number: u64) -> Result<Pull, String> {
        assert_eq!(number, 12);
        let mut reads = self.reads.lock().unwrap();
        *reads += 1;
        let merged = self.merged_after.is_some_and(|after| *reads >= after);
        Ok(Pull {
            merged,
            state: if merged { "closed" } else { "open" }.into(),
            merge_commit: merged.then(|| MERGE.to_string()),
            ..open_pull()
        })
    }
    async fn open_pull(
        &self,
        branch: &str,
        base: &str,
        title: &str,
        _: &str,
    ) -> Result<Pull, String> {
        self.opened
            .lock()
            .unwrap()
            .push((branch.into(), base.into(), title.into()));
        Ok(open_pull())
    }
}

fn asked(scratch: &Scratch, wait: bool) -> Asked {
    std::fs::write(scratch.0.join("pyproject.toml"), PYPROJECT).unwrap();
    Asked {
        dir: scratch.0.clone(),
        version: "0.4.0".into(),
        wait,
        every: Duration::from_millis(1),
        timeout: Duration::from_secs(5),
        today: "2026-10-18".into(),
    }
}

async fn run(asked: &Asked, repo: &Repo, hub: &Hub) -> (Result<Reached, String>, Vec<String>) {
    let mut said = Vec::new();
    let reached = release(asked, repo, hub, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;
    (reached, said)
}

#[tokio::test]
async fn a_release_checks_opens_its_pr_waits_for_ci_and_the_merge_then_signs_the_tag() {
    let scratch = Scratch::new("release-whole");
    let (repo, hub) = (Repo::default(), Hub::default());
    let (reached, said) = run(&asked(&scratch, true), &repo, &hub).await;
    assert_eq!(
        reached.unwrap(),
        Reached::Tagged {
            tag: "v0.4.0".into(),
            commit: MERGE.into()
        }
    );
    let pushed = repo.pushed.lock().unwrap().clone();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].0, "release/v0.4.0");
    let files: std::collections::BTreeMap<_, _> = pushed[0].1.iter().cloned().collect();
    assert!(files["pyproject.toml"].contains("version = \"0.4.0\""));
    assert!(files["CHANGELOG.md"].contains("## 0.4.0 (2026-10-18)\n\n- Something."));
    assert_eq!(
        *hub.opened.lock().unwrap(),
        [(
            "release/v0.4.0".to_string(),
            "main".to_string(),
            "Release 0.4.0".to_string()
        )]
    );
    assert!(said
        .iter()
        .any(|l| l.starts_with("Checked: on main, clean")));
    assert!(said
        .iter()
        .any(|l| l.contains("Waiting for it to be merged, which is a person's")));
    assert_eq!(
        repo.did()[1..],
        [
            "fetch".to_string(),
            "fast-forward main".to_string(),
            format!("sign v0.4.0 {MERGE} venue-desk 0.4.0"),
            "push v0.4.0".to_string(),
        ]
    );
}

#[tokio::test]
async fn no_wait_stops_once_the_pr_is_open_and_a_rerun_resumes_from_it() {
    let scratch = Scratch::new("release-no-wait");
    let (repo, hub) = (Repo::default(), Hub::default());
    let (reached, _) = run(&asked(&scratch, false), &repo, &hub).await;
    let Reached::Stopped(said) = reached.unwrap() else {
        panic!("not stopped")
    };
    assert!(said.contains("pull/12") && said.contains("meridian plugin release 0.4.0"));
    assert!(!repo.did().iter().any(|d| d.starts_with("sign")));

    // The rerun finds the open PR: no checks again, nothing opened again.
    let repo = Repo {
        clean: false,
        ..Repo::default()
    };
    let hub = Hub {
        existing: Some(open_pull()),
        merged_after: Some(1),
        ..Hub::default()
    };
    let (reached, said) = run(&asked(&scratch, true), &repo, &hub).await;
    // Merged, but the checkout holds changes: said, before any tag.
    assert!(reached.unwrap_err().contains("must be on main and clean"));
    assert!(said[0].contains("The release PR is #12"));
    assert!(hub.opened.lock().unwrap().is_empty());
    assert!(repo.pushed.lock().unwrap().is_empty());

    // A tag on origin already: straight to publishing.
    let repo = Repo {
        remote_tag: Some(MERGE.into()),
        ..Repo::default()
    };
    let (reached, _) = run(&asked(&scratch, true), &repo, &Hub::default()).await;
    assert_eq!(
        reached.unwrap(),
        Reached::Tagged {
            tag: "v0.4.0".into(),
            commit: MERGE.into()
        }
    );
}

#[tokio::test]
async fn each_check_refuses_before_anything_is_made() {
    let scratch = Scratch::new("release-checks");
    let cases: Vec<(Repo, Hub, &str)> = vec![
        (
            Repo {
                branch: Some("feature".into()),
                ..Repo::default()
            },
            Hub::default(),
            "on feature, not main",
        ),
        (
            Repo {
                clean: false,
                ..Repo::default()
            },
            Hub::default(),
            "holds changes",
        ),
        (
            Repo {
                upstream: MERGE.into(),
                ..Repo::default()
            },
            Hub::default(),
            "is not origin's",
        ),
        (
            Repo::default(),
            Hub {
                main_ci: Ci::Red(vec!["check (failure) https://github.com/x".into()]),
                ..Hub::default()
            },
            "is not green",
        ),
        (
            Repo::default(),
            Hub {
                main_ci: Ci::Pending,
                ..Hub::default()
            },
            "still running",
        ),
        (
            Repo::default(),
            Hub {
                main_ci: Ci::None,
                ..Hub::default()
            },
            "has no CI run",
        ),
        (
            Repo {
                tags: vec!["v0.4.0".into()],
                ..Repo::default()
            },
            Hub::default(),
            "not higher than the last release tag, v0.4.0",
        ),
    ];
    for (repo, hub, says) in cases {
        let refused = run(&asked(&scratch, true), &repo, &hub)
            .await
            .0
            .unwrap_err();
        assert!(refused.contains(says), "{says}: {refused}");
        assert!(repo.pushed.lock().unwrap().is_empty(), "{says}");
        assert!(hub.opened.lock().unwrap().is_empty(), "{says}");
    }
    let mut not_a_version = asked(&scratch, true);
    not_a_version.version = "4".into();
    assert!(run(&not_a_version, &Repo::default(), &Hub::default())
        .await
        .0
        .unwrap_err()
        .contains("not a version"));
}

#[tokio::test]
async fn a_red_pr_or_a_closed_one_stops_and_a_tag_needs_a_signing_key() {
    let scratch = Scratch::new("release-stops");
    let hub = Hub {
        pr_ci: vec![Ci::Red(vec![
            "check (failure) https://github.com/run/1".into()
        ])],
        merged_after: None,
        ..Hub::default()
    };
    let refused = run(&asked(&scratch, true), &Repo::default(), &hub)
        .await
        .0
        .unwrap_err();
    assert!(
        refused.contains("CI failed") && refused.contains("run/1"),
        "{refused}"
    );

    let closed = Hub {
        existing: Some(Pull {
            state: "closed".into(),
            ..open_pull()
        }),
        ..Hub::default()
    };
    let refused = run(&asked(&scratch, true), &Repo::default(), &closed)
        .await
        .0
        .unwrap_err();
    assert!(refused.contains("closed without being merged"), "{refused}");

    let unsigned = Repo {
        signing: false,
        ..Repo::default()
    };
    let refused = run(&asked(&scratch, true), &unsigned, &Hub::default())
        .await
        .0
        .unwrap_err();
    assert!(refused.contains("meridian plugin key setup"), "{refused}");

    // Waiting longer than --timeout stops, saying a rerun goes on.
    let mut short = asked(&scratch, true);
    short.timeout = Duration::from_millis(0);
    let never = Hub {
        merged_after: None,
        ..Hub::default()
    };
    let (reached, _) = run(&short, &Repo::default(), &never).await;
    let Reached::Stopped(said) = reached.unwrap() else {
        panic!("not stopped")
    };
    assert!(
        said.contains("Stopped waiting") && said.contains("again"),
        "{said}"
    );
}

#[test]
fn the_version_and_the_changelog_heading_are_written_as_the_plugins_write_them() {
    let bumped = with_version(PYPROJECT, "0.4.0").unwrap();
    assert!(bumped.contains("version = \"0.4.0\"") && !bumped.contains("0.3.0"));
    assert!(bumped.contains("open-meridian==0.30.0"));
    // Only [project]'s version.
    let other = "[tool.x]\nversion = \"9\"\n[project]\nname = \"a\"\nversion = \"1.0.0\"\n";
    let bumped = with_version(other, "1.1.0").unwrap();
    assert!(bumped.contains("[tool.x]\nversion = \"9\"") && bumped.contains("version = \"1.1.0\""));
    assert!(with_version("[project]\nname = \"a\"\n", "1.0.0").is_none());

    // Its own heading, marked not released.
    assert!(with_heading(Some(CHANGELOG), "0.4.0", "2026-10-18")
        .starts_with("# Changelog\n\n## 0.4.0 (2026-10-18)\n\n- Something."));
    // An Unreleased heading, renamed.
    let unreleased = "# Changelog\n\n## [Unreleased]\n\n- New.\n\n## 0.3.0\n";
    assert!(with_heading(Some(unreleased), "0.4.0", "2026-10-18")
        .contains("## 0.4.0 (2026-10-18)\n\n- New.\n\n## 0.3.0"));
    // Neither: a heading above the first release's.
    let neither = "# Changelog\n\nIntro.\n\n## 0.3.0 (2026-10-01)\n";
    assert_eq!(
        with_heading(Some(neither), "0.4.0", "2026-10-18"),
        "# Changelog\n\nIntro.\n\n## 0.4.0 (2026-10-18)\n\n## 0.3.0 (2026-10-01)\n"
    );
    // `0.4.0` is not `0.4.01`'s heading.
    let longer = "# Changelog\n\n## 0.4.01\n";
    assert!(with_heading(Some(longer), "0.4.0", "2026-10-18").contains("## 0.4.01\n"));
    assert_eq!(
        with_heading(None, "0.4.0", "2026-10-18"),
        "# Changelog\n\n## 0.4.0 (2026-10-18)\n"
    );
    assert_eq!(
        last_release(&["v0.9.0".into(), "v0.10.0".into(), "x".into()]).as_deref(),
        Some("0.10.0")
    );
}

#[test]
fn ci_is_green_only_when_every_run_and_status_passed() {
    let runs = |items: serde_json::Value| serde_json::json!({ "check_runs": items });
    let none = serde_json::json!({ "statuses": [] });
    assert_eq!(ci_of(&runs(serde_json::json!([])), &none), Ci::None);
    assert_eq!(
        ci_of(
            &runs(serde_json::json!([
                {"name": "check", "status": "completed", "conclusion": "success"},
                {"name": "e2e", "status": "completed", "conclusion": "skipped"},
            ])),
            &none
        ),
        Ci::Green
    );
    assert_eq!(
        ci_of(
            &runs(serde_json::json!([
                {"name": "check", "status": "in_progress", "conclusion": null},
            ])),
            &none
        ),
        Ci::Pending
    );
    let Ci::Red(failed) = ci_of(
        &runs(serde_json::json!([
            {"name": "check", "status": "completed", "conclusion": "failure",
             "html_url": "https://github.com/acme/x/runs/1"},
        ])),
        &serde_json::json!({"statuses": [{"context": "ci/other", "state": "error", "target_url": "u"}]}),
    ) else {
        panic!("not red")
    };
    assert_eq!(
        failed,
        [
            "check (failure) https://github.com/acme/x/runs/1",
            "ci/other (error) u"
        ]
    );
}

#[tokio::test]
async fn githubs_api_is_asked_with_the_persons_token_for_the_plugins_repository() {
    let (api, kept) = serve(|asked: &Request| {
        let path = asked.path.as_str();
        match (asked.method.as_str(), path) {
            ("GET", "/repos/acme/plugin-venue-desk") => {
                Reply::json(200, serde_json::json!({"default_branch": "main"}))
            }
            ("GET", p) if p.starts_with("/repos/acme/plugin-venue-desk/commits/abc/check-runs") => {
                Reply::json(
                    200,
                    serde_json::json!({"check_runs": [{"name": "check", "status": "completed", "conclusion": "success"}]}),
                )
            }
            ("GET", "/repos/acme/plugin-venue-desk/commits/abc/status") => {
                Reply::json(200, serde_json::json!({"statuses": []}))
            }
            ("GET", p) if p.starts_with("/repos/acme/plugin-venue-desk/pulls?") => Reply::json(
                200,
                serde_json::json!([{"number": 12, "html_url": "https://github.com/acme/plugin-venue-desk/pull/12",
                    "state": "closed", "merged_at": "2026-10-18T10:00:00Z", "merge_commit_sha": MERGE,
                    "head": {"sha": "5b4c"}}]),
            ),
            ("POST", "/repos/acme/plugin-venue-desk/pulls") => Reply::json(
                201,
                serde_json::json!({"number": 13, "html_url": "u", "state": "open", "head": {"sha": "5b4c"}}),
            ),
            _ => Reply::json(404, serde_json::json!({"message": "Not Found"})),
        }
    })
    .await;
    let rest = Rest {
        api,
        owner: "acme".into(),
        repository: "plugin-venue-desk".into(),
        token: "gho_test".into(),
    };
    assert_eq!(rest.default_branch().await.unwrap(), "main");
    assert_eq!(rest.ci("abc").await.unwrap(), Ci::Green);
    let merged = rest.pull_from("release/v0.4.0").await.unwrap().unwrap();
    assert!(merged.merged && merged.merge_commit.as_deref() == Some(MERGE));
    let opened = rest
        .open_pull("release/v0.4.0", "main", "Release 0.4.0", "body")
        .await
        .unwrap();
    assert_eq!((opened.number, opened.merged), (13, false));
    let refused = rest.pull(99).await.unwrap_err();
    assert!(
        refused.contains("may not reach acme/plugin-venue-desk"),
        "{refused}"
    );
    let asked: Vec<Request> = kept.lock().unwrap().clone();
    assert!(asked
        .iter()
        .all(|a| a.header("authorization") == Some("Bearer gho_test")));
    let listed = asked.iter().find(|a| a.path.contains("/pulls?")).unwrap();
    assert!(
        listed.path.contains("head=acme:release/v0.4.0"),
        "{}",
        listed.path
    );
    let posted = asked.iter().find(|a| a.method == "POST").unwrap();
    assert_eq!(
        posted.json(),
        serde_json::json!({"title": "Release 0.4.0", "head": "release/v0.4.0", "base": "main", "body": "body"})
    );
    assert_eq!(
        owner_and_name("https://github.com/acme/plugin-venue-desk"),
        Some(("acme".into(), "plugin-venue-desk".into()))
    );
}

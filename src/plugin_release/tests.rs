use std::sync::Mutex;

use super::*;
use crate::fake_http::{serve, Asked as Request, Reply, Scratch};

const HEAD: &str = "4e1d9c27b85a03f6e2c1d0b9a8f7e6d5c4b3a291";
const OTHER: &str = "9a8f7e6d5c4b3a2914e1d9c27b85a03f6e2c1d0b";
const PYPROJECT: &str = "[project]\nname = \"venue-desk\"\nversion = \"0.4.0\"\ndependencies = [\"open-meridian==0.30.0\"]\n\n[tool.meridian]\nroles = []\ninterface = true\n";
const CHANGELOG: &str = "# Changelog\n\n## 0.4.0\n\n- Something.\n\n## 0.3.0\n\n- Before.\n";

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
    pyproject: String,
    changelog: Option<String>,
    did: Mutex<Vec<String>>,
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
            pyproject: PYPROJECT.into(),
            changelog: Some(CHANGELOG.into()),
            did: Mutex::new(Vec::new()),
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
            "pyproject.toml" => Some(self.pyproject.clone()),
            "CHANGELOG.md" => self.changelog.clone(),
            _ => None,
        }
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

/// GitHub, answering as the test says: main's CI.
struct Hub {
    main_ci: Ci,
}

impl Default for Hub {
    fn default() -> Hub {
        Hub { main_ci: Ci::Green }
    }
}

#[async_trait::async_trait]
impl GitHub for Hub {
    async fn default_branch(&self) -> Result<String, String> {
        Ok("main".into())
    }
    async fn ci(&self, commit: &str) -> Result<Ci, String> {
        assert_eq!(commit, HEAD);
        Ok(self.main_ci.clone())
    }
}

fn asked(scratch: &Scratch) -> Asked {
    std::fs::write(scratch.0.join("pyproject.toml"), PYPROJECT).unwrap();
    Asked {
        dir: scratch.0.clone(),
        version: "0.4.0".into(),
    }
}

async fn run(asked: &Asked, repo: &Repo, hub: &Hub) -> (Result<Tagged, String>, Vec<String>) {
    let mut said = Vec::new();
    let reached = release(asked, repo, hub, &mut |line: &str| {
        said.push(line.to_string())
    })
    .await;
    (reached, said)
}

#[tokio::test]
async fn a_release_checks_then_signs_the_tag_on_mains_head_and_pushes_it() {
    let scratch = Scratch::new("release-whole");
    let (repo, hub) = (Repo::default(), Hub::default());
    let (reached, said) = run(&asked(&scratch), &repo, &hub).await;
    assert_eq!(
        reached.unwrap(),
        Tagged {
            tag: "v0.4.0".into(),
            commit: HEAD.into()
        }
    );
    assert!(
        said.iter().any(|l| l.starts_with("Checked: on main, clean")
            && l.contains("pyproject.toml and CHANGELOG.md at 0.4.0")),
        "{said:?}"
    );
    // Nothing but the fetch, the tag and its push: no branch, no PR, no file.
    assert_eq!(
        repo.did(),
        [
            "fetch".to_string(),
            format!("sign v0.4.0 {HEAD} venue-desk 0.4.0"),
            "push v0.4.0".to_string(),
        ]
    );

    // The tag here already, at main's head (a push that failed): pushed again.
    let repo = Repo {
        local_tag: Mutex::new(Some(HEAD.into())),
        ..Repo::default()
    };
    let (reached, _) = run(&asked(&scratch), &repo, &Hub::default()).await;
    assert!(reached.is_ok());
    assert_eq!(repo.did(), ["fetch".to_string(), "push v0.4.0".to_string()]);
}

#[tokio::test]
async fn a_tag_on_origin_already_is_published_without_the_checks() {
    let scratch = Scratch::new("release-resume");
    // The checkout has moved on since: the tag on origin is what is published.
    let repo = Repo {
        remote_tag: Some(OTHER.into()),
        clean: false,
        tags: vec!["v0.4.0".into()],
        ..Repo::default()
    };
    let (reached, said) = run(&asked(&scratch), &repo, &Hub::default()).await;
    assert_eq!(
        reached.unwrap(),
        Tagged {
            tag: "v0.4.0".into(),
            commit: OTHER.into()
        }
    );
    assert!(said[0].contains("on origin already"), "{said:?}");
    assert_eq!(repo.did(), ["fetch".to_string()]);
}

#[tokio::test]
async fn each_check_refuses_before_anything_is_made_saying_what_to_do() {
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
                upstream: OTHER.into(),
                ..Repo::default()
            },
            Hub::default(),
            "is not origin's",
        ),
        (
            Repo {
                tags: vec!["v0.4.0".into()],
                ..Repo::default()
            },
            Hub::default(),
            "not higher than the last release tag, v0.4.0",
        ),
        (
            Repo {
                pyproject: PYPROJECT.replace("0.4.0", "0.3.0"),
                ..Repo::default()
            },
            Hub::default(),
            "pyproject.toml's version is 0.3.0, not 0.4.0: raise it to 0.4.0, with its `## 0.4.0` CHANGELOG entry, in a pull request",
        ),
        (
            Repo {
                changelog: None,
                ..Repo::default()
            },
            Hub::default(),
            "no CHANGELOG.md in git, so no `## 0.4.0` entry: add it",
        ),
        (
            Repo {
                changelog: Some("# Changelog\n\n## 0.3.0\n\n- Before.\n".into()),
                ..Repo::default()
            },
            Hub::default(),
            "CHANGELOG.md has no `## 0.4.0` heading: add it, with this release's changes",
        ),
        (
            Repo {
                changelog: Some(CHANGELOG.replace("## 0.4.0", "## 0.4.0 (2026-10-18)")),
                ..Repo::default()
            },
            Hub::default(),
            "heading for 0.4.0 is `## 0.4.0 (2026-10-18)`: make it `## 0.4.0` alone, with no date and no \"(unreleased)\" (the date lives on the tag and the listing)",
        ),
        (
            Repo {
                changelog: Some(CHANGELOG.replace("## 0.4.0", "## 0.4.0 (unreleased)")),
                ..Repo::default()
            },
            Hub::default(),
            "heading for 0.4.0 is `## 0.4.0 (unreleased)`: make it `## 0.4.0` alone",
        ),
        (
            Repo::default(),
            Hub {
                main_ci: Ci::Red(vec!["check (failure) https://github.com/x".into()]),
            },
            "is not green",
        ),
        (
            Repo::default(),
            Hub {
                main_ci: Ci::Pending,
            },
            "still running",
        ),
        (
            Repo::default(),
            Hub { main_ci: Ci::None },
            "has no CI run",
        ),
        (
            Repo {
                signing: false,
                ..Repo::default()
            },
            Hub::default(),
            "`meridian plugin key setup` makes one ready",
        ),
        (
            Repo {
                local_tag: Mutex::new(Some(OTHER.into())),
                ..Repo::default()
            },
            Hub::default(),
            "v0.4.0 is here already, at 9a8f7e6, not at 4e1d9c2",
        ),
    ];
    for (repo, hub, says) in cases {
        let refused = run(&asked(&scratch), &repo, &hub).await.0.unwrap_err();
        assert!(refused.contains(says), "{says}: {refused}");
        assert_eq!(repo.did(), ["fetch".to_string()], "{says}");
    }
    let mut not_a_version = asked(&scratch);
    not_a_version.version = "4".into();
    assert!(run(&not_a_version, &Repo::default(), &Hub::default())
        .await
        .0
        .unwrap_err()
        .contains("not a version"));
}

#[test]
fn the_version_and_the_changelog_entry_are_read_as_the_plugins_write_them() {
    assert_eq!(version_of(PYPROJECT).as_deref(), Some("0.4.0"));
    // Only [project]'s version.
    let other = "[tool.x]\nversion = \"9\"\n[project]\nname = \"a\"\nversion = '1.0.0' # pinned\n";
    assert_eq!(version_of(other).as_deref(), Some("1.0.0"));
    assert_eq!(version_of("[project]\nname = \"a\"\n"), None);

    assert_eq!(changelog_entry(Some(CHANGELOG), "0.4.0"), Ok(()));
    // Trailing space and a CRLF file are the same heading.
    assert_eq!(
        changelog_entry(Some("# Changelog\r\n\r\n## 0.4.0 \r\n"), "0.4.0"),
        Ok(())
    );
    // `## 0.4.01` is not 0.4.0's heading, and `## [0.4.0]` is not `## 0.4.0`.
    assert!(changelog_entry(Some("## 0.4.01\n"), "0.4.0")
        .unwrap_err()
        .contains("has no `## 0.4.0` heading"));
    assert!(changelog_entry(Some("## [0.4.0]\n"), "0.4.0")
        .unwrap_err()
        .contains("is `## [0.4.0]`: make it `## 0.4.0` alone"));
    assert_eq!(
        last_release(&["v0.9.0".into(), "v0.10.0".into(), "x".into()]).as_deref(),
        Some("0.10.0")
    );
}

/// A file is read as git holds it: 0.1.48 read CHANGELOG.md and
/// pyproject.toml through a trimming `git`, and wrote them back without
/// their final newline.
#[test]
fn a_file_at_a_revision_is_read_byte_for_byte_with_its_final_newline() {
    let scratch = Scratch::new("release-show");
    let git = |arguments: &[&str]| {
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(&scratch.0)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(arguments)
            .output()
            .expect("git runs");
        assert!(done.status.success(), "git {arguments:?}: {done:?}");
    };
    git(&["init", "--quiet"]);
    std::fs::write(scratch.0.join("CHANGELOG.md"), CHANGELOG).unwrap();
    std::fs::write(scratch.0.join("pyproject.toml"), PYPROJECT).unwrap();
    git(&["add", "CHANGELOG.md", "pyproject.toml"]);
    git(&["commit", "--quiet", "-m", "one"]);
    let checkout = Checkout(scratch.0.clone());
    let head = checkout.commit_of("HEAD").unwrap();
    let changelog = checkout.show(&head, "CHANGELOG.md").unwrap();
    assert_eq!(changelog, CHANGELOG);
    assert!(changelog.ends_with("- Before.\n") && !changelog.ends_with("\n\n"));
    assert_eq!(checkout.show(&head, "pyproject.toml").unwrap(), PYPROJECT);
    assert_eq!(checkout.show(&head, "absent.md"), None);
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
    let refused = rest.ci("missing").await.unwrap_err();
    assert!(
        refused.contains("may not reach acme/plugin-venue-desk, or may not read its checks"),
        "{refused}"
    );
    let asked: Vec<Request> = kept.lock().unwrap().clone();
    assert!(asked
        .iter()
        .all(|a| a.header("authorization") == Some("Bearer gho_test")));
    // Release only reads: nothing is sent to GitHub but GETs.
    assert!(asked.iter().all(|a| a.method == "GET"));
    assert_eq!(
        owner_and_name("https://github.com/acme/plugin-venue-desk"),
        Some(("acme".into(), "plugin-venue-desk".into()))
    );
}

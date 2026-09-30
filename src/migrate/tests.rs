use std::path::PathBuf;
use std::sync::Mutex;

use super::*;

/// A plugin `meridian plugin new` wrote, moved back to 0.6.1, in a directory
/// of its own that is removed when the test is done with it.
struct Plugin(PathBuf);

impl Plugin {
    fn at(label: &str, version: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("meridian-migrate-{label}-{unique}"));
        crate::plugin::scaffold("migrated-plugin", &dir).expect("the template scaffolds");
        let plugin = Plugin(dir);
        let current = sdk_pin(&PYTHON, &plugin.read("pyproject.toml")).expect("pinned");
        for file in ["pyproject.toml", "Dockerfile"] {
            let text = plugin.read(file);
            plugin.write(
                file,
                &text
                    .replace(&format!("=={current}"), &format!("=={version}"))
                    .replace(&format!("python:{current}"), &format!("python:{version}")),
            );
        }
        plugin
    }

    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.0.join(file)).expect("a file")
    }

    fn write(&self, file: &str, text: &str) {
        let path = self.0.join(file);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("written");
    }

    fn asked(&self) -> Asked {
        Asked {
            dir: self.0.clone(),
            to: None,
            image: None,
            force: false,
            run_tests: false,
        }
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ran(code: i32, stdout: &str, stderr: &str) -> Ran {
    Ran {
        code: Some(code),
        stdout: stdout.into(),
        stderr: stderr.into(),
    }
}

/// The machine, answering as each test says, and keeping what it was asked.
struct Fake {
    git: Ran,
    index: Result<(u16, String), String>,
    build: Ran,
    migrations: Ran,
    asked: Mutex<Vec<(String, Vec<String>, String)>>,
}

impl Fake {
    fn new(migrations: &str) -> Self {
        Fake {
            git: ran(0, "", ""),
            index: Ok((
                200,
                r#"{"info": {"version": "0.7.0"}, "releases": {"0.6.0": [], "0.6.1": [], "0.7.0": []}}"#
                    .into(),
            )),
            build: ran(0, "sha256:abc", ""),
            migrations: ran(0, migrations, ""),
            asked: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, Vec<String>, String)> {
        self.asked.lock().unwrap().clone()
    }

    fn ran_docker(&self, verb: &str) -> Option<(Vec<String>, String)> {
        self.calls()
            .into_iter()
            .find(|(program, arguments, _)| program == "docker" && arguments[0] == verb)
            .map(|(_, arguments, stdin)| (arguments, stdin))
    }
}

fn copy(held: &Ran) -> Ran {
    Ran {
        code: held.code,
        stdout: held.stdout.clone(),
        stderr: held.stderr.clone(),
    }
}

#[async_trait::async_trait]
impl Host for Fake {
    async fn run(&self, program: &str, arguments: &[String], stdin: &[u8]) -> Result<Ran, String> {
        self.asked.lock().unwrap().push((
            program.into(),
            arguments.to_vec(),
            String::from_utf8_lossy(stdin).into_owned(),
        ));
        match (program, arguments.first().map(String::as_str)) {
            ("git", _) => Ok(copy(&self.git)),
            ("docker", Some("build")) => Ok(copy(&self.build)),
            ("docker", Some("run")) => Ok(copy(&self.migrations)),
            _ => Err(format!("{program} was not expected")),
        }
    }

    async fn fetch(&self, url: &str) -> Result<(u16, String), String> {
        self.asked
            .lock()
            .unwrap()
            .push(("GET".into(), vec![url.into()], String::new()));
        self.index.clone()
    }
}

/// What the migrations answer for the scaffold: its page rewritten, one
/// place left by hand.
fn answered(page: &str) -> String {
    serde_json::json!({
        "from": "0.6.1",
        "to": "0.7.0",
        "steps": [{
            "from": "0.6.1",
            "to": "0.7.0",
            "summary": "The unlinked refusal is NotLinked. Nothing else changed.",
            "breaking": false,
            "rewrote": [{
                "file": "src/migrated_plugin/page.py",
                "rule": "not-linked-isinstance",
                "what": "a test of the words became an isinstance",
                "places": 2,
            }],
        }],
        "files": {"src/migrated_plugin/page.py": page},
        "by_hand": [{
            "from": "0.6.1",
            "to": "0.7.0",
            "rule": "not-linked-by-text",
            "file": "src/migrated_plugin/page.py",
            "line": 3,
            "found": "if \"not linked\" in said:",
            "what": "the words matched",
            "instead": "catch meridian.NotLinked",
        }],
    })
    .to_string()
}

fn done(migrated: Migrated) -> Box<Report> {
    match migrated {
        Migrated::Done(report) => report,
        Migrated::Refused(said) => panic!("refused: {said}"),
        Migrated::Failed(said) => panic!("failed: {said}"),
    }
}

fn refused(migrated: Migrated) -> String {
    match migrated {
        Migrated::Refused(said) => said,
        Migrated::Failed(said) => panic!("failed, not refused: {said}"),
        Migrated::Done(_) => panic!("done, not refused"),
    }
}

fn failed(migrated: Migrated) -> String {
    match migrated {
        Migrated::Failed(said) => said,
        Migrated::Refused(said) => panic!("refused, not failed: {said}"),
        Migrated::Done(_) => panic!("done, not failed"),
    }
}

// ── The pins ─────────────────────────────────────────────────────────────

#[test]
fn the_pins_are_the_sdk_in_pyproject_and_the_base_in_the_dockerfile() {
    let plugin = Plugin::at("pins", "0.6.1");
    assert_eq!(pins(&PYTHON, &plugin.0).unwrap().version, "0.6.1");

    // Among other dependencies, over lines, as meridian-snaptrade's are.
    let listed = plugin.read("pyproject.toml").replace(
        "dependencies = [\"open-meridian==0.6.1\"]",
        "dependencies = [\n  \"open-meridian-extras==9.9\",\n  \"open-meridian==0.6.1\",\n  \"snaptrade-python-sdk==13.0.27\",\n]",
    );
    assert_eq!(sdk_pin(&PYTHON, &listed).unwrap(), "0.6.1");
}

#[test]
fn pins_that_disagree_or_are_missing_are_refused() {
    let plugin = Plugin::at("disagree", "0.6.1");
    plugin.write(
        "Dockerfile",
        &plugin
            .read("Dockerfile")
            .replace("python:0.6.1", "python:0.6.0"),
    );
    let said = pins(&PYTHON, &plugin.0).unwrap_err();
    assert!(
        said.contains("the pins disagree") && said.contains("plugin-python:0.6.0"),
        "{said}"
    );

    let ranged = plugin
        .read("pyproject.toml")
        .replace("open-meridian==0.6.1", "open-meridian>=0.6");
    assert!(sdk_pin(&PYTHON, &ranged).unwrap_err().contains("exactly"));
    assert!(sdk_pin(&PYTHON, "[project]\ndependencies = []\n")
        .unwrap_err()
        .contains("do not name open-meridian"));
}

#[test]
fn a_pin_moves_whole_and_says_where_it_was() {
    let plugin = Plugin::at("moved", "0.6.1");
    let held = pins(&PYTHON, &plugin.0).unwrap();
    let (pyproject, dockerfile, said) = moved(&PYTHON, &held, "0.7.0");
    assert!(pyproject.contains("open-meridian==0.7.0") && !pyproject.contains("0.6.1"));
    assert!(dockerfile.contains("plugin-python:0.7.0") && !dockerfile.contains("0.6.1"));
    assert_eq!(
        said.iter().map(|m| m.file.as_str()).collect::<Vec<_>>(),
        ["pyproject.toml", "Dockerfile"]
    );
    // `0.6.1` is not the start of `0.6.10`.
    let longer = Pins {
        version: "0.6.1".into(),
        pyproject: "x = \"open-meridian==0.6.10\"\n".into(),
        dockerfile: String::new(),
    };
    let (pyproject, _, said) = moved(&PYTHON, &longer, "0.7.0");
    assert!(pyproject.contains("==0.6.10") && said.is_empty());
}

// ── Refused before anything changes ──────────────────────────────────────

#[tokio::test]
async fn changes_git_does_not_hold_are_refused_unless_forced() {
    let plugin = Plugin::at("dirty", "0.6.1");
    let mut fake = Fake::new(&answered("x = 1\n"));
    fake.git = ran(0, " M src/migrated_plugin/page.py\n", "");
    let said = refused(migrate(&fake, &PYTHON, &plugin.asked()).await);
    assert!(
        said.contains("page.py") && said.contains("--force"),
        "{said}"
    );
    assert!(fake.ran_docker("run").is_none(), "nothing ran");

    let forced = Asked {
        force: true,
        ..plugin.asked()
    };
    let report = done(migrate(&fake, &PYTHON, &forced).await);
    assert_eq!(report.to, "0.7.0");
}

#[tokio::test]
async fn a_directory_git_does_not_hold_is_refused_unless_forced() {
    let plugin = Plugin::at("untracked", "0.6.1");
    let mut fake = Fake::new(&answered("x = 1\n"));
    fake.git = ran(128, "", "fatal: not a git repository");
    let said = refused(migrate(&fake, &PYTHON, &plugin.asked()).await);
    assert!(said.contains("not in a git repository"), "{said}");
}

#[tokio::test]
async fn a_migration_never_goes_backwards() {
    let plugin = Plugin::at("backwards", "0.7.0");
    let fake = Fake::new("");
    let asked = Asked {
        to: Some("0.6.0".into()),
        ..plugin.asked()
    };
    let said = refused(migrate(&fake, &PYTHON, &asked).await);
    assert!(said.contains("never goes backwards"), "{said}");
    assert_eq!(plugin.read("pyproject.toml").matches("==0.7.0").count(), 1);
}

#[tokio::test]
async fn a_release_that_is_not_released_is_refused_unless_its_image_is_named() {
    let plugin = Plugin::at("unreleased", "0.6.1");
    let fake = Fake::new(&answered("x = 1\n"));
    let asked = Asked {
        to: Some("0.8.0".into()),
        ..plugin.asked()
    };
    let said = refused(migrate(&fake, &PYTHON, &asked).await);
    assert!(
        said.contains("0.8.0 is not released") && said.contains("--image"),
        "{said}"
    );

    let local = Asked {
        image: Some("plugin-python:local".into()),
        ..asked
    };
    let fake = Fake::new(&answered("x = 1\n"));
    let report = done(migrate(&fake, &PYTHON, &local).await);
    assert_eq!(report.image.as_deref(), Some("plugin-python:local"));
    // Both named: the index is not asked at all.
    assert!(!fake.calls().iter().any(|(program, _, _)| program == "GET"));
}

#[tokio::test]
async fn a_version_that_is_not_one_is_refused() {
    let plugin = Plugin::at("latest", "0.6.1");
    let asked = Asked {
        to: Some("latest".into()),
        ..plugin.asked()
    };
    let said = refused(migrate(&Fake::new(""), &PYTHON, &asked).await);
    assert!(said.contains("not a release"), "{said}");
}

// ── Migrated ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_plugin_on_the_latest_already_is_only_checked() {
    let plugin = Plugin::at("current", "0.7.0");
    let fake = Fake::new("");
    let report = done(migrate(&fake, &PYTHON, &plugin.asked()).await);
    assert!(report.image.is_none() && report.steps.is_empty() && report.wrote.is_empty());
    assert!(fake.ran_docker("build").is_none());
    assert!(report.done(), "{}", text(&report));
    assert!(text(&report).contains("already there, so nothing moved"));
}

#[tokio::test]
async fn the_pins_move_the_steps_run_in_the_latest_image_and_what_they_wrote_is_written() {
    let plugin = Plugin::at("migrated", "0.6.1");
    // A Makefile naming the base too, as meridian-snaptrade's does.
    plugin.write(
        "Makefile",
        "BASE ?= ghcr.io/open-meridian/plugin-python:0.6.1\n",
    );
    let page = plugin
        .read("src/migrated_plugin/page.py")
        .replacen("\"\"\"", "\"\"\"Migrated. ", 1);
    let fake = Fake::new(&answered(&page));
    let report = done(migrate(&fake, &PYTHON, &plugin.asked()).await);

    // The image: the latest release's, with the migrate extra, cut off.
    let (build, dockerfile) = fake.ran_docker("build").unwrap();
    assert_eq!(
        build,
        [
            "build",
            "--progress=plain",
            "-t",
            "meridian-migrate:ghcr.io-open-meridian-plugin-python-0.7.0",
            "-"
        ]
    );
    assert!(dockerfile.starts_with("FROM ghcr.io/open-meridian/plugin-python:0.7.0\n"));
    assert!(dockerfile.contains("open-meridian[migrate]==$v"));
    let (run, given) = fake.ran_docker("run").unwrap();
    assert!(run.windows(2).any(|w| w == ["--network", "none"]));
    assert!(!run
        .iter()
        .any(|a| a == "-v" || a.starts_with("--volume") || a.starts_with("--mount")));
    assert!(run.ends_with(
        &[
            "python",
            "-m",
            "meridian.migrations",
            "--from",
            "0.6.1",
            "--to",
            "0.7.0"
        ]
        .map(String::from)
    ));

    // Given the code and pyproject.toml with its pin moved; not the rest.
    let given: serde_json::Value = serde_json::from_str(&given).unwrap();
    let files = given["files"].as_object().unwrap();
    assert!(files["pyproject.toml"]
        .as_str()
        .unwrap()
        .contains("open-meridian==0.7.0"));
    assert!(files.contains_key("src/migrated_plugin/page.py"));
    assert!(files.contains_key("tests/test_page.py"));
    assert!(!files.contains_key("Dockerfile") && !files.contains_key("AGENTS.md"));

    // Written: the pins, and the file the steps rewrote.
    assert!(plugin
        .read("pyproject.toml")
        .contains("open-meridian==0.7.0"));
    assert!(plugin.read("Dockerfile").contains("plugin-python:0.7.0"));
    assert_eq!(plugin.read("src/migrated_plugin/page.py"), page);
    assert_eq!(
        report.wrote,
        [
            "Dockerfile",
            "pyproject.toml",
            "src/migrated_plugin/page.py"
        ]
    );
    assert_eq!(report.pins.len(), 2);

    // Left by hand: the step's place, and the Makefile's pin, said not moved.
    let left: Vec<(&str, &str, Option<usize>)> = report
        .by_hand
        .iter()
        .map(|b| (b.rule.as_str(), b.file.as_str(), b.line))
        .collect();
    assert_eq!(
        left,
        [
            ("not-linked-by-text", "src/migrated_plugin/page.py", Some(3)),
            ("pin-elsewhere", "Makefile", Some(1)),
        ]
    );
    assert!(plugin.read("Makefile").contains("0.6.1"));
    assert!(report.check.passed() && !report.done());

    let said = text(&report);
    assert!(said.contains("open-meridian 0.6.1 to 0.7.0, the steps run in ghcr.io/open-meridian/plugin-python:0.7.0"), "{said}");
    assert!(
        said.contains("0.6.1 to 0.7.0  The unlinked refusal is NotLinked."),
        "{said}"
    );
    assert!(
        said.contains("src/migrated_plugin/page.py  not-linked-isinstance (2)"),
        "{said}"
    );
    assert!(said.contains("Left by hand, 2 place(s)"), "{said}");
    assert!(
        said.contains("pin-elsewhere") && said.contains("Makefile:1"),
        "{said}"
    );
    assert!(said.contains("meridian plugin check:"), "{said}");

    let json = json(&report);
    assert_eq!(json["from"], "0.6.1");
    assert_eq!(json["to"], "0.7.0");
    assert_eq!(json["done"], false);
    assert_eq!(json["by_hand"][0]["from"], "0.6.1");
    assert_eq!(json["by_hand"][1]["from"], serde_json::Value::Null);
    assert_eq!(json["check"]["passed"], true);
}

#[tokio::test]
async fn nothing_is_written_when_the_steps_cannot_run() {
    let plugin = Plugin::at("unrun", "0.6.1");
    let before = plugin.read("pyproject.toml");

    let mut fake = Fake::new("");
    fake.build = ran(
        1,
        "",
        "ModuleNotFoundError: No module named 'meridian.migrations'",
    );
    let said = failed(migrate(&fake, &PYTHON, &plugin.asked()).await);
    assert!(
        said.contains("carries no migrations") && said.contains("--image"),
        "{said}"
    );

    let mut fake = Fake::new("");
    fake.migrations = ran(
        2,
        "",
        "python -m meridian.migrations: no migration is recorded from 0.6.1",
    );
    let said = refused(migrate(&fake, &PYTHON, &plugin.asked()).await);
    assert!(
        said.contains("no migration is recorded from 0.6.1"),
        "{said}"
    );

    let mut fake = Fake::new("");
    fake.migrations = ran(1, "", "Traceback: KeyError");
    assert!(failed(migrate(&fake, &PYTHON, &plugin.asked()).await).contains("KeyError"));

    // A path it was not given is never written.
    let stranger = answered("x = 1\n").replace("src/migrated_plugin/page.py", "../outside.py");
    let fake = Fake::new(&stranger);
    let said = failed(migrate(&fake, &PYTHON, &plugin.asked()).await);
    assert!(
        said.contains("../outside.py") && said.contains("nothing is written"),
        "{said}"
    );

    let mut fake = Fake::new("");
    fake.index = Err("dns error".into());
    let said = failed(migrate(&fake, &PYTHON, &plugin.asked()).await);
    assert!(
        said.contains("dns error") && said.contains("--to"),
        "{said}"
    );

    assert_eq!(plugin.read("pyproject.toml"), before);
    assert!(plugin.read("Dockerfile").contains("plugin-python:0.6.1"));
}

#[test]
fn the_image_is_kept_under_a_tag_of_its_own() {
    assert_eq!(
        tag_for("ghcr.io/open-meridian/plugin-python:0.7.1"),
        "meridian-migrate:ghcr.io-open-meridian-plugin-python-0.7.1"
    );
    assert_eq!(
        tag_for("plugin-python:local"),
        "meridian-migrate:plugin-python-local"
    );
}

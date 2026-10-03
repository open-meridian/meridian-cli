use std::path::{Path, PathBuf};

use super::rules::{raw_colours, secrets, Secret};
use super::*;

/// A plugin `meridian plugin new` wrote, in a directory of its own that is
/// removed when the test is done with it.
struct Scaffolded(PathBuf);

const NAME: &str = "checked-plugin";
const MODULE: &str = "checked_plugin";

impl Scaffolded {
    fn new(label: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("meridian-check-{label}-{unique}"));
        crate::plugin::scaffold(NAME, &dir).expect("the template scaffolds");
        Scaffolded(dir)
    }

    /// The same, its test replaced by one that needs no SDK to run.
    fn tested(label: &str) -> Self {
        let plugin = Self::new(label);
        plugin.write(
            "tests/test_page.py",
            "from checked_plugin.page import TITLE\n\n\ndef test_the_page_has_a_title():\n    assert TITLE\n",
        );
        plugin
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }

    fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.path(file)).expect("a file the test wrote or scaffolded")
    }

    fn write(&self, file: &str, text: &str) {
        let path = self.path(file);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(path, text).expect("written");
    }

    /// `after` with `added` put on the line after it; the line `added` is on.
    fn add_after(&self, file: &str, after: &str, added: &str) -> usize {
        let text = self.read(file);
        let at = text.find(after).expect("the text to add after") + after.len();
        let at = at
            + text[at..]
                .find('\n')
                .map(|n| n + 1)
                .unwrap_or(text.len() - at);
        let changed = format!("{}{added}\n{}", &text[..at], &text[at..]);
        self.write(file, &changed);
        text[..at].matches('\n').count() + 1
    }

    fn replace(&self, file: &str, from: &str, to: &str) {
        let text = self.read(file);
        assert!(text.contains(from), "{file} holds {from:?}");
        self.write(file, &text.replacen(from, to, 1));
    }

    fn check(&self) -> Report {
        check(&self.0, false).expect("a directory")
    }
}

impl Drop for Scaffolded {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn failed_rules(report: &Report) -> Vec<&'static str> {
    let mut rules: Vec<&str> = report.failures.iter().map(|f| f.rule).collect();
    rules.dedup();
    rules
}

fn main_py() -> String {
    format!("src/{MODULE}/__main__.py")
}

fn page_py() -> String {
    format!("src/{MODULE}/page.py")
}

/// One of the pages' Jinja2 templates, which `pages.render` renders.
fn template(name: &str) -> String {
    format!("src/{MODULE}/templates/{name}")
}

#[test]
fn a_freshly_scaffolded_plugin_breaks_no_rule() {
    let plugin = Scaffolded::new("fresh");
    let report = plugin.check();
    assert!(
        plugin.path("tests").exists(),
        "the template ships its tests"
    );
    assert_eq!(
        failed_rules(&report),
        Vec::<&str>::new(),
        "{}",
        text(&report)
    );
}

#[test]
fn a_freshly_scaffolded_plugin_with_a_test_keeps_every_rule() {
    let plugin = Scaffolded::tested("fresh-tested");
    let report = plugin.check();
    assert!(report.passed(), "{}", text(&report));
    assert_eq!(report.outcomes.len(), RULES.len() + 1);
    let said = text(&report);
    assert!(
        said.contains("Every rule it was checked against holds."),
        "{said}"
    );
    assert!(
        said.contains("--    tests-pass") && said.contains("--run-tests runs them"),
        "{said}"
    );
}

#[test]
fn the_runtimes_harness_copied_for_e2e_is_not_the_plugin() {
    let plugin = Scaffolded::new("e2e-harness");
    // What core's plugin harness reads, copied where its README and the docs
    // tell an author to copy it.
    let harness = r#"import os

DASHBOARD = os.environ.get("MERIDIAN_HARNESS_DASHBOARD", "http://dashboard:8080")
"#;
    plugin.write(".e2e/harness/harness.py", harness);
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        Vec::<&str>::new(),
        "{}",
        text(&report)
    );

    // The same file anywhere else is the plugin's, and still checked.
    plugin.write("e2e/harness/harness.py", harness);
    assert_eq!(failed_rules(&plugin.check()), vec!["settings-declared"]);
}

/// One way to break each rule, on a plugin that otherwise keeps them all:
/// the rule, the file and line it must be found at, and what it must say.
#[test]
fn a_plugin_breaking_each_rule_fails_that_rule_alone_where_it_is_broken() {
    type Breaking = fn(&Scaffolded) -> (String, usize);
    let breakages: [(&str, Breaking, &str); 15] = [
        (
            "template-shape",
            |p| {
                std::fs::remove_file(p.path("AGENTS.md")).unwrap();
                ("AGENTS.md".into(), 0)
            },
            "there is no AGENTS.md",
        ),
        (
            "template-shape",
            |p| {
                p.replace("Dockerfile", "plugin-python:0.17.0", "plugin-python:0.5.0");
                ("Dockerfile".into(), 10)
            },
            "its base is plugin-python:0.5.0, and pyproject.toml pins open-meridian==0.17.0",
        ),
        (
            "tool-meridian",
            |p| {
                let line = p.add_after("pyproject.toml", "roles = []", "tags = [\"holdings\"]");
                ("pyproject.toml".into(), line)
            },
            "[tool.meridian] declares `tags`",
        ),
        (
            "tool-meridian",
            |p| {
                p.replace("pyproject.toml", "roles = []", "roles = [\"trading\"]");
                let line = p
                    .read("pyproject.toml")
                    .lines()
                    .position(|l| l.starts_with("roles"))
                    .unwrap()
                    + 1;
                ("pyproject.toml".into(), line)
            },
            "`trading` is not one of the deployment's roles",
        ),
        (
            "edge-storage",
            |p| {
                p.add_after(
                    "pyproject.toml",
                    "roles = []",
                    "declaration = \"checked_plugin.declaration:DECLARATION\"",
                );
                p.write(
                    &format!("src/{MODULE}/declaration.py"),
                    "from meridian.declaration import Declaration, Storage\n\n\
                     DECLARATION = Declaration(\n    storage=Storage(retention_days=30),\n)\n",
                );
                (format!("src/{MODULE}/declaration.py"), 4)
            },
            "the declaration asks for storage, and the plugin holds no edge role",
        ),
        (
            "role-suite",
            |p| {
                p.replace("pyproject.toml", "roles = []", "roles = [\"custody\"]");
                ("tests/".into(), 0)
            },
            "it holds `custody`, and no test runs the custody suite",
        ),
        (
            "kit-linked",
            |p| {
                p.write(
                    &format!("src/{MODULE}/static/index.html"),
                    "<!doctype html>\n<html lang=\"en\">\n<head><title>x</title></head>\n<body></body></html>\n",
                );
                (format!("src/{MODULE}/static/index.html"), 1)
            },
            "a page that does not link the kit",
        ),
        (
            "kit-linked",
            |p| {
                p.write(&format!("src/{MODULE}/static/meridian.css"), ":root {}\n");
                (format!("src/{MODULE}/static/meridian.css"), 0)
            },
            "a copy of the kit, in the plugin",
        ),
        (
            "no-raw-colour",
            |p| {
                let line = p.add_after(
                    &template("setup.html"),
                    "{% block content %}",
                    "<p style=\"color: #c0ffee\">",
                );
                (template("setup.html"), line)
            },
            "`#c0ffee` is a raw colour, which no scheme can change",
        ),
        (
            "own-origin",
            |p| {
                let line = p.add_after(
                    &template("accounts.html"),
                    "{% block content %}",
                    "<script src=\"https://cdn.example.com/chart.js\"></script>",
                );
                (template("accounts.html"), line)
            },
            "`https://cdn.example.com/chart.js` is loaded from another origin",
        ),
        (
            "settings-declared",
            |p| {
                let line = p.add_after(
                    &main_py(),
                    "port = int(",
                    "    token = os.environ[\"BROKER_TOKEN\"]",
                );
                (main_py(), line)
            },
            "BROKER_TOKEN is read from the environment",
        ),
        (
            "secrets-kept",
            |p| {
                p.add_after(
                    &main_py(),
                    "log = logging.getLogger",
                    "API_KEY = \"broker_api_key\"\nSETTINGS = [\n    meridian.Setting(API_KEY, str, required=True, secret=True),\n]",
                );
                let line = p.add_after(
                    &main_py(),
                    "        served = pages.serve(plugin, port)",
                    "        async for held in plugin.settings():\n            log.info(\n                \"using %s\",\n                held.values[API_KEY],\n            )",
                );
                (main_py(), line + 3)
            },
            "the secret setting `broker_api_key` is logged",
        ),
        (
            "through-the-sdk",
            |p| {
                let line = p.add_after(&main_py(), "import signal", "import nats");
                (main_py(), line)
            },
            "it imports nats, to reach the deployment directly",
        ),
        (
            "tools-cover-routes",
            |p| {
                p.replace(&page_py(), "    params=OpenStatement,\n", "");
                let text = p.read(&page_py());
                let at = text.find("@pages.route(").expect("the template's route");
                (page_py(), text[..at].matches('\n').count() + 1)
            },
            "/statement changes something and declares no typed record, so no tool is derived from it",
        ),
        (
            "tests-exist",
            |p| {
                std::fs::remove_dir_all(p.path("tests")).unwrap();
                ("tests/".into(), 0)
            },
            "no test_*.py with a test in it",
        ),
    ];

    for (rule, breaking, said) in breakages {
        let plugin = Scaffolded::tested(rule);
        let (file, line) = breaking(&plugin);
        let report = plugin.check();
        assert_eq!(failed_rules(&report), [rule], "{}", text(&report));
        let failure = report
            .failures
            .iter()
            .find(|failure| failure.found == said)
            .unwrap_or_else(|| panic!("{rule}: {said:?} in {}", text(&report)));
        assert_eq!(
            (failure.file.as_str(), failure.line),
            (file.as_str(), line),
            "{rule}"
        );
        assert!(
            !failure.instead.is_empty(),
            "{rule} says what to write instead"
        );
    }
}

#[test]
fn a_plugin_breaking_every_rule_is_told_each_with_its_place_and_its_fix() {
    let plugin = Scaffolded::new("broken");
    std::fs::remove_file(plugin.path("AGENTS.md")).unwrap();
    std::fs::remove_dir_all(plugin.path("tests")).unwrap();
    plugin.replace(
        "pyproject.toml",
        "roles = []",
        "roles = [\"trading\", \"custody\"]\ntags = []\ndeclaration = \"gone.declaration:DECLARATION\"",
    );
    plugin.write(
        &format!("src/{MODULE}/static/index.html"),
        "<!doctype html>\n<html><head>\n<script src=\"//cdn.example.com/x.js\"></script>\n</head>\n<body style=\"background: white\"></body></html>\n",
    );
    plugin.add_after(
        &main_py(),
        "log = logging.getLogger",
        "import nats\nTOKEN = os.environ.get(\"BROKER_TOKEN\")\nSECRET = meridian.Setting(\"api_key\", str, secret=True)\nprint(settings.values[\"api_key\"])",
    );
    plugin.replace(&page_py(), "    params=OpenStatement,\n", "");

    let report = plugin.check();
    let mut failed = failed_rules(&report);
    failed.sort();
    let mut every: Vec<&str> = RULES.iter().map(|rule| rule.id).collect();
    every.sort();
    assert_eq!(failed, every, "{}", text(&report));
    assert_eq!(report.outcomes.last().map(|o| o.2), Some(Outcome::Skipped));

    let said = text(&report);
    for rule in RULES.iter() {
        assert!(said.contains(&format!("FAIL  {}", rule.id)), "{said}");
    }
    assert!(
        said.contains(&format!(
            "src/{MODULE}/static/index.html:5: `white` is a raw colour"
        )),
        "{said}"
    );
    assert!(said.contains("instead: "), "{said}");
    assert!(
        said.contains(&format!(
            "{} of {} rules failed",
            RULES.len(),
            RULES.len() + 1
        )),
        "{said}"
    );

    let json = json(&report);
    assert_eq!(json["passed"], false);
    let first = &json["failures"][0];
    for key in ["rule", "file", "line", "found", "instead"] {
        assert!(first.get(key).is_some(), "{json}");
    }
    assert!(json["rules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rule| rule["rule"] == "tests-pass" && rule["outcome"] == "skipped"));
}

#[test]
fn what_is_not_a_colour_is_not_called_one() {
    for fine in [
        r##"<a href="#add">Add</a>"##,
        "<p>&#160;</p>",
        r#"f"{n:#06x}""#,
        "#fed { display: block }",
        "#bad .row { gap: var(--space-2) }",
        "color: var(--ink); background: var(--card)",
        "border: 1px solid var(--line-strong)",
        "see issue #1234 for why",
        r##"document.querySelector("#face")"##,
        "<div class=\"notice good\">",
        r#"{ key: "day_pnl", tone: "sign" }"#,
        "color: currentColor; fill: none",
    ] {
        assert!(
            raw_colours(fine).is_empty(),
            "{fine}: {:?}",
            raw_colours(fine)
        );
    }
    for (raw, colour) in [
        ("color: #fff;", "#fff"),
        ("border: 1px solid #ccc", "#ccc"),
        ("background:#0000ff80", "#0000ff80"),
        (r##"<rect fill="#abc"/>"##, "#abc"),
        (
            "box-shadow: 0 1px 2px rgba(0, 0, 0, .2)",
            "rgba(0, 0, 0, .2)",
        ),
        ("color: hsl(120 50% 50%)", "hsl(120 50% 50%)"),
        (r#"<p style="color: red">"#, "red"),
        (r#"<circle stroke="SteelBlue"/>"#, "SteelBlue"),
        (r#"el.style.backgroundColor = "white""#, "white"),
        (r#"{ color: "green" }"#, "green"),
        ("background: var(--card, #fff)", "#fff"),
    ] {
        assert_eq!(raw_colours(raw), [colour], "{raw}");
    }
}

#[test]
fn a_colour_in_a_comment_is_on_no_page() {
    let plugin = Scaffolded::tested("commented");
    plugin.add_after(
        &page_py(),
        "TITLE = ",
        "# was color: #ff0000 before the kit",
    );
    plugin.write(
        &format!("src/{MODULE}/static/page.css"),
        "/* red: #ff0000 */\n.x { color: var(--ink); }\n",
    );
    plugin.write(
        &format!("src/{MODULE}/static/page.js"),
        "// el.style.color = \"red\"\nconst url = \"https://example.com/not-loaded\";\n",
    );
    let report = plugin.check();
    assert!(report.passed(), "{}", text(&report));
}

#[test]
fn the_page_port_is_the_plugins_wiring_and_anything_else_is_a_setting() {
    let plugin = Scaffolded::tested("environment");
    // The template reads REFERENCE_PAGE_PORT, and it holds.
    assert!(plugin.read(&main_py()).contains("_PAGE_PORT"));
    assert!(plugin.check().passed());

    let line = plugin.add_after(
        &main_py(),
        "port = int(",
        "    base = os.getenv('BROKER_URL')",
    );
    let report = plugin.check();
    let failure = &report.failures[0];
    assert_eq!(
        (failure.rule, failure.line, failure.found.as_str()),
        (
            "settings-declared",
            line,
            "BROKER_URL is read from the environment"
        )
    );
    assert!(
        failure.instead.contains("meridian.Setting(\"broker_url\""),
        "{}",
        failure.instead
    );
}

#[test]
fn a_secret_is_found_by_its_constant_or_its_name() {
    let plugin = Scaffolded::tested("secrets");
    plugin.write(
        &format!("src/{MODULE}/settings.py"),
        r#"import meridian

CLIENT_ID = "broker_client_id"
DECLARED = (
    meridian.Setting(
        CLIENT_ID,
        str,
        required=True,
        secret=True,
    ),
    meridian.Setting(name="broker_secret", kind=str, secret=True),
    meridian.Setting("poll_seconds", int, default=60),
)
"#,
    );
    let plugin_read = read(&plugin.0).unwrap();
    assert_eq!(
        secrets(&plugin_read),
        [
            Secret {
                name: Some("broker_client_id".into()),
                constant: Some("CLIENT_ID".into())
            },
            Secret {
                name: Some("broker_secret".into()),
                constant: None
            },
        ]
    );
    // Its name alone, logged, is not its value.
    plugin.add_after(
        &main_py(),
        "log = logging.getLogger",
        "log.info(\"waiting for %s\", \"broker_client_id\")",
    );
    assert!(plugin.check().passed(), "{}", text(&plugin.check()));
    // Its value, in a page or an exception, is.
    let page = plugin.add_after(
        &page_py(),
        "def setup(",
        "    shown = f\"<p>{settings.values['broker_secret']}</p>\"",
    );
    let raised = plugin.add_after(
        &main_py(),
        "log = logging.getLogger",
        "def refuse(values):\n    raise ValueError(f\"refused with {values[CLIENT_ID]}\")",
    ) + 1;
    let report = plugin.check();
    let found: Vec<(String, usize, String)> = report
        .failures
        .iter()
        .map(|f| (f.file.clone(), f.line, f.found.clone()))
        .collect();
    assert_eq!(
        found,
        [
            (
                main_py(),
                raised,
                "the secret setting `broker_client_id` is put in an exception, which is logged"
                    .to_string()
            ),
            (
                page_py(),
                page,
                "the secret setting `broker_secret` is put in a page".to_string()
            ),
        ],
        "{}",
        text(&report)
    );
}

#[test]
fn the_deployment_named_directly_is_refused_wherever_it_is_named() {
    let plugin = Scaffolded::tested("direct");
    let line = plugin.add_after(
        &main_py(),
        "log = logging.getLogger",
        "STORE = \"postgres://meridian@meridian-postgres.meridian.svc.cluster.local/street\"",
    );
    plugin.add_after(
        &main_py(),
        "import signal",
        "from meridian.v1 import sidecar_pb2_grpc",
    );
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        ["through-the-sdk"],
        "{}",
        text(&report)
    );
    let found: Vec<&str> = report.failures.iter().map(|f| f.found.as_str()).collect();
    assert!(
        found.contains(&"it imports the sidecar's raw stubs, sidecar_pb2_grpc"),
        "{found:?}"
    );
    assert!(
        report
            .failures
            .iter()
            .any(|f| f.line == line + 1 && f.found.starts_with("`postgres://")),
        "{}",
        text(&report)
    );
}

#[test]
fn a_tests_run_is_the_plugins_own_python_and_says_what_failed() {
    use std::os::unix::fs::PermissionsExt as _;
    let plugin = Scaffolded::tested("run");
    let python = plugin.path(".venv/bin/python");
    let stand_in = |script: &str| {
        plugin.write(".venv/bin/python", script);
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
        check(&plugin.0, true).unwrap()
    };

    let passed = stand_in("#!/bin/sh\necho \"1 passed\"\n");
    assert!(passed.passed(), "{}", text(&passed));
    assert_eq!(
        passed.outcomes.last().map(|o| (o.0, o.2)),
        Some(("tests-pass", Outcome::Passed))
    );

    let failed = stand_in(
        "#!/bin/sh\necho \"FAILED tests/test_page.py::test_it\"\necho \"1 failed\"\nexit 1\n",
    );
    assert_eq!(failed_rules(&failed), ["tests-pass"]);
    assert!(
        failed.failures[0]
            .found
            .contains("FAILED tests/test_page.py::test_it"),
        "{}",
        text(&failed)
    );

    let none = stand_in("#!/bin/sh\nexit 5\n");
    assert_eq!(none.failures[0].found, "pytest collected no tests");

    let missing = stand_in("#!/bin/sh\necho 'No module named pytest' >&2\nexit 1\n");
    assert!(
        missing.failures[0]
            .found
            .starts_with("pytest is not installed for"),
        "{}",
        text(&missing)
    );
}

#[test]
fn a_directory_that_is_not_there_is_refused_rather_than_checked() {
    assert!(check(Path::new("/nowhere/at/all"), false)
        .err()
        .is_some_and(|refused| refused.contains("is not a directory")));
}

#[test]
fn a_custody_plugin_running_its_suite_and_asking_for_storage_keeps_both_rules() {
    let plugin = Scaffolded::tested("custody-suite");
    plugin.replace("pyproject.toml", "roles = []", "roles = [\"custody\"]");
    plugin.add_after(
        "pyproject.toml",
        "roles = [",
        "declaration = \"checked_plugin.declaration:DECLARATION\"",
    );
    plugin.write(
        &format!("src/{MODULE}/declaration.py"),
        "from meridian.declaration import Declaration, Storage\n\n\
         DECLARATION = Declaration(storage=Storage(retention_days=30))\n",
    );
    plugin.write(
        "tests/test_suite.py",
        "from meridian.suites import run\n\n\ndef test_custody():\n    \
         report = run(\"custody\", {})\n    assert report.passed\n",
    );
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        Vec::<&str>::new(),
        "{}",
        text(&report)
    );
}

// ── tools-cover-routes (contract v12) ────────────────────────────────────

/// A page module declaring `routes`, each one decorator and its view.
fn routes_page(routes: &str) -> String {
    format!("import meridian\n\npages = meridian.Pages(\"Checked\")\n\n\n{routes}\n")
}

fn tools_failures(report: &Report) -> Vec<String> {
    report
        .failures
        .iter()
        .filter(|f| f.rule == "tools-cover-routes")
        .map(|f| f.found.clone())
        .collect()
}

#[test]
fn a_route_that_changes_something_with_no_record_fails_and_one_with_a_record_holds() {
    let plugin = Scaffolded::tested("tools-cover");
    plugin.write(
        &format!("src/{MODULE}/more.py"),
        &routes_page(
            "@pages.route(\"/sync\", levels=\"write\", methods=[\"POST\"])\n\
             async def sync(request): ...\n\n\n\
             @pages.route(\n    \"/typed\",\n    levels=\"write\",\n    methods=[\"POST\"],\n    params=Typed,\n)\n\
             async def typed(request): ...\n\n\n\
             @pages.route(\"/read\", levels=\"read\")\n\
             async def read(request): ...\n\n\n\
             @pages.route(\"/swapped\", levels=\"write\", methods=[\"PUT\"])\n\
             async def swapped(request): ...\n\n\n\
             @pages.tool(replaces=\"/swapped\", method=\"PUT\", params=Typed)\n\
             async def swapped_tool(request): ...\n",
        ),
    );
    let found = tools_failures(&plugin.check());
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].starts_with("/sync changes something"), "{found:?}");
}

#[test]
fn a_route_kept_from_agents_says_why_and_a_verified_plugin_keeps_none() {
    let plugin = Scaffolded::tested("tools-why");
    plugin.write(
        &format!("src/{MODULE}/more.py"),
        &routes_page(
            "@pages.route(\"/upload\", levels=\"write\", methods=[\"POST\"], tool=False)\n\
             async def upload(request): ...\n\n\n\
             @pages.route(\"/file\", levels=\"write\", methods=[\"POST\"], tool=False, why=\"a file a person chooses\")\n\
             async def file(request): ...\n",
        ),
    );
    let found = tools_failures(&plugin.check());
    assert_eq!(
        found,
        vec!["/upload is kept from agents without saying why".to_string()]
    );
    let verified = check_as(&plugin.0, false, true).expect("a directory");
    let found = tools_failures(&verified);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[1].contains("a verified plugin keeps nothing"),
        "{found:?}"
    );
}

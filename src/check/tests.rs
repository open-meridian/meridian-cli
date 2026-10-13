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
        crate::plugin::scaffold(NAME, &dir, None).expect("the template scaffolds");
        Scaffolded(dir)
    }

    /// A plugin `meridian plugin new --role <role>` wrote.
    fn of_role(label: &str, role: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("meridian-check-{label}-{unique}"));
        crate::plugin::scaffold(NAME, &dir, Some(role)).expect("the role's template scaffolds");
        Scaffolded(dir)
    }

    /// Checked as a verified plugin is (`--verified`).
    fn check_verified(&self) -> Report {
        check_as(&self.0, false, true).expect("a directory")
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
    assert!(
        said.contains("--    role-suite") && said.contains("--verified holds it"),
        "{said}"
    );
}

// ── role-suite (contract v11, the data suite from v18, connectivity's from v22)

#[test]
fn a_custody_plugin_from_the_template_keeps_every_rule_and_a_verified_one_runs_its_suite() {
    // The tutorial's plugin: the reference plugin given `custody`, recording a
    // statement no vendor sent. It is not verified for custody, and that
    // breaks no rule; checked as verified, it must run the suite and keep the
    // role's common part, which it is told as advice meanwhile.
    let plugin = Scaffolded::tested("custody-fresh");
    plugin.replace("pyproject.toml", "roles = []", "roles = [\"custody\"]");
    let report = plugin.check();
    assert!(report.passed(), "{}", text(&report));
    for rule in ["role-suite", "role-pages"] {
        assert!(report
            .outcomes
            .iter()
            .any(|(id, _, outcome)| *id == rule && *outcome == Outcome::Skipped));
    }
    assert!(
        report
            .advice
            .iter()
            .all(|advice| advice.rule == "role-pages")
            && !report.advice.is_empty()
    );
    let said = text(&report);
    assert!(
        said.contains("it holds `custody`, and declares no Account links page")
            && said.contains("failing nothing")
            && said.contains("Every rule it was checked against holds."),
        "{said}"
    );

    let verified = plugin.check_verified();
    assert_eq!(
        failed_rules(&verified),
        ["role-suite", "role-pages"],
        "{}",
        text(&verified)
    );
    assert!(verified.advice.is_empty());
    let failure = &verified.failures[0];
    assert_eq!(
        (failure.found.as_str(), failure.file.as_str(), failure.line),
        (
            "it holds `custody`, and no test runs the custody suite",
            "tests/",
            0
        )
    );
    assert!(failure
        .instead
        .contains("meridian.suites.run(\"custody\", producers)"));
}

#[test]
fn each_roles_template_keeps_every_rule_verified_or_not() {
    for role in ["custody", "data", "operations", "reporting"] {
        let plugin = Scaffolded::of_role(role, role);
        for report in [plugin.check(), plugin.check_verified()] {
            assert!(report.passed(), "{role}: {}", text(&report));
            assert!(report.advice.is_empty(), "{role}: {}", text(&report));
        }
    }
}

#[test]
fn a_verified_data_plugin_runs_the_data_suite_in_its_tests() {
    let plugin = Scaffolded::of_role("data-no-suite", "data");
    std::fs::remove_file(plugin.path("tests/test_suite.py")).unwrap();
    assert!(plugin.check().passed(), "not verified, it keeps every rule");
    let verified = plugin.check_verified();
    assert_eq!(
        failed_rules(&verified),
        ["role-suite"],
        "{}",
        text(&verified)
    );
    assert_eq!(
        verified.failures[0].found,
        "it holds `data`, and no test runs the data suite"
    );
}

#[test]
fn a_case_not_presented_is_one_the_suite_lets_a_source_never_present() {
    // The data suite's cases about a field (`about_field`, contract v20) may
    // be named not presented, with why; reads-its-datasets-licences, every
    // data plugin's, may not; nor may a name no case has.
    let plugin = Scaffolded::of_role("data-not-presented", "data");
    plugin.replace(
        "tests/test_suite.py",
        "not_presented=NOT_PRESENTED,",
        "not_presented={\n        **NOT_PRESENTED,\n        \"the-takers-side-stated\": \"its trades name no side\",\n        \"reads-its-datasets-licences\": \"it reads none\",\n        \"a-made-up-case\": \"none\",\n    },",
    );
    assert!(plugin.check().passed(), "not verified, it keeps every rule");
    let verified = plugin.check_verified();
    assert_eq!(
        failed_rules(&verified),
        ["role-suite"],
        "{}",
        text(&verified)
    );
    let line = plugin
        .read("tests/test_suite.py")
        .lines()
        .position(|line| line.contains("reads-its-datasets-licences\": \"it reads none"))
        .unwrap()
        + 1;
    let found: Vec<(&str, usize)> = verified
        .failures
        .iter()
        .map(|failure| (failure.found.as_str(), failure.line))
        .collect();
    assert_eq!(
        found,
        [
            (
                "it names `reads-its-datasets-licences` not presented, a case of the data suite \
                 every plugin of the role passes",
                line
            ),
            (
                "it names `a-made-up-case` not presented, and the data suite has no such case",
                line + 1
            ),
        ]
    );
    assert_eq!(verified.failures[0].file, "tests/test_suite.py");
    assert!(verified.failures[0].instead.contains("about_field"));
}

#[test]
fn the_suites_are_suites_jsons() {
    for role in super::rules::SUITES {
        assert!(
            !super::rules::suite_cases(role).is_empty(),
            "{role} has a suite"
        );
    }
    let data = super::rules::suite_cases("data");
    let optional = |name: &str| {
        data.iter()
            .find(|(case, _)| case == name)
            .map(|(_, may)| *may)
    };
    assert_eq!(optional("the-takers-side-stated"), Some(true));
    assert_eq!(optional("a-daily-close"), Some(true), "about closes");
    assert_eq!(optional("reads-its-datasets-licences"), Some(false));
    assert_eq!(
        optional("a-restarts-gap-backfilled"),
        Some(true),
        "about trades"
    );
    // Ruled 2026-10-13: the calendar cases and records-a-locate carry `about`.
    for role in ["custody", "connectivity", "oms"] {
        let cases = super::rules::suite_cases(role);
        assert!(
            cases.contains(&("reads-a-calendar".to_string(), true)),
            "{role}: about calendars"
        );
    }
    let custody = super::rules::suite_cases("custody");
    assert!(custody.contains(&("asks-for-a-settlement-date".to_string(), true)));
    let connectivity = super::rules::suite_cases("connectivity");
    assert!(connectivity.contains(&("records-a-locate".to_string(), true)));
    assert!(connectivity.contains(&("acknowledges-a-placement-naming-it".to_string(), false)));
}

/// A connectivity plugin's test running its suite: `not_presented` as given.
fn connectivity_suite_test(not_presented: &str) -> String {
    format!(
        "from meridian.suites import run\n\nPRODUCERS = {{}}\n\n\ndef test_the_connectivity_suite():\n    \
         report = run(\n        \"connectivity\",\n        PRODUCERS,\n{not_presented}    )\n    \
         assert report.passed, report.failures\n"
    )
}

#[test]
fn a_verified_connectivity_plugin_runs_its_suite_and_names_no_pause_or_stop_not_presented() {
    // Build row 18 (plans/an-order-reaches-a-venue-through-the-book):
    // `check` runs the `connectivity` suite. Its cases are every connectivity
    // plugin's, sending nothing new on a pause and cancelling at the venue on
    // a stop among them, so none may be named not presented.
    let plugin = Scaffolded::tested("connectivity-suite");
    plugin.replace("pyproject.toml", "roles = []", "roles = [\"connectivity\"]");
    let unverified = plugin.check();
    assert!(
        unverified.passed(),
        "not verified, it keeps every rule: {}",
        text(&unverified)
    );

    let verified = plugin.check_verified();
    assert_eq!(
        failed_rules(&verified),
        ["role-suite"],
        "{}",
        text(&verified)
    );
    assert_eq!(
        (
            verified.failures[0].found.as_str(),
            verified.failures[0].file.as_str()
        ),
        (
            "it holds `connectivity`, and no test runs the connectivity suite",
            "tests/"
        )
    );
    assert!(verified.failures[0]
        .instead
        .contains("meridian.suites.run(\"connectivity\", producers)"));

    plugin.write(
        "tests/test_suite.py",
        &connectivity_suite_test(
            "        not_presented={\n            \"sends-nothing-new-on-a-pause\": \"it never pauses\",\n            \
             \"cancels-at-the-venue-on-a-stop\": \"its venue cancels nothing\",\n        },\n",
        ),
    );
    let verified = plugin.check_verified();
    assert_eq!(
        failed_rules(&verified),
        ["role-suite"],
        "{}",
        text(&verified)
    );
    let line = plugin
        .read("tests/test_suite.py")
        .lines()
        .position(|line| line.contains("sends-nothing-new-on-a-pause"))
        .unwrap()
        + 1;
    let found: Vec<(&str, &str, usize)> = verified
        .failures
        .iter()
        .map(|failure| (failure.found.as_str(), failure.file.as_str(), failure.line))
        .collect();
    assert_eq!(
        found,
        [
            (
                "it names `sends-nothing-new-on-a-pause` not presented, a case of the \
                 connectivity suite every plugin of the role passes",
                "tests/test_suite.py",
                line
            ),
            (
                "it names `cancels-at-the-venue-on-a-stop` not presented, a case of the \
                 connectivity suite every plugin of the role passes",
                "tests/test_suite.py",
                line + 1
            ),
        ]
    );

    plugin.write("tests/test_suite.py", &connectivity_suite_test(""));
    let verified = plugin.check_verified();
    assert!(verified.passed(), "{}", text(&verified));
}

#[test]
fn only_the_connectivity_suites_marked_cases_may_be_named_not_presented() {
    // Ruled 2026-10-13: the calendar, locate and compliance-wait cases are
    // `about` a kind of work, the execution-venue cases `about_field` a fill's
    // place of execution; every other case every connectivity plugin passes.
    let cases = super::rules::suite_cases("connectivity");
    for name in [
        "sends-nothing-new-on-a-pause",
        "cancels-at-the-venue-on-a-stop",
        "reads-the-instructions-in-force-on-start",
    ] {
        assert_eq!(
            cases
                .iter()
                .find(|(case, _)| case == name)
                .map(|(_, may)| *may),
            Some(false),
            "{name}"
        );
    }
    let marked = [
        "reads-a-calendar",
        "records-a-locate",
        "reads-whether-releases-wait-for-compliance",
        "hears-an-accounts-attributes-change",
        "resolves-an-executions-venue",
        "reports-a-venue-not-held",
    ];
    for (case, may) in &cases {
        assert_eq!(*may, marked.contains(&case.as_str()), "{case}");
    }
}

// ── role-pages (spec/plugin-pages-follow-their-role) ─────────────────────

/// The data template, changed one way, fails role-pages alone as verified,
/// each place said, and only advises it otherwise.
fn role_pages_found(plugin: &Scaffolded) -> Vec<(String, String, usize)> {
    let unverified = plugin.check();
    assert!(
        unverified.passed(),
        "advice fails nothing: {}",
        text(&unverified)
    );
    let verified = plugin.check_verified();
    assert_eq!(
        failed_rules(&verified),
        ["role-pages"],
        "{}",
        text(&verified)
    );
    let found: Vec<(String, String, usize)> = verified
        .failures
        .iter()
        .map(|failure| (failure.found.clone(), failure.file.clone(), failure.line))
        .collect();
    let advised: Vec<(String, String, usize)> = unverified
        .advice
        .iter()
        .map(|failure| (failure.found.clone(), failure.file.clone(), failure.line))
        .collect();
    assert_eq!(found, advised, "the same places, advised");
    found
}

fn line_of(plugin: &Scaffolded, file: &str, holding: &str) -> usize {
    plugin
        .read(file)
        .lines()
        .position(|line| line.contains(holding))
        .unwrap_or_else(|| panic!("{file} holds {holding:?}"))
        + 1
}

#[test]
fn a_data_plugin_keeps_its_roles_common_pages() {
    // Renamed: the common page is gone, and its title's words are the
    // plugin's own.
    let plugin = Scaffolded::of_role("pages-renamed", "data");
    plugin.replace(&page_py(), "\"/\", \"Connection\"", "\"/\", \"Status\"");
    let status = line_of(&plugin, &page_py(), "\"/\", \"Status\"") - 1;
    assert_eq!(
        role_pages_found(&plugin),
        [
            (
                "it holds `data`, and declares no Connection page".to_string(),
                page_py(),
                0
            ),
            (
                "a page of the plugin's own, Status, comes before the data role's Datasets page"
                    .into(),
                page_py(),
                status
            ),
        ]
    );

    // Its tool renamed and its record not the role's.
    let plugin = Scaffolded::of_role("pages-tool", "data");
    plugin.replace(
        &page_py(),
        "name=\"read_connection\"",
        "name=\"read_status\"",
    );
    plugin.replace(&page_py(), "answers=data.ConnectionRead", "answers=Status");
    plugin.add_after(
        &page_py(),
        "TITLE = \"Reference prices\"",
        "\n\nclass Status:\n    mode: str\n    calls: int\n",
    );
    let line = line_of(&plugin, &page_py(), "\"/\", \"Connection\"") - 1;
    assert_eq!(
        role_pages_found(&plugin),
        [(
            "the data role's Connection page: its tool is not named read_connection; its \
             record, Status, does not hold the role's common fields credentials, \
             entitled_at_vendor, limit, rate_limited, last_call_at, last_error, sessions"
                .into(),
            page_py(),
            line
        )]
    );

    // A record of the plugin's own extending the role's holds its fields.
    let plugin = Scaffolded::of_role("pages-extended", "data");
    plugin.replace(&page_py(), "answers=data.ConnectionRead", "answers=Feeds");
    plugin.add_after(
        &page_py(),
        "TITLE = \"Reference prices\"",
        "\n\nclass Feeds(data.ConnectionRead):\n    feeds: str = \"\"\n",
    );
    assert!(plugin.check_verified().passed());

    // Out of the role's order, and a page of its own ahead of them.
    let plugin = Scaffolded::of_role("pages-order", "data");
    plugin.replace(
        &page_py(),
        "\"/datasets\", \"Datasets\"",
        "\"/datasets\", \"Feeds\"",
    );
    plugin.replace(&page_py(), "\"/\", \"Connection\"", "\"/\", \"Datasets\"");
    plugin.replace(
        &page_py(),
        "\"/datasets\", \"Feeds\"",
        "\"/datasets\", \"Connection\"",
    );
    let datasets = line_of(&plugin, &page_py(), "\"/\", \"Datasets\"") - 1;
    let connection = line_of(&plugin, &page_py(), "\"/datasets\", \"Connection\"") - 1;
    let found = role_pages_found(&plugin);
    assert!(
        found.contains(&(
            "the data role's Datasets page comes before its Connection".into(),
            page_py(),
            datasets
        )),
        "{found:?}"
    );
    assert!(
        found.iter().any(|(said, _, at)| *at == connection
            && said.starts_with("the data role's Connection page: its path is not /;")),
        "{found:?}"
    );
}

#[test]
fn a_page_of_its_own_comes_after_the_roles_in_its_tab_row() {
    let plugin = Scaffolded::of_role("pages-own-first", "data");
    let line = plugin.add_after(
        &page_py(),
        "return running[0] if running else Converter(request.plugin)",
        "\n\n@pages.page(\"/feeds\", \"Feeds\", levels=\"admin\")\nasync def feeds(request):\n    return pages.answer(\"connection.html\", None)\n",
    ) + 2;
    assert_eq!(
        role_pages_found(&plugin),
        [(
            "a page of the plugin's own, Feeds, comes before the data role's Datasets page".into(),
            page_py(),
            line
        )]
    );
}

#[test]
fn a_plugin_keeps_the_common_settings_in_their_order() {
    let declaration = format!("src/{MODULE}/declaration.py");
    let plugin = Scaffolded::of_role("settings-order", "data");
    plugin.replace(
        &declaration,
        "    sample_data: bool = meridian.sample_data()",
        "    synthetic: bool = meridian.setting(\"Synthetic mode\", developer=True)\n    \
         sample_data: bool = meridian.sample_data()\n    \
         environment: str = meridian.environment()\n    \
         sandbox: bool = meridian.setting(\"Environment\")",
    );
    let at = |name: &str| line_of(&plugin, &declaration, &format!("    {name}:"));
    assert_eq!(
        role_pages_found(&plugin),
        [
            (
                "the setting synthetic, which is sample data now".to_string(),
                declaration.clone(),
                at("synthetic")
            ),
            (
                "the setting sample_data comes before another".into(),
                declaration.clone(),
                at("sample_data")
            ),
            (
                "the setting environment comes after another".into(),
                declaration.clone(),
                at("environment")
            ),
            (
                "the setting sandbox is labelled Environment, the common setting \
                 environment's label"
                    .into(),
                declaration.clone(),
                at("sandbox")
            ),
        ]
    );
}

#[test]
fn settings_declared_in_one_tuple_keep_the_order_they_are_written_in() {
    // As SnapTrade's `_VENDORS`: role helpers between `Setting(`s in one
    // statement, each read where it is written.
    let declaration = format!("src/{MODULE}/declaration.py");
    let plugin = Scaffolded::of_role("settings-tuple", "custody");
    let config = plugin.read(&declaration);
    let start = config
        .find("@meridian.settings")
        .expect("the settings class");
    plugin.write(
        &declaration,
        &format!(
            "{}_VENDORS = (\n    \
             meridian.Setting(\"api_key\", str, required=True, secret=True, label=\"API key\"),\n    \
             custody.poll_seconds(),\n    \
             custody.stale_after_hours(),\n    \
             meridian.Setting(\"account_note\", str, label=\"Account note\"),\n    \
             meridian.sample_data(),\n)\n\n\n\
             DECLARATION = meridian.Declaration(settings=list(_VENDORS))\n",
            &config[..start]
        ),
    );
    let ordered = |plugin: &Scaffolded| -> Vec<(String, usize)> {
        plugin
            .check_verified()
            .failures
            .into_iter()
            .filter(|failure| failure.rule == "role-pages")
            .map(|failure| (failure.found, failure.line))
            .collect()
    };
    assert_eq!(ordered(&plugin), [], "sample data is last, as written");

    // An environment written after the key comes after another.
    plugin.replace(
        &declaration,
        "    custody.poll_seconds(),",
        "    meridian.environment(),\n    custody.poll_seconds(),",
    );
    assert_eq!(
        ordered(&plugin),
        [(
            "the setting environment comes after another".into(),
            line_of(&plugin, &declaration, "meridian.environment(),")
        )]
    );
}

#[test]
fn a_reporting_plugin_values_in_its_reporting_currency() {
    let plugin = Scaffolded::of_role("reporting-currency", "reporting");
    plugin.replace(
        &main_py(),
        "settings=[reporting.reporting_currency()]",
        "settings=[]",
    );
    assert_eq!(
        role_pages_found(&plugin),
        [(
            "it holds `reporting`, and declares no setting reporting_currency".into(),
            page_py(),
            0
        )]
    );
}

#[test]
fn a_custody_plugin_links_by_the_roles_act() {
    let plugin = Scaffolded::of_role("custody-link", "custody");
    plugin.replace(
        &page_py(),
        "name=\"link_account\"",
        "name=\"connect_account\"",
    );
    plugin.replace(&page_py(), "    checked_by=\"check_link_account\",\n", "");
    let found: Vec<String> = plugin
        .check_verified()
        .failures
        .into_iter()
        .filter(|failure| failure.rule == "role-pages")
        .map(|failure| failure.found)
        .collect();
    assert_eq!(
        found,
        [
            "/links is an act and names no check tool, so an agent cannot prepare it",
            "it holds `custody`, and declares no route named link_account"
        ]
    );
}

#[test]
fn a_custody_plugin_keeping_raw_records_has_the_roles_page_for_them() {
    let plugin = Scaffolded::of_role("custody-raw", "custody");
    plugin.add_after(
        &format!("src/{MODULE}/declaration.py"),
        "from meridian.roles import custody",
        "\nKINDS = [meridian.RecordKind(\"responses\", \"Raw responses\", window_days=30)]",
    );
    let raw = |plugin: &Scaffolded| -> Vec<String> {
        plugin
            .check_verified()
            .failures
            .into_iter()
            .filter(|failure| {
                failure.rule == "role-pages"
                    && !failure.found.contains("is an act")
                    && !failure.found.contains("comes before")
            })
            .map(|failure| failure.found)
            .collect()
    };
    assert_eq!(
        raw(&plugin),
        ["it keeps raw records, and declares no Raw records page"]
    );
    let line = plugin.add_after(
        &page_py(),
        "return found if isinstance(found, Custodian) else Custodian()",
        "\n\n@pages.page(\"/raw\", \"Raw responses\", levels=\"admin\")\nasync def raw(request):\n    return \"\"\n",
    ) + 2;
    let found = raw(&plugin);
    assert_eq!(
        found,
        ["a page titled Raw responses, the custody role's Raw records page"]
    );
    assert_eq!(
        plugin
            .check_verified()
            .failures
            .iter()
            .find(|failure| failure.found == found[0])
            .map(|failure| failure.line),
        Some(line)
    );
    plugin.replace(
        &page_py(),
        "\"Raw responses\", levels=\"admin\"",
        "\"Raw records\", levels=\"admin\"",
    );
    assert_eq!(
        raw(&plugin),
        ["the custody role's Raw records page is under Manage"]
    );
    plugin.replace(
        &page_py(),
        "\"Raw records\", levels=\"admin\"",
        "\"Raw records\", levels=[\"write\", \"read\"]",
    );
    assert!(raw(&plugin).is_empty());
}

#[test]
fn a_secret_declared_as_a_settings_field_is_kept_too() {
    let plugin = Scaffolded::of_role("class-secret", "custody");
    let declaration = format!("src/{MODULE}/declaration.py");
    let line = plugin.add_after(
        &declaration,
        "DECLARATION = meridian.Declaration(",
        "print(Config.api_key)",
    );
    let found: Vec<(String, usize)> = plugin
        .check()
        .failures
        .into_iter()
        .filter(|failure| failure.rule == "secrets-kept")
        .map(|failure| (failure.found, failure.line))
        .collect();
    assert_eq!(
        found,
        [("the secret setting `api_key` is logged".to_string(), line)]
    );
    assert!(secrets(&super::read(&plugin.0).unwrap())
        .iter()
        .any(|secret| secret.name.as_deref() == Some("api_key")));
}

#[test]
fn every_act_names_its_check_tool_whatever_roles_the_plugin_holds() {
    // The reference plugin's two acts, their check tools taken away: advised,
    // and failed when verified, with no role's common part asked of it.
    for roles in ["[]", "[\"signal\"]", "[\"oms\"]"] {
        let plugin = Scaffolded::tested("acts");
        assert!(
            plugin.check_verified().passed(),
            "{roles}: the template's acts are checked"
        );
        plugin.replace(&page_py(), "    checked_by=\"check_link_account\",\n", "");
        plugin.replace(&page_py(), "    checked_by=\"check_open_statement\",\n", "");
        plugin.replace("pyproject.toml", "roles = []", &format!("roles = {roles}"));
        let verified = plugin.check_verified();
        assert_eq!(
            failed_rules(&verified),
            ["role-pages"],
            "{roles}: {}",
            text(&verified)
        );
        let found: Vec<(&str, usize)> = verified
            .failures
            .iter()
            .map(|failure| (failure.found.as_str(), failure.line))
            .collect();
        let at = |path: &str| line_of(&plugin, &page_py(), &format!("    \"{path}\",")) - 1;
        assert_eq!(
            found,
            [
                (
                    "/link is an act and names no check tool, so an agent cannot prepare it",
                    at("/link")
                ),
                (
                    "/statement is an act and names no check tool, so an agent cannot prepare it",
                    at("/statement")
                ),
            ],
            "{roles}"
        );
        assert_eq!(plugin.check().advice.len(), 2);
    }

    // Named, the check must be a read route the plugin declares.
    let plugin = Scaffolded::tested("acts-checked");
    plugin.replace(
        &page_py(),
        "    checked_by=\"check_open_statement\",\n",
        "    checked_by=\"check_statement\",\n",
    );
    let found: Vec<String> = plugin
        .check_verified()
        .failures
        .into_iter()
        .map(|failure| failure.found)
        .collect();
    assert_eq!(
        found,
        ["/statement names the check tool check_statement, which no read route declares"]
    );
}

/// A link's form replaced by a tool naming its check, as SnapTrade and
/// sample-operations declare linking: the route by a constant, and one by
/// its path as written.
const REPLACED_ACTS: &str = r#"import meridian

pages = meridian.Pages("Checked")

LINK = "/accounts/link"


@pages.route(LINK, levels="write", methods=["POST"], params=LinkAsked)
async def link_form(request): ...


@pages.route("/accounts/unlink", levels="write", methods=["POST"], params=LinkAsked)
async def unlink_form(request): ...


@pages.route(
    LINK + "/check",
    levels="write",
    methods=["POST"],
    params=LinkAsked,
    reads=True,
    name="check_link",
)
async def check_link(request): ...


@pages.tool(replaces=LINK, params=LinkAsked, name="link", checked_by="check_link")
async def link(request): ...


@pages.tool(replaces="/accounts/unlink", params=LinkAsked, name="unlink", checked_by="check_link")
async def unlink(request): ...
"#;

#[test]
fn a_route_a_tool_replaces_is_not_an_act_of_its_own() {
    let acts = |plugin: &Scaffolded| -> Vec<(String, usize)> {
        plugin
            .check_verified()
            .failures
            .into_iter()
            .filter(|failure| failure.rule == "role-pages")
            .map(|failure| (failure.found, failure.line))
            .collect()
    };
    let more = format!("src/{MODULE}/more.py");
    let plugin = Scaffolded::tested("acts-replaced");
    plugin.write(&more, REPLACED_ACTS);
    assert_eq!(
        acts(&plugin),
        [],
        "the tools are the acts, and name their check"
    );

    // Neither replaced, each route is an act naming no check tool.
    plugin.replace(&more, "replaces=LINK, ", "");
    plugin.replace(&more, "replaces=\"/accounts/unlink\", ", "");
    let at = |needle: &str| line_of(&plugin, &more, needle);
    assert_eq!(
        acts(&plugin),
        [
            (
                "LINK is an act and names no check tool, so an agent cannot prepare it".into(),
                at("@pages.route(LINK,")
            ),
            (
                "/accounts/unlink is an act and names no check tool, so an agent cannot \
                 prepare it"
                    .into(),
                at("@pages.route(\"/accounts/unlink\",")
            ),
        ]
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
    let breakages: [(&str, Breaking, &str); 17] = [
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
                p.replace("Dockerfile", "plugin-python:0.26.0", "plugin-python:0.5.0");
                ("Dockerfile".into(), 10)
            },
            "its base is plugin-python:0.5.0, and pyproject.toml pins open-meridian==0.26.0",
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
            "roles-declared",
            |p| {
                p.replace(
                    "pyproject.toml",
                    "roles = []",
                    "roles = [\"operations\", \"portfolio\"]",
                );
                let text = p.read(&page_py());
                let at = text.find("@pages.page(\"/setup\"").expect("the template's page");
                (page_py(), text[..at].matches('\n').count() + 1)
            },
            "page \"/setup\" names no roles, on a plugin holding operations, portfolio",
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
            "window-settings",
            |p| {
                let line = p.add_after(
                    &main_py(),
                    "log = logging.getLogger",
                    "KINDS = [meridian.RecordKind(\"activity\", \"Reported activity\", window_days=2555)]\nWINDOW = meridian.Setting(\"activity_window_days\", int)",
                );
                (main_py(), line + 1)
            },
            "the setting activity_window_days takes the name of a window setting of the kind \
             activity, which the SDK declares",
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
                p.replace(
                    &page_py(),
                    "    params=OpenStatement,\n    name=\"open_statement\",",
                    "    name=\"open_statement\",",
                );
                let text = p.read(&page_py());
                let at = text
                    .find("@pages.route(\n    \"/statement\",\n")
                    .expect("the template's statement route");
                (page_py(), text[..at].matches('\n').count() + 1)
            },
            "/statement changes something and declares no typed record, so no tool is derived from it",
        ),
        (
            "runs-as-checked",
            |p| {
                p.replace(&page_py(), "    checked_by=\"check_link_account\",\n", "");
                p.write(&format!("src/{MODULE}/sweep.py"), SWEEP_LINKING);
                (format!("src/{MODULE}/sweep.py"), 6)
            },
            "link_account (src/checked_plugin/page.py:218) is an act called through runs_as and \
             names no check tool, so a service account set to prepare only is refused it",
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
        "import nats\nTOKEN = os.environ.get(\"BROKER_TOKEN\")\nSECRET = meridian.Setting(\"api_key\", str, secret=True)\nprint(settings.values[\"api_key\"])\nKIND = meridian.RecordKind(\"fills\", \"Fills\", window_days=30)\nPAST = meridian.Setting(\"fills_past_window\", str)",
    );
    plugin.replace(&page_py(), "    params=OpenStatement,\n", "");
    plugin.replace(&page_py(), "    checked_by=\"check_link_account\",\n", "");
    plugin.write(&format!("src/{MODULE}/sweep.py"), SWEEP_LINKING);

    // As a verified plugin is checked, so the rules only --verified holds are
    // among them.
    let report = plugin.check_verified();
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
    let report = plugin.check_verified();
    assert_eq!(
        failed_rules(&report),
        ["role-pages"],
        "only the role's common part, which the reference plugin has not: {}",
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

// ── runs-as-checked (contract v21) ───────────────────────────────────────

/// A sweep linking an account as its role's service account, through the
/// template's `link_account`.
const SWEEP_LINKING: &str = "import meridian\n\n\nasync def sweep(arguments):\n    \
    async with meridian.runs_as(\"custody\") as act:\n        \
    await act(\"link_account\", arguments, reason=\"swept\")\n";

/// The acts a sweep calls, declared as sample-operations declares closing
/// cleared breaks: the form's route kept from agents, its check, and the
/// tool replacing it naming that check; and an act naming none.
const BREAK_ACTS: &str = r#"import meridian

pages = meridian.Pages("Checked")

CLEARED = "/breaks/close-cleared"


@pages.route(CLEARED, levels="write", methods=["POST"], tool=False, why="its tool is close_cleared_breaks's")
async def close_cleared(request): ...


@pages.route(
    CLEARED + "/check",
    levels="write",
    methods=["POST"],
    params=ClearedAsked,
    reads=True,
    name="check_close_cleared_breaks",
)
async def check_close_cleared_breaks(request): ...


@pages.tool(
    replaces=CLEARED,
    params=ClearedAsked,
    name="close_cleared_breaks",
    checked_by="check_close_cleared_breaks",
    accounts=["account_id"],
)
async def close_cleared_breaks(request): ...


@pages.route("/breaks/reopen", levels="write", methods=["POST"], params=ReopenAsked)
async def reopen_break(request): ...
"#;

/// sample-operations' sweep (0.11.0): `meridian.runs_as` a parameter's
/// default, entered as `self.act_as(ROLE)`, the act named by a constant.
const SAMPLE_SWEEP: &str = r#"from collections.abc import Callable
from contextlib import AbstractAsyncContextManager
from typing import Any

import meridian

ROLE = "operations"
CLOSE_CLEARED = "close_cleared_breaks"

ActAs = Callable[[str], AbstractAsyncContextManager[Any]]


class Follower:
    def __init__(self, book, *, act_as: ActAs = meridian.runs_as) -> None:
        self.book = book
        self.act_as = act_as

    async def sweep(self, arguments, why):
        async with self.act_as(ROLE) as act:
            reply = await act(CLOSE_CLEARED, arguments, reason=why)
        return reply
"#;

fn runs_as_failures(report: &Report) -> Vec<(String, usize, String)> {
    report
        .failures
        .iter()
        .filter(|f| f.rule == "runs-as-checked")
        .map(|f| (f.file.clone(), f.line, f.found.clone()))
        .collect()
}

fn refused(tool: &str, at: usize) -> String {
    format!(
        "{tool} (src/{MODULE}/acts.py:{at}) is an act called through runs_as and names no check \
         tool, so a service account set to prepare only is refused it"
    )
}

#[test]
fn an_act_the_sweep_calls_through_runs_as_names_its_check_tool() {
    let plugin = Scaffolded::tested("runs-as");
    let acts = format!("src/{MODULE}/acts.py");
    let follow = format!("src/{MODULE}/follow.py");
    plugin.write(&acts, BREAK_ACTS);
    plugin.write(&follow, SAMPLE_SWEEP);
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        Vec::<&str>::new(),
        "{}",
        text(&report)
    );

    // Its check taken away, the call is refused where it is made.
    plugin.replace(
        &acts,
        "    checked_by=\"check_close_cleared_breaks\",\n",
        "",
    );
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        ["runs-as-checked"],
        "{}",
        text(&report)
    );
    assert_eq!(
        runs_as_failures(&report),
        [(
            follow.clone(),
            line_in(&plugin, &follow, "reply = await act("),
            refused(
                "close_cleared_breaks",
                line_in(&plugin, &acts, "@pages.tool(")
            ),
        )]
    );
    assert!(report.failures[0].instead.contains("checked_by="));
}

#[test]
fn runs_as_is_read_however_it_is_reached_and_an_act_without_its_check_fails() {
    let plugin = Scaffolded::tested("runs-as-forms");
    let acts = format!("src/{MODULE}/acts.py");
    let work = format!("src/{MODULE}/work.py");
    plugin.write(&acts, BREAK_ACTS);
    plugin.write(
        &work,
        r#"import meridian
from meridian import runs_as
from meridian import runs_as as as_service

OPERATIONS = "operations"


async def first(arguments):
    async with meridian.runs_as("operations") as act:
        await act("reopen_break", arguments, reason="first")


async def second(arguments):
    async with runs_as(OPERATIONS) as act:
        await act(
            tool_name="reopen_break",
            arguments=arguments,
            reason="second",
        )


async def third(arguments):
    context = meridian.runs_as(OPERATIONS)
    async with context as act: await act("reopen_break", arguments, reason="third")


async def fourth(arguments):
    async with as_service("operations") as made, open("x") as other:
        await made("reopen_break", arguments, reason="fourth")
        other.write("reopen_break")
"#,
    );
    let at = |needle: &str| line_in(&plugin, &work, needle);
    let reopen = refused(
        "reopen_break",
        line_in(&plugin, &acts, "\"/breaks/reopen\""),
    );
    assert_eq!(
        runs_as_failures(&plugin.check()),
        [
            (work.clone(), at("reason=\"first\""), reopen.clone()),
            (
                work.clone(),
                at("tool_name=\"reopen_break\"") - 1,
                reopen.clone()
            ),
            (work.clone(), at("reason=\"third\""), reopen.clone()),
            (work.clone(), at("reason=\"fourth\""), reopen.clone()),
        ]
    );
}

#[test]
fn a_tool_the_text_cannot_read_or_not_among_the_plugins_acts_is_passed_over() {
    let plugin = Scaffolded::tested("runs-as-unread");
    plugin.write(&format!("src/{MODULE}/acts.py"), BREAK_ACTS);
    plugin.write(
        &format!("src/{MODULE}/work.py"),
        r#"import meridian


async def sweep(arguments, tool):
    async with meridian.runs_as("operations") as act:
        await act(tool, arguments, reason="chosen at run time")
        await act(f"reopen_{tool}", arguments, reason="formatted")
        await act("rebalance", arguments, reason="not one of the plugin's acts")
        await act("check_close_cleared_breaks", arguments, reason="a read, not an act")
        await act("close_cleared_breaks", arguments, reason="named with its check")
    await act("reopen_break", arguments, reason="outside the block")
"#,
    );
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        Vec::<&str>::new(),
        "{}",
        text(&report)
    );

    // A test calling the act so is not the plugin's work.
    plugin.write(
        "tests/test_sweep.py",
        "import meridian\n\n\nasync def test_sweep():\n    \
         async with meridian.runs_as(\"operations\") as act:\n        \
         await act(\"reopen_break\", {}, reason=\"tested\")\n",
    );
    assert!(runs_as_failures(&plugin.check()).is_empty());
}

// ── window-settings (contract v16) ───────────────────────────────────────

#[test]
fn a_kinds_window_setting_is_the_sdks_by_any_spelling_and_other_names_are_the_plugins() {
    let plugin = Scaffolded::tested("window-settings");
    let declaration = format!("src/{MODULE}/declaration.py");
    plugin.write(
        &declaration,
        "import meridian\n\
         from meridian.declaration import RecordKind, Storage\n\n\
         ACTIVITY = \"activity\"\n\
         PAST = \"responses_past_window\"\n\n\
         STORAGE = Storage(kinds=[\n\
         \x20   RecordKind(ACTIVITY, \"Reported activity\", window_days=2555),\n\
         \x20   RecordKind(name=\"responses\", label=\"Raw responses\", window_days=30),\n\
         ])\n\
         SETTINGS = [\n\
         \x20   meridian.Setting(\"plan_code_links\", str),\n\
         \x20   meridian.Setting(\"orders_window_days\", int),\n\
         \x20   meridian.Setting(name=PAST, kind=str),\n\
         ]\n",
    );
    let report = plugin.check();
    let found: Vec<(String, usize)> = report
        .failures
        .iter()
        .map(|failure| (failure.found.clone(), failure.line))
        .collect();
    assert_eq!(
        found,
        vec![(
            "the setting responses_past_window takes the name of a window setting of the kind \
             responses, which the SDK declares"
                .to_string(),
            14
        )],
        "{}",
        text(&report)
    );
    assert!(report.failures[0]
        .instead
        .contains("`responses_window_days` and `responses_past_window`"));

    // No kind declared, no name is reserved: the SDK declares no window.
    plugin.replace(&declaration, "STORAGE = Storage(kinds=[", "STORAGE = ([");
    plugin.replace(&declaration, "RecordKind(ACTIVITY", "dict(ACTIVITY");
    plugin.replace(&declaration, "RecordKind(name=", "dict(name=");
    assert!(plugin.check().passed(), "{}", text(&plugin.check()));
}

// ── roles-declared (contract v15) ────────────────────────────────────────

/// The template as a plugin holding custody and operations writes it on
/// SDK 0.20.0: every page, route and setting naming the roles it serves, a
/// tool taking its route's, and each changing route sending only commands
/// its roles hold.
fn two_roles(label: &str) -> Scaffolded {
    let plugin = Scaffolded::tested(label);
    plugin.replace(
        "pyproject.toml",
        "roles = []",
        "roles = [\"custody\", \"operations\"]",
    );
    plugin.write(
        "tests/test_suite.py",
        "from meridian.suites import run\n\n\ndef test_custody():\n    \
         report = run(\"custody\", {})\n    assert report.passed\n",
    );
    plugin.replace(
        &page_py(),
        "@pages.page(\"/setup\", \"Setup\", levels=\"admin\")",
        "ROLES = (\"custody\", \"operations\")\n\n\n\
         @pages.page(\"/setup\", \"Setup\", levels=\"admin\", roles=ROLES)",
    );
    plugin.replace(
        &page_py(),
        "@pages.page(\"/\", \"Accounts\", levels=[\"write\", \"read\"])",
        "@pages.page(\"/\", \"Accounts\", levels=[\"write\", \"read\"], roles=ROLES)",
    );
    plugin.replace(
        &page_py(),
        "    \"/statement\",\n    levels=\"write\",",
        "    \"/statement\",\n    levels=\"write\",\n    roles=[\"custody\"],",
    );
    plugin.replace(
        &page_py(),
        "    \"/link\",\n    levels=\"admin\",",
        "    \"/link\",\n    levels=\"admin\",\n    roles=[\"custody\"],",
    );
    for check in ["/link/check", "/statement/check"] {
        let level = if check == "/link/check" {
            "admin"
        } else {
            "write"
        };
        plugin.replace(
            &page_py(),
            &format!("    \"{check}\",\n    levels=\"{level}\","),
            &format!("    \"{check}\",\n    levels=\"{level}\",\n    roles=[\"custody\"],"),
        );
    }
    plugin.write(
        &format!("src/{MODULE}/balances.py"),
        "import meridian\n\n\
         from .page import pages\n\n\
         SETTINGS = [\n    meridian.Setting(\"api_key\", secret=True, roles=(\"custody\", \"operations\")),\n    \
         meridian.Setting(\"tolerance\", int, roles=\"operations\"),\n]\n\n\n\
         @pages.route(\"/balance\", levels=\"write\", roles=\"operations\", methods=[\"POST\"], params=Balance)\n\
         async def balance(request):\n    \
         await request.plugin.record_opening_balance(acting_for=request.caller.header)\n\n\n\
         @pages.tool(replaces=\"/balance\", params=Balance)\n\
         async def balance_tool(request):\n    \
         await request.plugin.record_opening_balance(acting_for=request.caller.header)\n\n\n\
         @pages.route(\"/balances\", levels=\"read\", roles=\"operations\")\n\
         async def balances(request):\n    ...\n",
    );
    plugin
}

fn roles_failures(report: &Report) -> Vec<(String, usize, String)> {
    report
        .failures
        .iter()
        .filter(|f| f.rule == "roles-declared")
        .map(|f| (f.file.clone(), f.line, f.found.clone()))
        .collect()
}

/// The line, from 1, that `needle` first appears on in a scaffolded file.
fn line_in(plugin: &Scaffolded, file: &str, needle: &str) -> usize {
    let text = plugin.read(file);
    let at = text.find(needle).expect("the text to find");
    text[..at].matches('\n').count() + 1
}

#[test]
fn a_two_role_plugin_naming_its_roles_everywhere_keeps_every_rule() {
    let plugin = two_roles("roles-kept");
    let report = plugin.check();
    assert!(report.passed(), "{}", text(&report));
    assert!(
        text(&report).contains("ok    roles-declared"),
        "{}",
        text(&report)
    );
}

#[test]
fn a_role_less_page_on_a_two_role_plugin_fails_naming_its_line() {
    let plugin = two_roles("roles-missing");
    plugin.replace(
        &page_py(),
        "levels=[\"write\", \"read\"], roles=ROLES)",
        "levels=[\"write\", \"read\"])",
    );
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        ["roles-declared"],
        "{}",
        text(&report)
    );
    let line = line_in(&plugin, &page_py(), "@pages.page(\"/\"");
    assert_eq!(
        roles_failures(&report),
        vec![(
            page_py(),
            line,
            "page \"/\" names no roles, on a plugin holding custody, operations".to_string()
        )]
    );
    assert!(
        text(&report).contains(&format!("{}:{line}: page \"/\" names no roles", page_py())),
        "{}",
        text(&report)
    );
}

#[test]
fn a_route_sending_a_command_none_of_its_roles_holds_fails_naming_those_that_do() {
    let plugin = two_roles("roles-command");
    plugin.replace(
        &page_py(),
        "    \"/statement\",\n    levels=\"write\",\n    roles=[\"custody\"],",
        "    \"/statement\",\n    levels=\"write\",\n    roles=[\"operations\"],",
    );
    let report = plugin.check();
    assert_eq!(
        failed_rules(&report),
        ["roles-declared"],
        "{}",
        text(&report)
    );
    let line = line_in(&plugin, &page_py(), "record_holdings_statement(");
    assert_eq!(
        roles_failures(&report),
        vec![(
            page_py(),
            line,
            "route \"/statement\" serves operations and sends RecordHoldingsStatement \
             (record_holdings_statement), which custody holds"
                .to_string()
        )]
    );
    let failure = report
        .failures
        .iter()
        .find(|f| f.rule == "roles-declared")
        .unwrap();
    assert!(failure.instead.contains("(custody)"), "{}", failure.instead);
}

#[test]
fn a_setting_or_tool_naming_a_role_the_plugin_does_not_hold_fails() {
    let plugin = two_roles("roles-stranger");
    let balances = format!("src/{MODULE}/balances.py");
    plugin.replace(&balances, "roles=\"operations\"),", "),");
    plugin.replace(
        &balances,
        "@pages.tool(replaces=\"/balance\", params=Balance)",
        "@pages.tool(replaces=\"/balance\", params=Balance, roles=[\"oms\"])",
    );
    let found: Vec<String> = roles_failures(&plugin.check())
        .into_iter()
        .map(|(_, _, found)| found)
        .collect();
    assert_eq!(
        found,
        vec![
            "tool \"/balance\" names `oms`, which this plugin does not hold (custody, operations)"
                .to_string(),
            "tool \"/balance\" serves oms and sends RecordOpeningBalance \
             (record_opening_balance), which operations holds"
                .to_string(),
            "setting \"tolerance\" names no roles, on a plugin holding custody, operations"
                .to_string(),
        ]
    );
}

#[test]
fn a_plugin_holding_one_role_names_none_and_the_rule_holds() {
    let plugin = Scaffolded::tested("roles-one");
    plugin.replace("pyproject.toml", "roles = []", "roles = [\"operations\"]");
    let report = plugin.check();
    assert!(report.passed(), "{}", text(&report));
}

#[test]
fn the_roles_and_commands_are_roles_jsons() {
    let mut published = super::rules::published_roles();
    published.sort();
    assert_eq!(published, super::rules::ROLES.to_vec());
    let commands = super::rules::commands();
    let held = |name: &str| {
        commands
            .iter()
            .find(|(_, named, _)| named == name)
            .map(|(call, _, roles)| (call.clone(), roles.clone()))
    };
    assert_eq!(
        held("RecordOpeningBalance"),
        Some(("record_opening_balance".into(), vec!["operations".into()]))
    );
    assert_eq!(
        held("RecordHoldingsStatement"),
        Some(("record_holdings_statement".into(), vec!["custody".into()]))
    );
    assert_eq!(held("ListBreaks"), None, "a query is no command");
}

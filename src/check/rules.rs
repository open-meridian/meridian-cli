//! Every rule `meridian plugin check` holds a plugin to, in one place, and
//! released with the CLI (decisions/025).
//!
//! A rule reads the plugin's files as text. The page rules read whatever a
//! page is written in -- HTML, CSS, SVG, script, or the Python that renders
//! them -- so they hold for a plugin in any language; the rules about the
//! plugin's code read Python. Each looks for what can be decided from the
//! text: the obvious forms of a raw colour, an environment read, a secret
//! logged. What needs judgement stays advice in the template's AGENTS.md.
//!
//! Tests, `conftest.py` and `test_*.py`, are not the plugin's pages or code:
//! a test may well assert that a colour is absent by naming it.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use super::{Failure, Plugin, Rule, Source};

/// The rules, in the order they are reported: what the project is, what it
/// declares, its pages, its settings, how it reaches the deployment, and its
/// tests. Running the tests is `TESTS_PASS`, which only `--run-tests` asks.
pub const RULES: [Rule; 16] = [
    Rule {
        id: "template-shape",
        holds: "the project keeps the template's shape",
        check: template_shape,
    },
    Rule {
        id: "tool-meridian",
        holds: "[tool.meridian] names roles from the fixed list, and no tags",
        check: tool_meridian,
    },
    Rule {
        id: "edge-storage",
        holds: "only a plugin at the edge asks for storage in its declaration",
        check: edge_storage,
    },
    Rule {
        id: "role-suite",
        holds: "a verified plugin holding a role with a suite runs the suite in its tests",
        check: role_suite,
    },
    Rule {
        id: "role-pages",
        holds: "a plugin holding a role with a common part keeps its common pages, tools and \
                settings, in the role's order, and every act names its check tool",
        check: role_pages,
    },
    Rule {
        id: "kit-linked",
        holds: "every page links the kit from /.meridian/ui/, and holds no copy of it",
        check: kit_linked,
    },
    Rule {
        id: "no-raw-colour",
        holds: "no raw colour: no hex, rgb(), hsl() or colour name in a page or its styles",
        check: no_raw_colour,
    },
    Rule {
        id: "own-origin",
        holds: "a page loads nothing from another origin",
        check: own_origin,
    },
    Rule {
        id: "tools-cover-routes",
        holds: "every route that changes something is a tool for agents, or says why not",
        check: tools_cover_routes,
    },
    Rule {
        id: "runs-as-checked",
        holds: "every act the plugin's own work calls through runs_as names its check tool",
        check: runs_as_checked,
    },
    Rule {
        id: "roles-declared",
        holds: "on a plugin holding several roles, every page, route and setting names its roles, \
                and a changing route sends only its roles' commands",
        check: roles_declared,
    },
    Rule {
        id: "settings-declared",
        holds: "settings are declared to the SDK, not read from the environment",
        check: settings_declared,
    },
    Rule {
        id: "window-settings",
        holds: "no setting of the plugin's own takes a declared kind's window setting's name",
        check: window_settings,
    },
    Rule {
        id: "secrets-kept",
        holds: "no secret setting's value is logged or put in a page",
        check: secrets_kept,
    },
    Rule {
        id: "through-the-sdk",
        holds: "the deployment is reached only through the SDK",
        check: through_the_sdk,
    },
    Rule {
        id: "tests-exist",
        holds: "the plugin has tests",
        check: tests_exist,
    },
];

pub const TESTS_PASS: Rule = Rule {
    id: "tests-pass",
    holds: "its tests pass",
    check: run_tests,
};

/// The roles a deployment has (decisions/020): meridian-design's
/// matrix/roles.tsv, as this release of the CLI knows it. What a plugin may
/// publish and subscribe to is its roles', and nothing else.
pub const ROLES: [&str; 14] = [
    "compliance",
    "confirmation",
    "connectivity",
    "custody",
    "data",
    "ems",
    "oms",
    "operations",
    "portfolio",
    "reference",
    "reporting",
    "servicing",
    "settlement",
    "signal",
];

/// The roles at the edge, which alone may own the storage a deployment
/// grants an instance for its raw external records (decisions/028).
pub const EDGE_ROLES: [&str; 7] = [
    "confirmation",
    "connectivity",
    "custody",
    "data",
    "reporting",
    "servicing",
    "settlement",
];

/// The roles whose conformance suite a verified plugin runs in its tests, as
/// this release of the CLI holds them: `custody` (contract v11), `data`
/// (contract v18, as `dgm`) and `connectivity` (contract v22: plans/an-order-
/// reaches-a-venue-through-the-book, build row 18, "`check` runs the
/// `connectivity` suite"). A plugin holding one is verified for it only by
/// passing every case (spec/vendor-differences-have-a-place-in-the-contract,
/// requirement 18): a connectivity plugin among them sending nothing new on a
/// pause and cancelling at its venue on a stop, cases no plugin of the role
/// may name not presented.
pub const SUITES: [&str; 3] = ["custody", "data", "connectivity"];

/// The rules only `--verified` holds a plugin to. A plugin holding a role
/// with a suite and not passing it is not verified, and there is nothing to
/// declare instead (requirement 18): it is not a plugin breaking the
/// framework's rules. So the reference plugin, given `custody` to record a
/// statement nobody's vendor sent, keeps every rule, and a plugin checked as
/// verified, as SnapTrade's CI checks it, is held to every case. A plugin
/// keeps its role's common part (`role-pages`) the same way: required of a
/// verified plugin (spec/plugin-pages-follow-their-role, Q7).
pub const ONLY_VERIFIED: [&str; 2] = ["role-suite", "role-pages"];

/// Of those, the rules whose findings are still said, as advice, to a plugin
/// not checked as verified: where it leaves its role's common part, without
/// failing it (the spec's Q7: "Advice, not failure, for community and local
/// plugins").
pub const ADVISED: [&str; 1] = ["role-pages"];

// ── Which files ──────────────────────────────────────────────────────────

fn extension(path: &str) -> &str {
    let name = file_name(path);
    name.rsplit_once('.').map(|(_, e)| e).unwrap_or_default()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A test, which is not the plugin's pages or code.
fn is_test(path: &str) -> bool {
    let name = file_name(path);
    path.starts_with("tests/")
        || path.starts_with("test/")
        || path.contains("/tests/")
        || path.contains("/test/")
        || (name.starts_with("test_") && name.ends_with(".py"))
        || name.ends_with("_test.py")
        || name == "conftest.py"
}

/// What a page may be written in, the Python that renders one included.
const PAGE: [&str; 16] = [
    "py", "html", "htm", "css", "svg", "js", "mjs", "cjs", "jsx", "ts", "tsx", "vue", "svelte",
    "jinja", "jinja2", "j2",
];

fn pages(plugin: &Plugin) -> impl Iterator<Item = &Source> {
    plugin
        .files
        .iter()
        .filter(|source| PAGE.contains(&extension(&source.path)) && !is_test(&source.path))
}

fn python(plugin: &Plugin) -> impl Iterator<Item = &Source> {
    plugin
        .files
        .iter()
        .filter(|source| extension(&source.path) == "py" && !is_test(&source.path))
}

/// The line, from 1, that a byte of `text` is on.
fn line_at(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

fn failure(rule: &'static str, file: &str, line: usize, found: String, instead: &str) -> Failure {
    Failure {
        rule,
        file: file.to_string(),
        line,
        found,
        instead: instead.to_string(),
    }
}

// ── Comments ─────────────────────────────────────────────────────────────

/// The text with its comments blanked and every line kept where it was: a
/// colour in a comment is on no page, and a line number still points at its
/// line. Strings are kept, since that is where a page is, in Python.
fn uncommented(source: &Source) -> String {
    match extension(&source.path) {
        "py" | "pyi" => python_uncommented(&source.text),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => script_uncommented(&source.text),
        "css" => blanked(&source.text, "/*", "*/"),
        "html" | "htm" | "svg" | "vue" | "svelte" | "jinja" | "jinja2" | "j2" => {
            blanked(&source.text, "<!--", "-->")
        }
        _ => source.text.clone(),
    }
}

/// Python, with `#` comments removed; strings, triple-quoted ones across
/// lines included, kept whole.
fn python_uncommented(text: &str) -> String {
    python_scrubbed(text, false)
}

/// Python's code alone: comments removed and every string's contents
/// blanked, its quotes and line breaks kept, so a word in a docstring is not
/// read as code and a line number still points at its line.
fn python_code(text: &str) -> String {
    python_scrubbed(text, true)
}

fn python_scrubbed(text: &str, blank_strings: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<(char, bool)> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            None => {
                if c == '#' {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                if c == '"' || c == '\'' {
                    let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
                    let width = if triple { 3 } else { 1 };
                    out.extend(std::iter::repeat_n(c, width));
                    i += width;
                    quote = Some((c, triple));
                    continue;
                }
                out.push(c);
                i += 1;
            }
            Some((q, triple)) => {
                let kept = |c: char| if blank_strings && c != '\n' { ' ' } else { c };
                if c == '\\' && i + 1 < chars.len() {
                    out.push(kept(c));
                    out.push(kept(chars[i + 1]));
                    i += 2;
                    continue;
                }
                if c == '\n' && !triple {
                    // An unclosed string ends with its line, as Python says.
                    quote = None;
                } else if c == q
                    && (!triple || (chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q)))
                {
                    let width = if triple { 3 } else { 1 };
                    out.extend(std::iter::repeat_n(q, width));
                    i += width;
                    quote = None;
                    continue;
                }
                out.push(kept(c));
                i += 1;
            }
        }
    }
    out
}

/// Script, with `//` and `/* */` comments removed and strings kept.
fn script_uncommented(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match quote {
            None if c == '/' && next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            None if c == '/' && next == Some('*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                i += 2;
            }
            None => {
                if matches!(c, '"' | '\'' | '`') {
                    quote = Some(c);
                }
                out.push(c);
                i += 1;
            }
            Some(q) => {
                if c == '\\' && next.is_some() {
                    out.push(c);
                    out.extend(next);
                    i += 2;
                    continue;
                }
                if c == q || (c == '\n' && q != '`') {
                    quote = None;
                }
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Everything from `open` to `close` blanked, its line breaks kept.
fn blanked(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after = &rest[start + open.len()..];
        let (inside, remaining) = match after.find(close) {
            Some(end) => (&after[..end], &after[end + close.len()..]),
            None => (after, ""),
        };
        out.extend(std::iter::repeat_n('\n', inside.matches('\n').count()));
        rest = remaining;
    }
    out.push_str(rest);
    out
}

/// Python's logical lines -- physical ones joined while a bracket or a
/// triple-quoted string is open, or a line ends in `\` -- each with the line
/// it starts on. From text already without comments.
fn python_statements(text: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    let (mut start, mut line, mut depth) = (1, 1, 0i32);
    let mut quote: Option<(char, bool)> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
        }
        match quote {
            None => {
                if c == '"' || c == '\'' {
                    let triple = chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&c);
                    let width = if triple { 3 } else { 1 };
                    current.extend(std::iter::repeat_n(c, width));
                    i += width;
                    quote = Some((c, triple));
                    continue;
                }
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth = (depth - 1).max(0),
                    '\n' if depth == 0 && !current.trim_end().ends_with('\\') => {
                        if !current.trim().is_empty() {
                            statements.push((start, std::mem::take(&mut current)));
                        }
                        current.clear();
                        start = line;
                        i += 1;
                        continue;
                    }
                    _ => {}
                }
                current.push(c);
                i += 1;
            }
            Some((q, triple)) => {
                if c == '\\' && i + 1 < chars.len() {
                    current.push(c);
                    current.push(chars[i + 1]);
                    if chars[i + 1] == '\n' {
                        line += 1;
                    }
                    i += 2;
                    continue;
                }
                if c == '\n' && !triple {
                    quote = None;
                } else if c == q
                    && (!triple || (chars.get(i + 1) == Some(&q) && chars.get(i + 2) == Some(&q)))
                {
                    let width = if triple { 3 } else { 1 };
                    current.extend(std::iter::repeat_n(q, width));
                    i += width;
                    quote = None;
                    continue;
                }
                current.push(c);
                i += 1;
            }
        }
    }
    if !current.trim().is_empty() {
        statements.push((start, current));
    }
    statements
}

// ── template-shape ───────────────────────────────────────────────────────

const PYPROJECT: &str = "the template's pyproject.toml: [project] with the plugin's name and \
    version, its dependencies pinning the SDK exactly (open-meridian==<version>), and \
    [tool.meridian]. `meridian plugin new <name> --into <somewhere>` writes one to copy from";

/// What `plugin new` writes, held where a plugin can drift from it without
/// any other rule noticing: the SDK pinned and built on, the package where
/// its scripts expect it, the connection through the SDK, and the files that
/// keep every coding agent on the same instructions.
fn template_shape(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "template-shape";
    let mut failures = Vec::new();
    let mut name = None;
    let mut pinned = None;

    match plugin.file("pyproject.toml") {
        None => failures.push(failure(
            ID,
            "pyproject.toml",
            0,
            "there is no pyproject.toml".into(),
            PYPROJECT,
        )),
        Some(source) => match source.text.parse::<toml::Table>() {
            // tool-meridian says where it does not read.
            Err(_) => {}
            Ok(table) => {
                let project = table.get("project").and_then(|p| p.as_table());
                let text = |key: &str| {
                    project
                        .and_then(|p| p.get(key))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                };
                match text("name") {
                    Some(given) if crate::catalogue::is_name(&given) => name = Some(given),
                    Some(given) => failures.push(failure(
                        ID,
                        "pyproject.toml",
                        line_of(&source.text, "name"),
                        format!("`{given}` is not a plugin's name"),
                        "lowercase letters, digits and single hyphens, a letter first: it is \
                         the package's name, its image's, and with hyphens as underscores the \
                         module it imports as",
                    )),
                    None => failures.push(failure(
                        ID,
                        "pyproject.toml",
                        0,
                        "[project] names no plugin".into(),
                        PYPROJECT,
                    )),
                }
                if text("version").is_none() {
                    failures.push(failure(
                        ID,
                        "pyproject.toml",
                        0,
                        "[project] has no version".into(),
                        "`version = \"0.1.0\"` under [project]: a version is what is \
                         uploaded, and is never replaced",
                    ));
                }
                let dependencies: Vec<&str> = project
                    .and_then(|p| p.get("dependencies"))
                    .and_then(|d| d.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d.as_str())
                    .collect();
                let sdk = dependencies.iter().find(|d| {
                    let d = d.trim_start().replace('_', "-").to_ascii_lowercase();
                    d.starts_with("open-meridian")
                        && !d["open-meridian".len()..].starts_with(|c: char| c.is_alphanumeric())
                });
                match sdk {
                    None => failures.push(failure(
                        ID,
                        "pyproject.toml",
                        line_of(&source.text, "dependencies"),
                        "the SDK is not among its dependencies".into(),
                        "`dependencies = [\"open-meridian==<version>\"]`: the SDK, pinned \
                         exactly, is how a plugin reaches its sidecar",
                    )),
                    Some(sdk) => match sdk.split_once("==") {
                        Some((_, version))
                            if !version.trim().is_empty()
                                && !version.contains([',', '*', ';', '<', '>', '=']) =>
                        {
                            pinned = Some(version.trim().to_string())
                        }
                        _ => failures.push(failure(
                            ID,
                            "pyproject.toml",
                            line_of(&source.text, sdk),
                            format!("`{sdk}` does not pin the SDK exactly"),
                            "`open-meridian==<version>`: the sidecar a plugin runs beside speaks \
                             one version of the contract, and a range lets a rebuild pick up \
                             another. The Dockerfile's base is the same version",
                        )),
                    },
                }
            }
        },
    }

    static BASE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"ghcr\.io/open-meridian/plugin-python:([A-Za-z0-9._-]+)").unwrap()
    });
    match plugin.file("Dockerfile") {
        None => failures.push(failure(
            ID,
            "Dockerfile",
            0,
            "there is no Dockerfile".into(),
            "the template's Dockerfile, built on the SDK's base image \
             (ARG BASE=ghcr.io/open-meridian/plugin-python:<version>, FROM ${BASE}): \
             `meridian plugin upload` builds the plugin's image from it",
        )),
        Some(source) => match BASE.captures(&source.text) {
            None => failures.push(failure(
                ID,
                "Dockerfile",
                line_of(&source.text, "FROM"),
                "the image is not built on the SDK's base image".into(),
                "`ARG BASE=ghcr.io/open-meridian/plugin-python:<the version pyproject.toml \
                 pins>` and `FROM ${BASE}`: Python and the SDK already installed, so an \
                 upload sends only the plugin's own layers",
            )),
            Some(base) => {
                if let Some(pinned) = pinned.as_deref().filter(|pinned| *pinned != &base[1]) {
                    failures.push(failure(
                        ID,
                        "Dockerfile",
                        line_at(&source.text, base.get(0).map(|m| m.start()).unwrap_or(0)),
                        format!(
                            "its base is plugin-python:{}, and pyproject.toml pins \
                             open-meridian=={pinned}",
                            &base[1]
                        ),
                        "move the two together: the base image is the SDK's release, the \
                         same version pyproject.toml pins",
                    ));
                }
            }
        },
    }

    if let Some(name) = &name {
        let main = format!("src/{}/__main__.py", name.replace('-', "_"));
        if !plugin.has(&main) {
            failures.push(failure(
                ID,
                &main,
                0,
                format!("there is no {main}"),
                "the template's layout: the package under src/, named after the plugin with \
                 hyphens as underscores, with __init__.py and __main__.py, which \
                 [project.scripts] runs",
            ));
        }
    }

    static CONNECTS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\bmeridian\.connect\s*\(|\bfrom\s+meridian\s+import\s[^\n]*\bconnect\b")
            .unwrap()
    });
    if !python(plugin).any(|source| CONNECTS.is_match(&python_uncommented(&source.text))) {
        let main = name
            .as_ref()
            .map(|name| format!("src/{}/__main__.py", name.replace('-', "_")))
            .unwrap_or_else(|| "src/".into());
        failures.push(failure(
            ID,
            &main,
            0,
            "nothing calls meridian.connect()".into(),
            "connect through the SDK, as the template's __main__.py does: `async with await \
             meridian.connect(interface=..., settings=...) as plugin:`, and do everything \
             through the plugin it returns",
        ));
    }

    if !plugin.has("AGENTS.md") {
        failures.push(failure(
            ID,
            "AGENTS.md",
            0,
            "there is no AGENTS.md".into(),
            "the template's AGENTS.md, the instructions every coding agent reads, committed \
             with the plugin (decisions/025). `meridian plugin new <name> --into <somewhere>` \
             writes one to start from; keep what is this plugin's own in it",
        ));
    }
    if let Some(claude) = plugin.file("CLAUDE.md") {
        if !claude.text.contains("@AGENTS.md") {
            failures.push(failure(
                ID,
                "CLAUDE.md",
                1,
                "CLAUDE.md does not lead to AGENTS.md".into(),
                "begin it with `@AGENTS.md`, and move what it says of its own into AGENTS.md: \
                 an agent's own file points to AGENTS.md and adds nothing of its own \
                 (decisions/025)",
            ));
        }
    }
    match plugin.file(".dockerignore") {
        None => failures.push(failure(
            ID,
            ".dockerignore",
            0,
            "there is no .dockerignore".into(),
            "the template's .dockerignore: .git, the build's leavings, AGENTS.md, .claude, \
             CLAUDE.md and .meridian, in neither the image nor what `plugin dev` sends",
        )),
        Some(source) => {
            let named: Vec<&str> = source
                .text
                .lines()
                .map(|line| line.trim().trim_start_matches('/').trim_end_matches('/'))
                .collect();
            for needed in ["AGENTS.md", ".meridian"] {
                if !named.contains(&needed) {
                    failures.push(failure(
                        ID,
                        ".dockerignore",
                        0,
                        format!(".dockerignore does not name {needed}"),
                        "name AGENTS.md, .claude, CLAUDE.md and .meridian, as the template's \
                         does: the agents' files and what `plugin dev` prints go in neither \
                         the image nor what `plugin dev` sends",
                    ));
                }
            }
        }
    }
    failures
}

/// The first line holding `needle`, or 0 when none does.
fn line_of(text: &str, needle: &str) -> usize {
    text.find(needle)
        .map(|offset| line_at(text, offset))
        .unwrap_or(0)
}

// ── tool-meridian ────────────────────────────────────────────────────────

const TOOL_MERIDIAN: &str = "[tool.meridian] with `roles = [...]`, from the deployment's fixed \
    list, `interface = true` when it serves a page, as the template's pyproject.toml has it, and \
    `declaration = \"<module>:<attribute>\"` naming its meridian.Declaration (contract v11)";

fn tool_meridian(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "tool-meridian";
    let Some(source) = plugin.file("pyproject.toml") else {
        return Vec::new(); // template-shape says so
    };
    let text = &source.text;
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(failed) => {
            let line = failed
                .span()
                .map(|span| line_at(text, span.start))
                .unwrap_or(0);
            return vec![failure(
                ID,
                "pyproject.toml",
                line,
                format!("pyproject.toml does not read: {}", failed.message()),
                "valid TOML: `meridian plugin upload` reads what the plugin declares from it",
            )];
        }
    };
    let Some(section) = table
        .get("tool")
        .and_then(|t| t.get("meridian"))
        .and_then(|m| m.as_table())
    else {
        return vec![failure(
            ID,
            "pyproject.toml",
            0,
            "pyproject.toml declares no [tool.meridian]".into(),
            TOOL_MERIDIAN,
        )];
    };
    // The line a key is on, within the section.
    let header = text.find("[tool.meridian]").unwrap_or(0);
    let key_line = |key: &str| {
        let pattern = Regex::new(&format!(r"(?m)^[ \t]*{key}[ \t]*=")).unwrap();
        pattern
            .find_at(text, header)
            .map(|found| line_at(text, found.start()))
            .unwrap_or_else(|| line_at(text, header))
    };

    let mut failures = Vec::new();
    if section.contains_key("tags") {
        failures.push(failure(
            ID,
            "pyproject.toml",
            key_line("tags"),
            "[tool.meridian] declares `tags`".into(),
            "remove the `tags` line: a plugin declares none. A person's access to a plugin is \
             read, write or admin, the same for every plugin, granted in the deployment's \
             access groups (decisions/026, decisions/027); `meridian plugin upload` refuses it",
        ));
    }
    let fixed = ROLES.join(", ");
    match section.get("roles") {
        None => {}
        Some(toml::Value::Array(roles)) => {
            for role in roles {
                match role.as_str() {
                    Some(role) if ROLES.contains(&role) => {}
                    Some(role) => failures.push(failure(
                        ID,
                        "pyproject.toml",
                        key_line("roles"),
                        format!("`{role}` is not one of the deployment's roles"),
                        &format!(
                            "roles from the fixed list: {fixed}. What the plugin may publish \
                             and subscribe to is its roles' (decisions/020); none is a plugin \
                             with no topics"
                        ),
                    )),
                    None => failures.push(failure(
                        ID,
                        "pyproject.toml",
                        key_line("roles"),
                        format!("`{role}` in roles is not a role's name"),
                        &format!("names, as strings, from the fixed list: {fixed}"),
                    )),
                }
            }
        }
        Some(_) => failures.push(failure(
            ID,
            "pyproject.toml",
            key_line("roles"),
            "`roles` is not a list".into(),
            &format!("`roles = [\"custody\"]`, or `roles = []`: names from {fixed}"),
        )),
    }
    if let Some(declaration) = section.get("declaration") {
        if !declaration.is_str() {
            failures.push(failure(
                ID,
                "pyproject.toml",
                key_line("declaration"),
                "`declaration` is not text".into(),
                "`declaration = \"<module>:<attribute>\"`, naming the plugin's \
                 meridian.Declaration, which `meridian plugin upload` reads from the built image",
            ));
        }
    }
    if let Some(interface) = section.get("interface") {
        if !interface.is_bool() {
            failures.push(failure(
                ID,
                "pyproject.toml",
                key_line("interface"),
                "`interface` is not true or false".into(),
                "`interface = true` when the plugin serves a page through its sidecar, and \
                 `false` or nothing when it does not",
            ));
        }
    }
    failures
}

// ── What pyproject.toml declares, for the rules below ───────────────────

/// The roles `[tool.meridian]` declares, and the declaration it names
/// (`declaration = "module:attribute"`, contract v11); nothing where it does
/// not read, which tool-meridian says.
fn declared(plugin: &Plugin) -> (Vec<String>, Option<String>) {
    let Some(table) = plugin
        .file("pyproject.toml")
        .and_then(|source| source.text.parse::<toml::Table>().ok())
    else {
        return (Vec::new(), None);
    };
    let section = table.get("tool").and_then(|t| t.get("meridian"));
    let roles = section
        .and_then(|m| m.get("roles"))
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().map(String::from))
        .collect();
    let declaration = section
        .and_then(|m| m.get("declaration"))
        .and_then(|d| d.as_str())
        .map(String::from);
    (roles, declaration)
}

/// The source file a `module:attribute` names, as the plugin's layout holds it.
fn module_file<'a>(plugin: &'a Plugin, named: &str) -> Option<&'a Source> {
    let module = named
        .split(':')
        .next()
        .unwrap_or_default()
        .replace('.', "/");
    [
        format!("src/{module}.py"),
        format!("src/{module}/__init__.py"),
        format!("{module}.py"),
        format!("{module}/__init__.py"),
    ]
    .iter()
    .find_map(|path| plugin.file(path))
}

// ── edge-storage ─────────────────────────────────────────────────────────

fn edge_storage(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "edge-storage";
    static STORAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bStorage\s*\(").unwrap());
    let (roles, declaration) = declared(plugin);
    let Some(named) = declaration else {
        return Vec::new();
    };
    if !named.contains(':') {
        return vec![failure(
            ID,
            "pyproject.toml",
            0,
            format!("`declaration = \"{named}\"` names no attribute"),
            "`declaration = \"<module>:<attribute>\"`, the module and the meridian.Declaration in \
             it, which `meridian plugin upload` reads from the built image",
        )];
    }
    let Some(source) = module_file(plugin, &named) else {
        return vec![failure(
            ID,
            "pyproject.toml",
            0,
            format!("`declaration = \"{named}\"` names a module this plugin does not have"),
            "the module, under src/, that holds the plugin's meridian.Declaration",
        )];
    };
    if roles.iter().any(|role| EDGE_ROLES.contains(&role.as_str())) {
        return Vec::new();
    }
    STORAGE
        .find(&source.text)
        .map(|found| {
            vec![failure(
                ID,
                &source.path,
                line_at(&source.text, found.start()),
                "the declaration asks for storage, and the plugin holds no edge role".into(),
                &format!(
                    "no Storage(...) in the declaration: only a plugin holding an edge role \
                     ({}) owns storage for its raw records, and the deployment refuses it to \
                     any other (decisions/028)",
                    EDGE_ROLES.join(", ")
                ),
            )]
        })
        .unwrap_or_default()
}

// ── role-suite ───────────────────────────────────────────────────────────

/// meridian-schema's boundaries/suites.json at a6a0af7 (contract v24), byte
/// for byte: each role's suite and its cases, every role's v24 cases on a
/// replacement and the template catalogue among them. role-suite reads a
/// case's name and its markers alone, never what it expects, so an
/// expectation written with the format's `any_of` (contract v23: each way's
/// fields, in place of `fields`) is read as any other; matching what a plugin
/// sent to it is the SDK's runner's, which the suite's test runs.
const SUITES_JSON: &str = include_str!("suites.json");

/// A role's suite's cases, each with whether a plugin may name it not
/// presented, with why: a case asserting a closed list's value its source
/// never presents (`closed_list`), from contract v19 one about a kind of data
/// the plugin never publishes (`about`; from v22 also a kind of work it never
/// does: calendars, settlement dates, locates, compliance waits; from v23
/// busts, value-date rolls, currencies not stated and disputed outcomes among
/// them), and from v20 one about a field its source never states
/// (`about_field`; from v22 a fill's place of execution). Every other case -- the data suite's
/// `reads-its-datasets-licences` among them -- every plugin of the role passes.
pub(crate) fn suite_cases(role: &str) -> Vec<(String, bool)> {
    let published: serde_json::Value =
        serde_json::from_str(SUITES_JSON).expect("suites.json reads");
    published["suites"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|suite| suite["role"] == role)
        .flat_map(|suite| suite["cases"].as_array().into_iter().flatten())
        .filter_map(|case| {
            let name = case["name"].as_str()?;
            let optional = ["closed_list", "about", "about_field"]
                .iter()
                .any(|key| case[*key].as_str().is_some_and(|value| !value.is_empty()));
            Some((name.to_string(), optional))
        })
        .collect()
}

const NOT_PRESENTED: &str = "name in `not_presented` only a case its source never presents, \
    with why: one asserting a closed list's value, one about a kind of data the plugin never \
    publishes or a kind of work it never does (`about`, contract v19 and v22), or one about a field its source never states \
    (`about_field`, contract v20). Every other case every plugin of the role passes -- the data \
    suite's reads-its-datasets-licences among them -- and the SDK's runner refuses it named";

/// The cases a test names in a suite's `not_presented`, as string keys of a
/// dict written there or bound at the top of the test's module, each with
/// its line. A dict built some other way (a comprehension over the suite's
/// cases, as the data template's) is the SDK's runner's to hold.
fn named_not_presented(text: &str) -> Vec<(usize, String)> {
    static GIVEN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bnot_presented\s*=\s*").unwrap());
    static KEY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"["']([a-z0-9]+(?:-[a-z0-9]+)*)["']\s*:"#).unwrap());
    let text = python_uncommented(text);
    let mut named = Vec::new();
    for given in GIVEN.find_iter(&text) {
        let rest = &text[given.end()..];
        let at = if rest.starts_with('{') {
            Some(given.end() + 1)
        } else {
            let value = rest
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .next()
                .unwrap_or_default();
            if is_identifier(value) {
                Regex::new(&format!(
                    r"(?m)^{}[ \t]*(?::[^=\n]*)?=[ \t]*\{{",
                    regex::escape(value)
                ))
                .unwrap()
                .find(&text)
                .map(|bound| bound.end())
            } else {
                None
            }
        };
        let Some(at) = at else {
            continue;
        };
        let dict = enclosed(&text[at..]);
        for key in KEY.captures_iter(dict) {
            let place = at + key.get(0).expect("matched").start();
            let entry = (line_at(&text, place), key[1].to_string());
            if !named.contains(&entry) {
                named.push(entry);
            }
        }
    }
    named
}

const EXCUSED: &str = "no xfail or skip where a role's suite is run: no marker, no \
    `pytest.xfail()` or `pytest.skip()`, no `pytestmark`, in the test running it, its module, \
    or a conftest.py over it. Only the contract's own markers excuse a case (`closed_list`, \
    `about`, `about_field`), named in the run's `not_presented` with why; every other case \
    passes, and a test skipped or expected to fail passes `--run-tests` whatever its cases do";

/// What excuses a pytest test from failing: a skip or an expected failure,
/// as pytest's marks and calls and unittest's decorators and calls say it.
static EXCUSES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(xfail|skipif|skipIf|skipUnless|importorskip|expectedFailure|skipTest|SkipTest)\b|\.\s*(skip)\b|\b(skip)\s*\(",
    )
    .unwrap()
});

/// Each excuse in a module's code -- its comments and strings aside -- with
/// its line and the word that excuses.
fn excuses(text: &str) -> Vec<(usize, String)> {
    let code = python_code(text);
    EXCUSES
        .captures_iter(&code)
        .filter_map(|found| {
            let word = (1..=3).find_map(|group| found.get(group))?;
            Some((line_at(&code, word.start()), word.as_str().to_string()))
        })
        .collect()
}

/// A test pytest collects: a `test_*.py` or `*_test.py`.
fn collected(path: &str) -> bool {
    let name = file_name(path);
    (name.starts_with("test_") && name.ends_with(".py")) || name.ends_with("_test.py")
}

/// The conftest.py files whose hooks and fixtures reach a test module: in
/// its directory, or one above it.
fn conftests_over<'a>(plugin: &'a Plugin, module: &str) -> impl Iterator<Item = &'a Source> {
    let dir = module
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("")
        .to_string();
    plugin.files.iter().filter(move |source| {
        file_name(&source.path) == "conftest.py" && {
            let over = source
                .path
                .rsplit_once('/')
                .map(|(dir, _)| dir)
                .unwrap_or("");
            over.is_empty() || dir == over || dir.starts_with(&format!("{over}/"))
        }
    })
}

/// Whether a test pytest collects -- a `test*` function at the top of the
/// module, or a method of a `Test*` class there -- binds `role`'s run and
/// asserts it passed in the statement after, both in the test's own body
/// (under no `try`, `with`, `if` or loop): the SDK's idiom,
/// `report = run("<role>", producers, ...)` then
/// `assert report.passed, report.failures`. `passed` is every case passing,
/// those named not presented aside; the report's other fields let a test
/// assert less.
fn asserts_passed(text: &str, role: &str) -> bool {
    static DEF: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:async\s+)?def\s+(\w+)").unwrap());
    static CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^class\s+(\w+)").unwrap());
    let binds = Regex::new(&format!(
        r#"^([A-Za-z_]\w*)\s*(?::[^=]*)?=\s*(?:await\s+)?(?:[A-Za-z_][\w.]*\.)?run(?:_async)?\(\s*["']{}["']"#,
        regex::escape(role)
    ))
    .unwrap();
    enum Scope {
        Class { test: bool },
        Def { test: bool, body: Option<usize> },
    }
    let statements = python_statements(&python_uncommented(text));
    let indent_of = |statement: &str| statement.len() - statement.trim_start().len();
    let mut scopes: Vec<(usize, Scope)> = Vec::new();
    for (at, (_, statement)) in statements.iter().enumerate() {
        let indent = indent_of(statement);
        let code = statement.trim();
        while scopes.last().is_some_and(|(open, _)| indent <= *open) {
            scopes.pop();
        }
        if let Some((
            _,
            Scope::Def {
                body: body @ None, ..
            },
        )) = scopes.last_mut()
        {
            *body = Some(indent);
        }
        if let Some(def) = DEF.captures(code) {
            let at_top = match scopes.as_slice() {
                [] => true,
                [(_, Scope::Class { test })] => *test,
                _ => false,
            };
            let test = at_top && def[1].starts_with("test");
            scopes.push((indent, Scope::Def { test, body: None }));
            continue;
        }
        if let Some(class) = CLASS.captures(code) {
            let test = scopes.is_empty() && class[1].starts_with("Test");
            scopes.push((indent, Scope::Class { test }));
            continue;
        }
        let Some((
            _,
            Scope::Def {
                test: true,
                body: Some(body),
            },
        )) = scopes.last()
        else {
            continue;
        };
        let Some(bound) = binds.captures(code).filter(|_| indent == *body) else {
            continue;
        };
        let asserted = Regex::new(&format!(r"^assert\s+{}\.passed\s*(?:,|$)", &bound[1])).unwrap();
        if statements
            .get(at + 1)
            .is_some_and(|(_, next)| indent_of(next) == indent && asserted.is_match(next.trim()))
        {
            return true;
        }
    }
    false
}

fn role_suite(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "role-suite";
    let (roles, _) = declared(plugin);
    let mut failures = Vec::new();
    let mut excused: Vec<(String, usize)> = Vec::new();
    for role in roles.iter().filter(|role| SUITES.contains(&role.as_str())) {
        let runs = Regex::new(&format!(
            r#"\brun(?:_async)?\(\s*["']{}["']"#,
            regex::escape(role)
        ))
        .unwrap();
        let tests: Vec<&Source> = plugin
            .files
            .iter()
            .filter(|source| {
                let name = file_name(&source.path);
                name.ends_with(".py")
                    && is_test(&source.path)
                    && source.text.contains("meridian.suites")
                    && runs.is_match(&source.text)
            })
            .collect();
        if tests.is_empty() {
            failures.push(failure(
                ID,
                "tests/",
                0,
                format!("it holds `{role}`, and no test runs the {role} suite"),
                &format!(
                    "a test that runs `meridian.suites.run(\"{role}\", producers)`: each case of \
                     the suite mapped to the plugin's own exchange with its source, run through \
                     its own conversion, and the report asserted passed. `--run-tests` then \
                     holds the plugin to every case, as a verified plugin is held \
                     (spec/vendor-differences-have-a-place-in-the-contract, requirement 18)"
                ),
            ));
            continue;
        }
        let cases = suite_cases(role);
        for source in &tests {
            let over = conftests_over(plugin, &source.path);
            for excusing in std::iter::once(*source).chain(over) {
                for (line, word) in excuses(&excusing.text) {
                    if excused.contains(&(excusing.path.clone(), line)) {
                        continue;
                    }
                    excused.push((excusing.path.clone(), line));
                    let place = if excusing.path == source.path {
                        "in a module that runs".to_string()
                    } else {
                        format!("in a conftest.py over {}, which runs", source.path)
                    };
                    failures.push(failure(
                        ID,
                        &excusing.path,
                        line,
                        format!(
                            "`{word}` {place} the {role} suite: a test skipped or expected to \
                             fail there passes whatever the suite's cases do"
                        ),
                        EXCUSED,
                    ));
                }
            }
            for (line, case) in named_not_presented(&source.text) {
                let found = match cases.iter().find(|(name, _)| *name == case) {
                    None => format!(
                        "it names `{case}` not presented, and the {role} suite has no such case"
                    ),
                    Some((_, true)) => continue,
                    Some((_, false)) => format!(
                        "it names `{case}` not presented, a case of the {role} suite every plugin \
                         of the role passes"
                    ),
                };
                failures.push(failure(ID, &source.path, line, found, NOT_PRESENTED));
            }
        }
        let asserted = tests
            .iter()
            .any(|source| collected(&source.path) && asserts_passed(&source.text, role));
        if !asserted {
            let first = tests[0];
            let line = runs
                .find(&first.text)
                .map(|run| line_at(&first.text, run.start()))
                .unwrap_or(0);
            failures.push(failure(
                ID,
                &first.path,
                line,
                format!("it runs the {role} suite, and no test asserts the report passed"),
                &format!(
                    "a test pytest collects (a `test_` function in a test_*.py) binding the run \
                     and asserting it passed in the very next statement, as the SDK's templates \
                     do: `report = run(\"{role}\", producers, ...)` then `assert report.passed, \
                     report.failures`, under no try, with, if or loop. `passed` is every case \
                     passing, those named not presented aside; a test asserting anything else -- \
                     a count, one case, nothing -- or catching its failure holds the plugin to \
                     less than its role's suite"
                ),
            ));
        }
    }
    failures
}

// ── role-pages (spec/plugin-pages-follow-their-role) ─────────────────────

/// One of a role's common pages: its title, its path where the role fixes
/// it, the levels it serves, its read tool's name, the record that tool
/// answers (`meridian.roles.<role>`'s `<Page>Read`) and that record's common
/// fields, which a plugin's own record holds where it does not extend it.
struct CommonPage {
    title: &'static str,
    path: Option<&'static str>,
    levels: &'static [&'static str],
    tool: &'static str,
    record: &'static str,
    fields: &'static [&'static str],
}

/// A role's common part as `role-pages` holds it: its pages in the role's
/// order, the tools of its acts, and the settings every plugin of it
/// declares. Only what is common: an item the SDK's template offers from the
/// one plugin of a role there is stays a candidate until a second needs it,
/// unless a duty of the role names it (the spec's Q4): custody's links and
/// statements, operations' breaks, reconciliations and opening balances,
/// reporting's valuation, in a currency.
struct CommonPart {
    role: &'static str,
    pages: &'static [CommonPage],
    /// The candidates' page titles: the role's, not yet held, and no page of
    /// the plugin's own when the role's order is checked.
    candidates: &'static [&'static str],
    /// The title of the page for the raw records it keeps, where it keeps
    /// any (a duty of the role).
    kept: Option<&'static str>,
    acts: &'static [&'static str],
    settings: &'static [&'static str],
}

const MANAGE: &[&str] = &["admin"];
const OPEN_AND_VIEW: &[&str] = &["write", "read"];

const COMMON: [CommonPart; 4] = [
    CommonPart {
        role: "data",
        pages: &[
            CommonPage {
                title: "Connection",
                path: Some("/"),
                levels: MANAGE,
                tool: "read_connection",
                record: "ConnectionRead",
                fields: &[
                    "mode",
                    "credentials",
                    "entitled_at_vendor",
                    "limit",
                    "calls",
                    "rate_limited",
                    "last_call_at",
                    "last_error",
                    "sessions",
                ],
            },
            CommonPage {
                title: "Datasets",
                path: Some("/datasets"),
                levels: MANAGE,
                tool: "read_datasets",
                record: "DatasetsRead",
                fields: &["served", "wants", "reads_instruments"],
            },
        ],
        candidates: &[],
        kept: None,
        acts: &[],
        settings: &[],
    },
    CommonPart {
        role: "custody",
        pages: &[
            CommonPage {
                title: "Account links",
                path: None,
                levels: MANAGE,
                tool: "read_account_links",
                record: "AccountLinksRead",
                fields: &["external_accounts", "accounts", "links"],
            },
            CommonPage {
                title: "Statements",
                path: None,
                levels: OPEN_AND_VIEW,
                tool: "read_statements",
                record: "StatementsRead",
                fields: &["statements"],
            },
        ],
        candidates: &["Connections", "History", "Raw records"],
        kept: Some("Raw records"),
        acts: &["link_account"],
        settings: &[],
    },
    CommonPart {
        role: "operations",
        pages: &[
            CommonPage {
                title: "Breaks",
                path: None,
                levels: OPEN_AND_VIEW,
                tool: "read_breaks",
                record: "BreaksRead",
                fields: &["open_breaks", "overdue_breaks"],
            },
            CommonPage {
                title: "Reconciliations",
                path: None,
                levels: OPEN_AND_VIEW,
                tool: "read_reconciliations",
                record: "ReconciliationsRead",
                fields: &["last_reconciliation"],
            },
            CommonPage {
                title: "Opening balances",
                path: None,
                levels: OPEN_AND_VIEW,
                tool: "read_opening_balances",
                record: "OpeningBalancesRead",
                fields: &["awaiting_an_opening_balance"],
            },
        ],
        candidates: &[],
        kept: None,
        acts: &["confirm_opening_balance"],
        settings: &[],
    },
    CommonPart {
        role: "reporting",
        pages: &[],
        candidates: &["Datasets"],
        kept: None,
        acts: &[],
        settings: &["reporting_currency"],
    },
];

/// A call's arguments, split where a comma stands outside every bracket and
/// string, each trimmed.
fn split_arguments(arguments: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (at, c) in arguments.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(arguments[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    parts.push(arguments[start..].trim());
    parts.retain(|part| !part.is_empty());
    parts
}

/// The `n`th positional argument of a call, from 0.
fn positional(arguments: &str, n: usize) -> Option<&str> {
    static KEYWORD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*\s*=[^=]").unwrap());
    split_arguments(arguments)
        .into_iter()
        .filter(|part| !KEYWORD.is_match(part) && !part.starts_with('*'))
        .nth(n)
}

/// A kind of raw record declared: what a plugin at the edge keeps.
static KIND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bRecordKind\s*\(").unwrap());

/// A page the plugin declares, as `role-pages` reads it.
struct PageAt<'a> {
    declaration: Declaration<'a>,
    title: Option<String>,
    path: Option<String>,
    /// None where the text cannot say.
    levels: Option<Vec<String>>,
}

impl PageAt<'_> {
    fn manage(&self) -> Option<bool> {
        self.levels
            .as_ref()
            .map(|levels| levels.iter().any(|level| level == "admin"))
    }
}

/// A setting the plugin declares, in either form the SDK takes -- a
/// `Setting("name", ...)` call, or a field of a class `@meridian.settings`
/// makes (`name: kind = meridian.setting(...)`, `meridian.environment()`,
/// `meridian.sample_data()` or a role's helper) -- in the order its module
/// declares them.
struct SettingAt<'a> {
    source: &'a Source,
    line: usize,
    name: String,
    label: Option<String>,
    /// Declared `secret=True`.
    secret: bool,
}

fn settings_in_order<'a>(
    plugin: &'a Plugin,
    constants: &HashMap<String, String>,
) -> Vec<SettingAt<'a>> {
    static DECORATOR: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*@\s*(?:[A-Za-z_][A-Za-z0-9_]*\.)*settings\s*$").unwrap());
    static CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*class\s+[A-Za-z_]").unwrap());
    static FIELD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)^[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*:[^=]*=[ \t]*(.*)$").unwrap()
    });
    static SETTING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bSetting\s*\(").unwrap());
    static SECRET: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bsecret\s*=\s*True\b").unwrap());
    static HELPER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:[A-Za-z_][A-Za-z0-9_]*\.)*(setting|sample_data|environment)\s*\(").unwrap()
    });
    // A common setting's helper called where a list of settings is given
    // (`settings=[reporting.reporting_currency()]`): the setting it declares
    // is named as the helper.
    static COMMON_HELPER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\b(?:meridian|data|custody|operations|reporting)\.(sample_data|environment|poll_seconds|stale_after_hours|reconcile|due_business_days|street_fresh_within_hours|reporting_currency)\s*\(",
        )
        .unwrap()
    });
    let mut found = Vec::new();
    for source in python(plugin) {
        let mut in_class: Option<usize> = None;
        let mut decorated = false;
        for (line, statement) in python_statements(&python_uncommented(&source.text)) {
            let indent = statement.len() - statement.trim_start().len();
            if let Some(class) = in_class {
                if indent > class {
                    if let Some(field) = FIELD.captures(&statement) {
                        let value = field[2].trim();
                        let label = HELPER.captures(value).and_then(|helper| match &helper[1] {
                            "sample_data" => Some("Sample data".to_string()),
                            "environment" => Some("Environment".to_string()),
                            _ => {
                                let arguments = enclosed(&value[helper.get(0)?.end()..]);
                                keyword(arguments, "label")
                                    .or_else(|| positional(arguments, 0))
                                    .and_then(|label| string_named(label, constants))
                            }
                        });
                        found.push(SettingAt {
                            source,
                            line,
                            name: field[1].to_string(),
                            label,
                            secret: SECRET.is_match(value),
                        });
                    }
                    continue;
                }
                in_class = None;
            }
            if DECORATOR.is_match(&statement) {
                decorated = true;
                continue;
            }
            if decorated && CLASS.is_match(&statement) {
                in_class = Some(indent);
                decorated = false;
                continue;
            }
            decorated = decorated && statement.trim_start().starts_with('@');
            // A statement may declare several, helpers among `Setting(`s
            // (`_VENDORS = (Setting(...), custody.poll_seconds(), ...)`):
            // each kept where it is written.
            let mut here: Vec<(usize, SettingAt)> = Vec::new();
            for call in COMMON_HELPER.captures_iter(&statement) {
                let whole = call.get(0).expect("matched");
                here.push((
                    whole.start(),
                    SettingAt {
                        source,
                        line: line + statement[..whole.start()].matches('\n').count(),
                        name: call[1].to_string(),
                        label: match &call[1] {
                            "sample_data" => Some("Sample data".to_string()),
                            "environment" => Some("Environment".to_string()),
                            _ => None,
                        },
                        secret: false,
                    },
                ));
            }
            for call in SETTING.find_iter(&statement) {
                let arguments = enclosed(&statement[call.end()..]);
                let named = keyword(arguments, "name").unwrap_or(first_argument(arguments));
                let Some(name) = string_named(named, constants) else {
                    continue;
                };
                here.push((
                    call.start(),
                    SettingAt {
                        source,
                        line: line + statement[..call.start()].matches('\n').count(),
                        name,
                        label: keyword(arguments, "label")
                            .and_then(|label| string_named(label, constants)),
                        secret: SECRET.is_match(arguments),
                    },
                ));
            }
            here.sort_by_key(|(at, _)| *at);
            found.extend(here.into_iter().map(|(_, setting)| setting));
        }
    }
    found
}

/// The record a page's `answers=` names holds a role's common fields: it is
/// the role's record, extends it, or is the plugin's own declaring every
/// common field. Otherwise what is missing, said.
fn holds_fields(plugin: &Plugin, answers: &str, page: &CommonPage) -> Result<(), String> {
    let named = answers.rsplit('.').next().unwrap_or(answers).trim();
    let class = Regex::new(&format!(
        r"(?m)^class[ \t]+{}\b[ \t]*(?:\(([^)]*)\))?[ \t]*:",
        regex::escape(named)
    ))
    .unwrap();
    let extends = Regex::new(&format!(r"\b{}\b", regex::escape(page.record))).unwrap();
    for source in python(plugin) {
        let text = python_uncommented(&source.text);
        let Some(found) = class.captures(&text) else {
            continue;
        };
        if found
            .get(1)
            .is_some_and(|bases| extends.is_match(bases.as_str()))
        {
            return Ok(());
        }
        let body = &text[found.get(0).expect("matched").end()..];
        let declared: Vec<String> = python_statements(body)
            .into_iter()
            .take_while(|(_, statement)| statement.starts_with([' ', '\t']))
            .filter_map(|(_, statement)| {
                let name = statement.trim_start();
                let end = name.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
                name[end..]
                    .trim_start()
                    .starts_with(':')
                    .then(|| name[..end].to_string())
            })
            .collect();
        let missing: Vec<&str> = page
            .fields
            .iter()
            .filter(|field| !declared.iter().any(|declared| declared == *field))
            .copied()
            .collect();
        return if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "its record, {named}, does not hold the role's common fields {}",
                missing.join(", ")
            ))
        };
    }
    if named == page.record {
        Ok(())
    } else {
        Err(format!(
            "it answers {named}, which is neither the role's {} nor a record of the plugin's own",
            page.record
        ))
    }
}

/// Every act a plugin declares -- a route that changes something, offered to
/// agents as a tool -- names its check tool (`checked_by=`), a read tool
/// taking the same record, so an agent may prepare it for a person to make
/// (contract v20, W6.26; the plan's Q5: required of a verified plugin). Where
/// a tool replaces the route (`@pages.tool(replaces=...)`), the tool is what
/// agents are offered, and it is held to it instead: unless it reads
/// (`reads=True`, or a method that changes nothing), or the route it
/// replaces does.
fn acts_checked(declared: &[Declaration], constants: &HashMap<String, String>) -> Vec<Failure> {
    let changes = |method: &str| method != "GET" && method != "HEAD";
    let reads =
        |declaration: &Declaration| keyword(&declaration.arguments, "reads") == Some("True");
    let path_of =
        |declaration: &Declaration| string_named(first_argument(&declaration.arguments), constants);
    let replaced = replaced_paths(declared, constants);
    let mut failures = Vec::new();
    let mut hold = |declaration: &Declaration, act: String| {
        let Some(check) = keyword(&declaration.arguments, "checked_by") else {
            failures.push(failure(
                "role-pages",
                &declaration.source.path,
                declaration.line,
                format!("{act} is an act and names no check tool, so an agent cannot prepare it"),
                "`checked_by=\"<tool>\"`: a read tool (`reads=True`) taking the same record, \
                 answering what the act would do and whether this person may, and `accounts=` \
                 naming the record's paths that hold the accounts it touches, so an agent may \
                 prepare the act for a person to make (contract v20)",
            ));
            return;
        };
        let check = check.trim_matches(['"', '\'']);
        let declared_reading = declared.iter().any(|other| {
            other.kind != "setting"
                && keyword(&other.arguments, "name")
                    .is_some_and(|name| name.trim_matches(['"', '\'']) == check)
                && reads(other)
        });
        if !declared_reading {
            failures.push(failure(
                "role-pages",
                &declaration.source.path,
                declaration.line,
                format!("{act} names the check tool {check}, which no read route declares"),
                "declare the check: a route with `reads=True`, `name=` the tool `checked_by=` \
                 names, taking the same record and answering what the act would do and \
                 whether this person may (contract v20)",
            ));
        }
    };
    for declaration in declared {
        match declaration.kind {
            "route" => {
                let changing = methods_of(&declaration.arguments)
                    .iter()
                    .any(|method| changes(method));
                let kept = keyword(&declaration.arguments, "tool") == Some("False");
                let replaced_by_tool =
                    path_of(declaration).is_some_and(|path| replaced.contains(&path));
                if !changing || reads(declaration) || kept || replaced_by_tool {
                    continue;
                }
                let path = first_argument(&declaration.arguments)
                    .trim()
                    .trim_matches(['"', '\'']);
                hold(declaration, path.to_string());
            }
            "tool" => {
                let Some(replaces) = keyword(&declaration.arguments, "replaces") else {
                    continue;
                };
                let method = keyword(&declaration.arguments, "method")
                    .and_then(|method| string_named(method, constants))
                    .map_or_else(|| "POST".to_string(), |method| method.to_ascii_uppercase());
                let tool_reads = match keyword(&declaration.arguments, "reads") {
                    Some("True") => true,
                    Some("False") => false,
                    _ => !changes(&method),
                };
                let route_reads = string_named(replaces, constants).is_some_and(|path| {
                    declared.iter().any(|route| {
                        route.kind == "route"
                            && path_of(route).as_deref() == Some(path.as_str())
                            && methods_of(&route.arguments).contains(&method)
                            && reads(route)
                    })
                });
                if tool_reads || route_reads {
                    continue;
                }
                let name = match keyword(&declaration.arguments, "name") {
                    Some(name) if name != "None" => string_named(name, constants),
                    _ => declaration.view.clone(),
                };
                let replaces = replaces.trim().trim_matches(['"', '\'']);
                let act = match name {
                    Some(name) => format!("the tool {name} replacing {replaces}"),
                    None => format!("the tool replacing {replaces}"),
                };
                hold(declaration, act);
            }
            _ => {}
        }
    }
    failures
}

fn role_pages(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "role-pages";
    let (held, _) = declared(plugin);
    let declared_all = declarations(plugin);
    let constants = bound_strings(plugin);
    let mut failures = acts_checked(&declared_all, &constants);
    let parts: Vec<&CommonPart> = COMMON
        .iter()
        .filter(|part| held.iter().any(|role| role == part.role))
        .collect();
    if parts.is_empty() {
        return failures;
    }
    let tools: Vec<String> = declared_all
        .iter()
        .filter(|declaration| declaration.kind != "setting")
        .filter_map(|declaration| keyword(&declaration.arguments, "name"))
        .filter_map(|name| string_named(name, &constants))
        .collect();
    let pages: Vec<PageAt> = declared_all
        .into_iter()
        .filter(|declaration| declaration.kind == "page")
        .map(|declaration| {
            let title = keyword(&declaration.arguments, "title")
                .or_else(|| positional(&declaration.arguments, 1))
                .and_then(|title| string_named(title, &constants));
            let path = keyword(&declaration.arguments, "path")
                .or_else(|| positional(&declaration.arguments, 0))
                .and_then(|path| string_named(path, &constants));
            let levels = keyword(&declaration.arguments, "levels")
                .and_then(|value| roles_named(plugin, &declaration, value));
            PageAt {
                declaration,
                title,
                path,
                levels,
            }
        })
        .collect();
    // Where a missing page or setting is said: the plugin's pages' module, or
    // its project.
    let pages_file = pages
        .first()
        .map(|page| page.declaration.source.path.clone())
        .unwrap_or_else(|| "pyproject.toml".into());
    let common_titles: Vec<&str> = parts
        .iter()
        .flat_map(|part| {
            part.pages
                .iter()
                .map(|page| page.title)
                .chain(part.candidates.iter().copied())
        })
        .collect();

    for part in &parts {
        let role = part.role;
        let template =
            format!("the role's common part, as `meridian plugin new --role {role}` writes it");
        // Each common page, by its title: once, where the role says, its tool
        // under the role's name answering the role's record.
        let mut placed: Vec<(usize, &PageAt)> = Vec::new();
        for (order, common) in part.pages.iter().enumerate() {
            let levels = common.levels.join("\", \"");
            let declare = format!(
                "`@pages.page({}\"{}\", levels={}, answers={role}.{}, name=\"{}\")`: {template} \
                 (spec/plugin-pages-follow-their-role)",
                common
                    .path
                    .map(|path| format!("\"{path}\", "))
                    .unwrap_or_else(|| "\"<path>\", ".into()),
                common.title,
                if common.levels.len() == 1 {
                    format!("\"{levels}\"")
                } else {
                    format!("[\"{levels}\"]")
                },
                common.record,
                common.tool,
            );
            let titled: Vec<&PageAt> = pages
                .iter()
                .filter(|page| page.title.as_deref() == Some(common.title))
                .collect();
            let Some(&page) = titled.first() else {
                failures.push(failure(
                    ID,
                    &pages_file,
                    0,
                    format!("it holds `{role}`, and declares no {} page", common.title),
                    &declare,
                ));
                continue;
            };
            for again in titled.iter().skip(1) {
                failures.push(failure(
                    ID,
                    &again.declaration.source.path,
                    again.declaration.line,
                    format!(
                        "a second page titled {}, the {role} role's common page's title",
                        common.title
                    ),
                    "title a page of the plugin's own in its own words: a common page's title is \
                     the role's, so a person finds it where every plugin of the role has it",
                ));
            }
            placed.push((order, page));
            let file = &page.declaration.source.path;
            let line = page.declaration.line;
            let mut wrong = Vec::new();
            if let Some(path) = common
                .path
                .filter(|path| page.path.as_deref() != Some(*path))
            {
                wrong.push(format!("its path is not {path}"));
            }
            if let Some(levels) = &page.levels {
                let mut given: Vec<&str> = levels.iter().map(String::as_str).collect();
                let mut wanted = common.levels.to_vec();
                given.sort_unstable();
                wanted.sort_unstable();
                if given != wanted {
                    wrong.push(format!(
                        "it serves {}, not {}",
                        levels.join(" and "),
                        common.levels.join(" and ")
                    ));
                }
            }
            if held.len() > 1 {
                let serves = keyword(&page.declaration.arguments, "roles")
                    .and_then(|value| roles_named(plugin, &page.declaration, value));
                if serves.is_some_and(|serves| !serves.iter().any(|served| served == role)) {
                    wrong.push(format!("its roles do not name `{role}`"));
                }
            }
            let tool = keyword(&page.declaration.arguments, "name")
                .and_then(|name| string_named(name, &constants));
            if tool.as_deref() != Some(common.tool) {
                wrong.push(format!("its tool is not named {}", common.tool));
            }
            match keyword(&page.declaration.arguments, "answers") {
                None => {
                    wrong.push("it declares no record (`answers=`), so it is no read tool".into())
                }
                Some(answers) => {
                    if let Err(said) = holds_fields(plugin, answers, common) {
                        wrong.push(said);
                    }
                }
            }
            if !wrong.is_empty() {
                failures.push(failure(
                    ID,
                    file,
                    line,
                    format!(
                        "the {role} role's {} page: {}",
                        common.title,
                        wrong.join("; ")
                    ),
                    &declare,
                ));
            }
        }

        // In the role's order, and ahead of the plugin's own pages in the
        // same tab row, where one module declares them.
        for (at, (order, page)) in placed.iter().enumerate() {
            let file = &page.declaration.source.path;
            if let Some((_, before)) = placed[..at].iter().find(|(earlier, other)| {
                earlier < order
                    && &other.declaration.source.path == file
                    && other.declaration.line > page.declaration.line
            }) {
                failures.push(failure(
                    ID,
                    file,
                    page.declaration.line,
                    format!(
                        "the {role} role's {} page comes before its {}",
                        part.pages[*order].title,
                        before.title.as_deref().unwrap_or_default()
                    ),
                    &format!(
                        "declare the role's common pages in the role's order: {}",
                        part.pages
                            .iter()
                            .map(|page| page.title)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        for own in pages.iter().filter(|own| {
            own.manage().is_some()
                && !own
                    .title
                    .as_deref()
                    .is_some_and(|title| common_titles.contains(&title))
        }) {
            // The last of the role's pages in its tab row, in its module.
            let Some((order, _)) = placed
                .iter()
                .filter(|(_, page)| {
                    page.declaration.source.path == own.declaration.source.path
                        && page.manage() == own.manage()
                        && page.declaration.line > own.declaration.line
                })
                .max_by_key(|(_, page)| page.declaration.line)
            else {
                continue;
            };
            failures.push(failure(
                ID,
                &own.declaration.source.path,
                own.declaration.line,
                format!(
                    "a page of the plugin's own, {}, comes before the {role} role's {} page",
                    own.title.as_deref().unwrap_or("untitled"),
                    part.pages[*order].title
                ),
                "declare the plugin's own pages after the role's common pages in the same \
                 tab row: a person finds the role's pages first on every plugin of it",
            ));
        }

        // Where the plugin keeps raw records (a `RecordKind` declared), the
        // role's page for them, under Open and View, titled as the role's.
        if let Some(kept) = part.kept {
            let keeps =
                python(plugin).any(|source| KIND.is_match(&python_uncommented(&source.text)));
            let titled = |title: &str| {
                pages
                    .iter()
                    .find(|page| page.title.as_deref() == Some(title))
            };
            for old in pages
                .iter()
                .filter(|page| page.title.as_deref() == Some("Raw responses"))
            {
                failures.push(failure(
                    ID,
                    &old.declaration.source.path,
                    old.declaration.line,
                    format!("a page titled Raw responses, the {role} role's {kept} page"),
                    &format!(
                        "title it `{kept}`, its path and tools unchanged: the role's page for \
                              the raw records it keeps"
                    ),
                ));
            }
            match titled(kept) {
                None if keeps && titled("Raw responses").is_none() => failures.push(failure(
                    ID,
                    &pages_file,
                    0,
                    format!("it keeps raw records, and declares no {kept} page"),
                    &format!(
                        "`@pages.page(\"<path>\", \"{kept}\", levels=[\"write\", \"read\"])`, \
                         drawn from `meridian/roles/{role}/raw_records.html`: each kind kept, its \
                         window, the archive, and a raw record followed from a row's reference"
                    ),
                )),
                Some(page) if page.manage() == Some(true) => failures.push(failure(
                    ID,
                    &page.declaration.source.path,
                    page.declaration.line,
                    format!("the {role} role's {kept} page is under Manage"),
                    "serve it at `levels=[\"write\", \"read\"]`, under Open and View, beside \
                     the records it is the source of",
                )),
                _ => {}
            }
        }

        for act in part
            .acts
            .iter()
            .filter(|act| !tools.iter().any(|tool| tool == *act))
        {
            failures.push(failure(
                ID,
                &pages_file,
                0,
                format!("it holds `{role}`, and declares no route named {act}"),
                &format!(
                    "the route whose tool is `{act}`, by that name (`name=\"{act}\"`): {template}"
                ),
            ));
        }
    }

    // The settings, in the common order: `environment` first where the
    // vendor has more than one, sample data last, and the common words kept
    // for the common settings.
    let settings = settings_in_order(plugin, &constants);
    for part in &parts {
        for wanted in part
            .settings
            .iter()
            .filter(|wanted| !settings.iter().any(|setting| setting.name == **wanted))
        {
            failures.push(failure(
                ID,
                &pages_file,
                0,
                format!("it holds `{}`, and declares no setting {wanted}", part.role),
                &format!(
                    "the role's common setting, `{wanted}: str = {}.{wanted}()`, as `meridian \
                     plugin new --role {}` declares it",
                    part.role, part.role
                ),
            ));
        }
    }
    for (at, setting) in settings.iter().enumerate() {
        let file = &setting.source.path;
        let same_file = |other: &&SettingAt| &other.source.path == file;
        match setting.name.as_str() {
            "synthetic" => failures.push(failure(
                ID,
                file,
                setting.line,
                "the setting synthetic, which is sample data now".into(),
                "`sample_data: bool = meridian.sample_data()`: one developer setting, accepted \
                 only on development deployments (ruled 2026-10-02)",
            )),
            "environment" if settings[..at].iter().any(|other| same_file(&other)) => failures.push(failure(
                ID,
                file,
                setting.line,
                "the setting environment comes after another".into(),
                "declare `environment: str = meridian.environment(...)` first, before every other \
                 setting, the credentials that differ by it gated on it (the spec's Q12)",
            )),
            "sample_data" if settings[at + 1..].iter().any(|other| same_file(&other)) => {
                failures.push(failure(
                    ID,
                    file,
                    setting.line,
                    "the setting sample_data comes before another".into(),
                    "declare `sample_data: bool = meridian.sample_data()` last, after the \
                     vendor's own settings",
                ))
            }
            _ => {}
        }
        for (common, label) in [
            ("environment", "Environment"),
            ("sample_data", "Sample data"),
        ] {
            if setting.name != common && setting.label.as_deref() == Some(label) {
                failures.push(failure(
                    ID,
                    file,
                    setting.line,
                    format!(
                        "the setting {} is labelled {label}, the common setting {common}'s label",
                        setting.name
                    ),
                    &format!("label it in its own words, or declare it as the common {common}"),
                ));
            }
        }
    }
    failures
}

// ── tools-cover-routes ───────────────────────────────────────────────────

/// What a route that changes something is told to write instead.
const TOOL_ROUTE: &str = "declare its inputs as one typed record, `params=Record` (a frozen \
    dataclass whose fields are named as its form's inputs, by the data dictionary's paths), so the \
    SDK derives its tool for agents on the deployment's MCP surface; answer with pages.answer and \
    refuse with pages.refuse. Or replace the derived tool with @pages.tool(replaces=...), or, \
    where it cannot be offered to an agent, say so: `tool=False, why=\"...\"` (contract v12)";

/// A route the plugin declares with a decorator: the method it is declared
/// with, where, and the decorator's arguments.
struct Declared<'a> {
    source: &'a Source,
    line: usize,
    path: String,
    arguments: String,
}

/// Every `@<pages>.page(...)` and `@<pages>.route(...)` in the plugin's
/// Python, and every `@<pages>.tool(replaces=...)`'s path.
fn declared_routes(plugin: &Plugin) -> (Vec<Declared<'_>>, Vec<String>) {
    static DECORATOR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"@\s*[A-Za-z_][A-Za-z0-9_.]*\.(page|route|tool)\s*\(").unwrap()
    });
    static FIRST_STRING: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"^\s*(?:[A-Za-z_][A-Za-z0-9_]*|["']([^"']*)["'])"#).unwrap());
    static REPLACES: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"\breplaces\s*=\s*(?:["']([^"']*)["']|([A-Za-z_][A-Za-z0-9_]*))"#).unwrap()
    });
    let mut routes = Vec::new();
    let mut replaced = Vec::new();
    for source in python(plugin) {
        let text = python_uncommented(&source.text);
        for found in DECORATOR.captures_iter(&text) {
            let whole = found.get(0).expect("matched");
            let arguments = enclosed(&text[whole.end()..]).to_string();
            if &found[1] == "tool" {
                if let Some(path) = REPLACES.captures(&arguments) {
                    replaced.push(
                        path.get(1)
                            .or(path.get(2))
                            .map(|m| m.as_str().to_string())
                            .unwrap_or_default(),
                    );
                }
                continue;
            }
            let first = first_argument(&arguments).trim().to_string();
            let path = FIRST_STRING
                .captures(&first)
                .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
                .unwrap_or(first);
            routes.push(Declared {
                source,
                line: line_at(&text, whole.start()),
                path,
                arguments,
            });
        }
    }
    (routes, replaced)
}

/// The methods a declaration names; GET where it names none, and a change
/// assumed where they are named some way the text cannot read.
fn methods_of(arguments: &str) -> Vec<String> {
    static METHODS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bmethods\s*=\s*([\[(])").unwrap());
    let Some(found) = METHODS.captures(arguments) else {
        return vec!["GET".into()];
    };
    let at = found.get(1).expect("matched").end();
    let listed = enclosed(&arguments[at..]);
    let named: Vec<String> = Regex::new(r#"["']([A-Za-z]+)["']"#)
        .unwrap()
        .captures_iter(listed)
        .map(|c| c[1].to_ascii_uppercase())
        .collect();
    if named.is_empty() {
        vec!["POST".into()]
    } else {
        named
    }
}

/// A keyword's value in a declaration's arguments, as written, or None.
fn keyword<'a>(arguments: &'a str, name: &str) -> Option<&'a str> {
    let found = Regex::new(&format!(r"\b{name}\s*=\s*"))
        .unwrap()
        .find(arguments)?;
    Some(first_argument(&arguments[found.end()..]).trim())
}

fn tools_cover_routes(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "tools-cover-routes";
    let (routes, replaced) = declared_routes(plugin);
    let mut failures = Vec::new();
    for route in routes {
        let changing = methods_of(&route.arguments)
            .iter()
            .any(|method| method != "GET" && method != "HEAD");
        if !changing || keyword(&route.arguments, "params").is_some_and(|v| v != "None") {
            continue;
        }
        if replaced.contains(&route.path) {
            continue;
        }
        let offered = keyword(&route.arguments, "tool");
        if offered == Some("False") {
            let why = keyword(&route.arguments, "why")
                .map(|v| v.trim_matches(|c| c == '"' || c == '\''))
                .unwrap_or_default();
            if why.trim().is_empty() {
                failures.push(failure(
                    ID,
                    &route.source.path,
                    route.line,
                    format!("{} is kept from agents without saying why", route.path),
                    "`tool=False, why=\"...\"`: why an agent may not do what a person does here",
                ));
            } else if plugin.verified {
                failures.push(failure(
                    ID,
                    &route.source.path,
                    route.line,
                    format!(
                        "{} changes something and is kept from agents; a verified plugin keeps \
                         nothing a person may do at a level from an agent at that level",
                        route.path
                    ),
                    TOOL_ROUTE,
                ));
            }
            continue;
        }
        failures.push(failure(
            ID,
            &route.source.path,
            route.line,
            format!(
                "{} changes something and declares no typed record, so no tool is derived from it",
                route.path
            ),
            TOOL_ROUTE,
        ));
    }
    failures
}

// ── runs-as-checked (contract v21) ───────────────────────────────────────

const RUNS_AS_CHECKED: &str = "`checked_by=\"<tool>\"` on the act: a read tool (`reads=True`) \
    taking the same record, answering what the act would do and whether the account may, so a \
    service account set to prepare only prepares it for a person to make (contract v21)";

/// The paths of the routes a tool replaces (`@pages.tool(replaces=...)`), as
/// written or through a constant bound at the top of a module.
fn replaced_paths(declared: &[Declaration], constants: &HashMap<String, String>) -> Vec<String> {
    declared
        .iter()
        .filter(|declaration| declaration.kind == "tool")
        .filter_map(|declaration| keyword(&declaration.arguments, "replaces"))
        .filter_map(|path| string_named(path, constants))
        .collect()
}

/// The acts the plugin declares, by their tools' names: a page, route or
/// tool deriving a tool (`params=`) for a method that changes something,
/// neither `reads=True` nor kept from agents nor replaced by a tool, each
/// with whether it names its check tool (`checked_by=`). One named some way
/// the text cannot read is left out.
fn acts_named<'d, 'a>(
    declared: &'d [Declaration<'a>],
    constants: &HashMap<String, String>,
) -> Vec<(String, &'d Declaration<'a>, bool)> {
    let changes = |method: &str| method != "GET" && method != "HEAD";
    let replaced = replaced_paths(declared, constants);
    let mut acts = Vec::new();
    for declaration in declared.iter().filter(|d| d.kind != "setting") {
        let arguments = &declaration.arguments;
        let derived = keyword(arguments, "params").is_some_and(|params| params != "None");
        let reads = keyword(arguments, "reads") == Some("True");
        let changing = match declaration.kind {
            "tool" => keyword(arguments, "method")
                .and_then(|method| string_named(method, constants))
                .is_none_or(|method| changes(&method.to_ascii_uppercase())),
            _ => {
                keyword(arguments, "tool") != Some("False")
                    && !string_named(first_argument(arguments), constants)
                        .is_some_and(|path| replaced.contains(&path))
                    && methods_of(arguments).iter().any(|method| changes(method))
            }
        };
        if !derived || reads || !changing {
            continue;
        }
        let name = match keyword(arguments, "name") {
            Some(name) if name != "None" => string_named(name, constants),
            _ => declaration.view.clone(),
        };
        let Some(name) = name else {
            continue; // named some way the text cannot read
        };
        let checked = keyword(arguments, "checked_by")
            .is_some_and(|check| !matches!(check, "\"\"" | "''" | "None"));
        acts.push((name, declaration, checked));
    }
    acts
}

/// A call the plugin's own work makes as its role's service account: where,
/// and the tool it names, None where the text cannot read it.
struct CalledAs<'a> {
    source: &'a Source,
    line: usize,
    tool: Option<String>,
}

/// Where a `with` statement's items end: its first `:` outside every
/// bracket and string.
fn header_end(text: &str) -> Option<usize> {
    let (mut depth, mut quote, mut escaped) = (0i32, None::<char>, false);
    for (at, c) in text.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ':' if depth == 0 => return Some(at),
            _ => {}
        }
    }
    None
}

/// Every call through `meridian.runs_as(...)` in the plugin's Python: in
/// the block of an `async with <runs_as>(...) as <act>:`, each `<act>(...)`,
/// with the tool its first argument (or `tool_name=`) names, as written or
/// through a constant bound at the top of a module. `<runs_as>` is
/// `runs_as` however it is reached (`meridian.runs_as`, imported, imported
/// as another name), or a name the module binds to it (`act_as =
/// meridian.runs_as`, a parameter's default included, then `self.act_as`);
/// `async with <ctx> as <act>:` too, for a name bound to `runs_as(...)`.
fn called_as_service_account<'a>(
    plugin: &'a Plugin,
    constants: &HashMap<String, String>,
) -> Vec<CalledAs<'a>> {
    static BOUND: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\b([A-Za-z_][A-Za-z0-9_]*)[ \t]*(?::([^=\n:()]*))?=[ \t]*(?:[A-Za-z_][A-Za-z0-9_]*\.)*runs_as\b",
        )
        .unwrap()
    });
    static IMPORTED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bruns_as[ \t]+as[ \t]+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
    static WITH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:async\s+)?with\b").unwrap());
    static ITEM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)^(.*?)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)$").unwrap());
    let last = |dotted: &str| {
        dotted
            .rsplit('.')
            .next()
            .unwrap_or(dotted)
            .trim()
            .to_string()
    };
    let mut found = Vec::new();
    for source in python(plugin) {
        let text = python_uncommented(&source.text);
        if !text.contains("runs_as") {
            continue;
        }
        // The names standing for runs_as itself, and for a context it made.
        let mut calls = vec!["runs_as".to_string()];
        let mut contexts = Vec::new();
        for bound in BOUND.captures_iter(&text) {
            let annotated = bound.get(2).map_or("", |m| m.as_str());
            if split_arguments(annotated).len() > 1 {
                continue; // a comma: the name is an earlier parameter's
            }
            let after = text[bound.get(0).expect("matched").end()..].trim_start();
            if after.starts_with('(') {
                contexts.push(bound[1].to_string());
            } else {
                calls.push(bound[1].to_string());
            }
        }
        calls.extend(
            IMPORTED
                .captures_iter(&text)
                .map(|named| named[1].to_string()),
        );

        let statements = python_statements(&text);
        for (at, (line, statement)) in statements.iter().enumerate() {
            let trimmed = statement.trim_start();
            let lead = statement.len() - trimmed.len();
            let Some(with) = WITH.find(trimmed) else {
                continue;
            };
            let rest = &trimmed[with.end()..];
            let Some(colon) = header_end(rest) else {
                continue;
            };
            let mut items = rest[..colon].trim();
            if items.starts_with('(')
                && items.ends_with(')')
                && enclosed(&items[1..]).len() == items.len() - 2
            {
                items = &items[1..items.len() - 1];
            }
            let acts: Vec<String> = split_arguments(items)
                .into_iter()
                .filter_map(|item| ITEM.captures(item))
                .filter(|item| {
                    let made = item[1].trim();
                    match made.find('(') {
                        Some(open) if made.ends_with(')') => calls.contains(&last(&made[..open])),
                        _ => contexts.contains(&last(made)),
                    }
                })
                .map(|item| item[2].to_string())
                .collect();
            if acts.is_empty() {
                continue;
            }
            // The block: what follows the colon on its line, and every
            // statement after it indented further.
            let start = lead + with.end() + colon + 1;
            let mut block = vec![(
                line + statement[..start].matches('\n').count(),
                &statement[start..],
            )];
            block.extend(
                statements[at + 1..]
                    .iter()
                    .take_while(|(_, next)| next.len() - next.trim_start().len() > lead)
                    .map(|(line, next)| (*line, next.as_str())),
            );
            for act in &acts {
                let call = Regex::new(&format!(
                    r"(?:^|[^A-Za-z0-9_.])({})\s*\(",
                    regex::escape(act)
                ))
                .unwrap();
                for (first, part) in &block {
                    for called in call.captures_iter(part) {
                        let named = called.get(1).expect("matched");
                        let arguments = enclosed(&part[called.get(0).expect("matched").end()..]);
                        let tool = keyword(arguments, "tool_name")
                            .or_else(|| positional(arguments, 0))
                            .and_then(|tool| string_named(tool, constants));
                        found.push(CalledAs {
                            source,
                            line: first + part[..named.start()].matches('\n').count(),
                            tool,
                        });
                    }
                }
            }
        }
    }
    found
}

/// Every act the plugin's own work calls as its role's service account
/// (`meridian.runs_as`) names its check tool: a role's admin may set the
/// account to prepare only, and the dashboard then refuses an act with no
/// check at run time (contract v21; the check rule ruled 2026-10-12).
fn runs_as_checked(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "runs-as-checked";
    let constants = bound_strings(plugin);
    let declared = declarations(plugin);
    let acts = acts_named(&declared, &constants);
    let mut failures = Vec::new();
    for call in called_as_service_account(plugin, &constants) {
        // A tool named some way the text cannot read, or none of the acts
        // the plugin declares, is passed over: the dashboard refuses a tool
        // that is not one of the role's acts, at run time.
        let Some(tool) = call.tool else {
            continue;
        };
        let Some((_, act, _)) = acts
            .iter()
            .find(|(name, _, checked)| *name == tool && !checked)
        else {
            continue;
        };
        failures.push(failure(
            ID,
            &call.source.path,
            call.line,
            format!(
                "{tool} ({}:{}) is an act called through runs_as and names no check tool, so a \
                 service account set to prepare only is refused it",
                act.source.path, act.line
            ),
            RUNS_AS_CHECKED,
        ));
    }
    failures
}

// ── roles-declared ───────────────────────────────────────────────────────

/// meridian-schema's boundaries/roles.json at a6a0af7 (contract v24), byte
/// for byte: its `operations` say which roles hold each command, v24's
/// booking (the oms's), the allocation instruction, the give-up, the alert and
/// the reference role's template offer among them.
const ROLES_JSON: &str = include_str!("roles.json");

/// Each command a plugin may send, as the SDK names its call
/// (`record_opening_balance`), with the operation's name and the roles that
/// hold it (roles.json's `operations`, kind `command`).
pub(crate) fn commands() -> &'static [(String, String, Vec<String>)] {
    static COMMANDS: LazyLock<Vec<(String, String, Vec<String>)>> = LazyLock::new(|| {
        let published: serde_json::Value =
            serde_json::from_str(ROLES_JSON).expect("roles.json reads");
        published["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|operation| operation["kind"] == "command")
            .filter_map(|operation| {
                let name = operation["name"].as_str()?;
                let roles = operation["roles"]
                    .as_object()?
                    .iter()
                    .filter(|(how, _)| *how == "publishes")
                    .flat_map(|(_, roles)| roles.as_array().into_iter().flatten())
                    .filter_map(|role| role.as_str().map(String::from))
                    .collect();
                Some((snake_case(name), name.to_string(), roles))
            })
            .collect()
    });
    &COMMANDS
}

/// The roles roles.json lists, which `ROLES` must equal.
#[cfg(test)]
pub(crate) fn published_roles() -> Vec<String> {
    let published: serde_json::Value = serde_json::from_str(ROLES_JSON).expect("roles.json reads");
    published["roles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|role| role["name"].as_str().map(String::from))
        .collect()
}

fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (at, c) in name.char_indices() {
        if c.is_ascii_uppercase() && at > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// A page, route or tool decorator, or a setting: where it is, its
/// arguments, and, for a decorator, its view's body.
struct Declaration<'a> {
    source: &'a Source,
    /// `page`, `route`, `tool` or `setting`.
    kind: &'static str,
    line: usize,
    arguments: String,
    /// The view's body, and the line it starts on.
    body: Option<(usize, String)>,
    /// The view's name, which its tool takes where it gives no `name=`.
    view: Option<String>,
}

impl Declaration<'_> {
    /// How it is told apart in a failure: its path, or a setting's name.
    fn named(&self) -> String {
        let first = first_argument(&self.arguments).trim();
        let what = match self.kind {
            "tool" => keyword(&self.arguments, "replaces").unwrap_or(first),
            _ => first,
        };
        format!("{} {what}", self.kind)
    }
}

fn declarations(plugin: &Plugin) -> Vec<Declaration<'_>> {
    static DECORATOR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"@\s*[A-Za-z_][A-Za-z0-9_.]*\.(page|route|tool)\s*\(").unwrap()
    });
    static SETTING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bSetting\s*\(").unwrap());
    static DEF: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^([ \t]*)(?:async[ \t]+)?def[ \t]+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
    });
    let mut found = Vec::new();
    for source in python(plugin) {
        let text = python_uncommented(&source.text);
        for decorator in DECORATOR.captures_iter(&text) {
            let whole = decorator.get(0).expect("matched");
            let arguments = enclosed(&text[whole.end()..]).to_string();
            let def = DEF.captures_at(&text, whole.end());
            let view = def.as_ref().map(|def| def[2].to_string());
            let body = def.map(|def| {
                let indent = def[1].len();
                let start = def.get(0).expect("matched").start();
                let first = line_at(&text, start) + 1;
                let lines: Vec<&str> = text[start..]
                    .lines()
                    .skip(1)
                    .take_while(|line| {
                        line.trim().is_empty() || line.len() - line.trim_start().len() > indent
                    })
                    .collect();
                (first, lines.join("\n"))
            });
            let kind = match &decorator[1] {
                "page" => "page",
                "route" => "route",
                _ => "tool",
            };
            found.push(Declaration {
                source,
                kind,
                line: line_at(&text, whole.start()),
                arguments,
                body,
                view,
            });
        }
        for call in SETTING.find_iter(&text) {
            found.push(Declaration {
                source,
                kind: "setting",
                line: line_at(&text, call.start()),
                arguments: enclosed(&text[call.end()..]).to_string(),
                body: None,
                view: None,
            });
        }
    }
    found
}

/// The roles a `roles=` value names, as written or through a constant the
/// plugin binds at the top of a module; None where the text cannot say.
fn roles_named(plugin: &Plugin, declaration: &Declaration, value: &str) -> Option<Vec<String>> {
    static STRING: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"["']([^"'\\\n]*)["']"#).unwrap());
    let strings = |text: &str| -> Vec<String> {
        STRING
            .captures_iter(text)
            .map(|named| named[1].to_string())
            .collect()
    };
    if value.starts_with(['"', '\'', '[', '(']) {
        return Some(strings(value));
    }
    let constant = value.rsplit('.').next().unwrap_or(value);
    if !is_identifier(constant) {
        return None;
    }
    let bound = Regex::new(&format!(
        r"(?m)^{}[ \t]*(?::[^=\n]*)?=[ \t]*",
        regex::escape(constant)
    ))
    .unwrap();
    // Its own module first, then the plugin's others.
    std::iter::once(declaration.source)
        .chain(python(plugin))
        .find_map(|source| {
            let text = python_uncommented(&source.text);
            let at = bound.find(&text)?.end();
            let rest = &text[at..];
            let expression = if rest.starts_with(['(', '[', '{']) {
                enclosed(&rest[1..])
            } else {
                rest.lines().next().unwrap_or_default()
            };
            Some(strings(expression))
        })
}

const ROLES_DECLARED: &str = "`roles=[...]` naming the roles it serves, from those \
    [tool.meridian] names: on a plugin holding several roles, a person's level is granted per \
    role, a page is served by the person's level on one of its roles, a setting is set by an admin \
    of every role it serves, and the sidecar refuses the registration of a page or setting naming \
    none, or a role the plugin was not launched with (contract v15). A tool takes its route's \
    roles unless it names its own";

fn roles_declared(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "roles-declared";
    // A call to any command, by the name the SDK gives it.
    static SENT: LazyLock<Regex> = LazyLock::new(|| {
        let calls: Vec<String> = commands()
            .iter()
            .map(|(call, _, _)| regex::escape(call))
            .collect();
        Regex::new(&format!(r"\b({})\s*\(", calls.join("|"))).unwrap()
    });
    let (held, _) = declared(plugin);
    if held.len() < 2 {
        return Vec::new();
    }
    let holding = held.join(", ");
    let found = declarations(plugin);
    let mut failures = Vec::new();
    // The roles each route serves, for a tool that names none of its own.
    let route_roles = |path: &str| {
        found.iter().find_map(|route| {
            let named = first_argument(&route.arguments).trim();
            (route.kind != "tool" && named.trim_matches(['"', '\'']) == path)
                .then(|| keyword(&route.arguments, "roles"))
                .flatten()
                .and_then(|value| roles_named(plugin, route, value))
        })
    };
    for declaration in &found {
        let file = &declaration.source.path;
        let serves = match keyword(&declaration.arguments, "roles") {
            Some(value) => roles_named(plugin, declaration, value),
            None if declaration.kind == "tool" => keyword(&declaration.arguments, "replaces")
                .and_then(|path| route_roles(path.trim_matches(['"', '\'']))),
            None => Some(Vec::new()),
        };
        let Some(serves) = serves else {
            continue; // named some way the text cannot read
        };
        if serves.is_empty() {
            failures.push(failure(
                ID,
                file,
                declaration.line,
                format!(
                    "{} names no roles, on a plugin holding {holding}",
                    declaration.named()
                ),
                ROLES_DECLARED,
            ));
            continue;
        }
        for role in serves.iter().filter(|role| !held.contains(role)) {
            failures.push(failure(
                ID,
                file,
                declaration.line,
                format!(
                    "{} names `{role}`, which this plugin does not hold ({holding})",
                    declaration.named()
                ),
                ROLES_DECLARED,
            ));
        }
        // A changing route's view sends only the commands its roles hold.
        let changing = match declaration.kind {
            "tool" => keyword(&declaration.arguments, "method")
                .map(|method| method.trim_matches(['"', '\'']).to_ascii_uppercase())
                .is_none_or(|method| method != "GET" && method != "HEAD"),
            "setting" => false,
            _ => methods_of(&declaration.arguments)
                .iter()
                .any(|method| method != "GET" && method != "HEAD"),
        };
        let Some((first, body)) = declaration.body.as_ref().filter(|_| changing) else {
            continue;
        };
        for (index, line) in body.lines().enumerate() {
            for sent in SENT.captures_iter(line) {
                let Some((call, name, holders)) =
                    commands().iter().find(|(call, _, _)| *call == sent[1])
                else {
                    continue;
                };
                if holders.iter().any(|role| serves.contains(role)) {
                    continue;
                }
                failures.push(failure(
                    ID,
                    file,
                    first + index,
                    format!(
                        "{} serves {} and sends {name} ({call}), which {} holds",
                        declaration.named(),
                        serves.join(", "),
                        holders.join(", ")
                    ),
                    &format!(
                        "name a role that holds it ({}) in `roles=`, or send it from a route \
                         serving that role: the sidecar admits a command sent for a person only \
                         by their write on a role holding it, and refuses it otherwise, naming \
                         the role (contract v15)",
                        holders.join(", ")
                    ),
                ));
            }
        }
    }
    failures
}

// ── kit-linked ───────────────────────────────────────────────────────────

const KIT_LINK: &str = "in its <head>, the kit from the path the dashboard serves it at on the \
    plugin's own host: `<link rel=\"stylesheet\" href=\"/.meridian/ui/<version>/meridian.css\">` \
    and `<script src=\"/.meridian/ui/<version>/meridian.js\"></script>`, the version the \
    template's page.py names in KIT";

fn kit_linked(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "kit-linked";
    static DOCUMENT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)<!doctype\s+html|<html[\s>]|<head[\s>]").unwrap());
    let mut failures = Vec::new();
    for source in pages(plugin) {
        let name = file_name(&source.path);
        if name == "meridian.css" || name == "meridian.js" {
            failures.push(failure(
                ID,
                &source.path,
                0,
                "a copy of the kit, in the plugin".into(),
                "delete it, and link the kit the dashboard serves at /.meridian/ui/<version>/: \
                 a copy follows neither the kit's fixes nor each person's scheme",
            ));
            continue;
        }
        let text = uncommented(source);
        if let Some(document) = DOCUMENT.find(&text) {
            if !text.contains("/.meridian/ui/") && !text.contains("meridian.css") {
                failures.push(failure(
                    ID,
                    &source.path,
                    line_at(&text, document.start()),
                    "a page that does not link the kit".into(),
                    KIT_LINK,
                ));
            }
        }
    }
    failures
}

// ── no-raw-colour ────────────────────────────────────────────────────────

const COLOUR: &str = "the kit's custom property for what the colour means, as var(--name): \
    surfaces --page, --card, --hover; text --ink, --ink-soft, --ink-faint; edges --line, \
    --line-soft, --line-strong; --accent, --primary; status --good, --danger, --warn-ink, \
    --violet, each with its -wash; a price move --buy and --sell, never green and red. Better \
    still, a kit class: .badge.buy, .sell-ink, .notice (AGENTS.md, \"Colour\")";

/// CSS's colour names. `transparent` and `currentColor` name no colour.
const NAMED: [&str; 148] = [
    "aliceblue",
    "antiquewhite",
    "aqua",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "black",
    "blanchedalmond",
    "blue",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "fuchsia",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "gray",
    "green",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "lime",
    "limegreen",
    "linen",
    "magenta",
    "maroon",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "navy",
    "oldlace",
    "olive",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "purple",
    "rebeccapurple",
    "red",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "silver",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "teal",
    "thistle",
    "tomato",
    "turquoise",
    "violet",
    "wheat",
    "white",
    "whitesmoke",
    "yellow",
    "yellowgreen",
];

fn no_raw_colour(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "no-raw-colour";
    let mut failures = Vec::new();
    for source in pages(plugin) {
        let text = uncommented(source);
        for (index, line) in text.lines().enumerate() {
            for found in raw_colours(line) {
                failures.push(failure(
                    ID,
                    &source.path,
                    index + 1,
                    format!("`{found}` is a raw colour, which no scheme can change"),
                    COLOUR,
                ));
            }
        }
    }
    failures
}

/// The raw colours on one line: hex where a colour goes, a colour function,
/// or a colour's name as a colour property's, attribute's or style's value.
pub fn raw_colours(line: &str) -> Vec<String> {
    static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#[0-9A-Fa-f]+").unwrap());
    // Where a hex is a link, a selector or a reference, not a colour.
    static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)(?:(?:href|src|action|xlink:href)\s*=\s*["']?|url\(\s*["']?|(?:querySelector(?:All)?|closest|matches)\(\s*["'`])$"#,
        )
        .unwrap()
    });
    // What comes before a colour in a value: a declaration, an attribute, an
    // argument, or the word or length before it in a shorthand.
    static VALUE_BEFORE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)(?:[:=("'`,)]|\b(?:solid|dashed|dotted|double|groove|ridge|inset|outset|important)|\d(?:px|em|rem|%|deg|vh|vw)?)\s*$"#,
        )
        .unwrap()
    });
    static FUNCTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(?:rgba?|hsla?|hwb|oklch|oklab)\s*\(").unwrap());
    static DECLARATION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)(?:^|[\s;{"'`,])(?:[a-z-]*color|background|border[a-z-]*|outline[a-z-]*|column-rule|text-decoration|box-shadow|text-shadow|fill|stroke)\s*:\s*["'`]?([^;"'`{}]*)"#,
        )
        .unwrap()
    });
    static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\b(?:fill|stroke|stop-color|flood-color|lighting-color|color|bgcolor)\s*=\s*["']([^"']*)["']"#,
        )
        .unwrap()
    });
    static STYLE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\.style\.[a-z]*(?:color|background|fill|stroke|border|outline)[a-z]*\s*=\s*["'`]([^"'`]*)"#,
        )
        .unwrap()
    });

    let mut found = Vec::new();
    for hex in HEX.find_iter(line) {
        if ![4, 5, 7, 9].contains(&hex.len()) {
            continue;
        }
        let (before, after) = (&line[..hex.start()], &line[hex.end()..]);
        let joined = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        if after.starts_with(joined)
            || before.ends_with(|c: char| joined(c) || c == '&' || c == '#')
        {
            continue;
        }
        if REFERENCE.is_match(before) || !VALUE_BEFORE.is_match(before) {
            continue;
        }
        // A selector: `#bad {`, `#fed .row`.
        if after.trim_start().starts_with(['{', '.', '>', '[', '~']) {
            continue;
        }
        found.push(hex.as_str().to_string());
    }
    for function in FUNCTION.find_iter(line) {
        let call = &line[function.start()..];
        let end = call.find(')').map(|end| end + 1).unwrap_or(call.len());
        found.push(call[..end].to_string());
    }
    for pattern in [&*DECLARATION, &*ATTRIBUTE, &*STYLE] {
        for value in pattern.captures_iter(line) {
            for word in value[1].split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
                if NAMED.contains(&word.to_ascii_lowercase().as_str()) {
                    found.push(word.to_string());
                }
            }
        }
    }
    found
}

// ── own-origin ───────────────────────────────────────────────────────────

fn own_origin(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "own-origin";
    // Each captures the address something is loaded from: an element's
    // source, a stylesheet's import or url(), and a script's fetch, stream,
    // socket or import.
    static LOADED: LazyLock<[Regex; 5]> = LazyLock::new(|| {
        let absolute = r#"((?:https?:|wss?:)?//[^"'`\s>)]+)"#;
        [
            format!(
                r#"(?i)<(?:script|link|img|iframe|source|video|audio|embed|object|track|image|use)\b[^>]*?\b(?:src|href|data|poster|srcset|xlink:href)\s*=\s*["']?\s*{absolute}"#
            ),
            format!(r#"(?i)@import\s+(?:url\(\s*)?["']?\s*{absolute}"#),
            format!(r#"(?i)\burl\(\s*["']?\s*{absolute}"#),
            format!(
                r#"(?i)\b(?:fetch|EventSource|WebSocket|import|importScripts|sendBeacon)\s*\(\s*["'`]\s*{absolute}"#
            ),
            format!(r#"(?i)\bfrom\s+["']{absolute}"#),
        ]
        .map(|pattern| Regex::new(&pattern).unwrap())
    });
    let mut failures = Vec::new();
    for source in pages(plugin) {
        let text = uncommented(source);
        for (index, line) in text.lines().enumerate() {
            for pattern in LOADED.iter() {
                for loaded in pattern.captures_iter(line) {
                    failures.push(failure(
                        ID,
                        &source.path,
                        index + 1,
                        format!("`{}` is loaded from another origin", &loaded[1]),
                        "serve it from the plugin and load it by a path on the page's own \
                         host; anything from further away, the plugin's server fetches and \
                         serves. The kit is at /.meridian/ui/ on that host already. A page \
                         reaches only its own origin",
                    ));
                }
            }
        }
    }
    failures
}

// ── settings-declared ────────────────────────────────────────────────────

fn settings_declared(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "settings-declared";
    static ENVIRONMENT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"\bos\.environ\b|\bos\.getenv\s*\(|(?:^|[^.\w])getenv\s*\(|(?:^|[^.\w])environ\s*(?:\[|\.get\b)",
        )
        .unwrap()
    });
    static NAMED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"["']([A-Za-z_][A-Za-z0-9_]*)["']"#).unwrap());
    let mut failures = Vec::new();
    for source in python(plugin) {
        let text = python_uncommented(&source.text);
        for (index, line) in text.lines().enumerate() {
            let Some(read) = ENVIRONMENT.find(line) else {
                continue;
            };
            let name = NAMED
                .captures(&line[read.start()..])
                .map(|named| named[1].to_string());
            // Where the page listens on loopback, for its sidecar alone, as
            // the template's does: the plugin's own wiring, which nobody
            // configures, not a setting.
            if name
                .as_deref()
                .is_some_and(|name| name.ends_with("_PAGE_PORT"))
            {
                continue;
            }
            let setting = name
                .as_deref()
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "<name>".into());
            failures.push(failure(
                ID,
                &source.path,
                index + 1,
                match &name {
                    Some(name) => format!("{name} is read from the environment"),
                    None => "the environment is read".into(),
                },
                &format!(
                    "declare it as a setting, `meridian.Setting(\"{setting}\", ...)` in \
                     `meridian.connect(settings=[...])`, and read it from \
                     `plugin.settings()`: a deployment admin gives it in the dashboard, and \
                     nothing passes a plugin environment variables. A credential is \
                     `secret=True`"
                ),
            ));
        }
    }
    failures
}

// ── window-settings (contract v16) ───────────────────────────────────────

/// The names the SDK declares for each kind of raw record a plugin at the
/// edge declares, `<kind>_window_days` and `<kind>_past_window`, are its own
/// (the names' ruling of 2026-10-06, choice d): a setting of the plugin's
/// taking one is refused by the SDK when its declaration is built and by the
/// sidecar at registration. Said here first, with the line, before the
/// plugin runs at all.
fn window_settings(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "window-settings";
    static KIND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bRecordKind\s*\(").unwrap());
    let constants = bound_strings(plugin);
    let kinds: Vec<String> = python(plugin)
        .flat_map(|source| {
            let text = python_uncommented(&source.text);
            KIND.find_iter(&text)
                .filter_map(|call| {
                    let arguments = enclosed(&text[call.end()..]);
                    let named = keyword(arguments, "name").unwrap_or(first_argument(arguments));
                    string_named(named, &constants)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    if kinds.is_empty() {
        return Vec::new();
    }
    let mut failures = Vec::new();
    for declaration in declarations(plugin)
        .iter()
        .filter(|declaration| declaration.kind == "setting")
    {
        let named = keyword(&declaration.arguments, "name")
            .unwrap_or(first_argument(&declaration.arguments));
        let Some(name) = string_named(named, &constants) else {
            continue;
        };
        let Some(kind) = kinds.iter().find(|kind| {
            name == format!("{kind}_window_days") || name == format!("{kind}_past_window")
        }) else {
            continue;
        };
        failures.push(failure(
            ID,
            &declaration.source.path,
            declaration.line,
            format!(
                "the setting {name} takes the name of a window setting of the kind {kind}, \
                 which the SDK declares"
            ),
            &format!(
                "name it otherwise, or leave it out: the SDK declares `{kind}_window_days` and \
                 `{kind}_past_window` for the kind {kind}, as for every edge plugin's kinds, \
                 and refuses a plugin declaring either itself. Read the kind's window from \
                 `plugin.settings()` as delivered"
            ),
        ));
    }
    failures
}

/// The strings bound to a name at the top of a module, `NAME = "value"`, in
/// every Python file of the plugin.
fn bound_strings(plugin: &Plugin) -> HashMap<String, String> {
    static CONSTANT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?m)^([A-Za-z_][A-Za-z0-9_]*)[ \t]*(?::[^=\n]*)?=[ \t]*["']([^"'\\\n]+)["'][ \t]*$"#,
        )
        .unwrap()
    });
    python(plugin)
        .flat_map(|source| {
            CONSTANT
                .captures_iter(&python_uncommented(&source.text))
                .map(|bound| (bound[1].to_string(), bound[2].to_string()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The string an argument is, as written or through a bound name; None where
/// the text cannot say.
fn string_named(argument: &str, constants: &HashMap<String, String>) -> Option<String> {
    let argument = argument.trim();
    match argument.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let inner = argument.strip_prefix(quote)?.strip_suffix(quote)?;
            (!inner.contains(['"', '\'', '{'])).then(|| inner.to_string())
        }
        _ => constants
            .get(argument.rsplit('.').next().unwrap_or(argument))
            .cloned(),
    }
}

// ── secrets-kept ─────────────────────────────────────────────────────────

/// A setting declared `secret=True`: its name, and the constant it is
/// declared by, when it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Secret {
    pub name: Option<String>,
    pub constant: Option<String>,
}

/// Every secret setting the plugin declares.
pub fn secrets(plugin: &Plugin) -> Vec<Secret> {
    static CONSTANT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?m)^[ \t]*([A-Za-z_][A-Za-z0-9_]*)[ \t]*(?::[^=\n]*)?=[ \t]*["']([^"'\\\n]+)["'][ \t]*$"#,
        )
        .unwrap()
    });
    static SETTING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bSetting\s*\(").unwrap());
    static SECRET: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bsecret\s*=\s*True\b").unwrap());
    static NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|,)\s*name\s*=\s*([^,]+)").unwrap());

    let texts: Vec<String> = python(plugin)
        .map(|source| python_uncommented(&source.text))
        .collect();
    let constants: HashMap<String, String> = texts
        .iter()
        .flat_map(|text| CONSTANT.captures_iter(text))
        .map(|bound| (bound[1].to_string(), bound[2].to_string()))
        .collect();
    let mut found = Vec::new();
    for text in &texts {
        for call in SETTING.find_iter(text) {
            let arguments = enclosed(&text[call.end()..]);
            if !SECRET.is_match(arguments) {
                continue;
            }
            let named = NAME
                .captures(arguments)
                .map(|named| named[1].to_string())
                .unwrap_or_else(|| first_argument(arguments).to_string());
            let named = named.trim();
            let secret =
                if let Some(quote) = named.chars().next().filter(|c| matches!(c, '"' | '\'')) {
                    Secret {
                        name: Some(named.trim_matches(quote).to_string()),
                        constant: None,
                    }
                } else {
                    let constant = named.rsplit('.').next().unwrap_or(named).to_string();
                    Secret {
                        name: constants.get(&constant).cloned(),
                        constant: Some(constant).filter(|c| is_identifier(c)),
                    }
                };
            if (secret.name.is_some() || secret.constant.is_some()) && !found.contains(&secret) {
                found.push(secret);
            }
        }
    }
    // A field of a class `@meridian.settings` makes, declared `secret=True`:
    // the setting is named as the field (SDK 0.24.0).
    for setting in settings_in_order(plugin, &constants) {
        if setting.secret
            && !found
                .iter()
                .any(|held| held.name.as_ref() == Some(&setting.name))
        {
            found.push(Secret {
                name: Some(setting.name),
                constant: None,
            });
        }
    }
    found
}

fn is_identifier(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A call's arguments: the text up to the bracket that closes the one before
/// it, strings passed over.
fn enclosed(text: &str) -> &str {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (at, c) in text.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return &text[..at],
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    text
}

fn first_argument(arguments: &str) -> &str {
    let mut depth = 0i32;
    for (at, c) in arguments.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => return &arguments[..at],
            _ => {}
        }
    }
    arguments
}

fn secrets_kept(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "secrets-kept";
    const INSTEAD: &str = "say that it is set, never what it is: `\"<label> is set\"`, or \
        which one is missing. Keep the value where it is used, in an object whose repr hides \
        it (dataclasses.field(repr=False)), and describe a failed call by its type and status, \
        not its text. A secret is set in the dashboard and never shown back (decisions/027)";
    static LOGGED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b\w*log\w*\.(?:debug|info|warning|warn|error|exception|critical|fatal|log)\s*\(|\bprint\s*\(|\bsys\.std(?:out|err)\.write\s*\(|^\s*raise\b",
        )
        .unwrap()
    });
    static PAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[A-Za-z!/][^<>]*>").unwrap());
    static DECLARING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bSetting\s*\(").unwrap());

    let secrets = secrets(plugin);
    // Where a secret's value is: looked up by its name or constant, or a
    // variable or attribute called what it is, put in a string or passed on.
    let reads: Vec<(String, Regex)> = secrets
        .iter()
        .map(|secret| {
            let mut ways = Vec::new();
            if let Some(constant) = &secret.constant {
                let constant = regex::escape(constant);
                ways.push(format!(r"\[\s*{constant}\s*\]"));
                ways.push(format!(r"\.get\(\s*{constant}\b"));
            }
            if let Some(name) = &secret.name {
                let quoted = regex::escape(name);
                ways.push(format!(r#"\[\s*["']{quoted}["']\s*\]"#));
                ways.push(format!(r#"\.get\(\s*["']{quoted}["']"#));
                if is_identifier(name) {
                    ways.push(format!(r"\.{quoted}\b"));
                    ways.push(format!(r"\{{\s*{quoted}\s*[}}!:]"));
                    ways.push(format!(r"[,(]\s*{quoted}\s*[,)]"));
                }
            }
            let said = secret
                .name
                .clone()
                .or_else(|| secret.constant.clone())
                .unwrap_or_default();
            (said, Regex::new(&ways.join("|")).unwrap())
        })
        .collect();
    if reads.is_empty() {
        return Vec::new();
    }

    let mut failures = Vec::new();
    for source in python(plugin) {
        for (start, statement) in python_statements(&python_uncommented(&source.text)) {
            if DECLARING.is_match(&statement) {
                continue;
            }
            let logged = LOGGED.is_match(&statement);
            if !logged && !PAGE.is_match(&statement) {
                continue;
            }
            for (name, read) in &reads {
                let Some(at) = read.find(&statement) else {
                    continue;
                };
                let line = start + statement[..at.start()].matches('\n').count();
                let found = if statement.trim_start().starts_with("raise") {
                    format!("the secret setting `{name}` is put in an exception, which is logged")
                } else if logged {
                    format!("the secret setting `{name}` is logged")
                } else {
                    format!("the secret setting `{name}` is put in a page")
                };
                failures.push(failure(ID, &source.path, line, found, INSTEAD));
            }
        }
    }
    failures
}

// ── through-the-sdk ──────────────────────────────────────────────────────

fn through_the_sdk(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "through-the-sdk";
    const OPERATIONS: &str = "call the SDK's typed operations on the plugin \
        `meridian.connect()` returns (`plugin.record_holdings_statement(...)`, \
        `plugin.report(...)`, `plugin.settings()`). Its sidecar is the plugin's one way into \
        the deployment, holding its grants; nothing else is reachable from its pod";
    static IMPORTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?m)^[ \t]*(?:from|import)[ \t]+(nats|psycopg2?|psycopg_pool|asyncpg|sqlalchemy|pg8000|aiopg|grpc|kubernetes(?:_asyncio)?)\b",
        )
        .unwrap()
    });
    static STUBS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^[ \t]*(?:from|import)[ \t]+[^\n]*\b(\w+_pb2_grpc)\b").unwrap()
    });
    static ADDRESSED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\b(?:nats|postgres(?:ql)?)://[^"'`\s]*|[a-z0-9-]\.svc(?:\.cluster\.local)?\b|\bmeridian\.localhost\b|/terminal/[^"'`\s]*|\bMERIDIAN_SIDECAR_ADDRESS\b"#,
        )
        .unwrap()
    });
    let mut failures = Vec::new();
    for source in python(plugin) {
        let text = python_uncommented(&source.text);
        for imported in IMPORTED.captures_iter(&text) {
            let module = &imported[1];
            let why = match module {
                "nats" => {
                    "the bus is reached only through the sidecar, with the plugin's \
                           grants: "
                }
                "grpc" => "the SDK is the sidecar's client: ",
                "kubernetes" | "kubernetes_asyncio" => {
                    "a plugin never reaches the cluster; \
                           what it may do is its roles': "
                }
                _ => {
                    "the deployment's stores are its kernel's, read and recorded through \
                      the sidecar: "
                }
            };
            failures.push(failure(
                ID,
                &source.path,
                line_at(&text, imported.get(1).map(|m| m.start()).unwrap_or(0)),
                format!("it imports {module}, to reach the deployment directly"),
                &format!("{why}{OPERATIONS}"),
            ));
        }
        for stub in STUBS.captures_iter(&text) {
            failures.push(failure(
                ID,
                &source.path,
                line_at(&text, stub.get(1).map(|m| m.start()).unwrap_or(0)),
                format!("it imports the sidecar's raw stubs, {}", &stub[1]),
                &format!("the SDK is the sidecar's client: {OPERATIONS}"),
            ));
        }
    }
    for source in pages(plugin) {
        let text = uncommented(source);
        for (index, line) in text.lines().enumerate() {
            for addressed in ADDRESSED.find_iter(line) {
                failures.push(failure(
                    ID,
                    &source.path,
                    index + 1,
                    format!(
                        "`{}` addresses the deployment's own services",
                        addressed.as_str()
                    ),
                    &format!(
                        "name no address of the deployment's: `meridian.connect()` finds the \
                         sidecar by itself. Then {OPERATIONS}"
                    ),
                ));
            }
        }
    }
    failures
}

// ── tests ────────────────────────────────────────────────────────────────

fn tests_exist(plugin: &Plugin) -> Vec<Failure> {
    static TEST: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^[ \t]*(?:async[ \t]+)?def[ \t]+test_\w*[ \t]*\(").unwrap()
    });
    let tested = plugin.files.iter().any(|source| {
        let name = file_name(&source.path);
        name.ends_with(".py")
            && (name.starts_with("test_") || name.ends_with("_test.py"))
            && TEST.is_match(&source.text)
    });
    if tested {
        return Vec::new();
    }
    vec![failure(
        "tests-exist",
        "tests/",
        0,
        "no test_*.py with a test in it".into(),
        "tests/test_<what>.py, which pytest collects: at least the page rendered for a caller \
         and each operation the plugin sends, against a stand-in for its sidecar. \
         `meridian plugin check --run-tests` runs them, and so does the plugin's CI",
    )]
}

/// The plugin's tests, run where they are: with pytest, in the plugin's own
/// `.venv` when it has one, and otherwise the `python3` on the PATH, which
/// must have the plugin and pytest installed. Nothing is cached or compiled
/// into the directory.
pub fn run_tests(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "tests-pass";
    let venv = plugin.root.join(".venv").join("bin").join("python");
    let python = if venv.exists() {
        venv.display().to_string()
    } else {
        "python3".to_string()
    };
    let ran = std::process::Command::new(&python)
        .args(["-m", "pytest", "-q", "-p", "no:cacheprovider"])
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .current_dir(&plugin.root)
        .output();
    let ran = match ran {
        Ok(ran) => ran,
        Err(failed) => {
            return vec![failure(
                ID,
                "tests/",
                0,
                format!("{python} could not be run: {failed}"),
                "run it where Python 3 is installed, with the plugin and pytest: \
                 `pip install -e . pytest`, or a .venv in the plugin's directory",
            )]
        }
    };
    if ran.status.success() {
        return Vec::new();
    }
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    let (found, instead) = if said.contains("No module named pytest") {
        (
            format!("pytest is not installed for {python}"),
            "install it where the tests run, beside the plugin: `pip install -e . pytest`",
        )
    } else if ran.status.code() == Some(5) {
        (
            "pytest collected no tests".to_string(),
            "tests/test_<what>.py with functions named test_<what>, which pytest collects",
        )
    } else {
        let tail: Vec<&str> = said
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let tail = tail[tail.len().saturating_sub(12)..].join("\n");
        (
            format!("the tests failed:\n{tail}"),
            "fix what failed, and run them again: `python3 -m pytest` in the plugin's \
             directory shows every failure in full",
        )
    };
    vec![failure(ID, "tests/", 0, found, instead)]
}

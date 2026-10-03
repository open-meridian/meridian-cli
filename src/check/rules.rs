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
pub const RULES: [Rule; 11] = [
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
        holds: "a plugin holding a role with a suite runs the suite in its tests",
        check: role_suite,
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
        id: "settings-declared",
        holds: "settings are declared to the SDK, not read from the environment",
        check: settings_declared,
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
pub const ROLES: [&str; 13] = [
    "ccm",
    "compliance",
    "custody",
    "dgm",
    "ems",
    "match",
    "oms",
    "operations",
    "portfolio",
    "reporting",
    "servicing",
    "settlement",
    "signal",
];

/// The roles at the edge, which alone may own the storage a deployment
/// grants an instance for its raw external records (decisions/028).
pub const EDGE_ROLES: [&str; 7] = [
    "ccm",
    "custody",
    "dgm",
    "match",
    "reporting",
    "servicing",
    "settlement",
];

/// The roles whose conformance suite the SDK carries, as this release of the
/// CLI knows them (contract v11): a plugin holding one is verified for it only
/// by passing every case (spec/vendor-differences-have-a-place-in-the-contract).
pub const SUITES: [&str; 1] = ["custody"];

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
                if c == '\\' && i + 1 < chars.len() {
                    out.push(c);
                    out.push(chars[i + 1]);
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
                out.push(c);
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

fn role_suite(plugin: &Plugin) -> Vec<Failure> {
    const ID: &str = "role-suite";
    let (roles, _) = declared(plugin);
    let mut failures = Vec::new();
    for role in roles.iter().filter(|role| SUITES.contains(&role.as_str())) {
        let runs = Regex::new(&format!(
            r#"\brun(?:_async)?\(\s*["']{}["']"#,
            regex::escape(role)
        ))
        .unwrap();
        let tested = plugin.files.iter().any(|source| {
            let name = file_name(&source.path);
            name.ends_with(".py")
                && is_test(&source.path)
                && source.text.contains("meridian.suites")
                && runs.is_match(&source.text)
        });
        if !tested {
            failures.push(failure(
                ID,
                "tests/",
                0,
                format!("it holds `{role}`, and no test runs the {role} suite"),
                &format!(
                    "a test that runs `meridian.suites.run(\"{role}\", producers)`: each case of \
                     the suite mapped to the plugin's own exchange with its source, run through \
                     its own conversion, and the report asserted passed. `--run-tests` then \
                     holds the plugin to every case, as its role requires (contract v11)"
                ),
            ));
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

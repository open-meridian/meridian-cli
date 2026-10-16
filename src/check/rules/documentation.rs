//! `documentation-sections` and `documentation-safe` (contract v25;
//! plans/a-plugin-carries-its-own-documentation, build row 6, the names
//! approved 2026-10-10).
//!
//! A plugin's vendor writes its documentation as plain Markdown in `docs/`,
//! one file per fixed section, and the upload carries it with the version.
//! The dashboard renders it in a safe subset on the plugin's Documentation
//! tab, before the version is ever launched, and screens it before an agent
//! reads it; the platform's builder refuses a version whose documentation
//! fails these rules. So the check says, before an upload or a publish:
//!
//! - **`documentation-sections`**: `overview.md` and `setup.md` are there,
//!   and the declaration names the folder (`Declaration(documentation=...)`);
//!   `troubleshooting.md` and `terms.md` are prompted as advice, and required
//!   of a verified plugin (the plan's Q6); any other file in `docs/` is the
//!   repository's own and is said not to be uploaded.
//! - **`documentation-safe`**: each section is UTF-8 within its bounds (64 KiB
//!   a section, 256 KiB in all), and holds no raw HTML, no image, no link to a
//!   script, data or a file, no link whose text names one host and whose
//!   target another, no character that hides text, and no text that reads as
//!   instructions to an agent -- the screen's eleven rules, named as core and
//!   the platform name them. A level-two heading's text past what one screen
//!   of the tab shows, and a link the tab will show as plain text, are advice.
//!
//! A plugin on an SDK before 0.29.0 cannot declare its documentation yet, so
//! what these find on one is advice, failing nothing.

use std::sync::LazyLock;

use regex::Regex;

use super::{declared, failure, line_of, module_file, python};
use crate::check::{Failure, Plugin};

const SECTIONS_ID: &str = "documentation-sections";
const SAFE_ID: &str = "documentation-safe";

/// Each section's file, `docs/<name>.md`, in the declaration's order.
pub const SECTIONS: [&str; 5] = ["overview", "setup", "troubleshooting", "terms", "pages"];
/// Required of every plugin documenting itself.
const REQUIRED: [&str; 2] = ["overview", "setup"];
/// Prompted as advice, and required of a verified plugin.
const PROMPTED: [&str; 2] = ["troubleshooting", "terms"];
/// The most bytes in a section, and in all of them (the conductor's bounds).
pub const SECTION_BYTES: usize = 65_536;
pub const BUNDLE_BYTES: usize = 262_144;
/// The words one level-two heading's text holds to show on one screen of the
/// Documentation tab, at the kit's 390x844 (the plan, "One screen"). Advice.
pub const HEADING_WORDS: usize = 200;
/// The first SDK whose declaration names its documentation.
const DOCUMENTED_FROM: (u64, u64, u64) = (0, 29, 0);

const SECTIONS_INSTEAD: &str = "write docs/overview.md (what the plugin does, for whom, and what \
    it does not do) and docs/setup.md (what to arrange with the vendor, and the steps after \
    launching), plain Markdown; `meridian plugin new` writes a prompt for each";
const DECLARED_INSTEAD: &str = "name the folder in the version's declaration -- \
    `meridian.Declaration(..., documentation=\"docs\")` -- in the module [tool.meridian]'s \
    `declaration` names, so the upload carries the sections and the deployment shows them";
const PROMPTED_INSTEAD: &str = "write it: troubleshooting.md, one heading per problem in the \
    words the plugin's Summary or pages show; terms.md, the vendor's terms linked, or that it \
    calls no vendor";
const OTHER_INSTEAD: &str = "nothing, if it is the repository's own (a README index, say): only \
    overview, setup, troubleshooting, terms and pages are uploaded";
const SAFE_INSTEAD: &str = "plain Markdown the Documentation tab shows as written: headings, \
    paragraphs, emphasis, code, lists, tables, block quotes and https:, http: or mailto: links, \
    or a link to another section (setup.md); no HTML and no images";
const SCREEN_INSTEAD: &str = "write for the person reading, in plain words: nothing addressed to \
    an agent or a model, nothing claiming to speak for Open Meridian or the deployment, and no \
    tool's name; the deployment withholds such text from agents, and the registries refuse it";
const HIDDEN_INSTEAD: &str = "remove the character: text a person cannot see is text an agent \
    reads and a reviewer does not";
const HEADING_INSTEAD: &str = "split it under more level-two headings (`## ...`): the tab shows \
    one heading's text at a time, and a page never scrolls";
const PLAIN_LINK_INSTEAD: &str = "link https:, http: or mailto:, or another section by its file \
    (setup.md, setup.md#heading); anything else is shown as plain text";

/// One section as the plugin holds it.
struct Section {
    path: String,
    bytes: Vec<u8>,
}

/// The SDK the plugin pins, as numbers, where it pins one.
fn sdk_pin(plugin: &Plugin) -> Option<(u64, u64, u64)> {
    let table = plugin
        .file("pyproject.toml")?
        .text
        .parse::<toml::Table>()
        .ok()?;
    let pinned = table
        .get("project")?
        .get("dependencies")?
        .as_array()?
        .iter()
        .filter_map(|d| d.as_str())
        .find_map(|d| d.strip_prefix("open-meridian=="))?;
    let mut parts = pinned
        .trim()
        .split('.')
        .map(|part| part.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

/// Whether the plugin's SDK declares documentation; a plugin pinning none,
/// which `template-shape` fails, is held as if it did.
fn documented(plugin: &Plugin) -> bool {
    sdk_pin(plugin).is_none_or(|pin| pin >= DOCUMENTED_FROM)
}

fn sections(plugin: &Plugin) -> Vec<Section> {
    SECTIONS
        .iter()
        .filter_map(|name| {
            let path = format!("docs/{name}.md");
            let bytes = std::fs::read(plugin.root.join(&path)).ok()?;
            Some(Section { path, bytes })
        })
        .collect()
}

/// What else `docs/` holds: the repository's own, never uploaded.
fn others(plugin: &Plugin) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(plugin.root.join("docs")) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            !SECTIONS
                .iter()
                .any(|section| *name == format!("{section}.md"))
        })
        .map(|name| format!("docs/{name}"))
        .collect();
    found.sort();
    found
}

// ── documentation-sections ──────────────────────────────────────────────

fn missing(plugin: &Plugin, names: &[&str]) -> Vec<Failure> {
    names
        .iter()
        .filter(|name| !plugin.root.join(format!("docs/{name}.md")).is_file())
        .map(|name| {
            failure(
                SECTIONS_ID,
                &format!("docs/{name}.md"),
                0,
                format!("there is no docs/{name}.md"),
                if REQUIRED.contains(name) {
                    SECTIONS_INSTEAD
                } else {
                    PROMPTED_INSTEAD
                },
            )
        })
        .collect()
}

/// Where the sections are not carried: no declaration named, or one naming
/// no `documentation`.
fn undeclared(plugin: &Plugin) -> Vec<Failure> {
    if sections(plugin).is_empty() {
        return Vec::new();
    }
    let (_, declaration) = declared(plugin);
    match declaration {
        None => vec![failure(
            SECTIONS_ID,
            "pyproject.toml",
            plugin
                .file("pyproject.toml")
                .map(|source| line_of(&source.text, "[tool.meridian]"))
                .unwrap_or(0),
            "docs/ holds the plugin's documentation, and [tool.meridian] names no declaration \
             to carry it"
                .into(),
            DECLARED_INSTEAD,
        )],
        Some(named) => {
            let carried = match module_file(plugin, &named) {
                Some(source) => source.text.contains("documentation="),
                // Wherever the module is, some code of the plugin's names it.
                None => python(plugin).any(|source| source.text.contains("documentation=")),
            };
            if carried {
                return Vec::new();
            }
            let file = module_file(plugin, &named)
                .map(|source| source.path.clone())
                .unwrap_or_else(|| "pyproject.toml".into());
            vec![failure(
                SECTIONS_ID,
                &file,
                0,
                format!("the declaration {named} names no documentation, so docs/ is not uploaded"),
                DECLARED_INSTEAD,
            )]
        }
    }
}

pub fn documentation_sections(plugin: &Plugin) -> Vec<Failure> {
    if !documented(plugin) {
        return Vec::new();
    }
    let mut failures = missing(plugin, &REQUIRED);
    if plugin.verified {
        failures.extend(missing(plugin, &PROMPTED));
    }
    failures.extend(undeclared(plugin));
    failures
}

/// What `documentation-sections` advises, failing nothing.
pub fn documentation_sections_advice(plugin: &Plugin) -> Vec<Failure> {
    let mut advice = Vec::new();
    if !documented(plugin) {
        advice.extend(missing(plugin, &REQUIRED).into_iter().map(|mut held| {
            held.found = format!(
                "{}; from SDK 0.29.0 (contract v25) it is required",
                held.found
            );
            held
        }));
    } else if !plugin.verified {
        advice.extend(missing(plugin, &PROMPTED));
    }
    advice.extend(others(plugin).into_iter().map(|path| {
        failure(
            SECTIONS_ID,
            &path,
            0,
            format!("{path} is not a section, so it is not uploaded"),
            OTHER_INSTEAD,
        )
    }));
    advice
}

// ── documentation-safe ──────────────────────────────────────────────────

/// The section as the tab reads it: fenced code blocks and code spans
/// blanked, every line kept where it was.
fn outside_code(text: &str) -> Vec<String> {
    static SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(`+)[^`]*?(`+)").unwrap());
    let mut fence: Option<String> = None;
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(open) = &fence {
                if trimmed.starts_with(open.as_str()) {
                    fence = None;
                }
                return String::new();
            }
            for marker in ["```", "~~~"] {
                if trimmed.starts_with(marker) {
                    fence = Some(marker.to_string());
                    return String::new();
                }
            }
            SPAN.replace_all(line, |c: &regex::Captures| " ".repeat(c[0].len()))
                .into_owned()
        })
        .collect()
}

/// A character that hides text: Unicode's tag characters, bidirectional
/// controls, and the zero-width ones.
fn hidden(c: char) -> bool {
    matches!(c,
        '\u{E0000}'..='\u{E007F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2066}'..='\u{2069}'
        | '\u{200B}'..='\u{200F}'
        | '\u{2060}'..='\u{2064}'
        | '\u{FEFF}'
        | '\u{00AD}')
}

/// HTML entities a scheme can be spelled with, read back: `&#x61;`, `&#97;`,
/// `&colon;`.
fn unentitied(text: &str) -> String {
    static ENTITY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"&#(?:[xX]([0-9a-fA-F]+)|([0-9]+));?|&colon;").unwrap());
    ENTITY
        .replace_all(text, |c: &regex::Captures| {
            let code = match (c.get(1), c.get(2)) {
                (Some(hex), _) => u32::from_str_radix(hex.as_str(), 16).ok(),
                (_, Some(dec)) => dec.as_str().parse().ok(),
                _ => Some(':' as u32),
            };
            code.and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_default()
        })
        .into_owned()
}

/// A link's scheme, lower case, its spelling's tricks undone; empty for none.
fn scheme(target: &str) -> String {
    let undone: String = unentitied(target)
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect::<String>()
        .to_ascii_lowercase();
    match undone.split_once(':') {
        Some((scheme, _))
            if !scheme.is_empty()
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) =>
        {
            scheme.to_string()
        }
        _ => String::new(),
    }
}

/// The host a URL names, lower case: `https://a.b/c` and `a.b/c` alike.
fn host_of(text: &str) -> Option<String> {
    let bare = text
        .trim()
        .trim_matches(|c| matches!(c, '*' | '_' | '`' | '<' | '>'));
    let rest = match bare.split_once("://") {
        Some((_, rest)) => rest,
        None => bare,
    };
    let host = rest
        .split(['/', '?', '#'])
        .next()?
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    static DOMAIN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^(?:www\.)?[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$",
        )
        .unwrap()
    });
    DOMAIN
        .is_match(&host)
        .then(|| host.trim_start_matches("www.").to_string())
}

/// A link to another section of the bundle: `setup.md`, `setup.md#heading`.
fn to_a_section(target: &str) -> bool {
    let file = target.split('#').next().unwrap_or_default();
    let file = file.strip_prefix("./").unwrap_or(file);
    SECTIONS.iter().any(|name| file == format!("{name}.md"))
}

/// The screen's rules, by the names core and the platform answer them with
/// (meridian-platform domain/text_rules.py; core's tickets/quarantine.rs),
/// matched on the text in lower case with its whitespace collapsed.
static SCREEN: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        (
            "override",
            r"\b(?:ignore|disregard|forget|override|bypass)\b.{0,40}?\b(?:instructions?|rules?|prompts?|directions?|procedures?|guidelines?)\b",
        ),
        ("role marker", r"(?:^|[^a-z0-9_])(?:system|assistant|developer|user|human)\s*:"),
        (
            "chat markup",
            r"<\||\|>|\[/?inst\]|<</?sys>>|<\s*/?\s*(?:system|assistant|user|instructions?)\s*>",
        ),
        ("instruction words", r"\binstructions?\b|\bfrom now on\b|\bhenceforth\b"),
        (
            "addressed to the reader",
            r"\byou (?:must|should|shall|need to|have to|are (?:required|instructed) to|will now|may now)\b|\b(?:curators?|agents?|assistants?|models?|llms?) (?:must|should|shall|need to)\b",
        ),
        (
            "claims authority",
            r"\b(?:server|platform|open meridian|staff|administrator|admin|system) (?:instructs?|says|requires?|directs?|orders?|mandates?|note)\b",
        ),
        ("tool name", r"\b[a-z][a-z0-9-]*__[a-z][a-z0-9_]*\b"),
        (
            "answer-shaped",
            r"^(?:refused|error|outcome)\s*:|\b(?:jsonrpc|structuredcontent|iserror|tools/call)\b",
        ),
        (
            "prompt talk",
            r"\b(?:system|hidden|original|initial) prompt\b|\bprompt injection\b|\bjailbreak\b|\bdeveloper mode\b|\bpretend (?:to be|you)\b",
        ),
        (
            "exfiltration",
            r"\b(?:send|email|e-mail|post|upload|forward|exfiltrate|leak)\b.{0,60}?(?:https?://|www\.|\b[\w.+-]+@[\w-]+\.[\w.]{2,})",
        ),
        (
            "concealment",
            r"\bdo not (?:tell|report|mention|log|reveal|disclose)\b|\bkeep this secret\b",
        ),
    ]
    .into_iter()
    .map(|(name, pattern)| (name, Regex::new(pattern).unwrap()))
    .collect()
});

/// A line as the screen reads it: full-width letters as themselves, lower
/// case, its whitespace collapsed.
fn screened_form(line: &str) -> String {
    let folded: String = line
        .chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every finding of `documentation-safe` in one section: failures, then
/// advice.
fn safe_findings(section: &Section) -> (Vec<Failure>, Vec<Failure>) {
    static TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"<!--|<!\[CDATA\[|<![a-zA-Z]|<\?|</?[A-Za-z][A-Za-z0-9-]*(?:\s[^<>]*)?/?>")
            .unwrap()
    });
    static IMAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"!\[[^\]]*\]\s*[\(\[]").unwrap());
    static INLINE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"\[([^\]]*)\]\(\s*(<[^>]*>|[^)\s]*)(?:\s+(?:"[^"]*"|'[^']*'))?\s*\)"#).unwrap()
    });
    static DEFINITION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s{0,3}\[([^\]]+)\]:\s*(<[^>]*>|\S+)").unwrap());
    static AUTOLINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<([A-Za-z][A-Za-z0-9+.-]*:[^<>\s]*)>").unwrap());
    static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{0,3}(#{1,6})\s").unwrap());

    let mut failures = Vec::new();
    let mut advice = Vec::new();
    let path = section.path.as_str();
    let fail =
        |line: usize, found: String, instead: &str| failure(SAFE_ID, path, line, found, instead);

    if section.bytes.is_empty() || section.bytes.len() > SECTION_BYTES {
        failures.push(fail(
            0,
            format!(
                "it is {} bytes, and a section is 1 to {SECTION_BYTES}",
                section.bytes.len()
            ),
            "write it within the bound, or move part of it to another section",
        ));
    }
    let text = match std::str::from_utf8(&section.bytes) {
        Ok(text) => text,
        Err(failed) => {
            failures.push(fail(
                line_at_bytes(&section.bytes, failed.valid_up_to()),
                "it is not UTF-8 text".into(),
                "save it as UTF-8",
            ));
            return (failures, advice);
        }
    };

    for (n, line) in text.lines().enumerate() {
        if let Some(c) = line
            .chars()
            .find(|c| hidden(*c) && !(n == 0 && *c == '\u{FEFF}'))
        {
            failures.push(fail(
                n + 1,
                format!("it holds U+{:04X}, a character that hides text", c as u32),
                HIDDEN_INSTEAD,
            ));
        }
        let screened = screened_form(line);
        for (rule, pattern) in SCREEN.iter() {
            if pattern.is_match(&screened) {
                failures.push(fail(
                    n + 1,
                    format!("it reads as instructions to an agent ({rule})"),
                    SCREEN_INSTEAD,
                ));
            }
        }
    }

    let mut words_under: Option<(usize, String, usize)> = None;
    let over_heading = |held: &mut Option<(usize, String, usize)>, advice: &mut Vec<Failure>| {
        if let Some((line, title, words)) = held.take() {
            if words > HEADING_WORDS {
                advice.push(failure(
                    SAFE_ID,
                    path,
                    line,
                    format!("the text under `{title}` is {words} words, and one screen of the tab shows about {HEADING_WORDS}"),
                    HEADING_INSTEAD,
                ));
            }
        }
    };
    for (n, line) in outside_code(text).iter().enumerate() {
        let at = n + 1;
        if let Some(tag) = TAG.find(line) {
            failures.push(fail(
                at,
                format!(
                    "`{}` is HTML, which the tab shows as literal text",
                    tag.as_str()
                ),
                SAFE_INSTEAD,
            ));
        }
        if IMAGE.is_match(line) {
            failures.push(fail(
                at,
                "it holds an image, which the tab never shows".into(),
                SAFE_INSTEAD,
            ));
        }
        let mut links: Vec<(String, String)> = INLINE
            .captures_iter(line)
            .filter(|c| {
                c.get(0)
                    .is_some_and(|m| m.start() == 0 || &line[m.start() - 1..m.start()] != "!")
            })
            .map(|c| (c[1].to_string(), c[2].trim_matches(['<', '>']).to_string()))
            .collect();
        links.extend(
            DEFINITION
                .captures_iter(line)
                .map(|c| (String::new(), c[2].trim_matches(['<', '>']).to_string())),
        );
        links.extend(
            AUTOLINK
                .captures_iter(line)
                .map(|c| (String::new(), c[1].to_string())),
        );
        for (shown, target) in links {
            let named = scheme(&target);
            match named.as_str() {
                "javascript" | "vbscript" | "data" | "file" => failures.push(fail(
                    at,
                    format!("a link to `{named}:`, which no page follows"),
                    SAFE_INSTEAD,
                )),
                "https" | "http" => {
                    if let (Some(said), Some(goes)) = (host_of(&shown), host_of(&target)) {
                        if said != goes {
                            failures.push(fail(
                                at,
                                format!("a link whose text names {said} and which goes to {goes}"),
                                "make the link's text say where it goes, or name no address in it",
                            ));
                        }
                    }
                }
                "mailto" => {}
                "" if to_a_section(&target) => {}
                _ => advice.push(failure(
                    SAFE_ID,
                    path,
                    at,
                    format!("the link to `{target}` will be shown as plain text"),
                    PLAIN_LINK_INSTEAD,
                )),
            }
        }
        match HEADING.captures(line) {
            Some(level) if level[1].len() <= 2 => {
                over_heading(&mut words_under, &mut advice);
                let title = line.trim().trim_start_matches('#').trim().to_string();
                words_under = Some((at, title, 0));
            }
            _ => {
                if let Some((_, _, words)) = words_under.as_mut() {
                    *words += line.split_whitespace().count();
                }
            }
        }
    }
    over_heading(&mut words_under, &mut advice);
    (failures, advice)
}

fn line_at_bytes(bytes: &[u8], offset: usize) -> usize {
    bytes[..offset.min(bytes.len())]
        .iter()
        .filter(|b| **b == b'\n')
        .count()
        + 1
}

/// Every finding of `documentation-safe`, over every section and the
/// bundle: failures, then advice.
fn safe(plugin: &Plugin) -> (Vec<Failure>, Vec<Failure>) {
    let mut failures = Vec::new();
    let mut advice = Vec::new();
    let held = sections(plugin);
    let total: usize = held.iter().map(|section| section.bytes.len()).sum();
    if total > BUNDLE_BYTES {
        failures.push(failure(
            SAFE_ID,
            "docs/",
            0,
            format!("the sections hold {total} bytes together, and the most is {BUNDLE_BYTES}"),
            "shorten them: the five together are at most 256 KiB",
        ));
    }
    for section in &held {
        let (failed, advised) = safe_findings(section);
        failures.extend(failed);
        advice.extend(advised);
    }
    (failures, advice)
}

pub fn documentation_safe(plugin: &Plugin) -> Vec<Failure> {
    if !documented(plugin) {
        return Vec::new();
    }
    safe(plugin).0
}

/// What `documentation-safe` advises, failing nothing: on a plugin before
/// SDK 0.29.0, everything it finds.
pub fn documentation_safe_advice(plugin: &Plugin) -> Vec<Failure> {
    let (failures, advice) = safe(plugin);
    if documented(plugin) {
        advice
    } else {
        failures.into_iter().chain(advice).collect()
    }
}

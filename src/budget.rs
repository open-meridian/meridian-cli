//! The output budget: how much the CLI says where people read it most, held
//! to `src/budget/baseline.tsv`, whose figures only go down.
//!
//! Each figure is measured here, from the code that prints it: the first
//! tier of help, the longest flag line, `up` and `status` on a healthy
//! install, and the refusals that print a whole usage. Over its baseline, a
//! figure warns before `FAILS_FROM` and fails from it.

use crate::help;

/// The release from which a figure over its baseline fails the tests.
const FAILS_FROM: (u32, u32, u32) = (0, 1, 64);

fn version() -> (u32, u32, u32) {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// What `up` prints on a healthy install through the browser wizard, with
/// a local certificate kept and the archive asked at a terminal.
pub fn up_healthy() -> String {
    let checks: Vec<crate::doctor::Finding> = (0..13)
        .map(|n| crate::doctor::Finding::Fine(format!("check {n}")))
        .collect();
    let address = "https://meridian.localhost";
    [
        crate::doctor::brief(&checks).0,
        crate::up::run::ARCHIVE_PROMPT.to_string() + "\n",
        crate::up::run::archive_said(&crate::up::Archive::None) + "\n",
        crate::authority::Certified::Kept {
            ends_s: 1_800_000_000,
        }
        .said("meridian.localhost")
            + "\n",
        crate::up::run::installing("meridian", "meridian") + "\n",
        crate::up::run::WAITING.to_string() + "\n",
        crate::up::run::wizard_at(address) + "\n",
        crate::up::run::stays_at(address) + "\n",
        crate::up::run::trusted_by(std::path::Path::new("/Users/me/.meridian/authority")) + "\n",
    ]
    .concat()
}

/// Refusals that print the whole usage or a command's whole help after
/// their line, in the source.
fn whole_usage_sites() -> usize {
    let source = include_str!("main.rs");
    let printed_after = [
        "\\n\\n{USAGE}",
        "\\n\\n{}\",\n                help_for(",
        "\\n\\n{}\", help::root()",
        "\\n\\n{}\", help_for(",
    ];
    printed_after
        .iter()
        .map(|pattern| source.matches(pattern).count())
        .sum()
}

/// Every figure, by its name in the baseline.
pub fn measured() -> Vec<(&'static str, usize)> {
    let root = help::root();
    let flag_words = help::COMMANDS
        .iter()
        .flat_map(|c| c.flags.iter())
        .map(|f| f.says.split_whitespace().count())
        .max()
        .unwrap_or(0);
    let status = crate::status::said(&crate::status::tests::healthy(), "meridian");
    let width = |text: &str| text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    vec![
        ("root_help_lines", root.lines().count()),
        ("root_help_width", width(&root)),
        ("flag_line_words", flag_words),
        ("up_healthy_lines", up_healthy().lines().count()),
        ("status_healthy_lines", status.lines().count()),
        ("status_healthy_width", width(&status)),
        ("whole_usage_error_sites", whole_usage_sites()),
    ]
}

fn baseline() -> Vec<(String, usize)> {
    include_str!("budget/baseline.tsv")
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, figure) = line.split_once('\t').expect("name<TAB>figure");
            (name.to_string(), figure.trim().parse().expect("a number"))
        })
        .collect()
}

#[test]
fn the_output_stays_within_its_baseline() {
    let held = baseline();
    let mut over = Vec::new();
    for (name, figure) in measured() {
        let Some((_, limit)) = held.iter().find(|(held, _)| held == name) else {
            over.push(format!("{name} is {figure} and has no baseline"));
            continue;
        };
        if figure > *limit {
            over.push(format!("{name} is {figure}, over its baseline {limit}"));
        } else if figure < *limit {
            eprintln!(
                "output budget: {name} is {figure}, under its baseline {limit}: lower it in \
                 src/budget/baseline.tsv"
            );
        }
    }
    if over.is_empty() {
        return;
    }
    let said = format!("output budget:\n  {}", over.join("\n  "));
    if version() >= FAILS_FROM {
        panic!("{said}");
    }
    eprintln!("warning: {said}\n(this fails from 0.1.64)");
}

#[test]
fn up_on_a_healthy_install_is_twelve_lines_or_fewer() {
    let said = up_healthy();
    assert!(said.lines().count() <= 12, "{said}");
}

use super::*;

fn lines_of(text: &str) -> Vec<&str> {
    text.lines().collect()
}

#[test]
fn the_first_tier_fits_a_screen_and_lists_no_flags() {
    let root = root();
    let lines = lines_of(&root);
    assert!(lines.len() <= 60, "{} lines:\n{root}", lines.len());
    for line in &lines {
        assert!(line.chars().count() <= ROOT_WIDTH, "too wide: {line}");
    }
    for group in ["Deployment:", "Plugin:", "Dev:", "Data:", "Self:"] {
        assert!(root.contains(&format!("\n{group}\n")), "{group}: {root}");
    }
    // No flag is described in the first tier.
    for line in &lines {
        assert!(
            !line.contains(" -")
                || line.contains("--version")
                || line.starts_with("Global flags:")
                || line.starts_with("Each command's flags:"),
            "a flag in the first tier: {line}"
        );
    }
}

#[test]
fn the_first_tier_is_as_written() {
    assert_eq!(root(), include_str!("golden/root.txt"));
}

#[test]
fn ups_help_is_as_written() {
    assert_eq!(
        help_for("up", &[]),
        include_str!("golden/up.txt"),
        "\n{}",
        help_for("up", &[])
    );
}

#[test]
fn every_commands_help_is_short() {
    for c in COMMANDS {
        let help = command(c);
        for line in help.lines() {
            assert!(
                line.chars().count() <= COMMAND_WIDTH,
                "{}: too wide: {line}",
                c.name
            );
        }
        for f in c.flags {
            let words = f.says.split_whitespace().count();
            assert!(words <= 14, "{} {}: {words} words", c.name, f.spelling);
        }
        assert!(
            !c.examples.is_empty() && c.examples.len() <= 2,
            "{}",
            c.name
        );
        assert!(
            help.contains("\nDocs: https://open-meridian.dev/"),
            "{}",
            c.name
        );
        assert!(c.about.ends_with('.'), "{}: one sentence", c.name);
        assert!(
            c.summary.chars().count() + ROOT_COLUMN <= ROOT_WIDTH,
            "{}",
            c.name
        );
    }
}

#[test]
fn a_common_flag_is_said_the_same_way_everywhere() {
    // Said once, in Common, never again as a command's own flag.
    for c in COMMANDS {
        for f in c.flags {
            for common in [
                "--namespace",
                "--release <name>",
                "--timeout",
                "--yes",
                "--json",
                "--deployment",
            ] {
                assert!(
                    !f.spelling.contains(common),
                    "{}: {} is a common flag",
                    c.name,
                    f.spelling
                );
            }
        }
    }
}

#[test]
fn a_command_and_its_subcommands_are_found() {
    let words = |said: &[&str]| said.iter().map(|w| w.to_string()).collect::<Vec<_>>();
    assert_eq!(
        find("plugin", &words(&["key", "setup"])).unwrap().name,
        "plugin key setup"
    );
    assert_eq!(
        find("plugin", &words(&["key", "list"])).unwrap().name,
        "plugin key"
    );
    assert_eq!(
        find("plugin", &words(&["launch", "p", "1.0"]))
            .unwrap()
            .name,
        "plugin launch"
    );
    assert_eq!(find("backup", &words(&["key"])).unwrap().name, "backup key");
    assert_eq!(find("backup", &[]).unwrap().name, "backup");
    assert!(find("plugin", &[]).is_none());
    let plugin = help_for("plugin", &[]);
    assert!(
        plugin.starts_with("Usage: meridian plugin <command>"),
        "{plugin}"
    );
    assert!(plugin.contains("  key setup "), "{plugin}");
    assert!(known("plugin") && known("up") && known("partition"));
    assert!(!known("nonsense") && !known("plug"));
}

#[test]
fn every_flag_the_parser_takes_is_in_a_commands_help() {
    let all: String = COMMANDS.iter().map(command).collect();
    let said = |spelling: &str| {
        all.lines()
            .any(|line| line.split([' ', ',']).any(|word| word == spelling))
    };
    let globals = [
        "-h",
        "--help",
        "-v",
        "--verbose",
        "-q",
        "--quiet",
        "-f",
        "-p",
    ];
    for spelling in crate::TAKES_A_VALUE.iter().chain(crate::SWITCHES.iter()) {
        if globals.contains(spelling) || HIDDEN.iter().any(|h| h == spelling) {
            continue;
        }
        assert!(said(spelling), "{spelling} is in no command's help");
    }
}

#[test]
fn advanced_flags_wait_at_the_end_and_test_flags_are_not_listed() {
    let up = help_for("up", &[]);
    let advanced = up.find("\nAdvanced:\n").expect("an Advanced block");
    for spelling in [
        "--chart <ref>",
        "--image <ref>",
        "--platform <url>",
        "--port <n>",
        "--no-doctor",
        "--release <name>",
    ] {
        let at = up
            .find(spelling)
            .unwrap_or_else(|| panic!("{spelling}: {up}"));
        assert!(at > advanced, "{spelling} before Advanced: {up}");
    }
    // Deprecated flags come last, after every flag that is to be used.
    let release = up.find("--release <name>").expect("--release");
    for spelling in DEPRECATED {
        let at = up
            .find(spelling)
            .unwrap_or_else(|| panic!("{spelling}: {up}"));
        assert!(at > release, "{spelling} before --release: {up}");
    }
    assert!(!up.contains("--plain-http"), "{up}");
    // Still a flag the parser takes.
    assert!(crate::SWITCHES.contains(&"--plain-http"));
    let root = root();
    assert!(
        root.contains("-q, --quiet") && root.contains("-v, --verbose"),
        "{root}"
    );
}

#[test]
fn a_pair_of_commands_is_one_line_of_the_first_tier() {
    let root = root();
    for (first, second, summary) in ROOT_PAIRS {
        assert!(find(first, &[]).is_some() && find(second, &[]).is_some());
        let line = format!(
            "  {:width$}{summary}",
            format!("{first} | {second}"),
            width = ROOT_COLUMN - 2
        );
        assert!(root.lines().any(|l| l == line), "{line}:\n{root}");
        for name in [first, second] {
            assert!(
                !root
                    .lines()
                    .any(|l| l.starts_with(&format!("  {name:width$}", width = ROOT_COLUMN - 2))),
                "{name} has a line of its own:\n{root}"
            );
        }
    }
}

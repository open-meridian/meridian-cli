//! How much a command says: `-q/--quiet`, `-v/--verbose`, or neither.
//!
//! Results go to stdout and progress to stderr, so a script reading stdout
//! gets the answer alone. Quiet drops the progress; verbose adds the detail a
//! command keeps back by default, such as the Helm command `up` runs.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Quiet,
    Normal,
    Verbose,
}

static LEVEL: AtomicU8 = AtomicU8::new(1);

/// Set once, from the flags, before any command runs. Both given is refused
/// by the caller.
pub fn set(level: Level) {
    LEVEL.store(
        match level {
            Level::Quiet => 0,
            Level::Normal => 1,
            Level::Verbose => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn level() -> Level {
    match LEVEL.load(Ordering::Relaxed) {
        0 => Level::Quiet,
        2 => Level::Verbose,
        _ => Level::Normal,
    }
}

pub fn verbose() -> bool {
    level() == Level::Verbose
}

/// A step as it happens, on stderr; nothing when quiet.
pub fn progress(line: &str) {
    if level() != Level::Quiet {
        eprintln!("{line}");
    }
}

/// Detail kept back unless asked for, on stderr.
pub fn detail(text: &str) {
    if verbose() {
        eprintln!("{text}");
    }
}

/// The flags a command was given that are going away, each with the one
/// line that says what replaces it. They still work in this release.
pub fn deprecated(command: &str, words: &[String], given: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut said = Vec::new();
    let mut warn = |flag: &str, instead: &str| {
        if given(flag) {
            said.push(format!(
                "meridian: {flag} is deprecated and goes in the next release; {instead}"
            ));
        }
    };
    if command == "up" {
        warn("--enrolment-code", "set MERIDIAN_ENROLMENT_CODE instead");
        warn("--first-run-code", "set MERIDIAN_FIRST_RUN_CODE instead");
        warn(
            "--shared-storage-class",
            "set pluginStorage.shared.storageClassName in a values file (-f) instead",
        );
    }
    if command == "plugin"
        && words
            .first()
            .is_some_and(|w| w == "publish" || w == "release")
    {
        warn("--details", "use -v instead");
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deprecated_flag_says_its_replacement_in_one_line() {
        let given = |flags: &'static [&'static str]| move |flag: &str| flags.contains(&flag);
        let said = deprecated("up", &[], &given(&["--enrolment-code", "--id"]));
        assert_eq!(
            said,
            [
                "meridian: --enrolment-code is deprecated and goes in the next release; \
              set MERIDIAN_ENROLMENT_CODE instead"
            ]
        );
        let said = deprecated(
            "up",
            &[],
            &given(&["--first-run-code", "--shared-storage-class"]),
        );
        assert_eq!(said.len(), 2);
        assert!(said[1].contains("pluginStorage.shared.storageClassName"));
        let publish = ["publish".to_string()];
        let said = deprecated("plugin", &publish, &given(&["--details"]));
        assert_eq!(said.len(), 1);
        assert!(said[0].ends_with("use -v instead"), "{said:?}");
        assert!(deprecated("plugin", &publish, &given(&[])).is_empty());
        let release = ["release".to_string(), "1.4.0".to_string()];
        assert_eq!(
            deprecated("plugin", &release, &given(&["--details"])).len(),
            1
        );
        // Nothing else is deprecated.
        assert!(deprecated("status", &[], &given(&["--details", "--json"])).is_empty());
    }
}

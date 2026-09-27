use super::*;

/// A directory of its own under the system's temporary one, removed when the
/// test is done with it.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("meridian-plugin-{label}-{unique}"));
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn every_file(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut waiting = vec![root.to_path_buf()];
    while let Some(dir) = waiting.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                waiting.push(path);
            } else {
                found.push(
                    path.strip_prefix(root)
                        .expect("under the root")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    found.sort();
    found
}

#[test]
fn every_file_in_the_template_is_one_it_writes() {
    // include_str! names each file by hand, so a file added to the template
    // and not to TEMPLATE would be silently left out of every scaffold.
    let on_disk = every_file(&Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin-template"));
    let mut embedded: Vec<String> = TEMPLATE.iter().map(|(path, _)| path.to_string()).collect();
    embedded.sort();
    assert_eq!(on_disk, embedded);
}

#[test]
fn a_new_plugin_is_the_template_under_its_own_name() {
    let scratch = Scratch::new("named");
    let into = scratch.0.join("meridian-snaptrade");

    let wrote = scaffold("meridian-snaptrade", &into).expect("scaffolded");

    assert_eq!(wrote.len(), TEMPLATE.len());
    let files = every_file(&into);
    assert!(
        files.contains(&"src/meridian_snaptrade/__main__.py".to_string()),
        "{files:?}"
    );
    let pyproject = std::fs::read_to_string(into.join("pyproject.toml")).expect("written");
    assert!(
        pyproject.contains("name = \"meridian-snaptrade\""),
        "{pyproject}"
    );
    assert!(
        pyproject.contains("\"open-meridian=="),
        "the SDK is pinned: {pyproject}"
    );
    assert!(pyproject.contains("meridian-snaptrade = \"meridian_snaptrade.__main__:main\""));

    // Nothing is left calling itself the template.
    for file in files {
        let text = std::fs::read_to_string(into.join(&file)).expect("readable");
        assert!(!file.contains("reference"), "{file}");
        assert!(!text.contains("reference-plugin"), "{file}");
        assert!(!text.contains("reference_plugin"), "{file}");
    }
}

#[test]
fn it_never_writes_into_something_already_there() {
    let scratch = Scratch::new("exists");
    let into = scratch.0.join("taken");
    std::fs::create_dir_all(&into).expect("made");
    std::fs::write(into.join("keep.txt"), "mine").expect("written");

    assert!(scaffold("taken", &into).is_err());
    assert_eq!(
        std::fs::read_to_string(into.join("keep.txt")).expect("still there"),
        "mine"
    );
}

#[test]
fn a_name_is_one_every_place_it_goes_accepts() {
    for good in ["snaptrade", "meridian-snaptrade", "custody-2", "a"] {
        assert!(check_name(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "Snaptrade",
        "2fast",
        "-x",
        "x-",
        "a--b",
        "snap_trade",
        "snap trade",
        "meridian",
    ] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_new_plugin_keeps_claudes_files_out_of_git_and_out_of_its_image() {
    let scratch = Scratch::new("claude");
    let into = scratch.0.join("meridian-snaptrade");
    scaffold("meridian-snaptrade", &into).expect("scaffolded");

    let files = every_file(&into);
    for written in [
        ".gitignore",
        "CLAUDE.md",
        ".claude/skills/develop-live/SKILL.md",
    ] {
        assert!(files.contains(&written.to_string()), "{written}: {files:?}");
    }
    assert!(
        !files.contains(&"gitignore".to_string()),
        "written as .gitignore"
    );

    let lines = |file: &str| -> Vec<String> {
        std::fs::read_to_string(into.join(file))
            .expect("written")
            .lines()
            .map(|line| line.trim().to_string())
            .collect()
    };
    let gitignore = lines(".gitignore");
    for ignored in ["CLAUDE.md", "CLAUDE.local.md", ".claude/"] {
        assert!(
            gitignore.iter().any(|l| l == ignored),
            "{ignored}: {gitignore:?}"
        );
    }
    let dockerignore = lines(".dockerignore");
    for ignored in ["CLAUDE.md", "CLAUDE.local.md", ".claude"] {
        assert!(
            dockerignore.iter().any(|l| l == ignored),
            "{ignored}: {dockerignore:?}"
        );
    }

    // And what they teach is about this plugin, not the template.
    let skill = std::fs::read_to_string(into.join(".claude/skills/develop-live/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: develop-live\n"), "{skill}");
    assert!(skill.contains("meridian plugin dev --instance meridian-snaptrade"));
    assert!(!skill.contains("reference-plugin") && !skill.contains("reference_plugin"));
    let claude = std::fs::read_to_string(into.join("CLAUDE.md")).unwrap();
    assert!(claude.contains("src/meridian_snaptrade/"), "{claude}");
}

#[test]
fn plugin_dev_sends_none_of_claudes_files() {
    let scratch = Scratch::new("claude-live");
    let into = scratch.0.join("meridian-snaptrade");
    scaffold("meridian-snaptrade", &into).expect("scaffolded");
    std::fs::write(into.join(".claude/dev.jsonl"), "{}\n").unwrap();
    let sent = crate::live::scan(&into, &crate::live::Ignored::of(&into));
    assert!(
        sent.keys()
            .all(|path| !path.contains("CLAUDE") && !path.starts_with(".claude")),
        "{:?}",
        sent.keys().collect::<Vec<_>>()
    );
    assert!(sent.contains_key("pyproject.toml"));
}

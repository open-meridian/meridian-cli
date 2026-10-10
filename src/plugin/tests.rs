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
fn every_file_in_each_roles_template_is_one_it_writes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugin-templates");
    let mut on_disk: Vec<String> = std::fs::read_dir(&root)
        .expect("plugin-templates")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    on_disk.sort();
    let mut held: Vec<String> = ROLE_TEMPLATES
        .iter()
        .map(|(role, _)| role.to_string())
        .collect();
    held.sort();
    assert_eq!(
        on_disk, held,
        "a template on disk for each role, and no other"
    );
    for (role, files) in ROLE_TEMPLATES {
        let mut embedded: Vec<String> = files.iter().map(|(path, _)| path.to_string()).collect();
        embedded.sort();
        assert_eq!(every_file(&root.join(role)), embedded, "{role}");
    }
}

#[test]
fn a_roles_template_is_a_plugin_holding_the_role_under_its_own_name() {
    for (role, files) in ROLE_TEMPLATES {
        let scratch = Scratch::new(role);
        let into = scratch.0.join("prices-2");

        let wrote = scaffold("prices-2", &into, Some(role)).expect("scaffolded");

        assert_eq!(wrote.len(), files.len(), "{role}");
        let pyproject = std::fs::read_to_string(into.join("pyproject.toml")).expect("written");
        assert!(
            pyproject.contains(&format!("roles = [\"{role}\"]")),
            "{role}: {pyproject}"
        );
        assert!(
            pyproject.contains("prices-2 = \"prices_2.__main__:main\""),
            "{pyproject}"
        );
        // Its tests run the role's suite.
        let suite = std::fs::read_to_string(into.join("tests/test_suite.py")).expect("written");
        assert!(
            suite.contains(&format!("run(\"{role}\", PRODUCERS")),
            "{role}"
        );
        for file in every_file(&into) {
            let text = std::fs::read_to_string(into.join(&file)).expect("readable");
            assert!(!file.contains("reference"), "{file}");
            assert!(
                !text.contains("reference-plugin") && !text.contains("reference_plugin"),
                "{role}: {file}"
            );
        }
    }
    let scratch = Scratch::new("dgm-pages");
    let into = scratch.0.join("prices");
    scaffold("prices", &into, Some("dgm")).expect("scaffolded");
    let page = std::fs::read_to_string(into.join("src/prices/page.py")).expect("written");
    assert!(page.contains("name=\"read_connection\"") && page.contains("name=\"read_datasets\""));
    let into = scratch.0.join("report");
    scaffold("report", &into, Some("reporting")).expect("scaffolded");
    let page = std::fs::read_to_string(into.join("src/report/page.py")).expect("written");
    assert!(page.contains("name=\"read_report\""), "{page}");
}

#[test]
fn a_role_with_no_template_is_refused_naming_those_that_have_one() {
    let scratch = Scratch::new("no-template");
    let refused = scaffold("orders", &scratch.0.join("orders"), Some("oms")).unwrap_err();
    assert!(
        refused.contains("no template for `oms`")
            && refused.contains("`--role dgm` or `--role reporting`"),
        "{refused}"
    );
    let refused = scaffold("orders", &scratch.0.join("orders"), Some("trading")).unwrap_err();
    assert!(refused.contains("\"trading\" is not a role"), "{refused}");
    assert!(!scratch.0.join("orders").exists(), "nothing written");
}

#[test]
fn what_to_do_next_says_how_a_roles_plugin_is_held_to_its_suite() {
    let said = next_steps("prices", Path::new("prices"), Some("dgm"));
    assert!(said.contains("a `dgm` plugin"), "{said}");
    assert!(
        said.contains("meridian plugin check --verified --run-tests"),
        "{said}"
    );
    let plain = next_steps("prices", Path::new("prices"), None);
    assert!(!plain.contains("--verified"), "{plain}");
    assert!(plain.contains("with the plugin, so whoever works on it next has them too.\n"));
}

#[test]
fn a_new_plugin_is_the_template_under_its_own_name() {
    let scratch = Scratch::new("named");
    let into = scratch.0.join("meridian-snaptrade");

    let wrote = scaffold("meridian-snaptrade", &into, None).expect("scaffolded");

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

    assert!(scaffold("taken", &into, None).is_err());
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
fn a_new_plugin_commits_the_agents_shared_files_and_keeps_all_of_them_out_of_its_image() {
    let scratch = Scratch::new("claude");
    let into = scratch.0.join("meridian-snaptrade");
    scaffold("meridian-snaptrade", &into, None).expect("scaffolded");

    let files = every_file(&into);
    for written in [
        ".gitignore",
        "AGENTS.md",
        "CLAUDE.md",
        ".claude/skills/develop-live/SKILL.md",
    ] {
        assert!(files.contains(&written.to_string()), "{written}: {files:?}");
    }

    let lines = |file: &str| -> Vec<String> {
        std::fs::read_to_string(into.join(file))
            .expect("written")
            .lines()
            .map(|line| line.trim().to_string())
            .collect()
    };
    // Committed, as Claude Code's own convention has it: what is shared is in
    // git, and only what is one person's is ignored.
    let gitignore = lines(".gitignore");
    for ignored in [
        "CLAUDE.local.md",
        ".claude/settings.local.json",
        ".meridian/",
    ] {
        assert!(
            gitignore.iter().any(|l| l == ignored),
            "{ignored}: {gitignore:?}"
        );
    }
    for shared in [
        "AGENTS.md",
        "CLAUDE.md",
        ".claude",
        ".claude/",
        ".claude/skills/",
    ] {
        assert!(
            !gitignore.iter().any(|l| l == shared),
            "{shared} is committed: {gitignore:?}"
        );
    }
    let dockerignore = lines(".dockerignore");
    for ignored in [
        "AGENTS.md",
        "CLAUDE.md",
        "CLAUDE.local.md",
        ".claude",
        ".meridian",
    ] {
        assert!(
            dockerignore.iter().any(|l| l == ignored),
            "{ignored}: {dockerignore:?}"
        );
    }

    // What they teach is about this plugin, not the template, and it is
    // taught once: AGENTS.md holds the loop, and Claude's files lead to it.
    let agents = std::fs::read_to_string(into.join("AGENTS.md")).unwrap();
    assert!(
        agents.contains("meridian plugin dev --instance meridian-snaptrade"),
        "{agents}"
    );
    assert!(agents.contains("src/meridian_snaptrade/"), "{agents}");
    let skill = std::fs::read_to_string(into.join(".claude/skills/develop-live/SKILL.md")).unwrap();
    assert!(skill.starts_with("---\nname: develop-live\n"), "{skill}");
    assert!(skill.contains("AGENTS.md"), "{skill}");
    let claude = std::fs::read_to_string(into.join("CLAUDE.md")).unwrap();
    assert!(claude.contains("@AGENTS.md"), "{claude}");
    for file in [&agents, &skill, &claude] {
        assert!(!file.contains("reference-plugin") && !file.contains("reference_plugin"));
    }
}

#[test]
fn plugin_dev_sends_none_of_the_agents_files() {
    let scratch = Scratch::new("claude-live");
    let into = scratch.0.join("meridian-snaptrade");
    scaffold("meridian-snaptrade", &into, None).expect("scaffolded");
    // Where the skill has `plugin dev` write its own output.
    std::fs::create_dir_all(into.join(".meridian")).unwrap();
    std::fs::write(into.join(".meridian/dev.jsonl"), "{}\n").unwrap();
    let sent = crate::live::scan(&into, &crate::live::Ignored::of(&into));
    assert!(
        sent.keys().all(|path| !path.contains("CLAUDE")
            && !path.contains("AGENTS")
            && !path.starts_with(".claude")
            && !path.starts_with(".meridian")),
        "{:?}",
        sent.keys().collect::<Vec<_>>()
    );
    assert!(sent.contains_key("pyproject.toml"));
}

use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("meridian-live-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, path: &str, text: &str) {
    let at = dir.join(path);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::fs::write(at, text).unwrap();
}

#[test]
fn what_a_build_leaves_and_what_dockerignore_names_is_never_sent() {
    let ignored = Ignored::from_dockerignore(
        "# comment\n*.log\ntests/fixtures/\n/notes\n**/scratch\n!keep.log\nsrc/*.tmp\n",
    );
    for held in [
        ".git",
        "src/pkg/__pycache__",
        "reference_plugin.egg-info",
        "src/x.pyc",
        ".venv",
        "run.log",
        "tests/fixtures",
        "tests/fixtures/a.json",
        "notes",
        "src/scratch",
    ] {
        assert!(ignored.holds(held), "{held} should not be sent");
    }
    for sent in [
        "pyproject.toml",
        "src/reference_plugin/page.py",
        "tests/test_page.py",
        "keep.log.txt",
        // A pattern it cannot read is sent rather than guessed at.
        "src/a.tmp",
    ] {
        assert!(!ignored.holds(sent), "{sent} should be sent");
    }
}

#[test]
fn a_change_is_the_files_that_differ_and_the_ones_gone() {
    let dir = scratch("change");
    write(&dir, "pyproject.toml", "[project]\n");
    write(&dir, "src/pkg/page.py", "TITLE = 'one'\n");
    write(&dir, "src/pkg/__pycache__/page.pyc", "bytes");
    write(&dir, "src/pkg/old.py", "gone soon\n");
    let ignored = Ignored::of(&dir);

    let first = scan(&dir, &ignored);
    assert_eq!(
        first.keys().collect::<Vec<_>>(),
        ["pyproject.toml", "src/pkg/old.py", "src/pkg/page.py"]
    );
    let everything = change(&dir, &Snapshot::new(), &first).unwrap();
    assert_eq!(everything.files.len(), 3);
    assert_eq!(
        everything.files["src/pkg/page.py"],
        STANDARD.encode("TITLE = 'one'\n")
    );
    assert!(everything.deleted.is_empty());

    std::thread::sleep(Duration::from_millis(20));
    write(&dir, "src/pkg/page.py", "TITLE = 'two, longer'\n");
    std::fs::remove_file(dir.join("src/pkg/old.py")).unwrap();
    let second = scan(&dir, &ignored);
    let changed = change(&dir, &first, &second).unwrap();
    assert_eq!(
        changed.files.keys().collect::<Vec<_>>(),
        ["src/pkg/page.py"]
    );
    assert_eq!(changed.deleted, ["src/pkg/old.py"]);

    assert!(change(&dir, &second, &scan(&dir, &ignored))
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_change_too_large_to_be_source_is_refused_with_where_to_say_so() {
    let dir = scratch("large");
    std::fs::write(dir.join("data.bin"), vec![0u8; (MOST + 1) as usize]).unwrap();
    let refused = change(&dir, &Snapshot::new(), &scan(&dir, &Ignored::of(&dir))).unwrap_err();
    assert!(refused.contains(".dockerignore"), "{refused}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_lapsed_session_is_said_apart_with_what_to_run() {
    let lapsed = refusal(
        "https://meridian.example",
        reqwest::StatusCode::UNAUTHORIZED,
        "session lapsed: idle for 30 minutes",
    );
    assert_eq!(lapsed.code(), 3);
    assert!(
        lapsed
            .said()
            .contains("`meridian connect https://meridian.example`"),
        "{}",
        lapsed.said()
    );
    let refused = refusal(
        "https://meridian.example",
        reqwest::StatusCode::NOT_FOUND,
        r#"{"error":"reference-plugin is not live"}"#,
    );
    assert_eq!(refused.code(), 1);
    assert_eq!(
        refused.said(),
        "404 Not Found: reference-plugin is not live"
    );
}

#[test]
fn each_event_is_reported_once_whatever_the_polls_return() {
    let said = serde_json::json!({ "revision": 2, "events": [
        { "revision": 1, "event": "synced", "at": 1.0, "by": "sidecar", "files": 2, "deleted": 0 },
        { "revision": 1, "event": "ready", "at": 2.0 },
    ]});
    let mut seen = Seen::default();
    assert_eq!(seen.new_in(&said).len(), 2);
    assert!(seen.new_in(&said).is_empty());
    let more = serde_json::json!({ "events": [
        { "revision": 1, "event": "ready", "at": 2.0 },
        { "revision": 2, "event": "restarted", "at": 3.0, "pid": 9 },
    ]});
    let fresh = seen.new_in(&more);
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0]["event"], "restarted");
}

#[test]
fn an_event_reads_with_its_revision_first() {
    let lines: Vec<String> = [
        serde_json::json!({ "revision": 3, "event": "synced", "files": 1, "deleted": 2 }),
        serde_json::json!({ "revision": 3, "event": "ready" }),
        serde_json::json!({ "revision": 4, "event": "refused", "reason": "no grant for x" }),
        serde_json::json!({ "revision": 5, "event": "crashed", "exit": 1,
                            "traceback": "Traceback\nRuntimeError: broken" }),
    ]
    .iter()
    .map(line)
    .collect();
    assert_eq!(
        lines,
        [
            "r3 synced (1 sent, 2 deleted)",
            "r3 ready",
            "r4 refused: no grant for x",
            "r5 crashed, exit 1\n    Traceback\n    RuntimeError: broken",
        ]
    );
}

#[test]
fn a_path_is_carried_as_one_query_value() {
    assert_eq!(query_escaped("/"), "/");
    assert_eq!(
        query_escaped("/orders?id=7&x=a b"),
        "/orders%3Fid%3D7%26x%3Da%20b"
    );
}

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
fn the_harness_the_plugins_e2e_copies_into_dot_e2e_is_never_sent() {
    let dir = scratch("e2e");
    write(&dir, "pyproject.toml", "[project]\n");
    write(&dir, "src/pkg/page.py", "TITLE = 'one'\n");
    write(&dir, ".e2e/harness/harness.py", "import os\n");
    write(&dir, ".e2e/harness/README.md", "core's harness\n");
    // Without a .dockerignore naming it: the fixed list holds it.
    assert!(!dir.join(".dockerignore").exists());

    let sent = scan(&dir, &Ignored::of(&dir));
    assert_eq!(
        sent.keys().collect::<Vec<_>>(),
        ["pyproject.toml", "src/pkg/page.py"]
    );
    let _ = std::fs::remove_dir_all(&dir);
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
fn the_dashboards_missing_session_is_said_by_its_reason_with_connect() {
    let local = "http://meridian.localhost";
    for (reason, why) in [
        ("lapsed", format!("your session with {local} lapsed")),
        ("ended", format!("your session with {local} was ended")),
        (
            "unknown",
            format!("{local} does not know your session; it may have restarted"),
        ),
    ] {
        let body = format!(r#"{{"error":"invalid_token","reason":"{reason}"}}"#);
        let failed = refusal(local, reqwest::StatusCode::UNAUTHORIZED, &body);
        assert_eq!(
            failed,
            Failed::Session(format!("{why}: `meridian connect` to sign in again"))
        );
        assert_eq!(failed.code(), 3);
    }
    // A 401 with nothing to say why is the session all the same.
    let bare = refusal(local, reqwest::StatusCode::UNAUTHORIZED, "");
    assert_eq!(bare.code(), 3);
    assert!(
        bare.said().ends_with("`meridian connect` to sign in again"),
        "{}",
        bare.said()
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

#[test]
fn every_spelling_the_dashboard_takes_is_a_level_and_nothing_else() {
    for (named, level) in [
        ("manage", Level::Manage),
        ("Admin", Level::Manage),
        ("open", Level::Open),
        ("WRITE", Level::Open),
        (" view ", Level::View),
        ("read", Level::View),
    ] {
        assert_eq!(Level::named(named), Some(level), "{named}");
    }
    for named in ["", "owner", "readwrite", "manager", "r"] {
        assert_eq!(Level::named(named), None, "{named:?}");
    }
    // Sent as the dashboard names it, whichever spelling was given.
    assert_eq!(Level::Manage.name(), "admin");
    assert_eq!(Level::Open.name(), "write");
    assert_eq!(Level::View.name(), "read");
    assert_eq!(Level::Manage.said(), "Manage (admin)");
}

#[test]
fn a_refusal_at_the_level_the_dashboard_chose_says_how_to_ask_at_another() {
    assert_eq!(
        page_failed("ref", "/", 403, None, Some(Level::Manage)),
        "ref answered 403 for / at Manage (admin), the first level you hold; a page serves \
         only the levels it is declared with, and `--level open` or `--level view` asks at \
         another"
    );
    // Asked for, the level is the person's choice, and said as it is.
    assert_eq!(
        page_failed("ref", "/setup", 403, Some(Level::Open), Some(Level::Open)),
        "ref answered 403 for /setup at Open (write)"
    );
    // Not a refusal of the level.
    assert_eq!(
        page_failed("ref", "/", 500, None, Some(Level::View)),
        "ref answered 500 for / at View (read)"
    );
    // A dashboard older than levels names none, and none is said.
    assert_eq!(
        page_failed("ref", "/", 403, None, None),
        "ref answered 403 for /"
    );
}

/// A dashboard that answers one request with `body`, and hands back the
/// request line it was asked.
async fn answering_once(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let asked = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut read = Vec::new();
        let mut chunk = [0u8; 4096];
        while !read.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0, "the request ended before its head did");
            read.extend_from_slice(&chunk[..n]);
        }
        let answer = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(answer.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&read)
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    });
    (address, asked)
}

#[tokio::test]
async fn open_carries_the_level_asked_and_reads_the_one_it_was_opened_at() {
    let (address, asked) =
        answering_once(r#"{"instance_id":"ref","level":"write","url":"http://ref/x"}"#).await;
    let deployment = Deployment {
        address: &address,
        session: "s",
    };
    let opened = deployment.open("ref", Some(Level::Open)).await.unwrap();
    assert_eq!(opened.url, "http://ref/x");
    assert_eq!(opened.level, Some(Level::Open));
    assert_eq!(
        asked.await.unwrap(),
        "POST /terminal/plugins/ref/open?level=write HTTP/1.1"
    );
}

#[tokio::test]
async fn open_with_no_level_names_none_and_leaves_the_dashboard_to_choose() {
    let (address, asked) = answering_once(r#"{"url":"http://ref/x"}"#).await;
    let deployment = Deployment {
        address: &address,
        session: "s",
    };
    let opened = deployment.open("ref", None).await.unwrap();
    assert_eq!(opened.level, None);
    assert_eq!(
        asked.await.unwrap(),
        "POST /terminal/plugins/ref/open HTTP/1.1"
    );
}

#[tokio::test]
async fn a_page_is_read_at_the_level_asked_after_its_path() {
    let (address, asked) =
        answering_once(r#"{"status":200,"level":"admin","body":"<p>x</p>"}"#).await;
    let deployment = Deployment {
        address: &address,
        session: "s",
    };
    let said = deployment
        .page("ref", "/setup?tab=a", Some(Level::Manage))
        .await
        .unwrap();
    assert_eq!(said["level"], "admin");
    assert_eq!(
        asked.await.unwrap(),
        "GET /terminal/plugins/ref/page?path=/setup%3Ftab%3Da&level=admin HTTP/1.1"
    );
}

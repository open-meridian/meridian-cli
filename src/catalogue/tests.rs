use super::*;

const TEMPLATE: &str = include_str!("../../plugin-template/pyproject.toml");

#[test]
fn the_templates_pyproject_is_metadata() {
    let held = metadata(TEMPLATE).unwrap();
    assert_eq!(
        held,
        Metadata {
            name: "reference-plugin".into(),
            version: "0.1.0".into(),
            roles: vec![],
            interface: true,
            sdk_version: "0.10.1".into(),
        }
    );
}

#[test]
fn roles_are_read_and_held_to_their_form() {
    let with = TEMPLATE.replace("roles = []", "roles = [\"custody\", \"reporting\"]");
    let held = metadata(&with).unwrap();
    assert_eq!(held.roles, ["custody", "reporting"]);

    let refused = metadata(&TEMPLATE.replace("roles = []", "roles = [\"Custody\"]")).unwrap_err();
    assert!(refused.contains("`Custody`"), "{refused}");
    let refused = metadata(&TEMPLATE.replace("roles = []", "roles = \"custody\"")).unwrap_err();
    assert!(refused.contains("roles is not a list"), "{refused}");
}

#[test]
fn a_pyproject_declaring_tags_is_refused_citing_the_decision() {
    // decisions/026: a plugin declares no tags. One made from an older
    // template still says `tags = []`; empty or not, it is a declaration.
    assert!(!TEMPLATE.contains("tags"), "the template declares none");
    for tags in ["tags = []", "tags = [\"holdings\"]"] {
        let declaring = TEMPLATE.replace("roles = []", &format!("roles = []\n{tags}"));
        let refused = metadata(&declaring).unwrap_err();
        assert!(
            refused.contains("declares `tags`") && refused.contains("decisions/026"),
            "{refused}"
        );
    }
}

#[test]
fn a_pyproject_missing_what_upload_sends_is_refused() {
    let unpinned = TEMPLATE.replace("open-meridian==0.10.1", "open-meridian>=0.3");
    assert!(metadata(&unpinned)
        .unwrap_err()
        .contains("pin open-meridian"));
    let undeclared = &TEMPLATE[..TEMPLATE.find("[tool.meridian]").unwrap()];
    assert!(metadata(undeclared)
        .unwrap_err()
        .contains("[tool.meridian]"));
    let misnamed = TEMPLATE.replace("name = \"reference-plugin\"", "name = \"Reference\"");
    assert!(metadata(&misnamed)
        .unwrap_err()
        .contains("not a plugin's name"));
    let unversioned = TEMPLATE.replace("version = \"0.1.0\"\n", "");
    assert!(metadata(&unversioned).unwrap_err().contains("no version"));
}

#[test]
fn names_are_host_labels() {
    for good in ["a", "reference-plugin", "p2", &"a".repeat(63)] {
        assert!(is_name(good), "{good}");
    }
    for bad in [
        "",
        "2p",
        "-a",
        "a-",
        "a--b",
        "A",
        "a_b",
        "a.b",
        &"a".repeat(64),
    ] {
        assert!(!is_name(bad), "{bad}");
    }
}

/// A layout as `docker save` writes one: an index naming an index naming a
/// manifest, and the blobs.
fn layout(dir: &Path, drop_config: bool) -> (String, Vec<String>) {
    let blobs = dir.join("blobs").join("sha256");
    std::fs::create_dir_all(&blobs).unwrap();
    let put = |bytes: &[u8]| {
        let hex = format!("{:x}", Sha256::digest(bytes));
        std::fs::write(blobs.join(&hex), bytes).unwrap();
        format!("sha256:{hex}")
    };
    let config = put(b"{\"architecture\":\"arm64\"}");
    let layer = put(b"a layer");
    if drop_config {
        std::fs::remove_file(blob_path(dir, &config).unwrap()).unwrap();
    }
    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"digest": config},
        "layers": [{"digest": layer}],
    })
    .to_string();
    let manifest_digest = put(manifest.as_bytes());
    // An attestation the save left out: named, not present, passed over.
    let missing = format!("sha256:{}", "0".repeat(64));
    let index = serde_json::json!({
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{"digest": missing}, {"digest": manifest_digest}],
    })
    .to_string();
    let index_digest = put(index.as_bytes());
    std::fs::write(
        dir.join("index.json"),
        serde_json::json!({"manifests": [{"digest": index_digest,
            "mediaType": "application/vnd.oci.image.index.v1+json"}]})
        .to_string(),
    )
    .unwrap();
    (manifest_digest, vec![config, layer])
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meridian-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn the_image_is_the_manifest_the_layout_holds() {
    let dir = scratch("layout");
    let (digest, blobs) = layout(&dir, false);
    let image = image(&dir).unwrap();
    assert_eq!(image.digest, digest);
    assert_eq!(
        image.media_type,
        "application/vnd.oci.image.manifest.v1+json"
    );
    assert_eq!(
        image
            .blobs
            .iter()
            .map(|(d, _)| d.clone())
            .collect::<Vec<_>>(),
        blobs
    );
    assert_eq!(
        format!("sha256:{:x}", Sha256::digest(&image.manifest)),
        digest
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_layout_missing_a_blob_is_refused() {
    let dir = scratch("short");
    layout(&dir, true);
    assert!(image(&dir).unwrap_err().contains("does not hold"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_manifest_that_is_not_its_digest_is_refused() {
    let dir = scratch("tampered");
    let (digest, _) = layout(&dir, false);
    let path = blob_path(&dir, &digest).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(b' ');
    std::fs::write(&path, bytes).unwrap();
    assert!(image(&dir).unwrap_err().contains("are not"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_digest_is_never_a_path() {
    assert!(blob_path(Path::new("/x"), "sha256:../../etc/passwd").is_err());
    assert!(blob_path(Path::new("/x"), "sha512:ab").is_err());
}

#[test]
fn an_upload_goes_where_the_dashboard_said_with_its_digest() {
    let at = "http://127.0.0.1:8443";
    assert_eq!(
        upload_url(at, "/terminal/registry/v2/plugins/p/blobs/uploads/u1?_state=s", "sha256:ab"),
        "http://127.0.0.1:8443/terminal/registry/v2/plugins/p/blobs/uploads/u1?_state=s&digest=sha256:ab"
    );
    assert_eq!(
        upload_url(
            at,
            "/terminal/registry/v2/plugins/p/blobs/uploads/u1",
            "sha256:ab"
        ),
        "http://127.0.0.1:8443/terminal/registry/v2/plugins/p/blobs/uploads/u1?digest=sha256:ab"
    );
    assert_eq!(
        upload_url(at, "https://elsewhere/u?x=1", "sha256:ab"),
        "https://elsewhere/u?x=1&digest=sha256:ab"
    );
}

#[test]
fn a_launch_approves_what_the_version_declared() {
    let held = serde_json::json!({
        "versions": [
            {"name": "p", "version": "0.1.0", "roles": ["custody"]},
            {"name": "p", "version": "0.2.0", "roles": []},
        ],
        "launches": [{"instance_id": "p", "name": "p", "version": "0.1.0",
                      "state": "failed", "failure": "no image"}],
    });
    assert_eq!(
        declared(&held, "p", "0.1.0"),
        Some(vec!["custody".to_string()])
    );
    assert_eq!(declared(&held, "p", "0.3.0"), None);
    let said = listed(&held);
    assert!(said.contains("p 0.1.0  roles: custody  page: no"), "{said}");
    assert!(said.contains("p 0.2.0  roles: none  page: no"), "{said}");
    assert!(said.contains("p  p 0.1.0  failed: no image"), "{said}");
}

#[test]
fn a_blob_another_plugin_holds_is_mounted_from_its_repository() {
    assert_eq!(
        mount_url(
            "http://localhost:18480/terminal/registry/v2/plugins/reference-custody",
            "sha256:ab",
            "reference-plugin"
        ),
        "http://localhost:18480/terminal/registry/v2/plugins/reference-custody\
         /blobs/uploads/?mount=sha256:ab&from=plugins/reference-plugin"
    );
}

/// A dashboard stand-in on loopback: each request answered by `answer`, from
/// its method and path, and the paths asked for kept in order.
async fn dashboard(
    answer: fn(&str, &str) -> (u16, &'static str),
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = asked.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let kept = kept.clone();
            tokio::spawn(async move {
                let mut read = Vec::new();
                let mut chunk = [0u8; 8192];
                // The head, then as much body as it says there is.
                let (head_end, length) = loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    read.extend_from_slice(&chunk[..n]);
                    if let Some(end) = read.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&read[..end]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while read.len() < head_end + length {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    read.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&read[..head_end]).to_string();
                let mut first = head.lines().next().unwrap_or_default().split(' ');
                let method = first.next().unwrap_or_default().to_string();
                let path = first.next().unwrap_or_default().to_string();
                kept.lock().unwrap().push(format!("{method} {path}"));
                let (status, body) = answer(&method, &path);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    if method == "HEAD" { "" } else { body }
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (address, asked)
}

/// What the dashboard answers every `/terminal/` path with when it holds no
/// such session: here, the one after it restarted.
const NO_SESSION: &str = r#"{"error":"invalid_token","reason":"unknown"}"#;

fn a_plugin() -> Metadata {
    metadata(TEMPLATE).unwrap()
}

#[tokio::test]
async fn an_upload_on_a_session_the_dashboard_lost_is_the_session_and_exits_3() {
    // As reported: after a dashboard restart, the registry proxy answered the
    // upload's start with 401, and upload said "the registry did not start an
    // upload: 401 Unauthorized: invalid_token" and exited 1.
    let (address, asked) = dashboard(|method, path| match method {
        "HEAD" => (404, ""),
        _ if path.starts_with("/terminal/") => (401, NO_SESSION),
        _ => (404, "{}"),
    })
    .await;
    let dir = scratch("lapsed-upload");
    layout(&dir, false);
    let failed = push(&address, "stale", &a_plugin(), &image(&dir).unwrap())
        .await
        .unwrap_err();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(failed.code(), 3, "{failed:?}");
    assert_eq!(
        failed.said(),
        format!(
            "{address} does not know your session; it may have restarted: \
             `meridian connect {address}` to sign in again"
        )
    );
    let asked = asked.lock().unwrap().clone();
    assert!(
        asked.iter().any(|line| line
            .starts_with("POST /terminal/registry/v2/plugins/reference-plugin/blobs/uploads/")),
        "{asked:?}"
    );
}

#[tokio::test]
async fn a_lapsed_session_is_seen_on_the_first_look_at_the_registry() {
    // A HEAD has no body to say why; its 401 is the session all the same, and
    // nothing is sent after it.
    let (address, asked) = dashboard(|_, _| (401, NO_SESSION)).await;
    let dir = scratch("lapsed-head");
    layout(&dir, false);
    let failed = push(&address, "stale", &a_plugin(), &image(&dir).unwrap())
        .await
        .unwrap_err();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(failed.code(), 3, "{failed:?}");
    assert!(
        failed
            .said()
            .ends_with(&format!("`meridian connect {address}` to sign in again")),
        "{failed:?}"
    );
    let asked = asked.lock().unwrap().clone();
    assert!(
        asked
            .iter()
            .all(|line| line.starts_with("HEAD ") || line.starts_with("GET /terminal/plugins")),
        "{asked:?}"
    );
}

#[tokio::test]
async fn every_catalogue_call_says_a_lapsed_session_as_the_session() {
    let (address, _) =
        dashboard(|_, _| (401, r#"{"error":"invalid_token","reason":"lapsed"}"#)).await;
    let lapsed = format!(
        "your session with {address} lapsed: `meridian connect {address}` to sign in again"
    );
    let failed = catalogue(&address, "stale").await.unwrap_err();
    assert_eq!(failed, Failed::Session(lapsed.clone()));
    let failed = record(&address, "stale", &a_plugin(), "sha256:ab")
        .await
        .unwrap_err();
    assert_eq!(failed, Failed::Session(lapsed.clone()));
    let failed = stop(&address, "stale", "reference-plugin")
        .await
        .unwrap_err();
    assert_eq!(failed, Failed::Session(lapsed.clone()));
    let asked = Launch {
        name: "reference-plugin",
        version: "0.1.0",
        instance: "reference-plugin",
        roles: &[],
        live: false,
    };
    let failed = launch(&address, "stale", &asked).await.unwrap_err();
    assert_eq!(failed, Failed::Session(lapsed));
}

#[tokio::test]
async fn anything_else_the_registry_refuses_is_a_refusal_after_what_was_being_done() {
    let (address, _) = dashboard(|method, _| match method {
        "HEAD" => (404, ""),
        _ => (503, r#"{"error":"the registry is not reachable"}"#),
    })
    .await;
    let dir = scratch("registry-down");
    layout(&dir, false);
    let failed = push(&address, "live", &a_plugin(), &image(&dir).unwrap())
        .await
        .unwrap_err();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        failed,
        Failed::Refused(
            "the registry did not start an upload: 503 Service Unavailable: \
             the registry is not reachable"
                .into()
        )
    );
}

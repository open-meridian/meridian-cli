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
            tags: vec![],
            interface: true,
            sdk_version: "0.2.0".into(),
        }
    );
}

#[test]
fn roles_and_tags_are_read_and_held_to_their_form() {
    let with = TEMPLATE
        .replace("roles = []", "roles = [\"custody\"]")
        .replace("tags = []", "tags = [\"positions\", \"orders\"]");
    let held = metadata(&with).unwrap();
    assert_eq!(held.roles, ["custody"]);
    assert_eq!(held.tags, ["positions", "orders"]);

    let refused = metadata(&TEMPLATE.replace("tags = []", "tags = [\"Positions\"]")).unwrap_err();
    assert!(refused.contains("`Positions`"), "{refused}");
    let refused = metadata(&TEMPLATE.replace("roles = []", "roles = \"custody\"")).unwrap_err();
    assert!(refused.contains("roles is not a list"), "{refused}");
}

#[test]
fn a_pyproject_missing_what_upload_sends_is_refused() {
    let unpinned = TEMPLATE.replace("open-meridian==0.2.0", "open-meridian>=0.2");
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
            {"name": "p", "version": "0.1.0", "roles": ["custody"], "tags": ["t"]},
            {"name": "p", "version": "0.2.0", "roles": [], "tags": []},
        ],
        "launches": [{"instance_id": "p", "name": "p", "version": "0.1.0",
                      "state": "failed", "failure": "no image"}],
    });
    assert_eq!(
        declared(&held, "p", "0.1.0"),
        Some((vec!["custody".to_string()], vec!["t".to_string()]))
    );
    assert_eq!(declared(&held, "p", "0.3.0"), None);
    let said = listed(&held);
    assert!(said.contains("p 0.1.0  roles: custody  tags: t"), "{said}");
    assert!(said.contains("p 0.2.0  roles: none  tags: none"), "{said}");
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

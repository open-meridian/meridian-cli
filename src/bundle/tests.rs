use std::sync::{Arc, Mutex};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{EcdsaKeyPair, KeyPair as _, ECDSA_P256_SHA256_ASN1_SIGNING};

use super::*;
use crate::fake_http::{serve, Asked, Reply, Scratch};

/// The platform's key pair, as the tests stand it in: its private half, and
/// its public half as the PEM this release would carry.
struct Platform {
    pair: EcdsaKeyPair,
}

impl Platform {
    fn new() -> Platform {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        Platform {
            pair: EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref())
                .unwrap(),
        }
    }

    fn pem(&self) -> String {
        let mut der = vec![
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
        ];
        der.extend_from_slice(self.pair.public_key().as_ref());
        format!(
            "-----BEGIN PUBLIC SGK-----\n{}\n-----END PUBLIC SGK-----\n",
            STANDARD.encode(der)
        )
    }

    fn sign(&self, message: &[u8]) -> String {
        STANDARD.encode(
            self.pair
                .sign(&SystemRandom::new(), message)
                .unwrap()
                .as_ref(),
        )
    }

    /// A DSSE envelope over an in-toto statement, as the platform signs one.
    fn envelope(
        &self,
        predicate_type: &str,
        digest: &str,
        predicate: serde_json::Value,
    ) -> Vec<u8> {
        let statement = serde_json::json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [{"name": "ghcr.io/open-meridian/plugins/acme/venue-desk",
                         "digest": {"sha256": digest.trim_start_matches("sha256:")}}],
            "predicateType": predicate_type,
            "predicate": predicate,
        });
        let body = canonical(&statement).into_bytes();
        serde_json::json!({
            "payloadType": IN_TOTO,
            "payload": STANDARD.encode(&body),
            "signatures": [{"keyid": "sha256:k", "sig": self.sign(&pae(IN_TOTO, &body))}],
        })
        .to_string()
        .into_bytes()
    }

    fn key_status(&self, revoked_before_listing: bool) -> serde_json::Value {
        let mut key = serde_json::json!({
            "key_id": "SGK-1", "organisation": "acme", "name": "release key — 2026",
            "fingerprint": "SHA256:q3Hn", "registered_at_ns": 1791000000000000000u64,
        });
        if revoked_before_listing {
            key["revoked_at_ns"] = 1792000000000000000u64.into();
            key["not_trusted_after_ns"] = 1791500000000000000u64.into();
        }
        let mut status = serde_json::json!({"issued_at_ns": 1792108000000000000u64, "keys": [key]});
        let signature = self.sign(&pae(KEY_STATUS_TYPE, canonical(&status).as_bytes()));
        status["signature"] = signature.into();
        status
    }
}

#[test]
fn canonical_json_is_the_platforms() {
    // Python's json.dumps(sort_keys=True, separators=(",", ":")).
    let value =
        serde_json::json!({"b": 1, "a": {"z": "é\n\u{1F600}\"", "y": [1, "x", null, true]}});
    assert_eq!(
        canonical(&value),
        concat!(
            r#"{"a":{"y":[1,"x",null,true],"z":""#,
            "\\u00e9\\n\\ud83d\\ude00\\\"",
            r#""},"b":1}"#
        )
    );
}

#[test]
fn a_signature_verifies_with_the_platforms_key_and_no_other() {
    let platform = Platform::new();
    let other = Platform::new();
    let message = pae(KEY_STATUS_TYPE, b"{}");
    let signature = platform.sign(&message);
    assert!(signed_by(&[platform.pem()], &message, &signature));
    assert!(!signed_by(&[other.pem()], &message, &signature));
    assert!(
        signed_by(&[other.pem(), platform.pem()], &message, &signature),
        "the next key too"
    );
    assert!(!signed_by(&[platform.pem()], b"something else", &signature));
    assert!(key_status_signed(
        &platform.key_status(false),
        &[platform.pem()]
    ));
    let mut forged = platform.key_status(false);
    forged["keys"][0]["revoked_at_ns"] = 0.into();
    forged["keys"][0]["name"] = "another".into();
    assert!(!key_status_signed(&forged, &[platform.pem()]));
}

/// An image, its referrers and the key status, as the platform and its
/// registry hold them.
struct Listed {
    bundle: Bundle,
    platform: Platform,
}

fn layer(bytes: &[u8]) -> serde_json::Value {
    serde_json::json!({"mediaType": "application/vnd.oci.image.layer.v1.tar", "digest": sha256(bytes), "size": bytes.len()})
}

fn listed(version_in_statement: &str, revoked: bool) -> Listed {
    let platform = Platform::new();
    let mut blobs = BTreeMap::new();
    let config = br#"{"architecture":"arm64","os":"linux"}"#.to_vec();
    let content = b"the plugin's layer".to_vec();
    let manifest = serde_json::json!({
        "schemaVersion": 2, "mediaType": OCI_MANIFEST,
        "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": sha256(&config), "size": config.len()},
        "layers": [layer(&content)],
    })
    .to_string()
    .into_bytes();
    let digest = sha256(&manifest);
    blobs.insert(sha256(&config), config);
    blobs.insert(sha256(&content), content);
    blobs.insert(digest.clone(), manifest);
    let mut referrers = Vec::new();
    for (predicate_type, predicate) in [
        (
            PROVENANCE,
            serde_json::json!({"repository": "acme/plugin-venue-desk", "commit": "4e1d", "tag": "v0.1.0", "signer": "SHA256:q3Hn"}),
        ),
        (
            LISTING,
            serde_json::json!({"listing": "acme/venue-desk", "version": version_in_statement, "registry": "community", "listed_at": "2026-10-16T12:00:00.123456+00:00", "signer_key": "SHA256:q3Hn"}),
        ),
        (
            METADATA,
            serde_json::json!({
                "metadata": {"name": "venue-desk", "version": "0.1.0", "roles": ["custody"], "interface": true,
                             "sdk_version": "0.29.0", "declaration": {"documentation": [{"name": "overview"}]}},
                "documentation": [{"name": "overview", "title": "Overview", "text": "# Overview\n", "digest": "sha256:x"}],
            }),
        ),
    ] {
        let envelope = platform.envelope(predicate_type, &digest, predicate);
        let referrer = serde_json::json!({
            "schemaVersion": 2, "mediaType": OCI_MANIFEST,
            "artifactType": "application/vnd.dsse.envelope.v1+json",
            "config": {"mediaType": "application/vnd.oci.empty.v1+json", "digest": sha256(b"{}"), "size": 2},
            "layers": [{"mediaType": "application/vnd.dsse.envelope.v1+json", "digest": sha256(&envelope), "size": envelope.len()}],
            "subject": {"mediaType": OCI_MANIFEST, "digest": digest, "size": 1},
        })
        .to_string()
        .into_bytes();
        blobs.insert(sha256(b"{}"), b"{}".to_vec());
        blobs.insert(sha256(&envelope), envelope);
        referrers.push((sha256(&referrer), OCI_MANIFEST.to_string()));
        blobs.insert(sha256(&referrer), referrer);
    }
    let key_status = platform.key_status(revoked);
    Listed {
        bundle: Bundle {
            listing: "acme/venue-desk".into(),
            version: "0.1.0".into(),
            registry: "community".into(),
            image_digest: digest,
            image_media_type: OCI_MANIFEST.into(),
            blobs,
            referrers,
            key_status,
            ..Bundle::default()
        },
        platform,
    }
}

#[test]
fn a_listed_version_verifies_and_carries_its_metadata_and_texts() {
    let Listed {
        mut bundle,
        platform,
    } = listed("0.1.0", false);
    verify(&mut bundle, &[platform.pem()]).unwrap();
    assert_eq!(bundle.metadata["roles"][0], "custody");
    assert_eq!(
        bundle.documentation,
        serde_json::json!([{"section": "overview", "markdown": "# Overview\n"}])
    );
    let body = record_body(&bundle).unwrap();
    assert_eq!(body["name"], "venue-desk");
    assert_eq!(body["listing"], "acme/venue-desk");
    assert_eq!(body["source_registry"], "community");
    assert_eq!(body["key_status"]["keys"][0]["key_id"], "SGK-1");
    assert_eq!(body["documentation"][0]["section"], "overview");
    assert_eq!(body["sdk_version"], "0.29.0");
}

#[test]
fn a_bundle_that_does_not_verify_is_refused_naming_the_check() {
    let refused = |mut held: Bundle, keys: &[String]| verify(&mut held, keys).unwrap_err();
    let Listed { bundle, platform } = listed("0.1.0", false);
    let keys = [platform.pem()];
    assert!(refused(bundle.clone(), &[]).contains("none of the platform's keys"));
    assert!(refused(bundle.clone(), &[Platform::new().pem()]).contains("no provenance"));

    let mut tampered = bundle.clone();
    let layer = tampered
        .blobs
        .iter()
        .find(|(_, bytes)| bytes.as_slice() == b"the plugin's layer")
        .map(|(digest, _)| digest.clone())
        .unwrap();
    tampered
        .blobs
        .insert(layer.clone(), b"another layer".to_vec());
    assert_eq!(
        refused(tampered, &keys),
        format!("{layer} is not the bytes it is named by")
    );

    let mut missing = bundle.clone();
    missing.blobs.remove(&layer);
    assert!(refused(missing, &keys).contains("is missing"));

    let Listed {
        bundle: other,
        platform,
    } = listed("0.2.0", false);
    assert!(refused(other, &[platform.pem()])
        .contains("the listing statement says version \"0.2.0\", not 0.1.0"));

    let Listed {
        bundle: revoked,
        platform,
    } = listed("0.1.0", true);
    assert!(refused(revoked, &[platform.pem()]).contains("was revoked"));

    let mut forged = bundle.clone();
    forged.key_status["keys"][0]["name"] = "forged".into();
    assert!(refused(forged, &keys).contains("the key status is not the platform's"));
}

#[test]
fn a_bundle_is_written_once_and_reads_back_whole() {
    let Listed {
        mut bundle,
        platform,
    } = listed("0.1.0", false);
    verify(&mut bundle, &[platform.pem()]).unwrap();
    let scratch = Scratch::new("bundle");
    let file = scratch.0.join("acme-venue-desk-0.1.0.plugin-bundle.tar");
    write(&bundle, &file, 1_792_108_800_000_000_000).unwrap();
    assert!(write(&bundle, &file, 0)
        .unwrap_err()
        .contains("never written over"));
    let mut back = read(&file).unwrap();
    assert_eq!(back.image_digest, bundle.image_digest);
    assert_eq!(back.blobs, bundle.blobs);
    assert_eq!(back.referrers, bundle.referrers);
    verify(&mut back, &[platform.pem()]).unwrap();
    assert_eq!(back.documentation, bundle.documentation);
    assert!(read(&scratch.0.join("nothing.tar")).is_err());
}

#[test]
fn a_listing_is_organisation_plugin_at_version() {
    assert_eq!(
        named("acme/venue-desk@0.1.0").unwrap(),
        ("acme".into(), "venue-desk".into(), "0.1.0".into())
    );
    assert!(named("acme/venue-desk").is_err());
    assert!(named("venue-desk@0.1.0").is_err());
    assert!(named("Acme/venue-desk@0.1.0").is_err());
    assert!(named("acme/venue-desk@latest").is_err());
}

/// The platform and its registry, standing in together: a resolution, the
/// key status, and the image and its referrers behind an anonymous token.
async fn stand_in(
    listed: &Listed,
    statement: Option<serde_json::Value>,
) -> (String, crate::fake_http::Kept) {
    let blobs = listed.bundle.blobs.clone();
    let digest = listed.bundle.image_digest.clone();
    let referrers = listed.bundle.referrers.clone();
    let key_status = listed.bundle.key_status.clone();
    let address = Arc::new(Mutex::new(String::new()));
    let here = address.clone();
    let served = serve(move |asked: &Asked| {
        let path = asked.path.split('?').next().unwrap_or_default();
        let base = here.lock().unwrap().clone();
        let host = base.trim_start_matches("http://").to_string();
        let name = "/v2/open-meridian/plugins/acme/venue-desk/";
        if path.starts_with("/v2/") && asked.header("authorization") != Some("Bearer anonymous") {
            return Reply::json(401, serde_json::json!({"errors": []})).with(
                "www-authenticate",
                &format!(r#"Bearer realm="{base}/token",service="{host}",scope="repository:open-meridian/plugins/acme/venue-desk:pull""#),
            );
        }
        match path {
            "/token" => Reply::json(200, serde_json::json!({"token": "anonymous"})),
            "/api/v1/registry/listings/acme/venue-desk/versions/0.1.0/resolve" => {
                let mut reply = serde_json::json!({"image_digest": digest,
                                   "pull_from": format!("{host}/open-meridian/plugins/acme/venue-desk")});
                // DownloadListedVersionReply.listing_statement, a DSSE envelope
                // as JSON (the coordinator's ruling on Q1, 2026-10-16).
                if let Some(statement) = &statement {
                    reply["listing_statement"] = canonical(statement).into();
                }
                Reply::json(200, reply)
            }
            "/api/v1/registry/signing-keys" => Reply::json(200, key_status.clone()),
            p if p.starts_with(name) => {
                let rest = &p[name.len()..];
                if let Some(wanted) = rest.strip_prefix("referrers/") {
                    assert_eq!(wanted, digest);
                    let index = serde_json::json!({
                        "schemaVersion": 2, "mediaType": OCI_INDEX,
                        "manifests": referrers.iter().map(|(d, m)| serde_json::json!({"digest": d, "mediaType": m})).collect::<Vec<_>>(),
                    });
                    return Reply::bytes(200, OCI_INDEX, index.to_string().into_bytes());
                }
                let wanted = rest.trim_start_matches("manifests/").trim_start_matches("blobs/");
                match blobs.get(wanted) {
                    Some(bytes) => Reply::bytes(200, OCI_MANIFEST, bytes.clone()),
                    None => Reply::json(404, serde_json::json!({"errors": []})),
                }
            }
            _ => Reply::json(404, serde_json::json!({"error": "no such row"})),
        }
    })
    .await;
    *address.lock().unwrap() = served.0.clone();
    served
}

#[tokio::test]
async fn a_download_resolves_pulls_verifies_and_writes_the_bundle() {
    let listed = listed("0.1.0", false);
    let (address, kept) = stand_in(&listed, None).await;
    // A sign-in already kept for the stand-in platform, so no browser opens.
    let scratch = Scratch::new("download");
    let within = scratch.0.join("platform");
    crate::sessions::write(
        &within,
        &crate::sessions::Held {
            address: address.clone(),
            client_id: crate::registry::CLIENT_ID.into(),
            access_token: "A-1".into(),
            access_expires_at_s: crate::credential::now_s() + 600,
            refresh_token: "R-1".into(),
            expires_at_s: crate::credential::now_s() + 86_400,
            ..Default::default()
        },
    )
    .unwrap();
    let signed_in = crate::registry::SignedIn::within(within, &address, Box::new(|_: &str| {}))
        .await
        .unwrap();
    let to = scratch.0.join("venue-desk.plugin-bundle.tar");
    let asked = Download {
        organisation: "acme".into(),
        plugin: "venue-desk".into(),
        version: "0.1.0".into(),
        registry: "community".into(),
        to: to.clone(),
    };
    let held = download(&asked, &signed_in, &[listed.platform.pem()])
        .await
        .unwrap();
    assert_eq!(held.image_digest, listed.bundle.image_digest);
    assert!(to.is_file());
    let resolve = kept
        .lock()
        .unwrap()
        .iter()
        .find(|a| a.path.ends_with("/resolve"))
        .cloned()
        .unwrap();
    assert_eq!(resolve.header("authorization"), Some("Bearer A-1"));
    assert_eq!(
        resolve.json(),
        serde_json::json!({"listing": "acme/venue-desk", "version": "0.1.0", "registry": "community"})
    );

    // Not twice over the same file, and nothing written when a check fails.
    assert!(download(&asked, &signed_in, &[listed.platform.pem()])
        .await
        .unwrap_err()
        .contains("there already"));
    let elsewhere = Download {
        to: scratch.0.join("unverified.tar"),
        ..asked
    };
    assert!(download(&elsewhere, &signed_in, &[Platform::new().pem()])
        .await
        .is_err());
    assert!(!elsewhere.to.exists());
}

#[tokio::test]
async fn a_bundle_is_carried_into_the_deployment_under_its_listing() {
    let Listed {
        mut bundle,
        platform,
    } = listed("0.1.0", false);
    verify(&mut bundle, &[platform.pem()]).unwrap();
    let scratch = Scratch::new("carry");
    let file = scratch.0.join("venue-desk.plugin-bundle.tar");
    write(&bundle, &file, 1).unwrap();
    let (address, kept) = serve(|asked: &Asked| match asked.method.as_str() {
        "HEAD" => Reply::json(404, serde_json::json!({})),
        "POST" if asked.path.ends_with("/blobs/uploads/") => {
            Reply::json(202, serde_json::json!({}))
                .with("location", "/terminal/registry/v2/upload/1")
        }
        _ => Reply::json(201, serde_json::json!({})),
    })
    .await;
    let session = Credential::terminal_session(&address, "a-session");
    let digest = upload(&address, &session, &file, &[platform.pem()])
        .await
        .unwrap();
    assert_eq!(digest, bundle.image_digest);
    let asked: Vec<Asked> = kept.lock().unwrap().clone();
    let repository = "/terminal/registry/v2/plugins/acme/venue-desk";
    assert!(asked
        .iter()
        .any(|a| a.method == "PUT" && a.path == format!("{repository}/manifests/0.1.0")));
    for (referrer, _) in &bundle.referrers {
        assert!(asked
            .iter()
            .any(|a| a.method == "PUT" && a.path == format!("{repository}/manifests/{referrer}")));
    }
    let tag = format!("sha256-{}", hex_of(&bundle.image_digest).unwrap());
    assert!(asked
        .iter()
        .any(|a| a.method == "PUT" && a.path == format!("{repository}/manifests/{tag}")));
    let recorded = asked
        .iter()
        .find(|a| a.method == "POST" && a.path == "/terminal/plugins")
        .unwrap()
        .json();
    assert_eq!(recorded["listing"], "acme/venue-desk");
    assert_eq!(recorded["source_registry"], "community");
    assert_eq!(recorded["image_digest"], bundle.image_digest);

    // A bundle that does not verify carries nothing in.
    let before = kept.lock().unwrap().len();
    assert!(upload(&address, &session, &file, &[Platform::new().pem()])
        .await
        .is_err());
    assert_eq!(kept.lock().unwrap().len(), before);
}

#[test]
fn the_listing_statement_may_come_as_the_listed_versions_rather_than_a_referrer() {
    // DESIGN25 at b0c4459: ListedVersion.listing_statement, a DSSE envelope
    // as JSON, beside the image's referrer.
    let Listed {
        mut bundle,
        platform,
    } = listed("0.1.0", false);
    let at = bundle
        .referrers
        .iter()
        .position(|(digest, _)| {
            let manifest: serde_json::Value =
                serde_json::from_slice(&bundle.blobs[digest]).unwrap();
            let layer = manifest["layers"][0]["digest"].as_str().unwrap();
            let envelope: serde_json::Value = serde_json::from_slice(&bundle.blobs[layer]).unwrap();
            predicate_of(&envelope) == LISTING
        })
        .unwrap();
    let (digest, _) = bundle.referrers.remove(at);
    let manifest: serde_json::Value = serde_json::from_slice(&bundle.blobs[&digest]).unwrap();
    let layer = manifest["layers"][0]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let envelope: serde_json::Value = serde_json::from_slice(&bundle.blobs[&layer]).unwrap();

    let mut without = bundle.clone();
    assert!(verify(&mut without, &[platform.pem()])
        .unwrap_err()
        .contains("no listing statement"));

    bundle.listing_statement = Some(envelope.clone());
    verify(&mut bundle, &[platform.pem()]).unwrap();
    let body = record_body(&bundle).unwrap();
    let carried: serde_json::Value =
        serde_json::from_str(body["listing_statement"].as_str().unwrap()).unwrap();
    assert_eq!(carried, envelope);

    let scratch = Scratch::new("statement");
    let file = scratch.0.join("b.tar");
    write(&bundle, &file, 1).unwrap();
    let mut back = read(&file).unwrap();
    assert_eq!(back.listing_statement, Some(envelope));
    verify(&mut back, &[platform.pem()]).unwrap();
}

/// The listed version's envelope for the listing statement, taken out of
/// its referrers, as a platform that attaches no such referrer leaves it.
fn without_the_listing_referrer(listed: &mut Listed) -> serde_json::Value {
    let bundle = &mut listed.bundle;
    let at = bundle
        .referrers
        .iter()
        .position(|(digest, _)| {
            let manifest: serde_json::Value =
                serde_json::from_slice(&bundle.blobs[digest]).unwrap();
            let layer = manifest["layers"][0]["digest"].as_str().unwrap();
            let envelope: serde_json::Value = serde_json::from_slice(&bundle.blobs[layer]).unwrap();
            predicate_of(&envelope) == LISTING
        })
        .unwrap();
    let (digest, _) = bundle.referrers.remove(at);
    let manifest: serde_json::Value = serde_json::from_slice(&bundle.blobs[&digest]).unwrap();
    let layer = manifest["layers"][0]["digest"]
        .as_str()
        .unwrap()
        .to_string();
    serde_json::from_slice(&bundle.blobs[&layer]).unwrap()
}

#[tokio::test]
async fn a_download_takes_the_listing_statement_from_the_reply_when_no_referrer_holds_it() {
    let mut listed = listed("0.1.0", false);
    let envelope = without_the_listing_referrer(&mut listed);
    let scratch = Scratch::new("download-statement");
    let within = scratch.0.join("platform");
    for (statement, verifies) in [(None, false), (Some(envelope.clone()), true)] {
        let (address, _) = stand_in(&listed, statement).await;
        crate::sessions::write(
            &within,
            &crate::sessions::Held {
                address: address.clone(),
                client_id: crate::registry::CLIENT_ID.into(),
                access_token: "A-1".into(),
                access_expires_at_s: crate::credential::now_s() + 600,
                refresh_token: "R-1".into(),
                expires_at_s: crate::credential::now_s() + 86_400,
                ..Default::default()
            },
        )
        .unwrap();
        let signed_in =
            crate::registry::SignedIn::within(within.clone(), &address, Box::new(|_: &str| {}))
                .await
                .unwrap();
        let to = scratch.0.join(format!("bundle-{verifies}.tar"));
        let asked = Download {
            organisation: "acme".into(),
            plugin: "venue-desk".into(),
            version: "0.1.0".into(),
            registry: "community".into(),
            to: to.clone(),
        };
        let done = download(&asked, &signed_in, &[listed.platform.pem()]).await;
        if verifies {
            let held = done.unwrap();
            assert_eq!(held.listing_statement, Some(envelope.clone()));
            assert_eq!(read(&to).unwrap().listing_statement, Some(envelope.clone()));
        } else {
            assert!(done.unwrap_err().contains("no listing statement"));
            assert!(!to.exists(), "nothing written");
        }
    }
}

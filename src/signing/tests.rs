use std::collections::BTreeMap;
use std::sync::Mutex;

use super::*;
use crate::fake_http::{serve, Asked, Reply, Scratch};

const DESKTOP: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFr2Kd8Wm4yT0sQ6nB1cJ9vX3gL7pE5hU2aR8oZ0iN4t ada tag signing";
const HARDWARE: &str =
    "sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29tAAAAIB9 ada tag signing";

/// A machine as the test says: git's configuration, files, and an
/// ssh-keygen writing the public half it is given.
#[derive(Default)]
struct Fake {
    config: Mutex<BTreeMap<String, String>>,
    set: Mutex<Vec<(bool, String, String)>>,
    files: Mutex<BTreeMap<PathBuf, String>>,
    made: Mutex<Vec<(String, PathBuf)>>,
    repository: bool,
}

impl Fake {
    fn with(config: &[(&str, &str)]) -> Fake {
        let fake = Fake::default();
        for (key, value) in config {
            fake.config
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
        }
        fake
    }

    fn file(self, path: &str, text: &str) -> Fake {
        self.files
            .lock()
            .unwrap()
            .insert(PathBuf::from(path), text.into());
        self
    }
}

impl Workstation for Fake {
    fn git_config(&self, _: &Path, key: &str) -> Option<String> {
        self.config.lock().unwrap().get(key).cloned()
    }
    fn set_git_config(&self, _: &Path, global: bool, key: &str, value: &str) -> Result<(), String> {
        self.set
            .lock()
            .unwrap()
            .push((global, key.into(), value.into()));
        self.config.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn in_repository(&self, _: &Path) -> bool {
        self.repository
    }
    fn read(&self, path: &Path) -> Option<String> {
        self.files.lock().unwrap().get(path).cloned()
    }
    fn exists(&self, path: &Path) -> bool {
        self.files.lock().unwrap().contains_key(path)
    }
    fn keygen(&self, kind: &str, file: &Path, comment: &str) -> Result<(), String> {
        assert_eq!(comment, "ada@acme.example tag signing");
        self.made.lock().unwrap().push((kind.into(), file.into()));
        let line = if kind == "ed25519-sk" {
            HARDWARE
        } else {
            DESKTOP
        };
        self.files.lock().unwrap().insert(
            PathBuf::from(format!("{}.pub", file.display())),
            format!("{line}\n"),
        );
        Ok(())
    }
    fn home(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/home/ada"))
    }
}

fn asked(hardware: bool) -> Setup {
    Setup {
        dir: PathBuf::from("."),
        hardware,
    }
}

#[test]
fn gits_ssh_signing_key_is_used_as_it_is() {
    let fake = Fake::with(&[
        ("gpg.format", "ssh"),
        ("user.signingkey", "~/.ssh/id_ed25519"),
    ])
    .file("/home/ada/.ssh/id_ed25519.pub", DESKTOP);
    let ready = prepare(&fake, &asked(false)).unwrap();
    assert_eq!(ready.line, DESKTOP);
    assert_eq!(
        ready.file,
        Some(PathBuf::from("/home/ada/.ssh/id_ed25519.pub"))
    );
    assert!(fake.made.lock().unwrap().is_empty(), "nothing made");
    assert!(fake.set.lock().unwrap().is_empty(), "nothing set");
    assert!(has_ssh_key(&fake, Path::new(".")));

    // A key given whole, as `key::`.
    let literal = format!("key::{DESKTOP}");
    let fake = Fake::with(&[("gpg.format", "ssh"), ("user.signingkey", literal.as_str())]);
    assert_eq!(prepare(&fake, &asked(false)).unwrap().line, DESKTOP);

    // Its public half gone: said, and nothing done.
    let fake = Fake::with(&[("gpg.format", "ssh"), ("user.signingkey", "/keys/release")]);
    let refused = prepare(&fake, &asked(false)).unwrap_err();
    assert!(
        refused.contains("/keys/release.pub, is not here"),
        "{refused}"
    );

    // --hardware with a desktop key configured: refused, saying how.
    let fake =
        Fake::with(&[("gpg.format", "ssh"), ("user.signingkey", "/k.pub")]).file("/k.pub", DESKTOP);
    let refused = prepare(&fake, &asked(true)).unwrap_err();
    assert!(refused.contains("--unset user.signingkey"), "{refused}");
}

#[test]
fn with_none_ssh_keygen_makes_one_and_git_is_set_to_sign_with_it() {
    for (hardware, kind, file, line) in [
        (
            false,
            "ed25519",
            "/home/ada/.ssh/id_ed25519_meridian",
            DESKTOP,
        ),
        (
            true,
            "ed25519-sk",
            "/home/ada/.ssh/id_ed25519_sk_meridian",
            HARDWARE,
        ),
    ] {
        let fake = Fake::with(&[("user.email", "ada@acme.example")]);
        assert!(!has_ssh_key(&fake, Path::new(".")));
        let ready = prepare(&fake, &asked(hardware)).unwrap();
        assert_eq!(
            *fake.made.lock().unwrap(),
            [(kind.to_string(), PathBuf::from(file))]
        );
        assert_eq!(ready.line, line);
        let public = format!("{file}.pub");
        assert_eq!(
            *fake.set.lock().unwrap(),
            [
                (true, "gpg.format".to_string(), "ssh".to_string()),
                (true, "user.signingkey".to_string(), public.clone()),
            ]
        );
        assert!(said(&ready).contains(&format!("made {file} with ssh-keygen ({kind})")));
    }
}

#[test]
fn a_key_made_before_is_used_and_never_made_over() {
    let fake = Fake::with(&[]).file("/home/ada/.ssh/id_ed25519_meridian.pub", DESKTOP);
    let ready = prepare(&fake, &asked(false)).unwrap();
    assert!(fake.made.lock().unwrap().is_empty());
    assert_eq!(ready.line, DESKTOP);
    // A private half without its public: never overwritten.
    let fake = Fake::with(&[]).file("/home/ada/.ssh/id_ed25519_meridian", "secret");
    let refused = prepare(&fake, &asked(false)).unwrap_err();
    assert!(refused.contains("ssh-keygen -y -f"), "{refused}");
    assert!(!refused.contains("secret"));
}

#[test]
fn where_git_signs_with_gpg_the_key_is_the_repositorys_alone() {
    let mut fake = Fake::with(&[
        ("user.signingkey", "4AEE18F83AFDEB23"),
        ("user.email", "ada@acme.example"),
    ]);
    fake.repository = true;
    prepare(&fake, &asked(false)).unwrap();
    assert!(fake
        .set
        .lock()
        .unwrap()
        .iter()
        .all(|(global, _, _)| !global));
    // Outside a repository: refused, changing nothing.
    let fake = Fake::with(&[("user.signingkey", "4AEE18F83AFDEB23")]);
    let refused = prepare(&fake, &asked(false)).unwrap_err();
    assert!(refused.contains("GPG key"), "{refused}");
    assert!(fake.made.lock().unwrap().is_empty() && fake.set.lock().unwrap().is_empty());
}

/// A platform whose signing keys are `listed`, and whose register answers
/// `register` (a status and a body).
async fn platform(
    listed: serde_json::Value,
    register: (u16, serde_json::Value),
) -> (String, crate::fake_http::Kept) {
    serve(
        move |asked: &Asked| match (asked.method.as_str(), asked.path.as_str()) {
            ("GET", "/api/v1/registry/organisations/acme/signing-keys") => {
                Reply::json(200, listed.clone())
            }
            ("POST", "/api/v1/registry/organisations/acme/signing-keys") => {
                Reply::json(register.0, register.1.clone())
            }
            _ => Reply::json(404, serde_json::json!({"error": "no such row"})),
        },
    )
    .await
}

async fn signed_in(address: &str, scratch: &Scratch) -> registry::SignedIn {
    let within = scratch.0.join("platform");
    let now = crate::credential::now_s();
    crate::sessions::write(
        &within,
        &crate::sessions::Held {
            address: address.into(),
            subject: String::new(),
            expires_at: String::new(),
            session: String::new(),
            client_id: registry::CLIENT_ID.into(),
            access_token: "A-1".into(),
            access_expires_at_s: now + 600,
            refresh_token: "R-1".into(),
            expires_at_s: now + 3600,
        },
    )
    .unwrap();
    registry::SignedIn::within(within, address, Box::new(|_: &str| panic!("no browser")))
        .await
        .unwrap()
}

fn ready() -> Ready {
    Ready {
        line: DESKTOP.into(),
        file: Some(PathBuf::from("/home/ada/.ssh/id_ed25519_meridian.pub")),
        did: Vec::new(),
    }
}

#[tokio::test]
async fn the_public_half_is_registered_unless_it_is_already() {
    let none = serde_json::json!({"keys": []});
    let (address, kept) = platform(
        none,
        (
            201,
            serde_json::json!({"key_id": "SGK-7", "fingerprint": "SHA256:Lm7p"}),
        ),
    )
    .await;
    let scratch = Scratch::new("setup-register");
    let session = signed_in(&address, &scratch).await;
    assert_eq!(
        register(&session, "acme", &ready()).await.unwrap(),
        Registered::Made {
            key_id: "SGK-7".into(),
            fingerprint: "SHA256:Lm7p".into()
        }
    );
    let posted = kept
        .lock()
        .unwrap()
        .iter()
        .find(|a| a.method == "POST")
        .cloned()
        .unwrap();
    assert_eq!(
        posted.json(),
        serde_json::json!({"organisation": "acme", "name": "ada tag signing", "public_key": DESKTOP})
    );

    // Registered already (another comment): nothing sent.
    let other_comment = DESKTOP.replace("ada tag signing", "laptop");
    let (address, kept) = platform(
        serde_json::json!({"keys": [{"key_id": "SGK-3", "public_key": other_comment}]}),
        (500, serde_json::json!({})),
    )
    .await;
    let scratch = Scratch::new("setup-already");
    let session = signed_in(&address, &scratch).await;
    assert_eq!(
        register(&session, "acme", &ready()).await.unwrap(),
        Registered::Already("SGK-3".into())
    );
    assert!(!kept.lock().unwrap().iter().any(|a| a.method == "POST"));
}

#[tokio::test]
async fn somebody_who_is_no_owner_or_admin_is_told_who_to_ask() {
    let (address, _) = platform(
        serde_json::json!({"keys": []}),
        (
            422,
            serde_json::json!({"error": "only an Owner or Admin of the organisation may do this on its page"}),
        ),
    )
    .await;
    let scratch = Scratch::new("setup-ask");
    let session = signed_in(&address, &scratch).await;
    let Registered::Ask(said) = register(&session, "acme", &ready()).await.unwrap() else {
        panic!("not told to ask");
    };
    assert!(said.contains("Only an Owner or Admin of acme"), "{said}");
    assert!(said.contains(&format!("  {DESKTOP}")), "{said}");
    assert!(
        said.contains("meridian plugin key register /home/ada/.ssh/id_ed25519_meridian.pub --organisation acme"),
        "{said}"
    );
}

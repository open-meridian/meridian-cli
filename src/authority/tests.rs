use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use super::*;
use crate::doctor::{Answered, Failure, Machine};

/// 2026-10-03T00:00:00Z.
const NOW: u64 = 1_790_985_600;

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("meridian-authority-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn parsed(pem: &str) -> Vec<u8> {
    pem_der(pem).expect("a PEM certificate")
}

#[test]
fn the_root_is_named_for_open_meridian_and_this_machine_for_ten_years_and_localhost_alone() {
    let made = Authority::make("ada-laptop", NOW).unwrap();
    let der = parsed(made.root_pem());
    let (_, root) = x509_parser::parse_x509_certificate(&der).unwrap();
    assert!(root.is_ca());
    let subject = root.subject().to_string();
    assert!(subject.contains("O=Open Meridian"), "{subject}");
    assert!(
        subject.contains("CN=Open Meridian local authority for ada-laptop"),
        "{subject}"
    );
    let ends = root.validity().not_after.timestamp() as u64;
    assert_eq!(ends, NOW + 3_650 * 86_400);
    let constraints = root.name_constraints().unwrap().expect("a name constraint");
    assert!(constraints.critical);
    let permitted = constraints.value.permitted_subtrees.as_ref().unwrap();
    assert_eq!(permitted.len(), 1);
    assert!(matches!(
        permitted[0].base,
        x509_parser::extensions::GeneralName::DNSName("localhost")
    ));
}

#[test]
fn the_key_is_never_printed() {
    let made = Authority::make("m", NOW).unwrap();
    let shown = format!("{made:?}");
    assert!(!shown.contains("PRIVATE KEY"), "{shown}");
    let leaf = made.issue("meridian.localhost", NOW).unwrap();
    let shown = format!("{leaf:?}");
    assert!(!shown.contains("PRIVATE KEY"), "{shown}");
}

#[test]
fn written_and_read_back_with_the_key_readable_by_its_owner_alone() {
    let dir = scratch("written");
    assert!(Authority::read(&dir).unwrap().is_none());
    let made = Authority::make("m", NOW).unwrap();
    made.write(&dir).unwrap();
    let read = Authority::read(&dir).unwrap().expect("held");
    assert_eq!(read.root_pem(), made.root_pem());
    assert_eq!(read.sha1(), made.sha1());
    assert_eq!(read.sha1().len(), 40);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join("root-key.pem")), 0o600);
        assert_eq!(mode(&dir), 0o700);
    }
    // Half of one is refused, never remade over a root trusted somewhere.
    std::fs::remove_file(dir.join("root-key.pem")).unwrap();
    let refused = Authority::read(&dir).unwrap_err();
    assert!(refused.contains("meridian authority remove"), "{refused}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_certificate_names_the_host_and_its_plugins_for_a_year_and_serves() {
    let made = Authority::make("m", NOW).unwrap();
    let leaf = made.issue("meridian.localhost", NOW).unwrap();
    assert_eq!(leaf.ends_s, NOW + 365 * 86_400);
    let der = parsed(&leaf.cert_pem);
    let (_, cert) = x509_parser::parse_x509_certificate(&der).unwrap();
    assert!(!cert.is_ca());
    let names: Vec<String> = cert
        .subject_alternative_name()
        .unwrap()
        .unwrap()
        .value
        .general_names
        .iter()
        .map(|name| name.to_string())
        .collect();
    assert!(names.iter().any(|n| n.contains("meridian.localhost")));
    assert!(names
        .iter()
        .any(|n| n.contains("*.plugins.meridian.localhost")));
    assert_eq!(
        made.standing(&leaf.cert_pem, "meridian.localhost", NOW),
        Standing::Current {
            ends_s: leaf.ends_s
        }
    );
    // A firm's own name is never this machine's to sign.
    assert!(made.issue("meridian.firm.example", NOW).is_err());
}

#[test]
fn a_certificate_is_due_within_thirty_days_of_its_end_or_from_another_authority() {
    let made = Authority::make("m", NOW).unwrap();
    let leaf = made.issue("meridian.localhost", NOW).unwrap();
    let day = 86_400;
    assert!(matches!(
        made.standing(&leaf.cert_pem, "meridian.localhost", NOW + 334 * day),
        Standing::Current { .. }
    ));
    let Standing::Due(why) = made.standing(&leaf.cert_pem, "meridian.localhost", NOW + 336 * day)
    else {
        panic!("due within thirty days of its end");
    };
    assert!(why.contains("ends on 2027-10-03"), "{why}");

    let other = Authority::make("m", NOW).unwrap();
    let Standing::Due(why) = other.standing(&leaf.cert_pem, "meridian.localhost", NOW) else {
        panic!("another authority's");
    };
    assert!(why.contains("another authority"), "{why}");

    let Standing::Due(why) = made.standing(&leaf.cert_pem, "trial.localhost", NOW) else {
        panic!("another name's");
    };
    assert!(why.contains("trial.localhost"), "{why}");
    assert!(matches!(
        made.standing("not a certificate", "meridian.localhost", NOW),
        Standing::Due(_)
    ));
}

#[test]
fn the_secret_is_a_tls_secret_named_for_the_release() {
    let made = Authority::make("m", NOW).unwrap();
    let leaf = made.issue("meridian.localhost", NOW).unwrap();
    let manifest: serde_json::Value =
        serde_json::from_str(&secret_manifest("meridian", "meridian", &leaf)).unwrap();
    assert_eq!(manifest["kind"], "Secret");
    assert_eq!(manifest["type"], "kubernetes.io/tls");
    assert_eq!(manifest["metadata"]["name"], "meridian-tls");
    assert_eq!(manifest["metadata"]["namespace"], "meridian");
    let crt = certificate_in(manifest["data"]["tls.crt"].as_str().unwrap()).unwrap();
    assert_eq!(crt, leaf.cert_pem);
    let key = certificate_in(manifest["data"]["tls.key"].as_str().unwrap()).unwrap();
    assert!(key.contains("PRIVATE KEY"));
}

#[test]
fn trusting_it_is_one_command_on_each_system() {
    let root = Path::new("/home/ada/.config/meridian/authority/root.pem");
    assert_eq!(
        trust_command("macos", root),
        "security add-trusted-cert -r trustRoot -k ~/Library/Keychains/login.keychain-db \
         /home/ada/.config/meridian/authority/root.pem"
    );
    assert!(trust_command("linux", root).contains("update-ca-certificates"));
    assert!(trust_command("windows", root).starts_with("certutil -user -addstore Root"));
    assert!(untrust_command("macos", "AB12").contains("delete-certificate -t -Z AB12"));
}

#[test]
fn naming_it_to_apps_is_one_command_on_each_system() {
    assert_eq!(
        apps_command("macos", Path::new("/Users/a b/root.pem")),
        "launchctl setenv NODE_EXTRA_CA_CERTS '/Users/a b/root.pem'"
    );
    assert_eq!(
        apps_command("linux", Path::new("/home/ada/root.pem")),
        "echo \"export NODE_EXTRA_CA_CERTS=/home/ada/root.pem\" >> ~/.profile"
    );
    assert_eq!(
        apps_command("windows", Path::new(r"C:\Users\ada\root.pem")),
        r#"setx NODE_EXTRA_CA_CERTS "C:\Users\ada\root.pem""#
    );
    assert_eq!(
        unapps_command("macos"),
        "launchctl unsetenv NODE_EXTRA_CA_CERTS"
    );
    assert!(unapps_command("windows").contains("reg delete HKCU\\Environment"));
}

#[test]
fn the_launch_agent_names_the_file_to_launchd_when_it_is_loaded() {
    let plist = agent_plist(Path::new("/Users/a&b/authority/root.pem"));
    assert!(plist.contains("<string>com.open-meridian.authority</string>"));
    assert!(plist.contains(
        "<string>/bin/launchctl</string>\n    <string>setenv</string>\n    \
         <string>NODE_EXTRA_CA_CERTS</string>\n    <string>/Users/a&amp;b/authority/root.pem</string>"
    ));
    assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
    // Loaded at each login, never kept running.
    assert!(!plist.contains("KeepAlive"));
}

/// A machine that is asked, and keeps what it was asked: its login keychain,
/// launchd's environment for apps, and a LaunchAgents directory beside the
/// test's own, never the person's.
struct Asked {
    os: &'static str,
    refuses: bool,
    said: RefCell<Vec<String>>,
    launchd: RefCell<Option<String>>,
    own: Option<String>,
    agents: PathBuf,
}

fn agents_of(dir: &Path) -> PathBuf {
    PathBuf::from(format!("{}-agents", dir.display()))
}

fn tidy(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(agents_of(dir));
}

impl Asked {
    fn on(os: &'static str, dir: &Path) -> Self {
        let _ = std::fs::remove_dir_all(agents_of(dir));
        Asked {
            os,
            refuses: false,
            said: RefCell::new(Vec::new()),
            launchd: RefCell::new(None),
            own: None,
            agents: agents_of(dir),
        }
    }
    fn events(&self) -> Vec<String> {
        self.said.borrow().clone()
    }
    fn named(&self) -> Option<String> {
        self.launchd.borrow().clone()
    }
    fn plist(&self) -> PathBuf {
        self.agents.join("com.open-meridian.authority.plist")
    }
}

impl Trust for Asked {
    fn os(&self) -> &str {
        self.os
    }
    fn add(&self, root: &Path) -> Result<(), String> {
        self.said
            .borrow_mut()
            .push(format!("add {}", root.display()));
        match self.refuses {
            true => Err("the person cancelled".into()),
            false => Ok(()),
        }
    }
    fn remove(&self, sha1: &str) -> Result<(), String> {
        self.said.borrow_mut().push(format!("remove {sha1}"));
        Ok(())
    }
    fn apps_env(&self) -> Option<String> {
        self.named()
    }
    fn own_env(&self) -> Option<String> {
        self.own.clone()
    }
    fn set_apps_env(&self, file: &str) -> Result<(), String> {
        self.said.borrow_mut().push(format!("setenv {file}"));
        *self.launchd.borrow_mut() = Some(file.to_string());
        Ok(())
    }
    fn unset_apps_env(&self) -> Result<(), String> {
        self.said.borrow_mut().push("unsetenv".into());
        *self.launchd.borrow_mut() = None;
        Ok(())
    }
    fn launch_agents(&self) -> Result<PathBuf, String> {
        Ok(self.agents.clone())
    }
    fn load(&self, plist: &Path) -> Result<(), String> {
        self.said
            .borrow_mut()
            .push(format!("load {}", plist.display()));
        Ok(())
    }
    fn unload(&self, plist: &Path) -> Result<(), String> {
        self.said
            .borrow_mut()
            .push(format!("unload {}", plist.display()));
        Ok(())
    }
}

/// A `NODE_EXTRA_CA_CERTS` file of the person's own, with no newline at its
/// end.
fn their_file(dir: &Path) -> String {
    let theirs = PathBuf::from(format!("{}-theirs.pem", dir.display()));
    std::fs::write(
        &theirs,
        "-----BEGIN CERTIFICATE-----\nTHEIRS\n-----END CERTIFICATE-----",
    )
    .unwrap();
    theirs.display().to_string()
}

#[test]
fn on_macos_it_says_what_the_root_is_for_then_asks_once_and_adds_it() {
    let dir = scratch("macos");
    let machine = Asked::on("macos", &dir);
    let mut said = Vec::new();
    let ask = |question: &str| {
        machine.said.borrow_mut().push(format!("ask {question}"));
        true
    };
    let prepared = prepare(&dir, "m", NOW, &machine, &ask, &mut |line: &str| {
        machine.said.borrow_mut().push(format!("say {line}"));
        said.push(line.to_string())
    })
    .unwrap();
    let events = machine.events();
    let purpose_at = events
        .iter()
        .position(|e| e.starts_with("say This machine has its own certificate authority"))
        .expect("said what it is for");
    let ask_at = events.iter().position(|e| e.starts_with("ask ")).unwrap();
    let add_at = events.iter().position(|e| e.starts_with("add ")).unwrap();
    assert!(purpose_at < ask_at && ask_at < add_at, "{events:?}");
    assert!(events[add_at].ends_with("root.pem"));
    assert!(events[purpose_at].contains(".localhost and nothing else"));
    assert_eq!(
        events.iter().filter(|e| e.starts_with("ask ")).count(),
        1,
        "one question: {events:?}"
    );

    // Once: asked again, it neither asks nor adds, and makes nothing new.
    let again = Asked::on("macos", &dir);
    let never = |_: &str| -> bool { panic!("asked twice") };
    let held = prepare(&dir, "m", NOW, &again, &never, &mut |_: &str| {}).unwrap();
    assert_eq!(held.authority.root_pem(), prepared.authority.root_pem());
    assert!(again.events().is_empty());
    tidy(&dir);
}

#[test]
fn yes_on_macos_names_the_root_to_apps_now_and_at_each_login() {
    let dir = scratch("apps");
    let machine = Asked::on("macos", &dir);
    let mut said = Vec::new();
    prepare(
        &dir,
        "m",
        NOW,
        &machine,
        &|_: &str| true,
        &mut |line: &str| said.push(line.to_string()),
    )
    .unwrap();
    let root = root_path(&dir).display().to_string();
    let plist = machine.plist().display().to_string();
    assert_eq!(
        machine.events(),
        [
            format!("add {root}"),
            format!("setenv {root}"),
            format!("unload {plist}"),
            format!("load {plist}"),
        ]
    );
    assert_eq!(machine.named().as_deref(), Some(root.as_str()));
    let written = std::fs::read_to_string(machine.plist()).unwrap();
    assert!(
        written.contains(&format!("<string>{root}</string>")),
        "{written}"
    );
    let said = said.join("\n");
    assert!(said.contains("Restart the Claude app"), "{said}");
    assert!(
        said.contains("the Claude Code CLI reads the login keychain"),
        "{said}"
    );

    // `meridian authority` says it is in place.
    let state = apps(&dir, &machine);
    assert_eq!(
        state,
        Apps::Named {
            file: root_path(&dir),
            at_login: true
        }
    );
    assert!(state.said().contains("in place"), "{}", state.said());

    // `meridian authority trust` again, as a second install runs it: nothing
    // asked, nothing changed, and said so.
    let before = machine.events().len();
    let mut again = Vec::new();
    trust_here(
        &dir,
        "m",
        NOW,
        &machine,
        &|_: &str| -> bool { panic!("asked again") },
        &mut |line: &str| again.push(line.to_string()),
    )
    .unwrap();
    assert_eq!(machine.events().len(), before);
    assert!(
        again.join("\n").contains("trusted here already"),
        "{again:?}"
    );

    // Removed: the LaunchAgent unloaded and gone, launchd names nothing, the
    // keychain entry and the files gone.
    let sha1 = Authority::read(&dir).unwrap().unwrap().sha1();
    let removed = remove(&dir, &machine).unwrap();
    let events = machine.events()[before..].to_vec();
    assert_eq!(
        events,
        [
            format!("unload {plist}"),
            "unsetenv".to_string(),
            format!("remove {sha1}")
        ]
    );
    assert!(!machine.plist().exists());
    assert_eq!(machine.named(), None);
    assert!(!dir.exists());
    assert!(removed.contains("no longer names it to apps"), "{removed}");
    tidy(&dir);
}

#[test]
fn one_named_already_is_never_replaced_a_file_holding_both_is_asked_for_and_put_back() {
    let dir = scratch("theirs");
    let theirs = their_file(&dir);
    let machine = Asked::on("macos", &dir);
    *machine.launchd.borrow_mut() = Some(theirs.clone());
    let questions = RefCell::new(Vec::new());
    let ask = |question: &str| {
        questions.borrow_mut().push(question.to_string());
        true
    };
    let mut said = Vec::new();
    prepare(&dir, "m", NOW, &machine, &ask, &mut |line: &str| {
        said.push(line.to_string())
    })
    .unwrap();
    let questions = questions.into_inner();
    assert_eq!(questions.len(), 2, "{questions:?}");
    assert!(questions[1].contains(&theirs), "{questions:?}");
    assert!(said.join("\n").contains("never replaces it"));

    let combined = dir.join("node-extra-ca-certs.pem");
    assert_eq!(
        machine.named().as_deref(),
        Some(combined.display().to_string().as_str())
    );
    let both = std::fs::read_to_string(&combined).unwrap();
    let root = std::fs::read_to_string(root_path(&dir)).unwrap();
    assert!(both.starts_with("-----BEGIN CERTIFICATE-----\nTHEIRS\n-----END CERTIFICATE-----\n"));
    assert!(both.ends_with(&root));
    assert_eq!(
        std::fs::read_to_string(dir.join("node-extra-ca-certs.before")).unwrap(),
        theirs
    );
    let agent = std::fs::read_to_string(machine.plist()).unwrap();
    assert!(agent.contains(&format!("<string>{}</string>", combined.display())));
    assert_eq!(
        apps(&dir, &machine),
        Apps::Named {
            file: combined,
            at_login: true
        }
    );

    // Removed: theirs named again, as it was, and never unset.
    let before = machine.events().len();
    let removed = remove(&dir, &machine).unwrap();
    assert_eq!(machine.named().as_deref(), Some(theirs.as_str()));
    let events = machine.events()[before..].to_vec();
    assert!(events.contains(&format!("setenv {theirs}")), "{events:?}");
    assert!(!events.contains(&"unsetenv".to_string()), "{events:?}");
    assert!(removed.contains("as before"), "{removed}");
    assert!(!machine.plist().exists());
    tidy(&dir);
    let _ = std::fs::remove_file(theirs);
}

#[test]
fn declined_a_file_holding_both_changes_nothing_and_says_what_to_do() {
    let dir = scratch("theirs-declined");
    let theirs = their_file(&dir);
    let machine = Asked::on("macos", &dir);
    *machine.launchd.borrow_mut() = Some(theirs.clone());
    let ask = |question: &str| question.starts_with("Trust it now?");
    let mut said = Vec::new();
    prepare(&dir, "m", NOW, &machine, &ask, &mut |line: &str| {
        said.push(line.to_string())
    })
    .unwrap();
    // The keychain took it; apps were left as they were.
    assert_eq!(machine.events().len(), 1, "{:?}", machine.events());
    assert_eq!(machine.named().as_deref(), Some(theirs.as_str()));
    assert!(!machine.plist().exists());
    assert!(!dir.join("node-extra-ca-certs.pem").exists());
    let said = said.join("\n");
    assert!(said.contains(&format!("cat {theirs} ")), "{said}");
    assert!(
        said.contains("launchctl setenv NODE_EXTRA_CA_CERTS ")
            && said.contains("node-extra-ca-certs.pem"),
        "{said}"
    );
    assert_eq!(apps(&dir, &machine), Apps::Other(theirs.clone()));
    tidy(&dir);
    let _ = std::fs::remove_file(theirs);
}

#[test]
fn one_in_the_environment_alone_is_kept_too_and_removal_unsets_what_launchd_never_had() {
    let dir = scratch("own");
    let theirs = their_file(&dir);
    let machine = Asked {
        own: Some(theirs.clone()),
        ..Asked::on("macos", &dir)
    };
    prepare(&dir, "m", NOW, &machine, &|_: &str| true, &mut |_: &str| {}).unwrap();
    let combined = dir.join("node-extra-ca-certs.pem").display().to_string();
    assert_eq!(machine.named().as_deref(), Some(combined.as_str()));
    assert_eq!(
        std::fs::read_to_string(dir.join("node-extra-ca-certs.before")).unwrap(),
        ""
    );
    remove(&dir, &machine).unwrap();
    assert_eq!(machine.named(), None);
    assert!(machine.events().contains(&"unsetenv".to_string()));
    tidy(&dir);
    let _ = std::fs::remove_file(theirs);
}

#[test]
fn removal_leaves_alone_what_the_person_has_since_named() {
    let dir = scratch("changed");
    let machine = Asked::on("macos", &dir);
    prepare(&dir, "m", NOW, &machine, &|_: &str| true, &mut |_: &str| {}).unwrap();
    *machine.launchd.borrow_mut() = Some("/Users/ada/firm.pem".into());
    let before = machine.events().len();
    remove(&dir, &machine).unwrap();
    assert_eq!(machine.named().as_deref(), Some("/Users/ada/firm.pem"));
    let events = machine.events()[before..].to_vec();
    assert!(
        !events
            .iter()
            .any(|e| e.starts_with("setenv") || e == "unsetenv"),
        "{events:?}"
    );
    assert!(!machine.plist().exists(), "the LaunchAgent goes regardless");
    tidy(&dir);
}

#[test]
fn trust_asks_about_apps_alone_where_the_keychain_took_it_before() {
    // As a person who trusted it with 0.1.30, which asked about the keychain
    // alone, and named it to launchd by hand since.
    let dir = scratch("before-apps");
    Authority::make("m", NOW).unwrap().write(&dir).unwrap();
    std::fs::write(dir.join("asked"), "").unwrap();
    let machine = Asked::on("macos", &dir);
    let root = root_path(&dir).display().to_string();
    *machine.launchd.borrow_mut() = Some(root.clone());
    assert_eq!(
        apps(&dir, &machine),
        Apps::Named {
            file: root_path(&dir),
            at_login: false
        }
    );

    let questions = RefCell::new(Vec::new());
    let ask = |question: &str| {
        questions.borrow_mut().push(question.to_string());
        true
    };
    let mut said = Vec::new();
    trust_here(&dir, "m", NOW, &machine, &ask, &mut |line: &str| {
        said.push(line.to_string())
    })
    .unwrap();
    let questions = questions.into_inner();
    assert_eq!(questions.len(), 1, "{questions:?}");
    assert!(questions[0].starts_with("Point apps"), "{questions:?}");
    assert!(
        !machine.events().iter().any(|e| e.starts_with("add ")),
        "the keychain is not asked again: {:?}",
        machine.events()
    );
    assert!(machine.plist().exists());
    assert_eq!(machine.named().as_deref(), Some(root.as_str()));
    assert!(said.join("\n").contains("is in your login keychain"));
    tidy(&dir);
}

#[test]
fn declined_or_refused_it_says_the_command_and_asks_again_next_time() {
    let dir = scratch("declined");
    let machine = Asked::on("macos", &dir);
    let mut said = Vec::new();
    prepare(
        &dir,
        "m",
        NOW,
        &machine,
        &|_: &str| false,
        &mut |line: &str| said.push(line.to_string()),
    )
    .unwrap();
    assert!(machine.events().is_empty(), "nothing added unasked");
    let said = said.join("\n");
    assert!(said.contains("security add-trusted-cert -r trustRoot"));
    assert!(
        said.contains("launchctl setenv NODE_EXTRA_CA_CERTS"),
        "{said}"
    );
    assert!(!machine.plist().exists());

    let refusing = Asked {
        refuses: true,
        ..Asked::on("macos", &dir)
    };
    let mut said = Vec::new();
    prepare(
        &dir,
        "m",
        NOW,
        &refusing,
        &|_: &str| true,
        &mut |line: &str| said.push(line.to_string()),
    )
    .unwrap();
    assert_eq!(refusing.events().len(), 1, "{:?}", refusing.events());
    assert!(said.join("\n").contains("the person cancelled"));
    assert_eq!(
        refusing.named(),
        None,
        "apps are not pointed at a root refused"
    );
    tidy(&dir);
}

#[test]
fn elsewhere_it_prints_each_step_once_and_runs_nothing() {
    let dir = scratch("linux");
    let machine = Asked::on("linux", &dir);
    let mut said = Vec::new();
    let never = |_: &str| -> bool { panic!("nothing to ask on linux") };
    prepare(&dir, "m", NOW, &machine, &never, &mut |line: &str| {
        said.push(line.to_string())
    })
    .unwrap();
    assert!(machine.events().is_empty());
    let said = said.join("\n");
    assert!(said.contains("update-ca-certificates"), "{said}");
    assert!(
        said.contains("echo \"export NODE_EXTRA_CA_CERTS="),
        "{said}"
    );
    let mut again = Vec::new();
    prepare(&dir, "m", NOW, &machine, &never, &mut |line: &str| {
        again.push(line.to_string())
    })
    .unwrap();
    assert!(again.is_empty(), "{again:?}");

    // `meridian authority trust` says the steps again, every time, and with
    // one of the person's own named, the file holding both.
    let theirs = their_file(&dir);
    let named = Asked {
        own: Some(theirs.clone()),
        ..Asked::on("linux", &dir)
    };
    let mut steps = Vec::new();
    trust_here(&dir, "m", NOW, &named, &never, &mut |line: &str| {
        steps.push(line.to_string())
    })
    .unwrap();
    let steps = steps.join("\n");
    assert!(steps.contains(&format!("cat {theirs} ")), "{steps}");
    assert!(
        steps.contains("node-extra-ca-certs.pem\" >> ~/.profile"),
        "{steps}"
    );
    assert!(named.events().is_empty());
    assert!(
        !dir.join("node-extra-ca-certs.pem").exists(),
        "nothing made"
    );

    let removed = remove(&dir, &named).unwrap();
    assert!(named.events().is_empty());
    assert!(
        removed.contains("remove the NODE_EXTRA_CA_CERTS line"),
        "{removed}"
    );
    tidy(&dir);
    let _ = std::fs::remove_file(theirs);
}

#[test]
fn removing_it_takes_it_out_of_the_keychain_and_off_the_disk() {
    let dir = scratch("removed");
    let machine = Asked::on("macos", &dir);
    let prepared = prepare(&dir, "m", NOW, &machine, &|_: &str| true, &mut |_: &str| {}).unwrap();
    let sha1 = prepared.authority.sha1();
    let said = remove(&dir, &machine).unwrap();
    assert!(machine.events().contains(&format!("remove {sha1}")));
    assert!(said.contains("login keychain"), "{said}");
    assert!(!dir.exists());
    assert!(remove(&dir, &machine)
        .unwrap()
        .contains("nothing to remove"));
    tidy(&dir);
}

/// A cluster that holds one Secret, or none, and keeps what was applied.
#[derive(Default)]
struct Cluster {
    held: Mutex<VecDeque<String>>,
    applied: Mutex<Vec<String>>,
    asked: Mutex<HashMap<String, usize>>,
}

#[async_trait::async_trait]
impl Machine for Cluster {
    async fn run(&self, program: &str, arguments: &[&str]) -> Result<String, Failure> {
        let command = format!("{program} {}", arguments.join(" "));
        *self
            .asked
            .lock()
            .unwrap()
            .entry(command.clone())
            .or_default() += 1;
        assert_eq!(
            command,
            "kubectl get secret meridian-tls -n meridian --ignore-not-found -o jsonpath={.data.tls\\.crt}"
        );
        Ok(self
            .held
            .lock()
            .unwrap()
            .front()
            .cloned()
            .unwrap_or_default())
    }
    async fn fetch(&self, url: &str, _: &[(&str, &str)]) -> Result<Answered, String> {
        Err(format!("fetches nothing: {url}"))
    }
    fn now_s(&self) -> u64 {
        NOW
    }
    async fn run_with_input(
        &self,
        program: &str,
        arguments: &[&str],
        input: &str,
    ) -> Result<String, Failure> {
        assert_eq!(
            format!("{program} {}", arguments.join(" ")),
            "kubectl apply --server-side --force-conflicts --field-manager meridian -f -"
        );
        self.applied.lock().unwrap().push(input.to_string());
        Ok(String::new())
    }
}

#[tokio::test]
async fn the_certificate_step_writes_a_secret_when_there_is_none_and_keeps_one_that_serves() {
    let made = Authority::make("m", NOW).unwrap();
    let cluster = Cluster::default();
    let first = certify(
        &cluster,
        &made,
        "meridian",
        "meridian",
        "meridian.localhost",
    )
    .await
    .unwrap();
    assert!(
        matches!(&first, Certified::Issued { why, .. } if why == "there was none"),
        "{first:?}"
    );
    let applied = cluster.applied.lock().unwrap().clone();
    assert_eq!(applied.len(), 1);
    // Its key went on standard input, never in an argument.
    assert!(applied[0].contains("\"tls.key\""));

    let written: serde_json::Value = serde_json::from_str(&applied[0]).unwrap();
    let crt = written["data"]["tls.crt"].as_str().unwrap().to_string();
    cluster.held.lock().unwrap().push_back(crt);
    let second = certify(
        &cluster,
        &made,
        "meridian",
        "meridian",
        "meridian.localhost",
    )
    .await
    .unwrap();
    assert!(matches!(second, Certified::Kept { .. }), "{second:?}");
    assert_eq!(
        cluster.applied.lock().unwrap().len(),
        1,
        "nothing rewritten"
    );
    assert!(second.said("meridian.localhost").contains("kept"));
}

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
    assert_eq!(
        node_line(Path::new("/Users/a b/root.pem")),
        "NODE_EXTRA_CA_CERTS='/Users/a b/root.pem'"
    );
    assert!(untrust_command("macos", "AB12").contains("delete-certificate -t -Z AB12"));
}

/// A machine that is asked, and keeps what it was asked.
struct Asked {
    os: &'static str,
    refuses: bool,
    said: RefCell<Vec<String>>,
}

impl Asked {
    fn on(os: &'static str) -> Self {
        Asked {
            os,
            refuses: false,
            said: RefCell::new(Vec::new()),
        }
    }
    fn events(&self) -> Vec<String> {
        self.said.borrow().clone()
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
}

#[test]
fn on_macos_it_says_what_the_root_is_for_then_asks_once_and_adds_it() {
    let dir = scratch("macos");
    let machine = Asked::on("macos");
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

    // Once: asked again, it neither asks nor adds, and makes nothing new.
    let again = Asked::on("macos");
    let never = |_: &str| -> bool { panic!("asked twice") };
    let held = prepare(&dir, "m", NOW, &again, &never, &mut |_: &str| {}).unwrap();
    assert_eq!(held.authority.root_pem(), prepared.authority.root_pem());
    assert!(again.events().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn declined_or_refused_it_says_the_command_and_asks_again_next_time() {
    let dir = scratch("declined");
    let machine = Asked::on("macos");
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
    assert!(said
        .join("\n")
        .contains("security add-trusted-cert -r trustRoot"));

    let refusing = Asked {
        refuses: true,
        ..Asked::on("macos")
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
    assert_eq!(refusing.events().len(), 1);
    assert!(said.join("\n").contains("the person cancelled"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn elsewhere_it_prints_the_command_once_and_runs_nothing() {
    let dir = scratch("linux");
    let machine = Asked::on("linux");
    let mut said = Vec::new();
    let never = |_: &str| -> bool { panic!("nothing to ask on linux") };
    prepare(&dir, "m", NOW, &machine, &never, &mut |line: &str| {
        said.push(line.to_string())
    })
    .unwrap();
    assert!(machine.events().is_empty());
    assert!(said.join("\n").contains("update-ca-certificates"));
    let mut again = Vec::new();
    prepare(&dir, "m", NOW, &machine, &never, &mut |line: &str| {
        again.push(line.to_string())
    })
    .unwrap();
    assert!(again.is_empty(), "{again:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removing_it_takes_it_out_of_the_keychain_and_off_the_disk() {
    let dir = scratch("removed");
    let machine = Asked::on("macos");
    let prepared = prepare(&dir, "m", NOW, &machine, &|_: &str| true, &mut |_: &str| {}).unwrap();
    let sha1 = prepared.authority.sha1();
    let said = remove(&dir, &machine).unwrap();
    assert!(machine.events().contains(&format!("remove {sha1}")));
    assert!(said.contains("login keychain"), "{said}");
    assert!(!dir.exists());
    assert!(remove(&dir, &machine)
        .unwrap()
        .contains("nothing to remove"));
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

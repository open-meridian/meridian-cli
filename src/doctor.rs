//! Whether this machine and this cluster can run a deployment.
//!
//! Every answer names the fix rather than the symptom, because the failures
//! this exists to prevent are the ones that surface three minutes into an
//! install as something else: a migration job failing on a database URL, a pod
//! refusing to start over a mount, an assertion refused for a reason that
//! looks exactly like a bad key.
//!
//! It changes nothing. A check that writes is a check nobody runs twice.

use std::fmt;

pub mod checks;

/// What one check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// Nothing to do.
    Fine(String),
    /// Would stop an install, with the fix.
    Stops { what: String, fix: String },
    /// Worth knowing, and not in the way.
    Worth { what: String, why: String },
    /// Could not be checked from here, which is not the same as passing.
    Unknown { what: String, why: String },
}

impl Finding {
    pub fn stops(&self) -> bool {
        matches!(self, Finding::Stops { .. })
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Finding::Fine(what) => write!(f, "  ok       {what}"),
            Finding::Stops { what, fix } => write!(f, "  stops    {what}\n           {fix}"),
            Finding::Worth { what, why } => write!(f, "  worth    {what}\n           {why}"),
            Finding::Unknown { what, why } => write!(f, "  unknown  {what}\n           {why}"),
        }
    }
}

/// Why a command did not answer.
///
/// The two are different fixes and were one message for an afternoon: a
/// missing `kubectl` was reported as an unreachable cluster, and the advice
/// was to check KUBECONFIG on a machine that had no kubectl to read it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The program is not installed here.
    Missing(String),
    /// It ran, and this is what it said.
    Said(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Missing(program) => write!(f, "{program} is not installed here"),
            Failure::Said(said) => write!(f, "{said}"),
        }
    }
}

/// What a check needs from the world, so a test can be the world.
#[async_trait::async_trait]
pub trait Machine: Send + Sync {
    /// Run a command and return its output, or why it could not be run.
    async fn run(&self, program: &str, arguments: &[&str]) -> Result<String, Failure>;

    /// Fetch a URL with the headers a registry or a platform needs, returning
    /// the status, the body, and the `Date` header the server stated, which is
    /// what the clock is compared against.
    async fn fetch(&self, url: &str, headers: &[(&str, &str)]) -> Result<Answered, String>;

    /// This machine's clock, in seconds since the epoch.
    fn now_s(&self) -> u64;
}

#[derive(Debug, Clone, Default)]
pub struct Answered {
    pub status: u16,
    pub body: String,
    /// As the server stated it, in seconds since the epoch.
    pub date_s: Option<u64>,
}

/// What the deployment being checked is meant to be.
#[derive(Debug, Clone)]
pub struct Intended {
    pub namespace: String,
    pub platform: String,
    pub image: String,
}

/// Every check, in the order somebody reads them.
pub async fn examine(machine: &dyn Machine, intended: &Intended) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.push(checks::helm(machine).await);
    findings.extend(checks::cluster(machine, &intended.namespace).await);
    findings.push(checks::storage_class(machine).await);
    findings.push(checks::image(machine, &intended.image).await);
    findings.extend(checks::platform(machine, &intended.platform).await);
    findings
}

/// What to print, and what to exit with.
pub fn verdict(findings: &[Finding]) -> (String, i32) {
    let mut said = String::new();
    for finding in findings {
        said.push_str(&format!("{finding}\n"));
    }
    let stopping = findings.iter().filter(|finding| finding.stops()).count();
    if stopping == 0 {
        said.push_str("\ndoctor: nothing here would stop an install\n");
        return (said, 0);
    }
    said.push_str(&format!(
        "\ndoctor: {stopping} thing(s) would stop an install, each with its fix above\n"
    ));
    (said, 1)
}

#[cfg(test)]
#[path = "doctor/tests.rs"]
mod tests;

//! `meridian up`: install the chart, and open the wizard.
//!
//! The order is the thing this adds over typing the commands by hand -- check,
//! install, wait for the dashboard, forward a port, open the wizard, and say
//! what went wrong at the step it went wrong on. It renders nothing itself:
//! the chart is what says what runs, and an embedded renderer would be a
//! second thing that says it (spec/the-cli, requirement 3).
//!
//! With `--params` it answers the wizard's own endpoints from a file, behind
//! the same first-run claim code a browser uses. One implementation, so the
//! scripted route and the browser's cannot drift.

pub mod run;

use std::collections::{BTreeMap, BTreeSet};

/// What an install is told. Everything else the chart defaults.
#[derive(Debug, Clone)]
pub struct Install {
    pub release: String,
    pub namespace: String,
    pub chart: String,
    pub chart_version: Option<String>,
    pub deployment_id: String,
    pub enrolment_code: String,
    /// Only when somebody said so. Empty is the platform, which the chart
    /// resolves: an install that writes the address down is an install
    /// carrying a value it was never given, and the one place that address
    /// lives should be the chart rather than every values file in existence.
    ///
    /// Set to reach a different one -- a staging platform, while somebody is
    /// testing a change to the platform itself.
    pub platform: Option<String>,
    pub image: Option<String>,
    /// Helm-style chart values, spelled as Helm spells them.
    pub values: Vec<String>,
    pub timeout: String,
}

/// The values this passes to Helm, as a document on its standard input.
///
/// On stdin rather than in `--set`, because an enrolment code in `--set` is an
/// argument, and arguments are readable by every process on the machine. It is
/// single use and a day long, and that is a reason to be careful with it
/// rather than a reason not to be.
pub fn values_document(install: &Install) -> String {
    let mut out = String::from("# Written by `meridian up`. Everything else the chart defaults.\n");
    out.push_str("deployment:\n");
    out.push_str(&format!("  id: {}\n", quoted(&install.deployment_id)));
    out.push_str(&format!(
        "  enrolmentCode: {}\n",
        quoted(&install.enrolment_code)
    ));
    if let Some(platform) = &install.platform {
        out.push_str("platform:\n");
        out.push_str(&format!("  address: {}\n", quoted(platform)));
    }
    if let Some(image) = &install.image {
        let (repository, tag) = image.rsplit_once(':').unwrap_or((image.as_str(), "latest"));
        out.push_str("image:\n");
        out.push_str(&format!("  repository: {}\n", quoted(repository)));
        out.push_str(&format!("  tag: {}\n", quoted(tag)));
    }
    out
}

/// A YAML scalar that cannot be read as anything but the string it is.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// What is run, in the order Helm takes it.
///
/// The person's `-f` files come first and this command's own values last, so
/// what was typed on the command line wins over what a file happens to hold.
pub fn helm_arguments(install: &Install) -> Vec<String> {
    let mut arguments = vec![
        "upgrade".to_string(),
        "--install".to_string(),
        install.release.clone(),
        install.chart.clone(),
        "--namespace".to_string(),
        install.namespace.clone(),
        "--create-namespace".to_string(),
    ];
    if let Some(version) = &install.chart_version {
        arguments.push("--version".to_string());
        arguments.push(version.clone());
    }
    for values in &install.values {
        arguments.push("--values".to_string());
        arguments.push(values.clone());
    }
    // `--wait` is deliberately not here. A fresh install has no database and
    // no directory, and the components that need them wait at their Secret
    // until the wizard fills it: waiting for the whole release would be
    // waiting for answers nobody has given yet (spec/the-cli, ruling 4).
    arguments.push("--values".to_string());
    arguments.push("-".to_string());
    arguments.push("--timeout".to_string());
    arguments.push(install.timeout.clone());
    arguments
}

/// The same command, as somebody would type it.
///
/// The enrolment code is a credential, and a terminal keeps what it prints, so
/// what is shown reads the code from the environment rather than holding it.
pub fn shown(install: &Install) -> String {
    let values = values_document(install).replace(
        &quoted(&install.enrolment_code),
        "\"$MERIDIAN_ENROLMENT_CODE\"",
    );
    format!(
        "helm {} <<'VALUES'\n{}VALUES",
        helm_arguments(install).join(" "),
        values
    )
}

// ── The answers ──────────────────────────────────────────────────────────────

/// What a params file may not hold, refused on sight.
///
/// The wizard's own form is the authority on which fields are credentials, and
/// it is consulted below once there is a deployment to ask. This is the cheap
/// refusal that happens first, so a file with a password in it is refused on a
/// machine that has never reached a cluster.
const CREDENTIAL: [&str; 4] = ["password", "secret", "token", "passphrase"];

/// The answers, read from the file.
///
/// Scalars only: the wizard's form is flat, and a nested document would be
/// answering a shape it does not have.
pub fn params_from(text: &str) -> Result<BTreeMap<String, String>, String> {
    let document: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|failed| format!("that params file is not YAML: {failed}"))?;
    let serde_yaml::Value::Mapping(mapping) = document else {
        return Err("a params file is one flat mapping of the wizard's fields to answers".into());
    };

    let mut params = BTreeMap::new();
    for (name, value) in mapping {
        let name = name
            .as_str()
            .ok_or("every field in a params file is named by a string")?
            .to_string();
        if CREDENTIAL.iter().any(|word| name.contains(word)) {
            return Err(format!(
                "`{name}` holds a credential, and a params file is a file that gets committed. \
                 Leave it out and put it in {}, which this reads (spec/the-cli, requirement 10).",
                variable(&name)
            ));
        }
        let answer = match value {
            serde_yaml::Value::String(said) => said,
            serde_yaml::Value::Number(number) => number.to_string(),
            serde_yaml::Value::Bool(yes) => yes.to_string(),
            _ => {
                return Err(format!(
                    "`{name}` answers with something that is not a scalar. The wizard's form is \
                     flat: every field is one answer."
                ))
            }
        };
        params.insert(name, answer);
    }
    Ok(params)
}

/// Where a credential comes from instead of the file.
pub fn variable(field: &str) -> String {
    format!("MERIDIAN_{}", field.to_uppercase().replace('-', "_"))
}

/// What the wizard's own page asks for: every field, and which of them are
/// credentials.
///
/// Read from the page rather than from a list kept here. A list would be a
/// second statement of the form, and the one nobody updated would be the one
/// that refused a perfectly good answer.
pub fn asked_for(page: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut fields = BTreeSet::new();
    let mut credentials = BTreeSet::new();

    for tag in page.split('<').skip(1) {
        let tag = tag.split('>').next().unwrap_or_default();
        let element = tag.split([' ', '\t', '\n']).next().unwrap_or_default();
        if !matches!(element, "input" | "select" | "textarea") {
            continue;
        }
        let Some(name) = attribute(tag, "name") else {
            continue;
        };
        if attribute(tag, "type").as_deref() == Some("password") {
            credentials.insert(name.clone());
        }
        fields.insert(name);
    }

    (fields, credentials)
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let rest = &tag[at..];
    Some(rest[..rest.find('"')?].to_string())
}

/// The answers to post, from the file and the environment.
///
/// Refuses a field the wizard does not ask for, because a typo in a params
/// file otherwise leaves that answer empty and the failure arrives three steps
/// later as a database that cannot be reached.
pub fn answers(
    params: &BTreeMap<String, String>,
    fields: &BTreeSet<String>,
    credentials: &BTreeSet<String>,
    from_environment: &dyn Fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, String>, Vec<String>> {
    let mut refusals = Vec::new();
    let mut answers = BTreeMap::new();

    for (name, answer) in params {
        if credentials.contains(name) {
            refusals.push(format!(
                "`{name}` is a credential: the wizard asks for it as a password. \
                 Put it in {} instead.",
                variable(name)
            ));
            continue;
        }
        if !fields.contains(name) {
            let near = fields
                .iter()
                .find(|field| field.contains(name.as_str()) || name.contains(field.as_str()));
            refusals.push(match near {
                Some(field) => {
                    format!("this wizard does not ask for `{name}`. Did you mean `{field}`?")
                }
                None => format!("this wizard does not ask for `{name}`."),
            });
            continue;
        }
        answers.insert(name.clone(), answer.clone());
    }

    for name in credentials {
        match from_environment(&variable(name)) {
            Some(secret) => {
                answers.insert(name.clone(), secret);
            }
            // Not a refusal: choosing one way to sign in leaves the other
            // two's fields empty, and an empty credential for a route nobody
            // chose is correct. The wizard's own check is what says
            // whether an answer is missing.
            None => continue,
        }
    }

    if refusals.is_empty() {
        Ok(answers)
    } else {
        Err(refusals)
    }
}

/// The answers, as a form post carries them.
///
/// Written here rather than taken from a crate: the wizard's form is flat
/// strings, this is the whole of the encoding, and a dependency that pulls in
/// its own serialization stack to do it is a dependency to keep pure Rust
/// across four targets forever.
pub fn form_encoded(fields: &BTreeMap<String, String>) -> String {
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", escaped(name), escaped(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encoding, keeping only what a form may carry unescaped.
fn escaped(said: &str) -> String {
    let mut out = String::with_capacity(said.len());
    for byte in said.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ── What the wizard says back ────────────────────────────────────────────────

/// The wizard's findings, as it lists them.
pub fn findings(page: &str) -> Vec<String> {
    let mut found = Vec::new();
    for item in page.split("<li>").skip(1) {
        if let Some(finding) = item.split("</li>").next() {
            found.push(unescape(finding));
        }
    }
    found
}

/// Whether the answers passed, by the class the wizard marks it with.
pub fn passes(page: &str) -> bool {
    page.contains("class=\"passed\"")
}

/// Whether this is the page that asks for a code, which is what the wizard
/// answers with when a session has gone.
pub fn wants_a_code(page: &str) -> bool {
    page.contains("first-run/claim")
}

/// The first administrator's code, shown once.
pub fn first_admin_code(page: &str) -> Option<String> {
    let at = page.find("Your first administrator's code")?;
    let rest = &page[at..];
    let open = rest.find("<code>")? + "<code>".len();
    let close = rest[open..].find("</code>")?;
    Some(unescape(&rest[open..open + close]))
}

fn unescape(said: &str) -> String {
    said.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
#[path = "up/tests.rs"]
mod tests;

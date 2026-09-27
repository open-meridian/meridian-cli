//! `meridian down`: `up`'s mirror. The release uninstalled, and the namespace
//! too with `--delete-namespace`, since the namespace is where the deployment's
//! database and its own key live. Nothing is asked: saying `down` is the
//! decision (the product owner, 2026-09-27). Never the cluster itself.
//!
//! It drives your own `helm` and `kubectl`, as `up` does.

use crate::{connect, sessions, up};

pub struct Down {
    pub release: String,
    pub namespace: String,
    pub delete_namespace: bool,
}

/// Where a release was reached, from `helm get values -o json`: its Ingress's
/// name, when it had one. A session held for it answers nothing once it is gone.
pub fn reached_at(values: &str) -> Option<String> {
    let values: serde_json::Value = serde_json::from_str(values).ok()?;
    let ingress = &values["ingress"];
    if ingress["enabled"] != true {
        return None;
    }
    ingress["host"]
        .as_str()
        .filter(|host| !host.is_empty())
        .map(up::address_of)
}

async fn run(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new(program)
        .args(arguments)
        .output()
        .await
        .map_err(|failed| format!("{program} could not be run: {failed}. Is it installed?"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "`{program} {}`: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Uninstall, and delete the namespace when that was asked for. What was
/// done, to say.
pub async fn down(asked: &Down) -> Result<String, String> {
    let (release, namespace) = (asked.release.as_str(), asked.namespace.as_str());
    // Read before it is gone: where it was reached.
    let values = run(
        "helm",
        &[
            "get",
            "values",
            release,
            "--namespace",
            namespace,
            "-o",
            "json",
        ],
    )
    .await;
    let present = values.is_ok();
    if !present && !asked.delete_namespace {
        return Err(format!(
            "there is no release {release} in the namespace {namespace}, so nothing to uninstall"
        ));
    }
    let address = values.ok().as_deref().and_then(reached_at);

    let mut said = String::new();
    if present {
        run("helm", &["uninstall", release, "--namespace", namespace]).await?;
        said.push_str(&format!("Uninstalled {release} from {namespace}.\n"));
    }
    if asked.delete_namespace {
        run("kubectl", &["delete", "namespace", namespace]).await?;
        said.push_str(&format!(
            "Deleted the namespace {namespace}, and everything in it: the database the deployment \
             brought, with its data, and the deployment's key. The platform refuses it a new \
             enrolment code until that key is revoked there.\n"
        ));
    } else {
        said.push_str(&format!(
            "The namespace {namespace} is kept: `meridian up` again picks up the database the \
             deployment brought, and its key.\n"
        ));
    }

    if let Some(address) = address {
        if let Ok(within) = sessions::directory() {
            if let Ok(address) = connect::address(&address) {
                if sessions::read(&within, &address).is_some()
                    && sessions::forget(&within, &address).is_ok()
                {
                    said.push_str(&format!(
                        "Forgot this machine's session with {address}, which answers nothing now.\n"
                    ));
                }
            }
        }
    }
    said.push_str(
        "The deployment still exists on the platform. Retiring it there is what revokes its key \
         and stops its codes working.\n",
    );
    Ok(said)
}

#[cfg(test)]
mod tests;

//! Bringing it up: the commands, the port-forward, and the wizard.
//!
//! Everything here touches the world. What can be decided without touching it
//! is in the parent module, with its tests.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use super::{
    administrator, answers, asked_for, findings, helm_arguments, params_from, passes, shown,
    values_document, wants_a_code,
};

/// How long to wait for the dashboard to answer before saying so.
const DASHBOARD_TIMEOUT: Duration = Duration::from_secs(300);

pub struct Wizard {
    address: String,
    client: reqwest::Client,
    cookies: Vec<(String, String)>,
}

impl Wizard {
    fn new(address: String) -> Result<Self, String> {
        Ok(Self {
            address,
            client: reqwest::Client::builder()
                // A redirect followed silently is a cookie dropped: the second
                // request would go without the session the first one started.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(120))
                .build()
                .map_err(|failed| failed.to_string())?,
            cookies: Vec::new(),
        })
    }

    async fn get(&mut self, path: &str) -> Result<(u16, String), String> {
        self.send(self.client.get(format!("{}{path}", self.address)))
            .await
    }

    async fn post(
        &mut self,
        path: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<(u16, String), String> {
        self.send(
            self.client
                .post(format!("{}{path}", self.address))
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(super::form_encoded(fields)),
        )
        .await
    }

    async fn send(&mut self, request: reqwest::RequestBuilder) -> Result<(u16, String), String> {
        let request = match self.cookies.is_empty() {
            true => request,
            false => request.header(
                reqwest::header::COOKIE,
                self.cookies
                    .iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        };

        let response = request
            .send()
            .await
            .map_err(|failed| format!("the wizard did not answer: {failed}"))?;
        let status = response.status().as_u16();

        for value in response.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            let Some((name, rest)) = value.split_once('=') else {
                continue;
            };
            let held = rest.split(';').next().unwrap_or_default().to_string();
            self.cookies.retain(|(each, _)| each != name);
            self.cookies.push((name.to_string(), held));
        }

        let body = response.text().await.unwrap_or_default();
        Ok((status, body))
    }
}

/// `meridian up`, from an empty namespace to the wizard.
pub async fn up(
    install: &super::Install,
    port: u16,
    params: Option<&str>,
    first_run_code: Option<&str>,
) -> Result<(), String> {
    // Read before anything is installed: a file with a password in it should
    // be refused on the person's own machine, not after a release exists.
    let params = match params {
        Some(path) => Some(read_params(path)?),
        None => None,
    };

    println!(
        "Installing {} into {}:\n",
        install.release, install.namespace
    );
    println!("  {}\n", shown(install).replace('\n', "\n  "));
    helm(install).await?;
    println!("Installed.");

    let service = dashboard_service(install).await?;
    println!("Waiting for {service} to answer its health check.");
    wait_for_rollout(install, &service).await?;

    let mut forward = port_forward(install, &service, port).await?;
    let address = format!("http://127.0.0.1:{port}");
    let outcome = wizard(&address, params, first_run_code).await;

    // The forward is this process's; nothing should outlive it.
    let _ = forward.kill().await;
    outcome
}

fn read_params(path: &str) -> Result<BTreeMap<String, String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|failed| format!("{path} could not be read: {failed}"))?;
    params_from(&text).map_err(|refusal| format!("{path}: {refusal}"))
}

async fn helm(install: &super::Install) -> Result<(), String> {
    let mut child = tokio::process::Command::new("helm")
        .args(helm_arguments(install))
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|failed| format!("helm could not be run: {failed}. Is it installed?"))?;

    child
        .stdin
        .as_mut()
        .ok_or("helm would not take its values")?
        .write_all(values_document(install).as_bytes())
        .await
        .map_err(|failed| format!("helm would not take its values: {failed}"))?;
    drop(child.stdin.take());

    match child.wait().await {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "helm refused this install ({status}). Nothing was forwarded and nothing was answered."
        )),
        Err(failed) => Err(format!("helm could not be waited for: {failed}")),
    }
}

/// Asked of the cluster rather than rebuilt from the chart's naming rules,
/// which are the chart's business and change without telling this.
async fn dashboard_service(install: &super::Install) -> Result<String, String> {
    let named = kubectl(
        install,
        &[
            "get",
            "service",
            "-l",
            &format!(
                "app.kubernetes.io/instance={},meridian.dev/component=dashboard",
                install.release
            ),
            "-o",
            "name",
        ],
    )
    .await?;

    named
        .split_whitespace()
        .next()
        .map(String::from)
        .ok_or_else(|| {
            "this release has no dashboard service, so there is no wizard to open. \
             A deployment installed with dashboard.enabled=false is configured by \
             values rather than by a wizard."
                .to_string()
        })
}

async fn wait_for_rollout(install: &super::Install, service: &str) -> Result<(), String> {
    let deployment = service.replace("service/", "deployment/");
    let said = kubectl(
        install,
        &[
            "rollout",
            "status",
            &deployment,
            "--timeout",
            &format!("{}s", DASHBOARD_TIMEOUT.as_secs()),
        ],
    )
    .await?;
    print!("{said}");
    Ok(())
}

async fn kubectl(install: &super::Install, arguments: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("kubectl")
        .args(["--namespace", &install.namespace])
        .args(arguments)
        .output()
        .await
        .map_err(|failed| format!("kubectl could not be run: {failed}"))?;

    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(format!(
            "kubectl {}: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

/// A first-run dashboard has no public address and should not get one, so the
/// way in is a forward this process holds and drops.
async fn port_forward(
    install: &super::Install,
    service: &str,
    port: u16,
) -> Result<tokio::process::Child, String> {
    let child = tokio::process::Command::new("kubectl")
        .args([
            "--namespace",
            &install.namespace,
            "port-forward",
            service,
            &format!("{port}:80"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|failed| format!("kubectl could not be run: {failed}"))?;
    Ok(child)
}

/// Wait for the forward to carry something, then either hand the address over
/// or answer the wizard from the file.
async fn wizard(
    address: &str,
    params: Option<BTreeMap<String, String>>,
    first_run_code: Option<&str>,
) -> Result<(), String> {
    let mut wizard = Wizard::new(address.to_string())?;
    let deadline = std::time::Instant::now() + DASHBOARD_TIMEOUT;
    loop {
        match wizard.get("/first-run").await {
            Ok((200, _)) => break,
            Ok((404, _)) => {
                return Err("this deployment has been through first run already. \
                            Sign in at its own address."
                    .into())
            }
            _ if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_secs(2)).await
            }
            other => {
                return Err(format!(
                    "the wizard never answered at {address}: {other:?}. \
                     The dashboard is running; the forward is what did not carry."
                ))
            }
        }
    }

    let Some(params) = params else {
        println!("\nThe wizard is at {address}/first-run");
        println!("It asks for a first-run code, which a deployment administrator issues on the platform.");
        println!("\nThis forward is held open here. Press Ctrl-C when the wizard says it is done.");
        let _ = tokio::signal::ctrl_c().await;
        println!("\nForward closed. Nothing else was left running.");
        return Ok(());
    };

    let code = first_run_code.ok_or(
        "answering the wizard needs a first-run claim code: --first-run-code, \
         or MERIDIAN_FIRST_RUN_CODE, which keeps it out of a shell history.",
    )?;

    scripted(&mut wizard, params, code).await
}

/// The same endpoints a browser posts to, behind the same code.
async fn scripted(
    wizard: &mut Wizard,
    params: BTreeMap<String, String>,
    code: &str,
) -> Result<(), String> {
    let (_, page) = wizard
        .post(
            "/first-run/claim",
            &BTreeMap::from([("code".into(), code.to_string())]),
        )
        .await?;
    // A redeemed code answers with a redirect and a session; a refused one
    // answers with the page that asks again, and the platform's reason.
    if wants_a_code(&page) {
        return Err(refusal(&page, "that first-run code was refused"));
    }

    let (_, form) = wizard.get("/first-run").await?;
    let (fields, credentials) = asked_for(&form);
    let posted = answers(&params, &fields, &credentials, &|named| {
        std::env::var(named).ok().filter(|held| !held.is_empty())
    })
    .map_err(|refusals| {
        format!(
            "these answers are not this wizard's:\n  - {}",
            refusals.join("\n  - ")
        )
    })?;

    println!("\nTesting {} answers.", posted.len());
    let (_, tested) = wizard.post("/first-run/check", &posted).await?;
    if !passes(&tested) {
        return Err(format!(
            "the wizard refuses these answers, and wrote nothing:\n  - {}",
            findings(&tested).join("\n  - ")
        ));
    }
    println!("They pass. Applying.");

    let (_, applied) = wizard.post("/first-run/apply", &posted).await?;
    let Some(who) = administrator(&applied) else {
        return Err(format!(
            "applying did not finish:\n  - {}",
            findings(&applied).join("\n  - ")
        ));
    };

    println!("\nThis deployment is configured. {who}.");
    println!("Sign in at its dashboard the way you chose; there is nothing to redeem.");
    Ok(())
}

/// What the wizard said, with its markup taken off.
fn refusal(page: &str, when: &str) -> String {
    let said: String = page
        .split("class=\"refusal\">")
        .nth(1)
        .and_then(|rest| rest.split('<').next())
        .unwrap_or_default()
        .into();
    match said.is_empty() {
        true => when.to_string(),
        false => format!("{when}: {said}"),
    }
}

//! `meridian`: bringing a deployment up where there is a terminal.
//!
//! No install in a cloud depends on this. A marketplace listing's form and the
//! deployment's own wizard are the whole path there, and anything this makes
//! convenient is possible without it (spec/the-cli, requirement 16).

mod catalogue;
mod connect;
mod doctor;
mod machine;
mod plugin;
mod sessions;
mod up;

use doctor::Intended;

const USAGE: &str = "\
meridian -- bringing a Meridian deployment up

  meridian doctor            can this machine and this cluster run a deployment?
  meridian up                install the chart, and open this deployment's wizard
  meridian plugin new <name> start a plugin: the SDK's reference plugin, named <name>
  meridian plugin upload     build the plugin here and put it in the deployment's catalogue
  meridian plugin list       the catalogue: versions uploaded, and what is launched
  meridian plugin launch <name> <version> --instance <id>
                             run a version, once you approve the roles and tags it asks for
  meridian plugin stop <id>  stop a launched instance
  meridian connect <address> sign in to a deployment's dashboard, and keep the session
  meridian sign-out [<address>]
                             end that session, here and at the deployment

Both:
  -n, --namespace <name>    where the deployment goes (default: meridian)
      --platform <url>      the platform it reports to (default: https://open-meridian.com)
      --image <ref>         the runtime image to run (default: the published one)
  -h, --help                this

up:
      --release <name>      the Helm release (default: meridian)
      --chart <ref>         the chart to install (default: the published one)
      --chart-version <v>   which version of it
      --id <id>             this deployment's identifier, from the platform
      --enrolment-code <c>  the one-time code it enrols its key with. Better in
                            MERIDIAN_ENROLMENT_CODE, which no shell writes down
  -p, --params <file>       answer the wizard from a file instead of a browser.
                            It holds the wizard's own answers and no credential:
                            each of those comes from MERIDIAN_<FIELD>
      --first-run-code <c>  the claim code --params redeems (or MERIDIAN_FIRST_RUN_CODE)
  -f, --values <file>       Helm-style chart values, passed straight through
      --port <n>            the local port the wizard is forwarded to (default: 8443)
      --timeout <d>         how long to give Helm (default: 10m)
      --no-doctor           skip the checks. A check nobody runs does not exist

plugin new:
      --into <dir>          where to write it (default: ./<name>). Never somewhere
                            that already exists

plugin upload, list, launch, stop: as the deployment's administrator, through the
session `meridian connect` keeps
      --deployment <addr>   which connected deployment, when there is more than one
      --dir <dir>           upload: the plugin's directory (default: .). Its image is
                            built with docker, from its own Dockerfile
      --instance <id>       launch: the instance's name, which its page is found by
      --yes                 launch: approve what it asks for without being asked. For
                            a script that has already read it

connect:
  <address> is the dashboard's: https://<host>, or http://127.0.0.1:<port> for a
  local one. The sign-in opens in your browser; this never takes a password.
";

struct Arguments {
    command: String,
    /// The words after a command that takes them: `plugin new <name>`.
    words: Vec<String>,
    flags: Vec<(String, String)>,
    switches: Vec<String>,
}

/// Flags that take a value, so a switch is never read as one.
const TAKES_A_VALUE: [&str; 17] = [
    "--into",
    "--deployment",
    "--dir",
    "--instance",
    "-n",
    "--namespace",
    "--platform",
    "--image",
    "--release",
    "--chart",
    "--chart-version",
    "--id",
    "--enrolment-code",
    "-p",
    "--params",
    "--first-run-code",
    "--port",
];

/// Everything else, which takes no value. An unknown one is refused rather
/// than ignored: a misspelled `--no-doctor` that is quietly dropped installs
/// something the person asked not to have checked.
const SWITCHES: [&str; 5] = ["--no-doctor", "--yes", "-h", "--help", "-v"];

fn parse(said: Vec<String>) -> Result<Arguments, String> {
    let mut said = said.into_iter();
    let command = said.next().unwrap_or_default();
    let mut words = Vec::new();
    let mut flags = Vec::new();
    let mut switches = Vec::new();

    while let Some(argument) = said.next() {
        if !argument.starts_with('-') {
            // Only these take words. Anywhere else a stray one is refused
            // rather than ignored, as it always was.
            if matches!(command.as_str(), "plugin" | "connect" | "sign-out") {
                words.push(argument);
                continue;
            }
            return Err(format!("{argument} is not an option this takes"));
        }
        // Both spellings, because `--params=x` is what a script writes and
        // `--params x` is what a person types.
        if let Some((flag, value)) = argument.split_once('=') {
            flags.push((flag.to_string(), value.to_string()));
            continue;
        }
        if TAKES_A_VALUE.contains(&argument.as_str())
            || argument == "-f"
            || argument == "--values"
            || argument == "--timeout"
        {
            let value = said
                .next()
                .ok_or_else(|| format!("{argument} needs a value"))?;
            flags.push((argument, value));
            continue;
        }
        if !SWITCHES.contains(&argument.as_str()) {
            return Err(format!("{argument} is not an option this takes"));
        }
        switches.push(argument);
    }

    Ok(Arguments {
        command,
        words,
        flags,
        switches,
    })
}

impl Arguments {
    fn value(&self, long: &str, short: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(flag, _)| flag == long || flag == short)
            .map(|(_, value)| value.as_str())
    }

    fn every(&self, long: &str, short: &str) -> Vec<String> {
        self.flags
            .iter()
            .filter(|(flag, _)| flag == long || flag == short)
            .map(|(_, value)| value.clone())
            .collect()
    }

    fn set(&self, switch: &str) -> bool {
        self.switches.iter().any(|each| each == switch)
    }
}

/// A credential from the environment, which no shell history holds, or from
/// the flag, which one does.
fn credential(arguments: &Arguments, flag: &str, variable: &str) -> Option<String> {
    arguments
        .value(flag, flag)
        .map(String::from)
        .or_else(|| std::env::var(variable).ok())
        .filter(|held| !held.is_empty())
}

#[tokio::main]
async fn main() {
    let arguments = match parse(std::env::args().skip(1).collect()) {
        Ok(arguments) => arguments,
        Err(refusal) => {
            eprintln!("meridian: {refusal}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    if arguments.set("-v") {
        eprintln!("meridian: -v is reserved for verbose output, which is not built yet.");
    }

    if arguments.set("-h") || arguments.set("--help") {
        print!("{USAGE}");
        return;
    }

    let intended = Intended {
        namespace: arguments
            .value("--namespace", "-n")
            .unwrap_or("meridian")
            .into(),
        platform: arguments
            .value("--platform", "--platform")
            .unwrap_or("https://open-meridian.com")
            .into(),
        image: arguments
            .value("--image", "--image")
            .unwrap_or("ghcr.io/open-meridian/meridian-runtime:latest")
            .into(),
    };

    match arguments.command.as_str() {
        "doctor" => {
            let (said, code) = examined(&intended).await;
            print!("{said}");
            std::process::exit(code);
        }
        "up" => std::process::exit(brought_up(&arguments, intended).await),
        "plugin" => std::process::exit(plugin_command(&arguments).await),
        "connect" => std::process::exit(connect_command(&arguments).await),
        "sign-out" => std::process::exit(sign_out_command(&arguments).await),
        "-h" | "--help" | "help" | "" => print!("{USAGE}"),
        other => {
            eprintln!("meridian: {other} is not a command\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

/// `meridian plugin new <name>`, and the catalogue's commands: upload, list,
/// launch and stop (spec/the-local-plugin-registry). Test and share are the
/// spec's, and not built yet.
async fn plugin_command(arguments: &Arguments) -> i32 {
    let words: Vec<&str> = arguments.words.iter().map(String::as_str).collect();
    if let Some(&verb) = words.first() {
        if matches!(verb, "upload" | "list" | "launch" | "stop") {
            return catalogue_command(arguments, &words).await;
        }
    }
    match arguments.words.as_slice() {
        [new, name] if new == "new" => {
            let into = arguments
                .value("--into", "--into")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(name));
            match plugin::scaffold(name, &into) {
                Ok(_) => {
                    print!("{}", plugin::next_steps(name, &into));
                    0
                }
                Err(refusal) => {
                    eprintln!("meridian: {refusal}");
                    1
                }
            }
        }
        _ => {
            eprintln!("meridian: plugin takes `new <name>`, `upload`, `list`, `launch <name> <version>` or `stop <instance>`\n\n{USAGE}");
            2
        }
    }
}

/// The session a catalogue command acts through: the one held, or the one
/// `--deployment` names.
fn held_session(arguments: &Arguments) -> Result<sessions::Held, String> {
    let within = sessions::directory()?;
    if let Some(given) = arguments.value("--deployment", "--deployment") {
        let address = connect::address(given)?;
        return sessions::read(&within, &address).ok_or(format!(
            "not connected to {address}: `meridian connect {address}` first"
        ));
    }
    let mut every = sessions::all(&within);
    match every.len() {
        0 => Err("not connected to a deployment: `meridian connect <address>` first".into()),
        1 => Ok(every.remove(0)),
        _ => Err(format!(
            "connected to more than one deployment; say which with --deployment: {}",
            every
                .iter()
                .map(|held| held.address.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Asked at the terminal; anything but yes is no.
fn approved(question: &str) -> bool {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};
    if !std::io::stdin().is_terminal() {
        return false;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

async fn catalogue_command(arguments: &Arguments, words: &[&str]) -> i32 {
    let held = match held_session(arguments) {
        Ok(held) => held,
        Err(refusal) => {
            eprintln!("meridian plugin: {refusal}");
            return 2;
        }
    };
    let (address, session) = (held.address.as_str(), held.session.as_str());
    let done = match words {
        ["upload"] => {
            let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
            catalogue::upload(address, session, &dir)
                .await
                .map(|digest| format!("Uploaded to {address}, as {digest}.\n"))
        }
        ["list"] => catalogue::catalogue(address, session)
            .await
            .map(|held| catalogue::listed(&held)),
        ["launch", name, version] => {
            let Some(instance) = arguments.value("--instance", "--instance") else {
                eprintln!("meridian plugin launch: --instance <id> names what it runs as");
                return 2;
            };
            if !catalogue::is_name(instance) {
                eprintln!("meridian plugin launch: `{instance}` is not an instance's name: lowercase letters, digits and single hyphens");
                return 2;
            }
            launched(arguments, address, session, name, version, instance).await
        }
        ["stop", instance] => catalogue::stop(address, session, instance)
            .await
            .map(|_| format!("Stopped {instance}.\n")),
        _ => {
            eprintln!("meridian: plugin takes `upload`, `list`, `launch <name> <version>` or `stop <instance>`\n\n{USAGE}");
            return 2;
        }
    };
    match done {
        Ok(said) => {
            print!("{said}");
            0
        }
        Err(refusal) => {
            eprintln!("meridian plugin {}: {refusal}", words[0]);
            1
        }
    }
}

/// W8.3: what the version asks for, shown, and approved by the person.
async fn launched(
    arguments: &Arguments,
    address: &str,
    session: &str,
    name: &str,
    version: &str,
    instance: &str,
) -> Result<String, String> {
    let held = catalogue::catalogue(address, session).await?;
    let (roles, tags) = catalogue::declared(&held, name, version).ok_or(format!(
        "{name} {version} is not in {address}'s catalogue: `meridian plugin list` shows what is"
    ))?;
    let listed = |names: &[String]| {
        if names.is_empty() {
            "none".to_string()
        } else {
            names.join(", ")
        }
    };
    println!("{name} {version} asks for");
    println!("  roles: {}", listed(&roles));
    println!("  tags:  {}", listed(&tags));
    if !arguments.set("--yes") && !approved(&format!("Launch it as {instance}, with these?")) {
        return Err(
            "not approved, so not launched. Where there is no terminal to ask at, --yes approves"
                .into(),
        );
    }
    catalogue::launch(address, session, name, version, instance, &roles, &tags).await?;
    Ok(format!("Launched {instance}: {name} {version}.\n"))
}

/// `meridian connect <address>`: W6.13 from this side.
async fn connect_command(arguments: &Arguments) -> i32 {
    let [given] = arguments.words.as_slice() else {
        eprintln!("meridian: connect takes the dashboard's address\n\n{USAGE}");
        return 2;
    };
    let address = match connect::address(given) {
        Ok(address) => address,
        Err(refusal) => {
            eprintln!("meridian connect: {refusal}");
            return 2;
        }
    };
    let within = match sessions::directory() {
        Ok(within) => within,
        Err(refusal) => {
            eprintln!("meridian connect: {refusal}");
            return 1;
        }
    };

    let (listener, redirect_uri) = match connect::listen().await {
        Ok(listening) => listening,
        Err(refusal) => {
            eprintln!("meridian connect: {refusal}");
            return 1;
        }
    };
    let pkce = connect::Pkce::new();
    let state = connect::state();
    let url = connect::authorize_url(&address, &redirect_uri, &pkce, &state);
    println!("Sign in to {address} in your browser. If it did not open, go to:\n\n  {url}\n");
    connect::open_browser(&url);

    let code = match connect::returned(&listener, &state, connect::WAIT).await {
        Ok(connect::Returned::Code(code)) => code,
        Ok(connect::Returned::Declined) => {
            eprintln!("meridian connect: not connected; the sign-in was declined or refused.");
            return 1;
        }
        Err(refusal) => {
            eprintln!("meridian connect: {refusal}. Run it again when you are ready.");
            return 1;
        }
    };
    let issued = match connect::exchange(&address, &code, &pkce.verifier, &redirect_uri).await {
        Ok(issued) => issued,
        Err(refusal) => {
            eprintln!("meridian connect: {refusal}");
            return 1;
        }
    };

    // One session per deployment on this machine: the one this replaces is
    // ended at the deployment, not left to lapse there on its own.
    if let Some(earlier) = sessions::read(&within, &address) {
        let _ = connect::sign_out(&address, &earlier.session).await;
    }
    let held = sessions::Held {
        address: address.clone(),
        session: issued.session,
        subject: issued.subject,
        expires_at: issued.expires_at,
    };
    if let Err(refusal) = sessions::write(&within, &held) {
        // Connected and unable to keep it: end it rather than leave a live
        // session nobody holds.
        let _ = connect::sign_out(&address, &held.session).await;
        eprintln!("meridian connect: {refusal}");
        return 1;
    }
    println!(
        "Connected to {address} as {}. The session ends after 30 minutes unused, \
         and at {} at the latest.",
        held.subject, held.expires_at
    );
    0
}

/// `meridian sign-out [<address>]`: W6.14 from this side.
async fn sign_out_command(arguments: &Arguments) -> i32 {
    let within = match sessions::directory() {
        Ok(within) => within,
        Err(refusal) => {
            eprintln!("meridian sign-out: {refusal}");
            return 1;
        }
    };
    let held = match arguments.words.as_slice() {
        [given] => {
            let address = match connect::address(given) {
                Ok(address) => address,
                Err(refusal) => {
                    eprintln!("meridian sign-out: {refusal}");
                    return 2;
                }
            };
            match sessions::read(&within, &address) {
                Some(held) => held,
                None => {
                    println!("Not connected to {address}.");
                    return 0;
                }
            }
        }
        [] => {
            let mut every = sessions::all(&within);
            match every.len() {
                0 => {
                    println!("Not connected to anything.");
                    return 0;
                }
                1 => every.remove(0),
                _ => {
                    eprintln!("meridian sign-out: connected to more than one; say which:");
                    for held in &every {
                        eprintln!("  meridian sign-out {}", held.address);
                    }
                    return 2;
                }
            }
        }
        _ => {
            eprintln!("meridian: sign-out takes at most one address\n\n{USAGE}");
            return 2;
        }
    };

    let told = connect::sign_out(&held.address, &held.session).await;
    if let Err(refusal) = sessions::forget(&within, &held.address) {
        eprintln!("meridian sign-out: could not forget the session: {refusal}");
        return 1;
    }
    match told {
        Ok(()) => println!("Signed out of {}.", held.address),
        // Forgotten here either way. What cannot be reached cannot be told,
        // and the session lapses there by itself within 30 minutes.
        Err(refusal) => println!(
            "Forgotten here, but {refusal}; the session there lapses on its own within 30 minutes."
        ),
    }
    0
}

async fn examined(intended: &Intended) -> (String, i32) {
    let findings = doctor::examine(&machine::ThisMachine, intended).await;
    doctor::verdict(&findings)
}

async fn brought_up(arguments: &Arguments, intended: Intended) -> i32 {
    // Run by `up` rather than asked for, because a check nobody runs is a
    // check that does not exist (spec/the-cli, ruling 3).
    if !arguments.set("--no-doctor") {
        let (said, code) = examined(&intended).await;
        print!("{said}");
        if code != 0 {
            eprintln!("Nothing was installed. Fix the above, or run again with --no-doctor.");
            return code;
        }
        println!();
    }

    let Some(deployment_id) = arguments.value("--id", "--id").map(String::from) else {
        eprintln!("meridian up: --id is this deployment's identifier, which the platform gave you when you registered it.");
        return 2;
    };
    let Some(enrolment_code) = credential(arguments, "--enrolment-code", "MERIDIAN_ENROLMENT_CODE")
    else {
        eprintln!(
            "meridian up: this install carries a one-time enrolment code in place of a key. \
             Put it in MERIDIAN_ENROLMENT_CODE, or pass --enrolment-code."
        );
        return 2;
    };

    let install = up::Install {
        release: arguments
            .value("--release", "--release")
            .unwrap_or("meridian")
            .into(),
        namespace: intended.namespace.clone(),
        chart: arguments
            .value("--chart", "--chart")
            .unwrap_or("oci://ghcr.io/open-meridian/charts/meridian-runtime")
            .into(),
        chart_version: arguments
            .value("--chart-version", "--chart-version")
            .map(String::from),
        deployment_id,
        enrolment_code,
        // Passed through only when given: the chart holds the address, so an
        // install that has not been told one should not write it down.
        platform: arguments
            .value("--platform", "--platform")
            .map(String::from),
        image: arguments.value("--image", "--image").map(String::from),
        values: arguments.every("--values", "-f"),
        timeout: arguments
            .value("--timeout", "--timeout")
            .unwrap_or("10m")
            .into(),
    };

    let port = match arguments
        .value("--port", "--port")
        .unwrap_or("8443")
        .parse::<u16>()
    {
        Ok(port) => port,
        Err(_) => {
            eprintln!("meridian up: --port takes a port number");
            return 2;
        }
    };

    let params = arguments.value("--params", "-p").map(String::from);
    let first_run_code = credential(arguments, "--first-run-code", "MERIDIAN_FIRST_RUN_CODE");

    match up::run::up(&install, port, params.as_deref(), first_run_code.as_deref()).await {
        Ok(()) => 0,
        Err(refusal) => {
            eprintln!("\nmeridian up: {refusal}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn said(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn a_switch_is_never_read_as_a_flags_value() {
        // `--no-doctor` after `-p` would eat it if switches and flags were not
        // told apart, and the install would run unchecked with no params.
        let arguments = parse(said("up -p first-run.yaml --no-doctor --id dep-7")).unwrap();

        assert_eq!(arguments.command, "up");
        assert_eq!(arguments.value("--params", "-p"), Some("first-run.yaml"));
        assert!(arguments.set("--no-doctor"));
        assert_eq!(arguments.value("--id", "--id"), Some("dep-7"));
    }

    #[test]
    fn both_spellings_of_a_flag_are_the_same_flag() {
        let arguments = parse(said("up --params=first-run.yaml")).unwrap();
        assert_eq!(arguments.value("--params", "-p"), Some("first-run.yaml"));
    }

    #[test]
    fn values_files_accumulate_in_the_order_they_were_given() {
        let arguments = parse(said("up -f a.yaml --values b.yaml")).unwrap();
        assert_eq!(arguments.every("--values", "-f"), vec!["a.yaml", "b.yaml"]);
    }

    #[test]
    fn a_flag_with_nothing_after_it_is_refused_rather_than_defaulted() {
        assert!(parse(said("up --id")).is_err());
    }

    #[test]
    fn an_unknown_switch_is_refused_rather_than_dropped() {
        // A misspelled --no-doctor that is quietly ignored installs something
        // the person asked not to have checked.
        assert!(parse(said("up --no-docter")).is_err());
    }

    #[test]
    fn plugin_takes_its_words_and_nothing_else_does() {
        let plugin = parse(said("plugin new snaptrade --into /tmp/s")).expect("parsed");
        assert_eq!(plugin.words, ["new", "snaptrade"]);
        assert_eq!(plugin.value("--into", "--into"), Some("/tmp/s"));
        assert!(parse(said("doctor snaptrade")).is_err());
    }

    #[test]
    fn connect_and_sign_out_take_an_address() {
        assert_eq!(
            parse(said("connect https://dash.firm.example"))
                .unwrap()
                .words,
            ["https://dash.firm.example"]
        );
        assert!(parse(said("sign-out")).unwrap().words.is_empty());
    }

    #[test]
    fn a_launch_takes_its_instance_and_its_approval() {
        let launch = parse(said(
            "plugin launch reference-plugin 0.1.0 --instance ref --deployment http://127.0.0.1:8443 --yes",
        ))
        .unwrap();
        assert_eq!(launch.words, ["launch", "reference-plugin", "0.1.0"]);
        assert_eq!(launch.value("--instance", "--instance"), Some("ref"));
        assert_eq!(
            launch.value("--deployment", "--deployment"),
            Some("http://127.0.0.1:8443")
        );
        assert!(launch.set("--yes"));
        assert!(parse(said("plugin upload --dir")).is_err());
    }

    #[test]
    fn a_stray_word_is_refused_rather_than_ignored() {
        assert!(parse(said("up dep-7")).is_err());
    }
}

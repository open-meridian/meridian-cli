//! `meridian`: bringing a deployment up where there is a terminal.
//!
//! No install in a cloud depends on this. A marketplace listing's form and the
//! deployment's own wizard are the whole path there, and anything this makes
//! convenient is possible without it (spec/the-cli, requirement 16).

mod doctor;
mod machine;
mod up;

use doctor::Intended;

const USAGE: &str = "\
meridian -- bringing a Meridian deployment up

  meridian doctor    can this machine and this cluster run a deployment?
  meridian up        install the chart, and open this deployment's wizard

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
";

struct Arguments {
    command: String,
    flags: Vec<(String, String)>,
    switches: Vec<String>,
}

/// Flags that take a value, so a switch is never read as one.
const TAKES_A_VALUE: [&str; 13] = [
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
const SWITCHES: [&str; 4] = ["--no-doctor", "-h", "--help", "-v"];

fn parse(said: Vec<String>) -> Result<Arguments, String> {
    let mut said = said.into_iter();
    let command = said.next().unwrap_or_default();
    let mut flags = Vec::new();
    let mut switches = Vec::new();

    while let Some(argument) = said.next() {
        if !argument.starts_with('-') {
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
        "-h" | "--help" | "help" | "" => print!("{USAGE}"),
        other => {
            eprintln!("meridian: {other} is not a command\n\n{USAGE}");
            std::process::exit(2);
        }
    }
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
        platform: intended.platform.clone(),
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
    fn a_stray_word_is_refused_rather_than_ignored() {
        assert!(parse(said("up dep-7")).is_err());
    }
}

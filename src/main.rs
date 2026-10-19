//! `meridian`: bringing a deployment up where there is a terminal.
//!
//! No install in a cloud depends on this. A marketplace listing's form and the
//! deployment's own wizard are the whole path there, and anything this makes
//! convenient is possible without it (spec/the-cli, requirement 16).

mod authority;
mod backup;
mod bundle;
mod catalogue;
mod check;
mod connect;
mod credential;
mod doctor;
mod down;
#[cfg(test)]
mod fake_http;
mod gc;
mod live;
mod machine;
mod migrate;
mod plugin;
mod plugin_release;
mod registry;
mod release;
mod rollback;
mod sessions;
mod signing;
mod status;
mod up;
mod upgrade_deployment;

use doctor::Intended;

const USAGE: &str = "\
meridian -- bringing a Meridian deployment up

  meridian doctor            can this machine and this cluster run a deployment?
  meridian up                install the chart, and open this deployment's wizard
  meridian down              uninstall it; --delete-namespace removes its namespace too
  meridian upgrade-deployment
                             move a running deployment to a newer chart, in place,
                             after checking it can; not this binary
  meridian plugin new <name> start a plugin: the SDK's reference plugin, named <name>;
                             with --role connectivity, custody, data, operations or
                             reporting, that role's template
  meridian plugin check      hold the plugin here to the framework's rules: its pages,
                             settings, SDK use, [tool.meridian], documentation (docs/),
                             tests and shape
  meridian plugin migrate    move the plugin here to a newer release of its SDK: its pins
                             (a base pinned by digest takes the new release's digest)
                             and its CI's meridian, each release's rewrite of its code,
                             then plugin check
  meridian plugin upload     build the plugin here and put it in the deployment's catalogue,
                             its documentation (docs/) with it
  meridian plugin list       the catalogue: versions uploaded, and what is launched
  meridian plugin launch <name> <version> --instance <id>
                             run a version, once you approve the roles it asks for
  meridian plugin stop <id>  stop a launched instance
  meridian plugin dev --instance <id>
                             run the plugin here live on a development deployment,
                             sending each save as it is made
  meridian plugin logs --instance <id>
                             what a live plugin printed
  meridian plugin events --instance <id>
                             what happened to a live plugin: synced, restarted,
                             ready, crashed, refused, each with its revision
  meridian plugin open --instance <id> [--level manage|open|view]
                             a link to a plugin's page that one browser opens once
  meridian plugin publish --registry community|verified --tag vX.Y.Z
                             ask the platform to build that signed release tag of the
                             plugin here and list it, after plugin check passes; its
                             report a line a check (--details: each check's full text)
  meridian plugin release <version>
                             release the plugin here: checks (on main, clean, up to
                             date, main's CI green, the version higher than the last
                             tag), opens the release PR (pyproject.toml's version and
                             the CHANGELOG heading) and waits for its CI and for a
                             person to merge it; then signs v<version> on the merge
                             commit, pushes it, publishes it and follows the build.
                             --no-wait stops once the PR is open; run it again to go on
  meridian plugin key setup  make the key release tags are signed with ready: git's
                             SSH signing key, or a new one ssh-keygen makes (--hardware:
                             on a security key); git set to sign with it, and its public
                             half registered on the organisation
  meridian plugin key register <file.pub> | revoke <key-id> | list
                             the organisation's tag-signing keys at the platform
  meridian plugin gc         collect the deployment's plugin registry's garbage: the layers
                             retired versions left. Read-only while it runs (pulls go
                             on, uploads wait), then back; shown, then asked
  meridian plugin download <organisation>/<plugin>@<version>
                             write a plugin bundle of a listed version, verified, for
                             a deployment that reaches nothing
  meridian plugin upload --bundle <file>
                             carry a plugin bundle into the deployment's catalogue
  meridian connect [<address>]
                             sign in to a deployment's dashboard and let this computer
                             act as you, for up to 90 days
                             (default: https://meridian.localhost, the local install)
  meridian sign-out [<address>]
                             revoke that delegation, here and at the deployment
  meridian status            the release (revision, state, chart; Helm's words where it
                             is not deployed); each component and each plugin: its
                             image, readiness, restarts and why, each pod's memory
                             against its limit; and the book's health per account
  meridian logs <component>  what a component printed: core's, a plugin's by its instance,
                             or a Job's (migrate, key, first-run, backup), its newest run;
                             --since 10m for the last ten minutes, --previous for what
                             it printed before its last restart
  meridian rollback-deployment [--to-revision <n>]
                             return the deployment to an earlier revision of its release:
                             by default the last one deployed before the newest. The
                             database is not rolled back with it; shown, then asked
  meridian upgrade           replace this binary with the latest release; not a
                             deployment, which is upgrade-deployment
  meridian authority         this machine's certificate authority, which signs a local
                             deployment's HTTPS: where it is, and how clients trust it
  meridian authority trust   trust it here: the login keychain, which browsers and the
                             Claude Code CLI read, and NODE_EXTRA_CA_CERTS, which the
                             Claude app reads. Asked first; the install script runs it
  meridian authority remove  take it out of the login keychain and apps' environment,
                             and off this machine
  meridian backup            back the deployment up now, as its schedule does: its keys,
                             credentials, values and the database it brought, sealed to
                             backup.recipient; --to <dir> copies the file here too
  meridian backup key        make the identity backups are sealed with, and print its
                             public half, for backup.recipient
  meridian restore <file> --identity <file>
                             restore a backup into a namespace holding no deployment
  meridian uninstall         revoke every delegation this holds, and remove it
  meridian --version         which release this is

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
                            each of those comes from MERIDIAN_<FIELD>; and
                            `archive`, up's own question (below)
      --first-run-code <c>  the claim code --params redeems (or MERIDIAN_FIRST_RUN_CODE)
  -f, --values <file>       Helm-style chart values, passed straight through
      --archive <path>      where edge plugins' older records go: a directory on the
                            cluster's node, a NAS export or a second disk mounted
                            there. A deployment admin allows each plugin its part
      --no-archive          none: records past their window stay in each plugin's
                            storage. Given neither, up asks at a terminal, and
                            takes none where there is no terminal. In a cloud, name
                            the bucket as pluginArchive in a values file (-f)
      --host <name>         the name it is reached by through the cluster's ingress
                            controller (default: meridian.localhost, which every
                            browser sends to this machine). Any other name is
                            reached over HTTPS, with its certificate's Secret
                            named in a values file (-f)
      --no-ingress          reach it by a port-forward this command holds, as on a
                            cluster with no ingress controller
      --development         install it for development: it may run plugin code as
                            it is being written, and says so on every page
                            At a name under .localhost it is served over HTTPS, with
                            a certificate from this machine's own authority, made
                            and trusted the first time (`meridian authority`)
      --plain-http          serve plain HTTP and make no certificate: for the
                            cluster tests, and nothing else
      --port <n>            the local port a port-forward uses (default: 8443)
      --timeout <d>         how long to give Helm (default: 10m)
      --no-doctor           skip the checks. A check nobody runs does not exist

plugin new:
      --into <dir>          where to write it (default: ./<name>). Never somewhere
                            that already exists
      --role <role>         write that role's template, a whole plugin holding it
                            on the role's common pages and settings:
                            connectivity (a venue placed at through the SDK's
                            Trader, with its Trading page), custody (a
                            custodian's accounts and statements), data (a
                            vendor's prices into the lake, its tests running the
                            suite), operations (opening balances, reconciliation
                            and breaks) or reporting (the book valued from the
                            lake, its tests running the suite). Without it, the
                            reference plugin

plugin check: needs no deployment. Exits 0 when every rule holds, 1 when one does
not, each failure with its file, line and what to write instead. A warning
(replaced-ids-followed: an instrument ID kept in the code) fails nothing, and
neither does advice: a prompted section of docs/ missing (troubleshooting.md,
terms.md), a file there that is not uploaded, a heading's text past one screen
of the Documentation tab, or a link it shows as plain text
      --dir <dir>           the plugin's directory (default: .)
      --run-tests           run its tests too, with pytest
      --verified            hold it as a verified plugin is: every changing route
                            a tool for agents, none kept from them, each role it
                            holds with a suite (custody, data, connectivity) run
                            in its tests, each role's common pages, tools and
                            settings kept, and every act naming its check tool
                            (role-pages; without --verified, said as advice)
      --json                one JSON object on stdout

plugin migrate: needs no deployment, and docker. Exits 0 when migrated with nothing
left by hand and every rule holding; 1 when something is left by hand, a rule does
not hold, or it could not run (then nothing was changed); 2 when asked wrongly or
refused before changing anything: no pins, pins disagreeing, an older --to, or
changes git does not hold yet
      --to <version>        the release to move to (default: the latest). Never older
      --dir <dir>           the plugin's directory (default: .)
      --image <ref>         the SDK image the steps run in (default: the latest
                            release's, which carries every step)
      --force               migrate a directory git does not hold as it is
      --run-tests           run its tests in the check, as plugin check does
      --json                one JSON object on stdout

plugin upload, list, launch, stop, dev, logs, events, open: through the session
`meridian connect` keeps. They exit 0 when done, 1 when refused or failed, 2 when
asked wrongly, and 3 when there is no session or it has lapsed
      --deployment <addr>   which connected deployment, when there is more than one
      --dir <dir>           upload, dev: the plugin's directory (default: .). Its image
                            is built with docker, from its own Dockerfile
      --instance <id>       launch, dev, logs, events, open: the instance's name,
                            which its page is found by
      --yes                 launch, dev: approve what it asks for without being asked.
                            For a script that has already read it
      --json                dev, logs, events, open: JSON on stdout, one object, or one
                            per line for dev and --follow; progress goes to stderr
      --release             dev: upload the plugin as it is now as a version, and run
                            that version in place of the live instance
      --since <revision>    logs, events: only what came after that revision
      --follow              events: keep reporting them as they happen
      --print <path>        open: the page at that path on the plugin's host, as you
                            are served it, instead of a link
      --level <level>       open: the level its session carries, as the dashboard's
                            home offers them: manage (admin), open (write) or view
                            (read). Without it, the first you hold: Manage, then
                            Open, then View, as the home's first button opens

plugin publish, release, key, download: through the platform sign-in, made in your
browser the first time and kept 30 days; never a password. The platform is
--platform's. release asks GitHub with your own GitHub sign-in: GH_TOKEN or
GITHUB_TOKEN, else the credential git keeps for github.com
      --registry <r>        publish: community or verified; release: the same (default:
                            community); download: which registry's version (default:
                            community)
      --tag <vX.Y.Z>        publish: the release tag, signed with an SSH key registered on
                            the organisation, of the version pyproject.toml holds
      --organisation <namespace>
                            publish, release, key: the organisation the listing or the key is its
                            (default: the one the plugin's git remote names); a listing
                            is organisation/plugin
      --hardware            key setup: where git has no SSH signing key, make one held
                            on a FIDO security key (ssh-keygen -t ed25519-sk) rather
                            than a desktop key (-t ed25519)
      --reason <text>       key revoke: why, as the revocation records it
      --not-trusted-after <time>
                            key revoke: when the key stopped being trusted, as
                            2026-10-16T12:00Z (default: now)
      --to <file>           download: where the bundle goes (default:
                            <organisation>-<plugin>-<version>.plugin-bundle.tar)
      --timeout <d>         publish: how long to follow the build (default: 30m);
                            release: that, and how long to wait for the PR's CI and
                            its merge (default: 30m each)
      --no-wait             release: stop once the release PR is open
      --details             publish: every check's full text, where it prints one line
                            a check that fits the terminal

status, logs: read-only; the components and logs through kubectl and the release through
helm, as doctor and backup read them, and status's book health from the deployment,
through the session meridian connect keeps (--deployment names one)
      --release <name>      the Helm release (default: meridian)
      --since <span>        logs: only what was printed in the last 30s, 10m, 2h or 1d
      --previous            logs: what each container printed before its last restart
      --json                status: one JSON object on stdout

plugin gc: through kubectl, as backup is; checks first and changes nothing if one fails
      --release <name>      the Helm release (default: meridian)
      --timeout <d>         how long to give the registry each restart (default: 5m)
      --yes                 collect without being asked. For a script that has read
                            what it will do

down: nothing is asked, since saying down is the decision
      --release <name>      the Helm release (default: meridian)
      --delete-namespace    remove the namespace too, and with it the database the
                            deployment brought and the deployment's own key. Never
                            the cluster itself

upgrade-deployment: checks first, and changes nothing if a check fails; then shows
what it will do and asks
      --release <name>      the Helm release (default: meridian)
      --chart <ref>         the chart it was installed from (default: the published one)
      --chart-version <v>   the version to move to (default: the latest published).
                            Never an older one
      --timeout <d>         how long to wait for the migration and for every
                            component on the new image (default: 10m)
  -f, --values <file>       chart values to change, merged over the deployment's own
                            as --reset-then-reuse-values merges them; repeatable,
                            each over the last. What they change is shown, by path
                            and never by value, before it asks
      --https               move a local deployment, at a name under .localhost, to
                            HTTPS: a certificate from this machine's authority, then
                            the upgrade setting its address to https://<name>. A
                            certificate it wrote is renewed within 30 days of its end
                            with or without this
      --yes                 upgrade without being asked. For a script that has read
                            what it will do

rollback-deployment: checks first, and changes nothing if a check fails; then shows the
revision it returns to and asks. Then waits as upgrade-deployment does
      --release <name>      the Helm release (default: meridian)
      --to-revision <n>     the revision to return to, as `meridian status` and Helm number
                            them (default: the last one deployed before the newest)
      --timeout <d>         how long to wait for every component (default: 10m)
      --yes                 roll back without being asked. For a script that has read
                            what it will do

upgrade: this binary, not a deployment
      --to <version>        a named release instead of the latest, older or newer.
                            Nothing is looked up unless asked: this never checks
                            for a newer release on its own

uninstall:
      --yes                 remove without being asked. Sessions with deployments
                            it cannot reach are forgotten here and lapse there.
                            This machine's certificate authority goes too, out of
                            the login keychain and apps' environment

authority remove:
      --yes                 remove without being asked

backup: runs the deployment's backup CronJob once (set backup.recipient and
backup.path or backup.existingClaim first), and says what it wrote
      --release <name>      the Helm release (default: meridian)
      --to <dir>            copy the file to this directory here as well, its hash checked
      --timeout <d>         how long to wait for it (default: 10m)

backup key: needs no deployment
      --to <file>           where to write the identity (default: ./meridian-backup-identity.txt).
                            Never over a file already there. Keep it off the cluster:
                            a backup opens with it alone

restore: only into a namespace without the release and without anything a deployment
keeps: the backup's Secrets and ConfigMaps, the key's Secret, the database's and the
registry's claims. Checks first and changes nothing if one fails; then shows the
backup, the cluster, the namespace, the chart and what it will make, and asks. A
backup volume a deleted namespace left is deleted within that one approval, the
node's directory with its backups kept. The database's URL is withheld until the
database is loaded, the brought one or (from a dump of one somebody else runs) an
empty external one, and every table's row count matches the backup's; then the
components start
      --identity <file>     the identity the backup was sealed to (required)
  -n, --namespace <name>    where to restore it (default: where it was taken)
      --release <name>      must be the backup's own release, which it defaults to
      --chart <ref>         the chart to install (default: the published one)
      --chart-version <v>   which version (default: the one the backup was taken on,
                            from whatever --chart names). Never older than that one
      --timeout <d>         how long to wait for each step (default: 15m)
      --yes                 restore without being asked. For a script that has read
                            what it will do

connect:
  <address> is the dashboard's: https://<host>, or http://127.0.0.1:<port> for a
  local one. The sign-in opens in your browser; this never takes a password.
";

/// `meridian <command> --help`: that command's lines of the list, every
/// section of flags naming it, and the flags both take; the whole of the
/// usage for a command it does not know.
fn help_for(command: &str, words: &[String]) -> String {
    let with_sub = matches!(command, "plugin" | "backup" | "authority")
        .then(|| words.first().map(|sub| format!("{command} {sub}")))
        .flatten();
    let named = with_sub.clone().unwrap_or_else(|| command.to_string());
    if command.is_empty() || matches!(command, "help" | "-h" | "--help") {
        return USAGE.to_string();
    }
    let blocks: Vec<&str> = USAGE.split("\n\n").collect();
    // The list: each `  meridian <named>...` entry with its continuation.
    let mut listed = String::new();
    let mut taking = false;
    for line in blocks.iter().take(2).flat_map(|block| block.lines()) {
        if let Some(entry) = line.strip_prefix("  meridian ") {
            let first: Vec<&str> = entry.split_whitespace().collect();
            let says = |name: &str| {
                let parts: Vec<&str> = name.split(' ').collect();
                first.len() >= parts.len() && first[..parts.len()] == parts[..]
            };
            taking = says(&named) || (with_sub.is_none() && says(command));
        } else if !line.starts_with("    ") {
            taking = false;
        }
        if taking {
            listed.push_str(line);
            listed.push('\n');
        }
    }
    // The sections: a block whose head, before its first colon, names it.
    let mut sections = Vec::new();
    for block in &blocks[1..] {
        let head = block.lines().next().unwrap_or_default();
        if head.starts_with(' ') {
            continue;
        }
        let Some((names, _)) = head.split_once(':') else {
            continue;
        };
        let items: Vec<String> = names.split(", ").map(str::trim).map(String::from).collect();
        let plugin = items
            .first()
            .is_some_and(|first| first.starts_with("plugin "));
        let items: Vec<String> = items
            .into_iter()
            .map(|item| {
                if plugin && !item.starts_with("plugin ") {
                    format!("plugin {item}")
                } else {
                    item
                }
            })
            .collect();
        if items.iter().any(|item| *item == named || *item == command) {
            sections.push(*block);
        }
    }
    if listed.is_empty() && sections.is_empty() {
        return USAGE.to_string();
    }
    let both = blocks
        .iter()
        .find(|block| block.starts_with("Both:"))
        .copied()
        .unwrap_or_default();
    let mut out = format!("meridian {named}\n\n{listed}");
    for section in sections {
        out.push('\n');
        out.push_str(section);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(both);
    out.push('\n');
    out
}

struct Arguments {
    command: String,
    /// The words after a command that takes them: `plugin new <name>`.
    words: Vec<String>,
    flags: Vec<(String, String)>,
    switches: Vec<String>,
}

/// Flags that take a value, so a switch is never read as one.
const TAKES_A_VALUE: [&str; 32] = [
    "--to-revision",
    "--registry",
    "--tag",
    "--organisation",
    "--reason",
    "--not-trusted-after",
    "--bundle",
    "--identity",
    "--into",
    "--role",
    "--since",
    "--print",
    "--level",
    "--host",
    "--to",
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
    "--archive",
];

/// Everything else, which takes no value. An unknown one is refused rather
/// than ignored: a misspelled `--no-doctor` that is quietly dropped installs
/// something the person asked not to have checked.
const SWITCHES: [&str; 20] = [
    "--previous",
    "--details",
    "--hardware",
    "--no-wait",
    "--no-doctor",
    "--force",
    "--run-tests",
    "--verified",
    "--delete-namespace",
    "--json",
    "--follow",
    "--no-ingress",
    "--development",
    "--plain-http",
    "--no-archive",
    "--https",
    "--yes",
    "-h",
    "--help",
    "-v",
];

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
            if matches!(
                command.as_str(),
                "plugin" | "connect" | "sign-out" | "authority" | "backup" | "restore" | "logs"
            ) {
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
        // `up --release <name>` and `plugin gc --release <name>` name a Helm
        // release; `plugin dev --release` releases the code, and takes nothing.
        if argument == "--release" && command == "plugin" && words.first().is_some_and(|w| w == "dev") {
            switches.push(argument);
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
        print!("{}", help_for(&arguments.command, &arguments.words));
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
        "down" => std::process::exit(down_command(&arguments, &intended.namespace).await),
        "plugin" => std::process::exit(plugin_command(&arguments).await),
        "connect" => std::process::exit(connect_command(&arguments).await),
        "sign-out" => std::process::exit(sign_out_command(&arguments).await),
        "upgrade" => std::process::exit(upgrade_command(&arguments).await),
        "upgrade-deployment" => {
            std::process::exit(upgrade_deployment_command(&arguments, &intended.namespace).await)
        }
        "rollback-deployment" => {
            std::process::exit(rollback_command(&arguments, &intended.namespace).await)
        }
        "uninstall" => std::process::exit(uninstall_command(&arguments).await),
        "authority" => std::process::exit(authority_command(&arguments)),
        "backup" => std::process::exit(backup_command(&arguments, &intended.namespace).await),
        "status" => std::process::exit(status_command(&arguments, &intended.namespace).await),
        "logs" => std::process::exit(logs_command(&arguments, &intended.namespace).await),
        "restore" => std::process::exit(restore_command(&arguments).await),
        "--version" | "version" => println!("{}", release::version_line()),
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
    if words.first() == Some(&"check") {
        return check_command(arguments, &words);
    }
    if words.first() == Some(&"migrate") {
        return migrate_command(arguments, &words).await;
    }
    if words.first() == Some(&"publish") {
        return publish_command(arguments, &words, &platform_of(arguments)).await;
    }
    if words.first() == Some(&"gc") {
        return gc_command(arguments, &words).await;
    }
    if words.first() == Some(&"release") {
        return release_command(arguments, &words, &platform_of(arguments)).await;
    }
    if words.first() == Some(&"key") {
        return key_command(arguments, &words, &platform_of(arguments)).await;
    }
    if words.first() == Some(&"download") {
        return download_command(arguments, &words, &platform_of(arguments)).await;
    }
    if let Some(&verb) = words.first() {
        if matches!(verb, "upload" | "list" | "launch" | "stop") {
            return catalogue_command(arguments, &words).await;
        }
        if matches!(verb, "dev" | "logs" | "events" | "open") {
            return live_command(arguments, &words).await;
        }
    }
    match arguments.words.as_slice() {
        [new, name] if new == "new" => {
            let into = arguments
                .value("--into", "--into")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from(name));
            let role = arguments.value("--role", "--role");
            match plugin::scaffold(name, &into, role) {
                Ok(_) => {
                    print!("{}", plugin::next_steps(name, &into, role));
                    offer_key_setup(&into, &platform_of(arguments)).await;
                    0
                }
                Err(refusal) => {
                    eprintln!("meridian: {refusal}");
                    1
                }
            }
        }
        _ => {
            eprintln!("meridian: plugin takes `new <name>`, `check`, `migrate`, `upload`, `list`, `launch <name> <version>`, `stop <instance>`, `dev`, `logs`, `events`, `open`, `publish`, `release <version>`, `key` or `download`\n\n{USAGE}");
            2
        }
    }
}

/// `plugin new` on first use: where git has no SSH key to sign release tags
/// with, offered at a terminal, and said otherwise (ruling 256).
async fn offer_key_setup(into: &std::path::Path, platform: &str) {
    use std::io::IsTerminal as _;
    if signing::has_ssh_key(&signing::ThisWorkstation, into) {
        return;
    }
    if !std::io::stdin().is_terminal() {
        println!(
            "\nIts releases are tags signed with an SSH key, which git has none of here yet: \
             `meridian plugin key setup` makes one ready."
        );
        return;
    }
    println!(
        "\nIts releases are tags signed with an SSH key, which git has none of here yet. \
         `meridian plugin key setup` makes one ready (a desktop key; --hardware for one on a \
         security key), and you may run it later."
    );
    if approved("Set up a signing key now?") {
        set_up_key(into, false, None, platform).await;
    }
}

/// `meridian plugin gc` (ruling 166): the in-cluster registry read-only, its
/// garbage collected, then back; checked, shown and asked first.
async fn gc_command(arguments: &Arguments, words: &[&str]) -> i32 {
    if words.len() != 1 {
        eprintln!("meridian plugin gc: takes no words\n\n{USAGE}");
        return 2;
    }
    let timeout = arguments.value("--timeout", "--timeout").unwrap_or("5m");
    if upgrade_deployment::duration(timeout).is_none() {
        eprintln!("meridian plugin gc: --timeout takes a duration: 5m, 90s");
        return 2;
    }
    let asked = gc::Asked {
        release: arguments
            .value("--release", "--release")
            .unwrap_or("meridian")
            .into(),
        namespace: arguments
            .value("--namespace", "-n")
            .unwrap_or("meridian")
            .into(),
        timeout: timeout.into(),
    };
    let plan = match gc::check(&machine::ThisMachine, &asked).await {
        Ok(plan) => plan,
        Err(refused) => {
            eprintln!("meridian plugin gc: {refused}. Nothing was changed.");
            return 1;
        }
    };
    eprintln!("{}", gc::plan_text(&asked, &plan));
    if !arguments.set("--yes") && !approved("Collect it?") {
        eprintln!("meridian plugin gc: not approved, so nothing was changed. Where there is no terminal to ask at, --yes approves");
        return 1;
    }
    let mut say = |line: &str| eprintln!("{line}");
    match gc::apply(&machine::ThisMachine, &asked, &plan, &mut say).await {
        Ok(done) => {
            print!("{done}");
            0
        }
        Err(failed) => {
            eprintln!("meridian plugin gc: {failed}");
            1
        }
    }
}

/// `meridian plugin check`: the plugin in --dir held to the framework's
/// rules (decisions/025). It needs no session: it reads the directory.
fn check_command(arguments: &Arguments, words: &[&str]) -> i32 {
    if words.len() != 1 {
        eprintln!(
            "meridian plugin check: takes no words; --dir names the plugin's directory\n\n{USAGE}"
        );
        return 2;
    }
    let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
    let report = match check::check_as(
        &dir,
        arguments.set("--run-tests"),
        arguments.set("--verified"),
    ) {
        Ok(report) => report,
        Err(refusal) => {
            eprintln!("meridian plugin check: {refusal}");
            return 2;
        }
    };
    if arguments.set("--json") {
        println!("{}", check::json(&report));
    } else {
        print!("{}", check::text(&report));
    }
    if report.passed() {
        0
    } else {
        1
    }
}

/// `meridian plugin migrate`: the plugin in --dir moved to a newer release
/// of its SDK (decisions/025). It needs no session, and docker.
async fn migrate_command(arguments: &Arguments, words: &[&str]) -> i32 {
    if words.len() != 1 {
        eprintln!(
            "meridian plugin migrate: takes no words; --dir names the plugin's directory\n\n{USAGE}"
        );
        return 2;
    }
    let asked = migrate::Asked {
        dir: std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or(".")),
        to: arguments.value("--to", "--to").map(String::from),
        image: arguments.value("--image", "--image").map(String::from),
        force: arguments.set("--force"),
        run_tests: arguments.set("--run-tests"),
    };
    match migrate::migrate(&migrate::ThisHost, &migrate::PYTHON, &asked).await {
        migrate::Migrated::Refused(refusal) => {
            eprintln!("meridian plugin migrate: {refusal}");
            2
        }
        migrate::Migrated::Failed(failed) => {
            eprintln!("meridian plugin migrate: {failed}");
            1
        }
        migrate::Migrated::Done(report) => {
            if arguments.set("--json") {
                println!("{}", migrate::json(&report));
            } else {
                print!("{}", migrate::text(&report));
            }
            if report.done() {
                0
            } else {
                1
            }
        }
    }
}

/// The credential a catalogue command acts through: the one held, or the
/// one `--deployment` names.
fn held_session(arguments: &Arguments) -> Result<credential::Credential, String> {
    let within = sessions::directory()?;
    if let Some(given) = arguments.value("--deployment", "--deployment") {
        let address = connect::address(given)?;
        return sessions::read(&within, &address)
            .map(|held| credential::Credential::new(within, held))
            .ok_or(format!(
                "not connected to {address}: `{}` first",
                connect::command_for(&address)
            ));
    }
    let mut every = sessions::all(&within);
    match every.len() {
        0 => Err(
            "not connected to a deployment: `meridian connect` first, with the address if it is not this machine's"
                .into(),
        ),
        1 => Ok(credential::Credential::new(within, every.remove(0))),
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
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

async fn catalogue_command(arguments: &Arguments, words: &[&str]) -> i32 {
    let held = match held_session(arguments) {
        Ok(held) => held,
        Err(refusal) => {
            eprintln!("meridian plugin: {refusal}");
            return 3;
        }
    };
    let (address, session) = (held.address(), &held);
    let done = match words {
        ["upload"] if arguments.value("--bundle", "--bundle").is_some() => {
            let from = std::path::PathBuf::from(arguments.value("--bundle", "--bundle").unwrap());
            let keys: Vec<String> = bundle::PLATFORM_KEYS
                .iter()
                .map(|k| k.to_string())
                .collect();
            bundle::upload(address, session, &from, &keys)
                .await
                .map(|digest| {
                    format!(
                        "Carried {} into {address}, as {digest}; the deployment verifies it again \
                         against its chart's keys.\n",
                        from.display()
                    )
                })
        }
        ["upload"] => {
            let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
            catalogue::upload(address, session, &dir).await.map(|made| {
                let mut said = format!("Uploaded to {address}, as {}.\n", made.digest);
                if made.documented {
                    said.push_str(&format!(
                        "Its documentation, readable before it is launched: {}\n",
                        catalogue::version_documentation(address, &made.name, &made.version)
                    ));
                }
                said
            })
        }
        ["list"] => catalogue::catalogue(address, session)
            .await
            .map(|held| catalogue::listed(&held, address)),
        ["launch", name, version] => {
            let Some(instance) = arguments.value("--instance", "--instance") else {
                eprintln!("meridian plugin launch: --instance <id> names what it runs as");
                return 2;
            };
            if !catalogue::is_name(instance) {
                eprintln!("meridian plugin launch: `{instance}` is not an instance's name: lowercase letters, digits and single hyphens");
                return 2;
            }
            launched(arguments, address, session, name, version, instance, false).await
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
        Err(failed) => {
            eprintln!("meridian plugin {}: {}", words[0], failed.said());
            failed.code()
        }
    }
}

/// W8.3: what the version asks for, shown, and approved by the person.
async fn approval(
    arguments: &Arguments,
    address: &str,
    session: &credential::Credential,
    name: &str,
    version: &str,
    instance: &str,
    live: bool,
) -> Result<Vec<String>, live::Failed> {
    let held = catalogue::catalogue(address, session).await?;
    let roles = catalogue::declared(&held, name, version).ok_or(format!(
        "{name} {version} is not in {address}'s catalogue: `meridian plugin list` shows what is"
    ))?;
    let listed = |names: &[String]| {
        if names.is_empty() {
            "none".to_string()
        } else {
            names.join(", ")
        }
    };
    eprintln!("{name} {version} asks for");
    eprintln!("  roles: {}", listed(&roles));
    let how = if live { "live " } else { "" };
    if !arguments.set("--yes") && !approved(&format!("Launch it {how}as {instance}, with these?")) {
        return Err(live::Failed::Refused(
            "not approved, so not launched. Where there is no terminal to ask at, --yes approves"
                .into(),
        ));
    }
    Ok(roles)
}

async fn launched(
    arguments: &Arguments,
    address: &str,
    session: &credential::Credential,
    name: &str,
    version: &str,
    instance: &str,
    live: bool,
) -> Result<String, live::Failed> {
    let roles = approval(arguments, address, session, name, version, instance, live).await?;
    let asked = catalogue::Launch {
        name,
        version,
        instance,
        roles: &roles,
        live,
    };
    catalogue::launch(address, session, &asked).await?;
    // A deployment admin opens any plugin's page; anybody else, one they
    // hold access on (spec/deployment-dashboard-and-access, ruling 19).
    Ok(format!(
        "Launched {instance}: {name} {version}.\nIts page, if it serves one: {address}/plugins/{instance}\n\
         Its documentation: {}\n",
        catalogue::instance_documentation(address, instance)
    ))
}

// ── The live loop (spec/live-plugin-development, requirements 10 to 14) ──

async fn live_command(arguments: &Arguments, words: &[&str]) -> i32 {
    let verb = words[0];
    let held = match held_session(arguments) {
        Ok(held) => held,
        Err(refusal) => {
            eprintln!("meridian plugin {verb}: {refusal}");
            return 3;
        }
    };
    let Some(instance) = arguments.value("--instance", "--instance") else {
        eprintln!("meridian plugin {verb}: --instance <id> names the instance");
        return 2;
    };
    if !catalogue::is_name(instance) {
        eprintln!("meridian plugin {verb}: `{instance}` is not an instance's name: lowercase letters, digits and single hyphens");
        return 2;
    }
    let since = match arguments.value("--since", "--since").map(str::parse::<u64>) {
        None => None,
        Some(Ok(since)) => Some(since),
        Some(Err(_)) => {
            eprintln!("meridian plugin {verb}: --since takes a revision, a whole number");
            return 2;
        }
    };
    // Refused here rather than by the dashboard, so a misspelt level never
    // opens the plugin at the default one instead.
    let level = match arguments.value("--level", "--level") {
        None => None,
        Some(_) if verb != "open" => {
            eprintln!(
                "meridian plugin {verb}: --level is open's: the level a plugin's page is opened at"
            );
            return 2;
        }
        Some(named) => match live::Level::named(named) {
            Some(level) => Some(level),
            None => {
                eprintln!(
                    "meridian plugin open: `{named}` is not a level: {}",
                    live::LEVELS
                );
                return 2;
            }
        },
    };
    let deployment = live::Deployment {
        address: held.address(),
        session: &held,
    };
    let json = arguments.set("--json");
    let done = match words {
        ["dev"] if arguments.set("--release") => {
            release(arguments, &deployment, instance, json).await
        }
        ["dev"] => develop(arguments, &deployment, instance, json).await,
        ["logs"] => logs(&deployment, instance, since, json).await,
        ["events"] => {
            events(
                &deployment,
                instance,
                since,
                arguments.set("--follow"),
                json,
            )
            .await
        }
        ["open"] => match arguments.value("--print", "--print") {
            Some(path) => printed(&deployment, instance, path, level, json).await,
            None => opened(&deployment, instance, level, json).await,
        },
        _ => {
            eprintln!("meridian: plugin {verb} takes no words; --instance <id> names the instance\n\n{USAGE}");
            return 2;
        }
    };
    match done {
        Ok(()) => 0,
        Err(failed) => {
            eprintln!("meridian plugin {verb}: {}", failed.said());
            failed.code()
        }
    }
}

/// `plugin dev`: upload once, launch live, send the directory, then each
/// save, reporting every event with its revision (requirement 10).
async fn develop(
    arguments: &Arguments,
    deployment: &live::Deployment<'_>,
    instance: &str,
    json: bool,
) -> Result<(), live::Failed> {
    let (address, session) = (deployment.address, deployment.session);
    let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
    let pyproject = std::fs::read_to_string(dir.join("pyproject.toml"))
        .map_err(|failed| format!("{} has no pyproject.toml: {failed}", dir.display()))?;
    let metadata = catalogue::metadata(&pyproject)?;
    let (name, version) = (metadata.name.as_str(), metadata.version.as_str());

    let held = catalogue::catalogue(address, session).await?;
    let running = held["launches"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|launch| launch["instance_id"] == instance && launch["state"] == "launched");
    match running {
        Some(launch) if launch["live"] == true => {
            eprintln!(
                "{instance} is live already; sending {} to it.",
                dir.display()
            );
        }
        Some(launch) => {
            return Err(live::Failed::Refused(format!(
                "{instance} runs {} {} as a version, not live: `meridian plugin stop {instance}` \
                 first, or choose another --instance",
                launch["name"].as_str().unwrap_or_default(),
                launch["version"].as_str().unwrap_or_default(),
            )));
        }
        None => {
            if catalogue::declared(&held, name, version).is_some() {
                eprintln!(
                    "{name} {version} is uploaded already, so it is what runs, with {}'s files \
                     sent over it. A change to its dependencies needs a new version.",
                    dir.display()
                );
            } else {
                catalogue::upload(address, session, &dir).await?;
            }
            let said = launched(arguments, address, session, name, version, instance, true).await?;
            eprint!("{said}");
        }
    }
    watch(deployment, instance, &dir, json).await
}

/// Send what changed, as it changes, until interrupted; report every event.
async fn watch(
    deployment: &live::Deployment<'_>,
    instance: &str,
    dir: &std::path::Path,
    json: bool,
) -> Result<(), live::Failed> {
    let ignored = live::Ignored::of(dir);
    let mut sent = live::Snapshot::new();
    let mut seen = live::Seen::default();
    let mut since: Option<u64> = None;
    let mut waiting_said = String::new();
    let mut scan = tokio::time::interval(live::SCAN);
    let mut poll = tokio::time::interval(live::POLL);
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    eprintln!(
        "Watching {} for {instance}. Ctrl-C stops watching; the instance keeps running.",
        dir.display()
    );
    loop {
        tokio::select! {
            _ = &mut interrupted => {
                eprintln!("Stopped watching. {instance} runs on, live: `meridian plugin stop {instance}` stops it.");
                return Ok(());
            }
            _ = scan.tick() => {
                if live::scan(dir, &ignored) == sent {
                    continue;
                }
                // A save is often several writes: the directory as it is a
                // moment later, not halfway.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let now = live::scan(dir, &ignored);
                let change = live::change(dir, &sent, &now)?;
                if change.is_empty() {
                    sent = now;
                    continue;
                }
                match deployment.send(instance, &change).await {
                    Ok(revision) => {
                        waiting_said.clear();
                        since = Some(revision.saturating_sub(1));
                        if json {
                            println!("{}", serde_json::json!({ "event": "sent", "revision": revision,
                                "files": change.files.len(), "deleted": change.deleted.len() }));
                        } else {
                            println!("r{revision} sent ({} files, {} deleted)", change.files.len(), change.deleted.len());
                        }
                        sent = now;
                    }
                    Err(live::Failed::Session(said)) => return Err(live::Failed::Session(said)),
                    // Not up yet, or restarting: tried again at the next scan,
                    // and said once.
                    Err(live::Failed::Refused(said)) => {
                        if said != waiting_said {
                            eprintln!("Not sent yet, trying again: {said}");
                            waiting_said = said;
                        }
                    }
                }
            }
            _ = poll.tick() => {
                let Ok(said) = deployment.events(instance, since).await else {
                    continue;
                };
                for event in seen.new_in(&said) {
                    if json {
                        println!("{event}");
                    } else {
                        println!("{}", live::line(&event));
                    }
                }
            }
        }
    }
}

/// `plugin dev --release`: the directory as it is, uploaded as a version and
/// run in place of the live instance (requirement 13, ruling 7).
async fn release(
    arguments: &Arguments,
    deployment: &live::Deployment<'_>,
    instance: &str,
    json: bool,
) -> Result<(), live::Failed> {
    let (address, session) = (deployment.address, deployment.session);
    let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
    let pyproject = std::fs::read_to_string(dir.join("pyproject.toml"))
        .map_err(|failed| format!("{} has no pyproject.toml: {failed}", dir.display()))?;
    let metadata = catalogue::metadata(&pyproject)?;
    let (name, version) = (metadata.name.as_str(), metadata.version.as_str());
    let digest = catalogue::upload(address, session, &dir)
        .await
        .map_err(|failed| match failed {
            live::Failed::Refused(said) if said.contains("recorded already") => {
                live::Failed::Refused(format!(
                    "{name} {version} is recorded already, and a version is never replaced: \
                     raise the version in pyproject.toml, then release again"
                ))
            }
            other => other,
        })?
        .digest;
    let roles = approval(arguments, address, session, name, version, instance, false).await?;
    let held = catalogue::catalogue(address, session).await?;
    let running = held["launches"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|launch| launch["instance_id"] == instance && launch["state"] == "launched");
    if running {
        catalogue::stop(address, session, instance).await?;
    }
    let asked = catalogue::Launch {
        name,
        version,
        instance,
        roles: &roles,
        live: false,
    };
    catalogue::launch(address, session, &asked).await?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "instance_id": instance, "name": name,
            "version": version, "digest": digest })
        );
    } else {
        println!("Released {name} {version} as {digest}, and launched it as {instance}.");
    }
    Ok(())
}

/// `plugin logs`: what the plugin printed after a revision (requirement 11).
async fn logs(
    deployment: &live::Deployment<'_>,
    instance: &str,
    since: Option<u64>,
    json: bool,
) -> Result<(), live::Failed> {
    let said = deployment.output(instance, since).await?;
    if json {
        println!("{said}");
    } else {
        for line in said["lines"].as_array().into_iter().flatten() {
            println!("{}", line.as_str().unwrap_or_default());
        }
    }
    Ok(())
}

/// `plugin events`: what happened to it, after a revision, and as it happens
/// with --follow (requirement 11).
async fn events(
    deployment: &live::Deployment<'_>,
    instance: &str,
    since: Option<u64>,
    follow: bool,
    json: bool,
) -> Result<(), live::Failed> {
    let mut seen = live::Seen::default();
    let said = deployment.events(instance, since).await?;
    if json && !follow {
        println!("{said}");
        return Ok(());
    }
    let report = |event: &serde_json::Value| {
        if json {
            println!("{event}");
        } else {
            println!("{}", live::line(event));
        }
    };
    seen.new_in(&said).iter().for_each(report);
    if !follow {
        return Ok(());
    }
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    let mut poll = tokio::time::interval(live::POLL);
    loop {
        tokio::select! {
            _ = &mut interrupted => return Ok(()),
            _ = poll.tick() => {
                let said = deployment.events(instance, since).await?;
                seen.new_in(&said).iter().for_each(report);
            }
        }
    }
}

/// `plugin open`: a link to the plugin's host that one browser opens, once
/// (W6.15), at the level asked or the first the person holds (W6.9).
async fn opened(
    deployment: &live::Deployment<'_>,
    instance: &str,
    level: Option<live::Level>,
    json: bool,
) -> Result<(), live::Failed> {
    let opened = deployment.open(instance, level).await?;
    if json {
        let mut said = serde_json::json!({ "instance_id": instance, "url": opened.url });
        if let Some(level) = opened.level {
            said["level"] = level.name().into();
        }
        println!("{said}");
    } else {
        println!("{}", opened.url);
        let at = opened
            .level
            .map(|level| format!(", at {}", level.said()))
            .unwrap_or_default();
        eprintln!("One browser may open it, within a minute; it signs that browser in to {instance}'s page alone{at}.");
    }
    Ok(())
}

/// `plugin open --print <path>`: the page as the person is served it
/// (W6.15), at the level asked or the first they hold (W6.9). The plugin's
/// own error is printed, and is a failure.
async fn printed(
    deployment: &live::Deployment<'_>,
    instance: &str,
    path: &str,
    level: Option<live::Level>,
    json: bool,
) -> Result<(), live::Failed> {
    use std::io::Write as _;
    let said = deployment.page(instance, path, level).await?;
    let status = said["status"].as_u64().unwrap_or(0);
    if json {
        println!("{said}");
    } else if let Some(body) = said["body"].as_str() {
        print!("{body}");
    } else if let Some(encoded) = said["body_base64"].as_str() {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|failed| failed.to_string())?;
        let _ = std::io::stdout().write_all(&bytes);
    }
    if !(200..300).contains(&status) {
        let served = said["level"].as_str().and_then(live::Level::named);
        return Err(live::Failed::Refused(live::page_failed(
            instance, path, status, level, served,
        )));
    }
    Ok(())
}

/// `meridian down`: `up`'s mirror (spec/the-cli, "teardown and relaunch is
/// scriptable").
async fn down_command(arguments: &Arguments, namespace: &str) -> i32 {
    if !arguments.words.is_empty() {
        eprintln!("meridian down: takes no words\n\n{USAGE}");
        return 2;
    }
    let asked = down::Down {
        release: arguments
            .value("--release", "--release")
            .unwrap_or("meridian")
            .to_string(),
        namespace: namespace.to_string(),
        delete_namespace: arguments.set("--delete-namespace"),
    };
    match down::down(&asked).await {
        Ok(said) => {
            print!("{said}");
            0
        }
        Err(refusal) => {
            eprintln!("meridian down: {refusal}");
            1
        }
    }
}

// ── The registries (contract v25) ────────────────────────────────────────

/// The platform the registries are, as `--platform` names it.
fn platform_of(arguments: &Arguments) -> String {
    arguments
        .value("--platform", "--platform")
        .unwrap_or("https://open-meridian.com")
        .trim_end_matches('/')
        .to_string()
}

/// The platform sign-in a command acts with, made in the browser if none is
/// kept.
async fn signed_in(platform: &str) -> Result<registry::SignedIn, String> {
    connect::address(platform)?;
    registry::SignedIn::for_platform(platform, Box::new(|url: &str| connect::open_browser(url)))
        .await
}

/// `meridian plugin publish --registry community|verified --tag vX.Y.Z`
/// (W8.8): exits 0 when the version is listed or queued for review, 1 when
/// it is refused or could not be asked, 2 when asked wrongly.
async fn publish_command(arguments: &Arguments, words: &[&str], platform: &str) -> i32 {
    if words.len() != 1 {
        eprintln!("meridian plugin publish: takes no words; --dir names the plugin's directory\n\n{USAGE}");
        return 2;
    }
    let (Some(registry), Some(tag)) = (
        arguments.value("--registry", "--registry"),
        arguments.value("--tag", "--tag"),
    ) else {
        eprintln!(
            "meridian plugin publish: --registry community|verified and --tag vX.Y.Z say what is \
             published, and where"
        );
        return 2;
    };
    let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
    let given = arguments.value("--organisation", "--organisation");
    let organisation = match registry::organisation(given, &dir, &registry::Git) {
        Ok(organisation) => organisation,
        Err(refused) => {
            eprintln!("meridian plugin publish: {refused}");
            return 2;
        }
    };
    if given.is_none() {
        eprintln!(
            "Publishing as {organisation}, the organisation the plugin's git remote names; \
             --organisation names another."
        );
    }
    let follow_for = match arguments.value("--timeout", "--timeout") {
        None => registry::FOLLOW_FOR,
        Some(given) => match upgrade_deployment::duration(given) {
            Some(duration) => duration,
            None => {
                eprintln!("meridian plugin publish: --timeout takes a duration: 30m, 1h");
                return 2;
            }
        },
    };
    let asked = registry::Publish {
        dir,
        registry: registry.into(),
        tag: tag.into(),
        organisation,
        follow_for,
        follow_every: registry::FOLLOW_EVERY,
        shown: registry::Shown {
            width: registry::terminal_width(),
            details: arguments.set("--details"),
        },
    };
    published(&asked, platform, "publish").await
}

/// The publish path, `plugin publish`'s and `plugin release`'s: the
/// pre-flight, the request under a push token, the build followed, and its
/// report.
async fn published(asked: &registry::Publish, platform: &str, verb: &str) -> i32 {
    // The pre-flight first, so a plugin that would fail is told before any
    // browser is opened.
    if let Err(refused) = registry::preflight(asked, &registry::Git) {
        eprintln!("meridian plugin {verb}: {refused}");
        return 1;
    }
    let session = match signed_in(platform).await {
        Ok(session) => session,
        Err(failed) => {
            eprintln!("meridian plugin {verb}: {failed}");
            return 1;
        }
    };
    match registry::publish(asked, &registry::Git, &session).await {
        Ok(build) => {
            print!("{}", registry::said(&build, platform, &asked.shown));
            match build["state"].as_str() {
                Some("PLUGIN_BUILD_STATE_REFUSED") => 1,
                Some("PLUGIN_BUILD_STATE_IN_REVIEW") => {
                    if let Some(listing) = build["listing"].as_str() {
                        println!("  its listing, once reviewed: {platform}/plugins/{listing}");
                    }
                    0
                }
                _ => 0,
            }
        }
        Err(failed) => {
            eprintln!("meridian plugin {verb}: {failed}");
            1
        }
    }
}

/// `meridian plugin release <version>` (ruling 256): the checks, the release
/// PR and its CI, the merge a person makes, then the signed tag on the merge
/// commit, published and followed. `--no-wait` stops after the PR; running
/// it again resumes.
async fn release_command(arguments: &Arguments, words: &[&str], platform: &str) -> i32 {
    let ["release", version] = words else {
        eprintln!("meridian plugin release: takes the version, as `meridian plugin release 1.4.0`\n\n{USAGE}");
        return 2;
    };
    let version = version.trim_start_matches('v');
    let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
    let registry = arguments
        .value("--registry", "--registry")
        .unwrap_or("community");
    if !matches!(registry, "community" | "verified") {
        eprintln!("meridian plugin release: --registry is community or verified");
        return 2;
    }
    let organisation = match registry::organisation(
        arguments.value("--organisation", "--organisation"),
        &dir,
        &registry::Git,
    ) {
        Ok(organisation) => organisation,
        Err(refused) => {
            eprintln!("meridian plugin release: {refused}");
            return 2;
        }
    };
    let timeout = match arguments.value("--timeout", "--timeout") {
        None => registry::FOLLOW_FOR,
        Some(given) => match upgrade_deployment::duration(given) {
            Some(duration) => duration,
            None => {
                eprintln!("meridian plugin release: --timeout takes a duration: 30m, 1h");
                return 2;
            }
        },
    };
    let repo = plugin_release::Checkout(dir.clone());
    let github = match plugin_release::Repository::origin(&repo)
        .and_then(|origin| registry::github_repository(&origin))
        .and_then(|url| {
            plugin_release::owner_and_name(&url).ok_or_else(|| format!("{url} names no repository"))
        }) {
        Ok((owner, repository)) => match plugin_release::github_token() {
            Ok(token) => plugin_release::Rest {
                api: "https://api.github.com".into(),
                owner,
                repository,
                token,
            },
            Err(refused) => {
                eprintln!("meridian plugin release: {refused}");
                return 1;
            }
        },
        Err(refused) => {
            eprintln!("meridian plugin release: {refused}");
            return 2;
        }
    };
    let today = registry::moment(credential::now_s() * 1_000_000_000)[..10].to_string();
    let asked = plugin_release::Asked {
        dir: dir.clone(),
        version: version.to_string(),
        wait: !arguments.set("--no-wait"),
        every: std::time::Duration::from_secs(15),
        timeout,
        today,
    };
    let mut say = |line: &str| eprintln!("{line}");
    let tag = match plugin_release::release(&asked, &repo, &github, &mut say).await {
        Ok(plugin_release::Reached::Stopped(said)) => {
            println!("{said}");
            return 0;
        }
        Ok(plugin_release::Reached::Tagged { tag, .. }) => tag,
        Err(refused) => {
            eprintln!("meridian plugin release: {refused}");
            return 1;
        }
    };
    let publish = registry::Publish {
        dir,
        registry: registry.into(),
        tag,
        organisation,
        follow_for: timeout,
        follow_every: registry::FOLLOW_EVERY,
        shown: registry::Shown {
            width: registry::terminal_width(),
            details: arguments.set("--details"),
        },
    };
    published(&publish, platform, "release").await
}

/// `meridian plugin key register <file.pub> | revoke <key-id> | list`
/// (W5.30), the organisation's tag-signing keys; and `setup` (ruling 256).
async fn key_command(arguments: &Arguments, words: &[&str], platform: &str) -> i32 {
    let dir = std::path::PathBuf::from(arguments.value("--dir", "--dir").unwrap_or("."));
    if words == ["key", "setup"] {
        return key_setup(arguments, &dir, platform).await;
    }
    let organisation = match registry::organisation(
        arguments.value("--organisation", "--organisation"),
        &dir,
        &registry::Git,
    ) {
        Ok(organisation) => organisation,
        Err(refused) => {
            eprintln!("meridian plugin key: {refused}");
            return 2;
        }
    };
    let organisation = organisation.as_str();
    enum Act {
        Register(String, String),
        Revoke(String, u64, String),
        List,
    }
    let act = match words {
        ["key", "register", file] => {
            let text = match std::fs::read_to_string(file) {
                Ok(text) => text,
                Err(failed) => {
                    eprintln!("meridian plugin key register: {file}: {failed}");
                    return 2;
                }
            };
            let line = match registry::public_key(&text) {
                Ok(line) => line,
                Err(refused) => {
                    eprintln!("meridian plugin key register: {refused}");
                    return 2;
                }
            };
            let name = registry::comment_of(&line).unwrap_or_else(|| {
                std::path::Path::new(file)
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "signing key".into())
            });
            Act::Register(line, name)
        }
        ["key", "revoke", key_id] if !key_id.starts_with("SGK-") => {
            eprintln!("meridian plugin key revoke: `{key_id}` is not a signing key's ID (SGK-...): `meridian plugin key list` names each");
            return 2;
        }
        ["key", "revoke", key_id] => {
            let Some(reason) = arguments
                .value("--reason", "--reason")
                .filter(|r| !r.trim().is_empty())
            else {
                eprintln!("meridian plugin key revoke: --reason <text> says why, as the revocation records it");
                return 2;
            };
            let now_ns = credential::now_s() * 1_000_000_000;
            let after = match arguments.value("--not-trusted-after", "--not-trusted-after") {
                None => now_ns,
                Some(given) => match registry::ns_of(given) {
                    Some(at) if at <= now_ns => at,
                    Some(_) => {
                        eprintln!("meridian plugin key revoke: --not-trusted-after is a time already past");
                        return 2;
                    }
                    None => {
                        eprintln!("meridian plugin key revoke: --not-trusted-after takes a time, as 2026-10-16T12:00Z");
                        return 2;
                    }
                },
            };
            Act::Revoke(key_id.to_string(), after, reason.to_string())
        }
        ["key", "list"] => Act::List,
        _ => {
            eprintln!("meridian plugin key: takes `setup`, `register <file.pub>`, `revoke <key-id>` or `list`\n\n{USAGE}");
            return 2;
        }
    };
    let session = match signed_in(platform).await {
        Ok(session) => session,
        Err(failed) => {
            eprintln!("meridian plugin key: {failed}");
            return 1;
        }
    };
    let done = match act {
        Act::Register(line, name) => registry::register_key(&session, organisation, &name, &line)
            .await
            .map(|key| {
                format!(
                    "Registered {} as {}, {}: a tag it signs is built, from now.\n",
                    key["fingerprint"].as_str().unwrap_or_default(),
                    key["key_id"].as_str().unwrap_or_default(),
                    key["name"].as_str().unwrap_or_default()
                )
            }),
        Act::Revoke(key_id, after, reason) => {
            registry::revoke_key(&session, organisation, &key_id, after, &reason)
                .await
                .map(|key| {
                    format!(
                "Revoked {}, not trusted after {}: a version built after it is refused, and \
                         installed copies are warned.\n",
                key["fingerprint"].as_str().unwrap_or(&key_id),
                registry::moment(after)
            )
                })
        }
        Act::List => registry::list_keys(&session, organisation)
            .await
            .map(|listed| registry::keys_said(&listed)),
    };
    match done {
        Ok(said) => {
            print!("{said}");
            0
        }
        Err(failed) => {
            eprintln!("meridian plugin key: {failed}");
            1
        }
    }
}

/// `meridian plugin key setup [--hardware]` (ruling 256): git's SSH signing
/// key, or one ssh-keygen makes, git set to sign with it, and its public half
/// registered on the organisation where the person may.
async fn key_setup(arguments: &Arguments, dir: &std::path::Path, platform: &str) -> i32 {
    let organisation = match arguments.value("--organisation", "--organisation") {
        Some(given) => match registry::organisation(Some(given), dir, &registry::Git) {
            Ok(organisation) => Some(organisation),
            Err(refused) => {
                eprintln!("meridian plugin key setup: {refused}");
                return 2;
            }
        },
        None => registry::organisation(None, dir, &registry::Git).ok(),
    };
    set_up_key(
        dir,
        arguments.set("--hardware"),
        organisation.as_deref(),
        platform,
    )
    .await
}

/// The key made ready and registered: `key setup`, and `plugin new`'s offer.
async fn set_up_key(
    dir: &std::path::Path,
    hardware: bool,
    organisation: Option<&str>,
    platform: &str,
) -> i32 {
    let asked = signing::Setup {
        dir: dir.to_path_buf(),
        hardware,
    };
    let ready = match signing::prepare(&signing::ThisWorkstation, &asked) {
        Ok(ready) => ready,
        Err(refused) => {
            eprintln!("meridian plugin key setup: {refused}");
            return 1;
        }
    };
    print!("{}", signing::said(&ready));
    let Some(organisation) = organisation else {
        println!(
            "Git signs release tags with it. Register its public half on your organisation once \
             the plugin's repository is on GitHub: `meridian plugin key setup` there again, or \
             `meridian plugin key register {} --organisation <namespace>`.",
            ready
                .file
                .as_ref()
                .map(|file| file.display().to_string())
                .unwrap_or_else(|| "<the key's .pub>".into())
        );
        return 0;
    };
    let session = match signed_in(platform).await {
        Ok(session) => session,
        Err(failed) => {
            eprintln!("meridian plugin key setup: {failed}");
            return 1;
        }
    };
    match signing::register(&session, organisation, &ready).await {
        Ok(signing::Registered::Already(key_id)) => {
            println!(
                "It is registered on {organisation} already, as {key_id}: a tag it signs is built."
            );
            0
        }
        Ok(signing::Registered::Made {
            key_id,
            fingerprint,
        }) => {
            println!(
                "Registered {fingerprint} on {organisation}, as {key_id}: a tag it signs is built, from now."
            );
            0
        }
        Ok(signing::Registered::Ask(said)) => {
            print!("{said}");
            1
        }
        Err(failed) => {
            eprintln!("meridian plugin key setup: {failed}");
            1
        }
    }
}

/// `meridian plugin download organisation/plugin@version` (W8.14): a
/// verified plugin bundle, or nothing written.
async fn download_command(arguments: &Arguments, words: &[&str], platform: &str) -> i32 {
    let ["download", given] = words else {
        eprintln!("meridian plugin download: takes <organisation>/<plugin>@<version>\n\n{USAGE}");
        return 2;
    };
    let (organisation, plugin, version) = match bundle::named(given) {
        Ok(named) => named,
        Err(refused) => {
            eprintln!("meridian plugin download: {refused}");
            return 2;
        }
    };
    let registry = arguments
        .value("--registry", "--registry")
        .unwrap_or("community");
    if !matches!(registry, "community" | "verified") {
        eprintln!("meridian plugin download: --registry is community or verified");
        return 2;
    }
    let to = arguments
        .value("--to", "--to")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| bundle::default_file(&organisation, &plugin, &version));
    let keys: Vec<String> = bundle::PLATFORM_KEYS
        .iter()
        .map(|k| k.to_string())
        .collect();
    let session = match signed_in(platform).await {
        Ok(session) => session,
        Err(failed) => {
            eprintln!("meridian plugin download: {failed}");
            return 1;
        }
    };
    let asked = bundle::Download {
        organisation,
        plugin,
        version,
        registry: registry.into(),
        to: to.clone(),
    };
    match bundle::download(&asked, &session, &keys).await {
        Ok(held) => {
            println!(
                "Wrote {}: {}@{} ({}), {}, verified against the platform's keys. Carry it in with \
                 `meridian plugin upload --bundle {}`.",
                to.display(),
                held.listing,
                held.version,
                held.registry,
                held.image_digest,
                to.display()
            );
            0
        }
        Err(failed) => {
            eprintln!("meridian plugin download: {failed}");
            1
        }
    }
}

// ── status and logs (item 122) ───────────────────────────────────────────

async fn status_command(arguments: &Arguments, namespace: &str) -> i32 {
    if !arguments.words.is_empty() {
        eprintln!("meridian status: takes no words\n\n{USAGE}");
        return 2;
    }
    let asked = status::Asked {
        release: arguments
            .value("--release", "--release")
            .unwrap_or("meridian")
            .into(),
        namespace: namespace.into(),
    };
    match status::status(&status::ThisCluster, &asked).await {
        Ok(mut held) => {
            held.accounts = match held_session(arguments) {
                Ok(session) => status::book_health(session.address(), &session).await,
                Err(refusal) => Err(refusal),
            };
            if arguments.set("--json") {
                println!("{}", status::json_of(&held));
            } else {
                print!(
                    "{}",
                    status::said(&held, credential::now_s() * 1_000_000_000, namespace)
                );
            }
            0
        }
        Err(failed) => {
            eprintln!("meridian status: {failed}");
            1
        }
    }
}

async fn logs_command(arguments: &Arguments, namespace: &str) -> i32 {
    let [component] = arguments.words.as_slice() else {
        eprintln!("meridian logs: takes the component, as `meridian status` names them\n\n{USAGE}");
        return 2;
    };
    let asked = status::Asked {
        release: arguments
            .value("--release", "--release")
            .unwrap_or("meridian")
            .into(),
        namespace: namespace.into(),
    };
    let read = status::Read {
        since: arguments.value("--since", "--since").map(String::from),
        previous: arguments.set("--previous"),
    };
    match status::logs(&status::ThisCluster, &asked, component, &read).await {
        Ok(printed) => {
            print!("{printed}");
            0
        }
        Err(failed) => {
            eprintln!("meridian logs: {failed}");
            1
        }
    }
}

/// `meridian backup`, once and now, and `meridian backup key` (task
/// kernel/a-deployment-is-backed-up-and-restored).
async fn backup_command(arguments: &Arguments, namespace: &str) -> i32 {
    match arguments.words.as_slice() {
        [word] if word == "key" => {
            let to = arguments
                .value("--to", "--to")
                .unwrap_or(backup::IDENTITY_FILE);
            let now = time::OffsetDateTime::now_utc().to_string();
            match backup::write_identity(std::path::Path::new(to), &now) {
                Ok(recipient) => {
                    println!(
                        "Wrote the identity to {to}, readable by you alone. Keep it off the cluster, \
                         and where a reset of this machine does not reach: a backup opens with it \
                         and nothing else.\n\nIts public half, for backup.recipient:\n  {recipient}"
                    );
                    0
                }
                Err(refusal) => {
                    eprintln!("meridian backup key: {refusal}");
                    1
                }
            }
        }
        [] => {
            let timeout = arguments.value("--timeout", "--timeout").unwrap_or("10m");
            let Some(timeout) = upgrade_deployment::duration(timeout) else {
                eprintln!("meridian backup: --timeout takes a duration as Helm writes one: 10m, 90s, 1h30m");
                return 2;
            };
            let asked = backup::Take {
                release: arguments
                    .value("--release", "--release")
                    .unwrap_or("meridian")
                    .into(),
                namespace: namespace.into(),
                to: arguments
                    .value("--to", "--to")
                    .map(std::path::PathBuf::from),
                timeout,
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or_default();
            match backup::take(&asked, now).await {
                Ok(said) => {
                    print!("{said}");
                    0
                }
                Err(refusal) => {
                    eprintln!("meridian backup: {refusal}");
                    1
                }
            }
        }
        _ => {
            eprintln!("meridian backup: takes `key` or nothing\n\n{USAGE}");
            2
        }
    }
}

/// `meridian restore <file> --identity <file>`.
async fn restore_command(arguments: &Arguments) -> i32 {
    let [file] = arguments.words.as_slice() else {
        eprintln!("meridian restore: name one backup file\n\n{USAGE}");
        return 2;
    };
    let Some(identity) = arguments.value("--identity", "--identity") else {
        eprintln!(
            "meridian restore: --identity <file> names the identity the backup was sealed to, \
             which `meridian backup key` wrote"
        );
        return 2;
    };
    let timeout = arguments.value("--timeout", "--timeout").unwrap_or("15m");
    let Some(timeout) = upgrade_deployment::duration(timeout) else {
        eprintln!(
            "meridian restore: --timeout takes a duration as Helm writes one: 10m, 90s, 1h30m"
        );
        return 2;
    };
    let asked = backup::Restore {
        file: file.into(),
        identity: identity.into(),
        namespace: arguments.value("--namespace", "-n").map(String::from),
        release: arguments.value("--release", "--release").map(String::from),
        chart: arguments
            .value("--chart", "--chart")
            .unwrap_or(backup::PUBLISHED_CHART)
            .into(),
        chart_version: arguments
            .value("--chart-version", "--chart-version")
            .map(String::from),
        timeout,
    };
    let mut step = |said: &str| eprintln!("  {said}");
    // Shown, then asked, once every check has passed and before anything is
    // changed; --yes answers for a script, and without a terminal and
    // without it nothing is changed.
    let yes = arguments.set("--yes");
    let mut approve = |plan: &str| {
        eprintln!("\n{plan}");
        yes || approved("Restore it here?")
    };
    match backup::restore(&asked, &mut step, &mut approve).await {
        Ok(report) => {
            print!("{report}");
            0
        }
        Err(refusal) => {
            eprintln!("meridian restore: {refusal}");
            1
        }
    }
}

/// `meridian upgrade-deployment`: a running deployment moved to a newer chart
/// (task kernel/upgrading-a-deployment-in-place). Not `upgrade`, which
/// replaces this binary.
async fn upgrade_deployment_command(arguments: &Arguments, namespace: &str) -> i32 {
    use upgrade_deployment::{run, watch};
    let timeout = arguments.value("--timeout", "--timeout").unwrap_or("10m");
    let Some(waited) = upgrade_deployment::duration(timeout) else {
        eprintln!(
            "meridian upgrade-deployment: --timeout takes a duration as Helm writes one: 10m, 90s, 1h30m"
        );
        return 2;
    };
    let asked = upgrade_deployment::Asked {
        release: arguments
            .value("--release", "--release")
            .unwrap_or("meridian")
            .into(),
        namespace: namespace.into(),
        chart: arguments
            .value("--chart", "--chart")
            .unwrap_or("oci://ghcr.io/open-meridian/charts/meridian-runtime")
            .into(),
        chart_version: arguments
            .value("--chart-version", "--chart-version")
            .map(String::from),
        timeout: timeout.into(),
        https: arguments.set("--https"),
        authority: None,
        values: arguments.every("--values", "-f"),
    };
    // This machine's authority: made and trusted for --https, the certificate
    // step coming before the upgrade; otherwise only read, to renew a
    // certificate it wrote.
    let mut asked = asked;
    // Where the authority is, when --https made or found it: said at the end.
    let mut prepared = None;
    asked.authority = if asked.https {
        match prepare_authority() {
            Ok(ready) => {
                prepared = Some(ready.dir);
                Some(std::sync::Arc::new(ready.authority))
            }
            Err(refusal) => {
                eprintln!("meridian upgrade-deployment: {refusal}. Nothing was changed.");
                return 1;
            }
        }
    } else {
        authority::directory()
            .ok()
            .and_then(|dir| authority::Authority::read(&dir).ok().flatten())
            .map(std::sync::Arc::new)
    };
    let asked = asked;
    // The steps as they go: a block redrawn in place on a terminal, and
    // lines anywhere else.
    let mut watch = watch::to_stdout(format!(
        "Upgrading {} in {}",
        asked.release, asked.namespace
    ));
    let plan = match run::check(&machine::ThisMachine, &asked, watch.as_mut()).await {
        run::Checked::Refused => {
            watch.finish();
            return 1;
        }
        run::Checked::Current(said) => {
            watch.finish();
            println!("\n{said}");
            return 0;
        }
        run::Checked::Upgrade(plan) => plan,
    };
    watch.begin(watch::Step::Confirmation);
    watch.say(&format!(
        "\n{}",
        upgrade_deployment::plan_text(&asked, &plan)
    ));
    let yes = arguments.set("--yes") || approved("Upgrade it?");
    watch.end(watch::Step::Confirmation, yes);
    if !yes {
        watch.finish();
        eprintln!("meridian upgrade-deployment: not approved, so nothing was changed. Where there is no terminal to ask at, --yes approves");
        return 1;
    }
    let pace = run::Pace {
        poll: std::time::Duration::from_secs(2),
        timeout: waited,
    };
    let applied = run::apply(&machine::ThisMachine, &asked, &plan, &pace, watch.as_mut()).await;
    watch.finish();
    match applied {
        Ok(report) => {
            print!("{}", upgrade_deployment::report_text(&asked, &report));
            if let (Some(host), Some(dir)) = (&plan.https, &prepared) {
                println!("\nIt is at https://{host} now.");
                println!("{}", up::run::trusted_by(dir));
            }
            0
        }
        Err(refusal) => {
            eprintln!("\nmeridian upgrade-deployment: {refusal}");
            1
        }
    }
}

/// `meridian rollback-deployment [--to-revision <n>]` (audit D4): checks,
/// shows the revision it returns to and that the database stays, asks, then
/// rolls back and waits.
async fn rollback_command(arguments: &Arguments, namespace: &str) -> i32 {
    if !arguments.words.is_empty() {
        eprintln!("meridian rollback-deployment: takes no words; --to-revision <n> names a revision\n\n{USAGE}");
        return 2;
    }
    let to_revision = match arguments.value("--to-revision", "--to-revision") {
        None => None,
        Some(given) => match given.parse::<u64>() {
            Ok(revision) if revision > 0 => Some(revision),
            _ => {
                eprintln!("meridian rollback-deployment: --to-revision takes a revision's number, as `meridian status` says it");
                return 2;
            }
        },
    };
    let timeout = arguments.value("--timeout", "--timeout").unwrap_or("10m");
    let Some(waited) = upgrade_deployment::duration(timeout) else {
        eprintln!(
            "meridian rollback-deployment: --timeout takes a duration as Helm writes one: 10m, 90s, 1h30m"
        );
        return 2;
    };
    let asked = rollback::Asked {
        release: arguments
            .value("--release", "--release")
            .unwrap_or("meridian")
            .into(),
        namespace: namespace.into(),
        to_revision,
        timeout: timeout.into(),
    };
    let plan = match rollback::check(&machine::ThisMachine, &asked).await {
        Ok(plan) => plan,
        Err(refused) => {
            eprintln!("meridian rollback-deployment: {refused}. Nothing was changed.");
            return 1;
        }
    };
    eprintln!("{}", rollback::plan_text(&asked, &plan));
    if !arguments.set("--yes") && !approved("Roll it back?") {
        eprintln!("meridian rollback-deployment: not approved, so nothing was changed. Where there is no terminal to ask at, --yes approves");
        return 1;
    }
    let pace = rollback::Pace {
        poll: std::time::Duration::from_secs(2),
        timeout: waited,
    };
    let mut say = |line: &str| eprintln!("{line}");
    match rollback::apply(&machine::ThisMachine, &asked, &plan, &pace, &mut say).await {
        Ok(done) => {
            print!("{done}");
            0
        }
        Err(failed) => {
            eprintln!("meridian rollback-deployment: {failed}");
            1
        }
    }
}

/// The address `connect` signs in to: the one given, or with none, the
/// deployment `meridian up` installs on this machine by default.
fn connect_address(words: &[String]) -> Option<String> {
    match words {
        [] => Some(up::local_address()),
        [given] => Some(given.clone()),
        _ => None,
    }
}

/// `meridian connect [<address>]`: W6.13 and W6.17 from this side.
async fn connect_command(arguments: &Arguments) -> i32 {
    let Some(given) = connect_address(&arguments.words) else {
        eprintln!(
            "meridian: connect takes one dashboard address, or none for this machine's\n\n{USAGE}"
        );
        return 2;
    };
    let address = match connect::address(&given) {
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
    // This computer as a client: the one it registered as, when it holds a
    // delegation there already, so consenting again renews that one; or a
    // new registration.
    let earlier = sessions::read(&within, &address);
    let client_id = match earlier.as_ref().filter(|held| held.is_delegation()) {
        Some(held) => held.client_id.clone(),
        None => match connect::register(&address, &redirect_uri).await {
            Ok(client_id) => client_id,
            Err(refusal) => {
                eprintln!("meridian connect: {refusal}");
                return 1;
            }
        },
    };
    let pkce = connect::Pkce::new();
    let state = connect::state();
    let url = connect::authorize_url(&address, &client_id, &redirect_uri, &pkce, &state);
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
    let issued =
        match connect::exchange(&address, &client_id, &code, &pkce.verifier, &redirect_uri).await {
            Ok(issued) => issued,
            Err(refusal) => {
                eprintln!("meridian connect: {refusal}");
                return 1;
            }
        };

    let held = issued.held(&address, &client_id, credential::now_s());
    // Written under the lock, so a command refreshing at this moment reads
    // this pair after it rather than spending the one it replaces.
    let written = sessions::lock(&within, &address).and_then(|lock| {
        let written = sessions::write(&within, &held);
        drop(lock);
        written
    });
    if let Err(refusal) = written {
        // Connected and unable to keep it: revoke it rather than leave a live
        // delegation nobody holds.
        let _ = connect::sign_out(&held).await;
        eprintln!("meridian connect: {refusal}");
        return 1;
    }
    println!(
        "Connected to {address} as {}, until {}. This computer refreshes its access \
         by itself; `meridian sign-out` ends it.",
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

    let told = connect::sign_out(&held).await;
    if let Err(refusal) = sessions::forget(&within, &held.address) {
        eprintln!("meridian sign-out: could not forget the session: {refusal}");
        return 1;
    }
    match told {
        Ok(()) if !held.is_delegation() => println!(
            "Forgotten the session an older meridian kept for {}.",
            held.address
        ),
        Ok(()) => println!("Signed out of {}.", held.address),
        // Forgotten here either way. What cannot be reached cannot be told.
        Err(refusal) => println!(
            "Forgotten here, but {refusal}; revoke this computer's delegation from Connected \
             clients on the dashboard, or it lapses at {}.",
            held.expires_at
        ),
    }
    0
}

/// `meridian upgrade [--to <version>]` (spec/the-cli, ruling 8).
async fn upgrade_command(arguments: &Arguments) -> i32 {
    match upgraded(arguments).await {
        Ok(said) => {
            println!("{said}");
            0
        }
        Err(refusal) => {
            eprintln!("meridian upgrade: {refusal}");
            1
        }
    }
}

async fn upgraded(arguments: &Arguments) -> Result<String, String> {
    let base = release::releases()?;
    let exe = release::this_binary()?;
    let tag = match arguments.value("--to", "--to") {
        Some(named) => format!("v{}", named.trim_start_matches('v')),
        None => release::latest(&base).await?,
    };
    let now = release::VERSION;
    if release::compare(&tag, now) == std::cmp::Ordering::Equal {
        return Ok(format!("meridian {now} is {tag} already; nothing to do."));
    }
    // Before anything is fetched: a directory it cannot write is said now,
    // not after a download.
    release::writable(exe.parent().ok_or("this binary is in no directory")?)?;
    let bytes = release::download(&base, &tag).await?;
    release::replace(&exe, &bytes)?;
    let direction = match release::compare(&tag, now) {
        std::cmp::Ordering::Less => "down",
        _ => "up",
    };
    Ok(format!(
        "meridian {now} -> {}: {direction}graded {}.",
        tag.trim_start_matches('v'),
        exe.display()
    ))
}

/// `meridian uninstall [--yes]` (spec/the-cli, ruling 8): every session
/// ended and forgotten, then the sessions directory, then the binary. A
/// session left on disk after the binary is gone is a credential nobody
/// would think to remove.
async fn uninstall_command(arguments: &Arguments) -> i32 {
    let (exe, within) = match (release::this_binary(), sessions::directory()) {
        (Ok(exe), Ok(within)) => (exe, within),
        (Err(refusal), _) | (_, Err(refusal)) => {
            eprintln!("meridian uninstall: {refusal}");
            return 1;
        }
    };
    // Checked first, so nothing is ended if the binary cannot then go.
    if let Err(refusal) = exe
        .parent()
        .ok_or_else(|| "this binary is in no directory".to_string())
        .and_then(release::writable)
    {
        eprintln!("meridian uninstall: {refusal}");
        return 1;
    }
    let held = sessions::all(&within);
    println!("This removes:");
    for session in &held {
        if session.is_delegation() {
            println!(
                "  your delegation to this computer at {}, revoked there and forgotten here",
                session.address
            );
        } else {
            println!(
                "  the session an older meridian kept for {}, forgotten here",
                session.address
            );
        }
    }
    if within.exists() {
        println!("  {}", within.display());
    }
    // The platform sign-ins (contract v25): forgotten here, and lapsing at
    // the platform within 30 days, which takes no revocation from the CLI.
    let platform = registry::directory().ok().filter(|dir| dir.exists());
    if let Some(dir) = &platform {
        println!(
            "  your sign-ins to the platform's registries, {}, forgotten here",
            dir.display()
        );
    }
    let certificates = authority::directory().ok().filter(|dir| dir.exists());
    if let Some(dir) = &certificates {
        println!(
            "  this machine's certificate authority, {}, and its place in the login keychain \
             and in apps' NODE_EXTRA_CA_CERTS",
            dir.display()
        );
    }
    println!("  {}", exe.display());
    if !arguments.set("--yes") && !approved("Remove them?") {
        eprintln!("meridian uninstall: not approved, so nothing was removed. Where there is no terminal to ask at, --yes approves");
        return 1;
    }
    for session in &held {
        match connect::sign_out(session).await {
            Ok(()) if !session.is_delegation() => println!(
                "Forgotten the session an older meridian kept for {}.",
                session.address
            ),
            Ok(()) => println!("Signed out of {}.", session.address),
            Err(refusal) => println!(
                "Forgotten here, but {refusal}; revoke this computer's delegation from Connected \
                 clients on {}, or it lapses at {}.",
                session.address, session.expires_at
            ),
        }
        let _ = sessions::forget(&within, &session.address);
    }
    if let Some(dir) = &platform {
        if let Err(failed) = std::fs::remove_dir_all(dir) {
            eprintln!("meridian uninstall: {}: {failed}", dir.display());
            return 1;
        }
    }
    if let Some(dir) = &certificates {
        match authority::remove(dir, &authority::ThisMachine) {
            Ok(said) => print!("{said}"),
            Err(refusal) => {
                eprintln!("meridian uninstall: {refusal}");
                return 1;
            }
        }
    }
    if within.exists() {
        if let Err(failed) = std::fs::remove_dir_all(&within) {
            eprintln!("meridian uninstall: {}: {failed}", within.display());
            return 1;
        }
        // Its parent too, when this made it and nothing else is in it.
        if let Some(parent) = within.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    if let Err(failed) = std::fs::remove_file(&exe) {
        eprintln!("meridian uninstall: {}: {failed}", exe.display());
        return 1;
    }
    println!("Removed meridian {}.", release::VERSION);
    0
}

/// This machine's name, which its certificate authority is named for.
fn machine_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this machine".into())
}

/// Seconds since the epoch, which the authority's certificates are dated by.
fn seconds_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// This machine's certificate authority, made the first time, and the
/// machine asked to trust it once, having been told what it is for.
fn prepare_authority() -> Result<authority::Prepared, String> {
    let dir = authority::directory()?;
    authority::prepare(
        &dir,
        &machine_name(),
        seconds_now(),
        &authority::ThisMachine,
        &approved,
        &mut |line: &str| println!("{line}"),
    )
}

/// `meridian authority trust`: this machine's certificate authority, made the
/// first time, and trusted here once the person says so, the keychain and apps
/// on Node both; what the install script runs at a terminal on macOS.
fn trust_authority(dir: &std::path::Path) -> Result<(), String> {
    authority::trust_here(
        dir,
        &machine_name(),
        seconds_now(),
        &authority::ThisMachine,
        &approved,
        &mut |line: &str| println!("{line}"),
    )
}

/// `meridian authority [trust|remove] [--yes]`: where this machine's
/// certificate authority is and how a client trusts it; trusted here; or
/// taken away (ruling 5).
fn authority_command(arguments: &Arguments) -> i32 {
    let dir = match authority::directory() {
        Ok(dir) => dir,
        Err(refusal) => {
            eprintln!("meridian authority: {refusal}");
            return 1;
        }
    };
    let words: Vec<&str> = arguments.words.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => match authority::Authority::read(&dir) {
            Ok(Some(held)) => {
                let root = authority::root_path(&dir);
                println!("{}\n", authority::purpose(&dir));
                let os = std::env::consts::OS;
                let readers = match os {
                    "macos" => "A browser and the Claude Code CLI read the login keychain",
                    _ => "A browser reads the system's roots",
                };
                println!(
                    "{readers}; trusting it there:\n  {}",
                    authority::trust_command(os, &root)
                );
                if os == "macos" {
                    println!("{}", authority::apps(&dir, &authority::ThisMachine).said());
                } else {
                    println!(
                        "The Claude app, like any app on Node, reads {} instead:\n  {}",
                        authority::NODE_EXTRA_CA_CERTS,
                        authority::apps_command(os, &root)
                    );
                }
                println!("Its SHA-1 fingerprint: {}", held.sha1());
                0
            }
            Ok(None) => {
                println!(
                    "This machine has no certificate authority yet. `meridian up` makes one the \
                     first time it serves a deployment at a name under .localhost, and keeps it in {}.",
                    dir.display()
                );
                0
            }
            Err(refusal) => {
                eprintln!("meridian authority: {refusal}");
                1
            }
        },
        ["remove"] => {
            println!(
                "This removes this machine's certificate authority, {}, takes it out of the \
                 login keychain, and stops naming it to apps, putting back any \
                 NODE_EXTRA_CA_CERTS it replaced: deployments it signed for stop being trusted here.",
                dir.display()
            );
            if !arguments.set("--yes") && !approved("Remove it?") {
                eprintln!("meridian authority remove: not approved, so nothing was removed. Where there is no terminal to ask at, --yes approves");
                return 1;
            }
            match authority::remove(&dir, &authority::ThisMachine) {
                Ok(said) => {
                    print!("{said}");
                    0
                }
                Err(refusal) => {
                    eprintln!("meridian authority remove: {refusal}");
                    1
                }
            }
        }
        ["trust"] => match trust_authority(&dir) {
            Ok(()) => 0,
            Err(refusal) => {
                eprintln!("meridian authority trust: {refusal}");
                1
            }
        },
        _ => {
            eprintln!("meridian authority: takes nothing, `trust` or `remove`\n\n{USAGE}");
            2
        }
    }
}

async fn examined(intended: &Intended) -> (String, i32) {
    let findings = doctor::examine(&machine::ThisMachine, intended).await;
    doctor::verdict(&findings)
}

async fn brought_up(arguments: &Arguments, intended: Intended) -> i32 {
    // Before the doctor and long before Helm: a wrong identifier is otherwise
    // learned from a refused enrolment in a pod's log.
    let Some(deployment_id) = arguments.value("--id", "--id").map(String::from) else {
        eprintln!("meridian up: --id is this deployment's identifier, which the platform gave you when you registered it.");
        return 2;
    };
    if let Err(refusal) = up::check_id(&deployment_id) {
        eprintln!("meridian up: {refusal}. Nothing was installed.");
        return 2;
    }
    let archive = match up::archive_from_flags(
        arguments.value("--archive", "--archive"),
        arguments.set("--no-archive"),
    ) {
        Ok(archive) => archive,
        Err(refusal) => {
            eprintln!("meridian up: {refusal}. Nothing was installed.");
            return 2;
        }
    };

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
        ingress: None,
        development: arguments.set("--development"),
        tls_secret: None,
        plain_http: arguments.set("--plain-http"),
        archive,
    };
    let ingress_host = match arguments.set("--no-ingress") {
        true => None,
        false => Some(
            arguments
                .value("--host", "--host")
                .unwrap_or(up::LOCAL_HOST)
                .to_string(),
        ),
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

    match up::run::up(
        &install,
        port,
        params.as_deref(),
        first_run_code.as_deref(),
        ingress_host.as_deref(),
        &prepare_authority,
    )
    .await
    {
        Ok(()) => 0,
        Err(refusal) => {
            eprintln!("\nmeridian up: {refusal}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{connect_address, help_for, parse};

    #[test]
    fn a_commands_help_is_its_own_flags() {
        // `meridian logs --help` printed the whole usage in 0.1.46.
        let logs = help_for("logs", &[]);
        assert!(logs.starts_with("meridian logs\n"), "{logs}");
        assert!(logs.contains("--previous            logs:"), "{logs}");
        assert!(logs.contains("--since <span>"), "{logs}");
        assert!(logs.contains("Both:"), "{logs}");
        assert!(!logs.contains("--first-run-code"), "{logs}");
        assert!(!logs.contains("meridian plugin logs"), "{logs}");
        let release = help_for("plugin", &["release".to_string()]);
        assert!(
            release.contains("meridian plugin release <version>"),
            "{release}"
        );
        assert!(release.contains("--no-wait"), "{release}");
        assert!(!release.contains("--first-run-code"), "{release}");
        let rollback = help_for("rollback-deployment", &[]);
        assert!(rollback.contains("--to-revision <n>"), "{rollback}");
        let key = help_for("plugin", &["key".to_string()]);
        assert!(
            key.contains("meridian plugin key setup") && key.contains("--hardware"),
            "{key}"
        );
        let gc = help_for("plugin", &["gc".to_string()]);
        assert!(gc.contains("meridian plugin gc"), "{gc}");
        // A command it does not know: the whole usage.
        assert_eq!(help_for("nonsense", &[]), super::USAGE);
    }

    #[test]
    fn the_v0_1_47_commands_parse() {
        let release = parse(said("plugin release 1.4.0 --no-wait --registry community")).unwrap();
        assert_eq!(release.words, ["release", "1.4.0"]);
        assert!(release.set("--no-wait"));
        let setup = parse(said("plugin key setup --hardware --organisation acme")).unwrap();
        assert_eq!(setup.words, ["key", "setup"]);
        assert!(setup.set("--hardware"));
        let rollback = parse(said("rollback-deployment --to-revision 12 --yes")).unwrap();
        assert_eq!(rollback.value("--to-revision", "--to-revision"), Some("12"));
        assert!(rollback.set("--yes"));
        let previous = parse(said("logs bor --previous")).unwrap();
        assert!(previous.set("--previous"));
        let details = parse(said(
            "plugin publish --registry community --tag v1.0.0 --details",
        ))
        .unwrap();
        assert!(details.set("--details"));
        let gc = parse(said("plugin gc --yes")).unwrap();
        assert_eq!(gc.words, ["gc"]);
        // `plugin gc --release <name>` names a Helm release, as `up` does;
        // only `plugin dev --release` takes nothing.
        let named = parse(said("plugin gc --release meridian --yes")).unwrap();
        assert_eq!(named.words, ["gc"]);
        assert_eq!(named.value("--release", "--release"), Some("meridian"));
        let dev = parse(said("plugin dev --instance x --release")).unwrap();
        assert!(dev.set("--release"));
    }

    #[test]
    fn the_release_commands_github_client_says_who_it_is() {
        // GitHub refuses a REST request with no User-Agent (403); the client
        // must carry one. Built here the way `Rest::client` builds it.
        let request = super::plugin_release::Rest::client()
            .unwrap()
            .get("https://api.github.com/repos/o/r")
            .build()
            .unwrap();
        let agent = request
            .headers()
            .get(reqwest::header::USER_AGENT)
            .map(|v| v.to_str().unwrap().to_string());
        assert_eq!(agent, Some(format!("meridian/{}", super::release::VERSION)));
    }

    fn said(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn the_registries_and_operations_commands_parse() {
        // Contract v25: publish, the keys, download, a bundle; status and logs.
        let publish = parse(said(
            "plugin publish --registry community --tag v0.18.0 --organisation open-meridian",
        ))
        .unwrap();
        assert_eq!(publish.words, ["publish"]);
        assert_eq!(publish.value("--registry", "--registry"), Some("community"));
        assert_eq!(publish.value("--tag", "--tag"), Some("v0.18.0"));
        assert_eq!(
            publish.value("--organisation", "--organisation"),
            Some("open-meridian")
        );
        let revoke = parse(said(
            "plugin key revoke SGK-01 --organisation acme --reason copied --not-trusted-after 2026-10-16T12:00Z",
        ))
        .unwrap();
        assert_eq!(revoke.words, ["key", "revoke", "SGK-01"]);
        assert_eq!(revoke.value("--reason", "--reason"), Some("copied"));
        let download = parse(said("plugin download acme/venue@0.1.0 --to b.tar")).unwrap();
        assert_eq!(download.words, ["download", "acme/venue@0.1.0"]);
        let carried = parse(said("plugin upload --bundle b.tar")).unwrap();
        assert_eq!(carried.value("--bundle", "--bundle"), Some("b.tar"));
        let logs = parse(said("logs bor --since 10m -n meridian")).unwrap();
        assert_eq!(logs.words, ["bor"]);
        assert_eq!(logs.value("--since", "--since"), Some("10m"));
        let status = parse(said("status --json --release meridian")).unwrap();
        assert!(status.set("--json"));
        assert!(parse(said("status bor")).is_err(), "status takes no words");
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
    fn release_names_a_helm_release_for_up_and_releases_the_code_for_plugin() {
        let up = parse(said("up --release trial --no-doctor")).unwrap();
        assert_eq!(up.value("--release", "--release"), Some("trial"));
        assert!(up.set("--no-doctor"));

        let dev = parse(said("plugin dev --release --dir ./p --instance x --yes")).unwrap();
        assert!(dev.set("--release"));
        assert_eq!(dev.words, ["dev"]);
        assert_eq!(dev.value("--dir", "--dir"), Some("./p"));
        assert_eq!(dev.value("--instance", "--instance"), Some("x"));
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
    fn connect_with_no_address_signs_in_to_the_local_install() {
        assert_eq!(
            connect_address(&[]).as_deref(),
            Some("https://meridian.localhost")
        );
        assert_eq!(
            connect_address(&["https://dash.firm.example".into()]).as_deref(),
            Some("https://dash.firm.example")
        );
        assert_eq!(connect_address(&["a".into(), "b".into()]), None);
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
    fn upgrade_deployment_names_its_release_version_and_wait() {
        let arguments = parse(said(
            "upgrade-deployment --release trial -n firm --chart-version 0.1.182 --timeout 15m --yes",
        ))
        .unwrap();
        assert_eq!(arguments.command, "upgrade-deployment");
        assert_eq!(arguments.value("--release", "--release"), Some("trial"));
        assert_eq!(arguments.value("--namespace", "-n"), Some("firm"));
        assert_eq!(
            arguments.value("--chart-version", "--chart-version"),
            Some("0.1.182")
        );
        assert_eq!(arguments.value("--timeout", "--timeout"), Some("15m"));
        assert!(arguments.set("--yes"));
        assert!(parse(said("upgrade-deployment 0.1.182")).is_err());
    }

    #[test]
    fn plugin_migrate_takes_its_target_directory_image_and_force() {
        let migrate = parse(said(
            "plugin migrate --to 0.7.0 --dir ./p --image plugin-python:local --force --run-tests --json",
        ))
        .unwrap();
        assert_eq!(migrate.words, ["migrate"]);
        assert_eq!(migrate.value("--to", "--to"), Some("0.7.0"));
        assert_eq!(migrate.value("--dir", "--dir"), Some("./p"));
        assert_eq!(
            migrate.value("--image", "--image"),
            Some("plugin-python:local")
        );
        assert!(migrate.set("--force") && migrate.set("--run-tests") && migrate.set("--json"));
        assert!(parse(said("plugin migrate --forced")).is_err());
    }

    #[test]
    fn plugin_check_takes_its_directory_and_whether_to_run_the_tests() {
        let check = parse(said("plugin check --dir ./p --run-tests --json")).unwrap();
        assert_eq!(check.words, ["check"]);
        assert_eq!(check.value("--dir", "--dir"), Some("./p"));
        assert!(check.set("--run-tests") && check.set("--json"));
        assert!(parse(said("plugin check --run-test")).is_err());
    }

    #[test]
    fn plugin_open_takes_the_level_its_session_carries() {
        let open = parse(said(
            "plugin open --instance ref --level manage --print /setup",
        ))
        .unwrap();
        assert_eq!(open.words, ["open"]);
        assert_eq!(open.value("--level", "--level"), Some("manage"));
        assert_eq!(open.value("--print", "--print"), Some("/setup"));
        assert_eq!(
            parse(said("plugin open --instance ref --level=view"))
                .unwrap()
                .value("--level", "--level"),
            Some("view")
        );
        assert!(parse(said("plugin open --instance ref --level")).is_err());
    }

    #[test]
    fn https_plain_http_and_the_authority_are_said_plainly() {
        assert!(parse(said("upgrade-deployment --https --yes"))
            .unwrap()
            .set("--https"));
        assert!(parse(said("up --id DEP-X --plain-http"))
            .unwrap()
            .set("--plain-http"));
        let remove = parse(said("authority remove --yes")).unwrap();
        assert_eq!(remove.words, ["remove"]);
        assert_eq!(parse(said("authority trust")).unwrap().words, ["trust"]);
        assert!(remove.set("--yes"));
        assert!(parse(said("up --http")).is_err());
    }

    #[test]
    fn a_stray_word_is_refused_rather_than_ignored() {
        assert!(parse(said("up dep-7")).is_err());
    }
}

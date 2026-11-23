//! `meridian --help` and `meridian <command> --help`.
//!
//! Two tiers. The first lists the commands under their groups, a line each
//! and no flags. The second is one command's: its usage, a sentence, each
//! flag on a line, an example or two and where its page is. Anything longer
//! belongs on that page.

/// The command line reference, which each command's help links into.
pub const DOCS: &str = "https://open-meridian.dev/api/cli/";

/// The groups the first tier lists the commands under, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Deployment,
    Plugin,
    Dev,
    Data,
    SelfCommands,
}

impl Group {
    const ALL: [Group; 5] = [
        Group::Deployment,
        Group::Plugin,
        Group::Dev,
        Group::Data,
        Group::SelfCommands,
    ];

    fn heading(self) -> &'static str {
        match self {
            Group::Deployment => "Deployment:",
            Group::Plugin => "Plugin:",
            Group::Dev => "Dev:",
            Group::Data => "Data:",
            Group::SelfCommands => "Self:",
        }
    }
}

/// One flag, on one line.
pub struct Flag {
    pub spelling: &'static str,
    pub says: &'static str,
}

const fn flag(spelling: &'static str, says: &'static str) -> Flag {
    Flag { spelling, says }
}

/// The flags many commands take, each said once, the same way everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Common {
    Namespace,
    Release,
    /// With the command's own default.
    Timeout(&'static str),
    Yes,
    Json,
    Deployment,
}

impl Common {
    fn spelling(self) -> &'static str {
        match self {
            Common::Namespace => "-n, --namespace <name>",
            Common::Release => "--release <name>",
            Common::Timeout(_) => "--timeout <d>",
            Common::Yes => "--yes",
            Common::Json => "--json",
            Common::Deployment => "--deployment <addr>",
        }
    }

    fn says(self) -> String {
        match self {
            Common::Namespace => "the deployment's namespace (default: meridian)".into(),
            Common::Release => "the Helm release (default: meridian)".into(),
            Common::Timeout(default) => format!("how long to wait (default: {default})"),
            Common::Yes => {
                "go ahead without asking, for a script that has read what it does".into()
            }
            Common::Json => "JSON on stdout".into(),
            Common::Deployment => "which connected deployment, when there is more than one".into(),
        }
    }
}

/// One command's help.
pub struct Command {
    /// As typed after `meridian`: `up`, `plugin launch`.
    pub name: &'static str,
    pub group: Group,
    pub usage: &'static [&'static str],
    /// Its line in the first tier.
    pub summary: &'static str,
    /// The sentence its own help opens with.
    pub about: &'static str,
    pub flags: &'static [Flag],
    pub common: &'static [Common],
    pub examples: &'static [&'static str],
    /// Its section's anchor on the reference page; none for the page itself.
    pub anchor: &'static str,
}

use Common::{Deployment, Json, Namespace, Release, Timeout, Yes};

const INSTANCE: Flag = flag(
    "--instance <id>",
    "the instance's name, which its page is found by",
);
const DIR: Flag = flag("--dir <dir>", "the plugin's directory (default: .)");
const ORGANISATION: Flag = flag(
    "--organisation <name>",
    "the organisation it is under (default: the git remote's)",
);
const PLATFORM: Flag = flag(
    "--platform <url>",
    "the platform (default: https://open-meridian.com)",
);
const REASON: Flag = flag(
    "--reason <why>",
    "required: kept with the record, at most 2,000 characters",
);

/// Every command, in the order the first tier lists them.
pub const COMMANDS: &[Command] = &[
    // Deployment.
    Command {
        name: "doctor",
        group: Group::Deployment,
        usage: &["meridian doctor [flags]"],
        summary: "Check this machine and cluster can run a deployment",
        about: "Check whether this machine and cluster can run a deployment. Changes nothing.",
        flags: &[
            PLATFORM,
            flag("--image <ref>", "the runtime image (default: the published one)"),
        ],
        common: &[Namespace],
        examples: &["meridian doctor", "meridian doctor -n firm"],
        anchor: "meridian-doctor",
    },
    Command {
        name: "up",
        group: Group::Deployment,
        usage: &["meridian up --id <id> [flags]"],
        summary: "Install a deployment and open its first-run wizard",
        about: "Install the chart, then open the deployment's first-run wizard.",
        flags: &[
            flag("--id <id>", "this deployment's identifier, from the platform"),
            flag(
                "--enrolment-code <c>",
                "deprecated: set MERIDIAN_ENROLMENT_CODE instead",
            ),
            flag("-p, --params <file>", "answer the wizard from a file, not a browser"),
            flag(
                "--first-run-code <c>",
                "deprecated: set MERIDIAN_FIRST_RUN_CODE instead",
            ),
            flag("-f, --values <file>", "chart values, passed to Helm as they are; repeatable"),
            flag("--chart-version <v>", "the chart version (default: the latest)"),
            flag("--archive <path>", "where edge plugins' older records go, on the node"),
            flag("--no-archive", "no archive: older records stay in each plugin's storage"),
            flag(
                "--shared-storage-class <class>",
                "deprecated: use pluginStorage.shared.storageClassName",
            ),
            flag("--host <name>", "the name it is reached at (default: meridian.localhost)"),
            flag("--no-ingress", "reach it through a port-forward this command holds"),
            flag("--development", "for development: plugins may run code as it is written"),
            flag("--plain-http", "plain HTTP and no certificate, for cluster tests only"),
            flag("--port <n>", "the port a port-forward uses (default: 8443)"),
            flag("--no-doctor", "skip the checks"),
            flag("--chart <ref>", "the chart (default: the published one)"),
            PLATFORM,
            flag("--image <ref>", "the runtime image (default: the published one)"),
        ],
        common: &[Namespace, Release, Timeout("10m")],
        examples: &[
            "meridian up --id DEP-01ABC",
            "meridian up --id DEP-01ABC -p first-run.yaml --no-archive",
        ],
        anchor: "meridian-up",
    },
    Command {
        name: "down",
        group: Group::Deployment,
        usage: &["meridian down [flags]"],
        summary: "Uninstall a deployment",
        about: "Uninstall the deployment. Nothing is asked: saying down is the decision.",
        flags: &[flag(
            "--delete-namespace",
            "remove the namespace too, with its database and key",
        )],
        common: &[Namespace, Release],
        examples: &["meridian down", "meridian down --delete-namespace"],
        anchor: "meridian-down",
    },
    Command {
        name: "status",
        group: Group::Deployment,
        usage: &["meridian status [flags]"],
        summary: "Show a deployment's health",
        about: "Show the release, components, database, members, plugins and book health.",
        flags: &[
            flag("--wide", "every component's image, pod and member probe"),
            flag("--profile", "the resilience profile alone, as the deployment computes it"),
        ],
        common: &[Namespace, Release, Json, Deployment],
        examples: &["meridian status", "meridian status --json"],
        anchor: "meridian-status",
    },
    Command {
        name: "logs",
        group: Group::Deployment,
        usage: &["meridian logs <component> [flags]"],
        summary: "Show what a component printed",
        about: "Show what a component, a plugin instance or a Job printed in its newest run.",
        flags: &[
            flag("--since <span>", "only the last 30s, 10m, 2h or 1d"),
            flag("--previous", "what it printed before its last restart"),
        ],
        common: &[Namespace, Release],
        examples: &["meridian logs bor --since 10m", "meridian logs migrate"],
        anchor: "meridian-logs",
    },
    Command {
        name: "upgrade-deployment",
        group: Group::Deployment,
        usage: &["meridian upgrade-deployment [flags]"],
        summary: "Move a deployment to a newer chart, in place",
        about: "Move a running deployment to a newer chart after checking it can. Shown, then asked.",
        flags: &[
            flag("--chart-version <v>", "the version to move to (default: the latest; never older)"),
            flag("-f, --values <file>", "chart values to change, over the deployment's; repeatable"),
            flag("--set <key=value>", "one chart value to change, as Helm reads it; repeatable"),
            flag("--enable-checksums", "turn page checksums on in the database it brought"),
            flag("--https", "move a local deployment to HTTPS"),
            flag("--chart <ref>", "the chart it was installed from (default: the published one)"),
        ],
        common: &[Namespace, Release, Timeout("10m"), Yes],
        examples: &[
            "meridian upgrade-deployment",
            "meridian upgrade-deployment --set resilience.profile=ha",
        ],
        anchor: "meridian-upgrade-deployment",
    },
    Command {
        name: "rollback-deployment",
        group: Group::Deployment,
        usage: &["meridian rollback-deployment [flags]"],
        summary: "Return a deployment to an earlier revision",
        about: "Return the release to an earlier revision. The database is not rolled back.",
        flags: &[flag(
            "--to-revision <n>",
            "the revision to return to (default: the one before the newest)",
        )],
        common: &[Namespace, Release, Timeout("10m"), Yes],
        examples: &[
            "meridian rollback-deployment",
            "meridian rollback-deployment --to-revision 12",
        ],
        anchor: "meridian-rollback-deployment",
    },
    Command {
        name: "connect",
        group: Group::Deployment,
        usage: &["meridian connect [<address>]"],
        summary: "Sign in to a deployment's dashboard",
        about: "Sign in through your browser and let this computer act as you for up to 90 days.",
        flags: &[],
        common: &[],
        examples: &[
            "meridian connect",
            "meridian connect https://meridian.firm.example",
        ],
        anchor: "meridian-connect",
    },
    Command {
        name: "sign-out",
        group: Group::Deployment,
        usage: &["meridian sign-out [<address>]"],
        summary: "Revoke this computer's sign-in to a deployment",
        about: "Revoke the delegation connect made, here and at the deployment.",
        flags: &[],
        common: &[],
        examples: &["meridian sign-out"],
        anchor: "meridian-sign-out",
    },
    // Plugin: running plugins on a deployment.
    Command {
        name: "plugin install",
        group: Group::Plugin,
        usage: &[
            "meridian plugin install <organisation>/<plugin>[@<version>] --note <why>",
            "meridian plugin install --pack <pack> --note <why>",
        ],
        summary: "Install a listed plugin or a starter pack",
        about: "Install a listed version or a starter pack into the deployment. Launches nothing.",
        flags: &[
            flag("--note <why>", "required: why, kept with the install"),
            flag("--registry <r>", "community or verified (default: the one Browse shows)"),
            flag("--pack <pack>", "a starter pack: organisation/pack, or open-meridian's by name"),
            flag("--with <plugin>", "with --pack, also install this optional member; repeatable"),
            flag("--without <plugin>", "with --pack, leave out this member; repeatable"),
        ],
        common: &[Yes, Deployment],
        examples: &[
            "meridian plugin install open-meridian/plugin-kraken --note \"desk feed\"",
            "meridian plugin install --pack crypto --note desk",
        ],
        anchor: "meridian-plugin-install",
    },
    Command {
        name: "plugin pack",
        group: Group::Plugin,
        usage: &[
            "meridian plugin pack list",
            "meridian plugin pack show <pack>",
            "meridian plugin pack update <pack> --note <why>",
            "meridian plugin pack remove <pack> --note <why>",
        ],
        summary: "List, show, update or remove starter packs",
        about: "Work with the deployment's starter packs. Update and remove are shown, then asked.",
        flags: &[flag("--note <why>", "update, remove: required, why, kept with the change")],
        common: &[Yes, Json, Deployment],
        examples: &[
            "meridian plugin pack show crypto",
            "meridian plugin pack update crypto --note quarterly",
        ],
        anchor: "meridian-plugin-pack",
    },
    Command {
        name: "plugin list",
        group: Group::Plugin,
        usage: &["meridian plugin list [flags]"],
        summary: "List the catalogue and what is launched",
        about: "List the versions in the deployment's catalogue and the instances launched.",
        flags: &[],
        common: &[Deployment],
        examples: &["meridian plugin list"],
        anchor: "meridian-plugin-list",
    },
    Command {
        name: "plugin launch",
        group: Group::Plugin,
        usage: &["meridian plugin launch <name> <version> --instance <id> [flags]"],
        summary: "Run a plugin version as an instance",
        about: "Run a version from the catalogue, once you approve the roles it asks for.",
        flags: &[
            INSTANCE,
            flag("--in-flight <n>", "most requests in flight at its sidecar"),
            flag("--requests-per-second <n>", "most requests a second, with a second's burst"),
            flag("--answer-mib <n>", "MiB of answers held in flight for it"),
            flag("--shards <n>", "pods sharing its subjects, up to the version's most"),
            flag("--standby", "a standby pod that takes over within 10 s"),
        ],
        common: &[Yes, Deployment],
        examples: &["meridian plugin launch reference-plugin 0.1.0 --instance ref"],
        anchor: "meridian-plugin-launch",
    },
    Command {
        name: "plugin stop",
        group: Group::Plugin,
        usage: &["meridian plugin stop <instance> [flags]"],
        summary: "Stop a launched instance",
        about: "Stop a launched instance.",
        flags: &[],
        common: &[Deployment],
        examples: &["meridian plugin stop ref"],
        anchor: "meridian-plugin-stop",
    },
    Command {
        name: "plugin upload",
        group: Group::Plugin,
        usage: &[
            "meridian plugin upload [flags]",
            "meridian plugin upload --bundle <file>",
        ],
        summary: "Build a plugin and add it to the catalogue",
        about: "Build the plugin here with docker and add it, with its docs, to the catalogue.",
        flags: &[
            DIR,
            flag("--bundle <file>", "carry in a downloaded plugin bundle instead"),
        ],
        common: &[Deployment],
        examples: &[
            "meridian plugin upload",
            "meridian plugin upload --bundle acme-venue-0.1.0.plugin-bundle.tar",
        ],
        anchor: "meridian-plugin-upload",
    },
    Command {
        name: "plugin gc",
        group: Group::Plugin,
        usage: &["meridian plugin gc [flags]"],
        summary: "Collect the plugin registry's garbage",
        about: "Remove the layers retired versions left. Uploads wait while it runs. Shown, then asked.",
        flags: &[],
        common: &[Namespace, Release, Timeout("5m"), Yes],
        examples: &["meridian plugin gc"],
        anchor: "meridian-plugin-gc",
    },
    Command {
        name: "plugin logs",
        group: Group::Plugin,
        usage: &["meridian plugin logs --instance <id> [flags]"],
        summary: "Show what a live plugin printed",
        about: "Show what a live plugin printed.",
        flags: &[
            INSTANCE,
            flag("--since <revision>", "only what came after that revision"),
        ],
        common: &[Json, Deployment],
        examples: &["meridian plugin logs --instance ref"],
        anchor: "meridian-plugin-logs",
    },
    Command {
        name: "plugin events",
        group: Group::Plugin,
        usage: &["meridian plugin events --instance <id> [flags]"],
        summary: "Show what happened to a live plugin",
        about: "Show a live plugin's events: synced, restarted, ready, crashed or refused.",
        flags: &[
            INSTANCE,
            flag("--since <revision>", "only what came after that revision"),
            flag("--follow", "keep reporting them as they happen"),
        ],
        common: &[Json, Deployment],
        examples: &["meridian plugin events --instance ref --follow"],
        anchor: "meridian-plugin-events",
    },
    Command {
        name: "plugin open",
        group: Group::Plugin,
        usage: &["meridian plugin open --instance <id> [flags]"],
        summary: "Open a plugin's page with a one-time link",
        about: "Print a link to a plugin's page that one browser opens once.",
        flags: &[
            INSTANCE,
            flag("--level <level>", "manage, open or view (default: the first you hold)"),
            flag("--print <path>", "print the page at that path instead of a link"),
        ],
        common: &[Json, Deployment],
        examples: &["meridian plugin open --instance ref --level view"],
        anchor: "meridian-plugin-open",
    },
    Command {
        name: "plugin download",
        group: Group::Plugin,
        usage: &["meridian plugin download <organisation>/<plugin>@<version> [flags]"],
        summary: "Download a verified plugin bundle",
        about: "Write a verified bundle of a listed version, for a deployment that reaches nothing.",
        flags: &[
            flag("--to <file>", "where the bundle goes (default: named for the version)"),
            flag("--registry <r>", "community or verified (default: community)"),
            PLATFORM,
        ],
        common: &[],
        examples: &["meridian plugin download acme/venue@0.1.0"],
        anchor: "meridian-plugin-download",
    },
    // Dev: writing and releasing plugins.
    Command {
        name: "plugin new",
        group: Group::Dev,
        usage: &["meridian plugin new <name> [flags]"],
        summary: "Start a plugin from a template",
        about: "Write a new plugin: the SDK's reference plugin, or a role's template.",
        flags: &[
            flag("--into <dir>", "where to write it (default: ./<name>); never an existing path"),
            flag(
                "--role <role>",
                "connectivity, custody, data, operations, pricing or reporting",
            ),
        ],
        common: &[],
        examples: &[
            "meridian plugin new my-venue",
            "meridian plugin new my-venue --role connectivity",
        ],
        anchor: "meridian-plugin-new",
    },
    Command {
        name: "plugin check",
        group: Group::Dev,
        usage: &["meridian plugin check [flags]"],
        summary: "Check a plugin against the framework's rules",
        about: "Check a plugin's pages, settings, SDK use, docs and tests. Needs no deployment.",
        flags: &[
            DIR,
            flag("--run-tests", "run its tests too, with pytest in its own environment"),
            flag("--verified", "hold it to the verified registry's rules"),
        ],
        common: &[Json],
        examples: &[
            "meridian plugin check",
            "meridian plugin check --run-tests --verified",
        ],
        anchor: "plugin-check",
    },
    Command {
        name: "plugin migrate",
        group: Group::Dev,
        usage: &["meridian plugin migrate [flags]"],
        summary: "Move a plugin to a newer SDK release",
        about: "Move a plugin's pins and code to a newer SDK release, then check it. Needs docker.",
        flags: &[
            flag("--to <version>", "the release to move to (default: the latest; never older)"),
            DIR,
            flag("--force", "migrate a directory git does not hold"),
            flag("--run-tests", "run its tests in the check"),
            flag("--image <ref>", "the SDK image the steps run in (default: the release's)"),
        ],
        common: &[Json],
        examples: &["meridian plugin migrate", "meridian plugin migrate --to 0.7.0"],
        anchor: "plugin-migrate",
    },
    Command {
        name: "plugin dev",
        group: Group::Dev,
        usage: &["meridian plugin dev --instance <id> [flags]"],
        summary: "Run a plugin live on a development deployment",
        about: "Run the plugin here live on a development deployment, sending each save.",
        flags: &[
            INSTANCE,
            DIR,
            flag("--release", "upload it as a version and run that in place of the live one"),
        ],
        common: &[Yes, Json, Deployment],
        examples: &["meridian plugin dev --instance my-venue"],
        anchor: "plugin-dev",
    },
    Command {
        name: "plugin publish",
        group: Group::Dev,
        usage: &["meridian plugin publish --registry <r> --tag <vX.Y.Z> [flags]"],
        summary: "Ask the platform to build and list a release",
        about: "Ask the platform to build a signed release tag and list it, after plugin check passes.",
        flags: &[
            flag("--registry <r>", "community or verified"),
            flag("--tag <vX.Y.Z>", "the signed release tag of the version pyproject.toml holds"),
            ORGANISATION,
            DIR,
            flag("--details", "deprecated: use -v for each check's full text"),
            PLATFORM,
        ],
        common: &[Timeout("30m")],
        examples: &["meridian plugin publish --registry community --tag v1.0.0"],
        anchor: "meridian-plugin-publish",
    },
    Command {
        name: "plugin release",
        group: Group::Dev,
        usage: &["meridian plugin release <version> [flags]"],
        summary: "Sign, push and publish a plugin release",
        about: "Check the repository, then sign v<version> on main, push it, publish it and follow the build.",
        flags: &[
            flag("--registry <r>", "community or verified (default: community)"),
            ORGANISATION,
            DIR,
            flag("--details", "each check's full text, as -v prints it"),
            PLATFORM,
        ],
        common: &[Timeout("30m")],
        examples: &["meridian plugin release 1.4.0"],
        anchor: "meridian-plugin-release",
    },
    Command {
        name: "plugin key setup",
        group: Group::Dev,
        usage: &["meridian plugin key setup [flags]"],
        summary: "Set up the key release tags are signed with",
        about: "Make git sign tags with an SSH key, and register its public half on the organisation.",
        flags: &[
            flag("--hardware", "make a new key on a FIDO security key"),
            ORGANISATION,
            PLATFORM,
        ],
        common: &[],
        examples: &["meridian plugin key setup", "meridian plugin key setup --hardware"],
        anchor: "meridian-plugin-key-setup",
    },
    Command {
        name: "plugin key",
        group: Group::Dev,
        usage: &[
            "meridian plugin key register <file.pub>",
            "meridian plugin key revoke <key-id> --reason <why>",
            "meridian plugin key list",
        ],
        summary: "Register, revoke or list tag-signing keys",
        about: "Register, revoke or list the organisation's tag-signing keys at the platform.",
        flags: &[
            ORGANISATION,
            flag("--reason <text>", "revoke: why, as the revocation records it"),
            flag("--not-trusted-after <time>", "revoke: when it stopped being trusted (default: now)"),
            PLATFORM,
        ],
        common: &[],
        examples: &["meridian plugin key list"],
        anchor: "meridian-plugin-key",
    },
    // Data.
    Command {
        name: "backup",
        group: Group::Data,
        usage: &["meridian backup [flags]"],
        summary: "Back the deployment up now",
        about: "Run the deployment's backup once, sealed to backup.recipient, and say what it wrote.",
        flags: &[flag("--to <dir>", "copy the file here too, its hash checked")],
        common: &[Namespace, Release, Timeout("10m")],
        examples: &["meridian backup", "meridian backup --to ./backups"],
        anchor: "meridian-backup",
    },
    Command {
        name: "backup key",
        group: Group::Data,
        usage: &["meridian backup key [flags]"],
        summary: "Make the identity backups are sealed to",
        about: "Make the identity backups are sealed to, and print its public half. Needs no deployment.",
        flags: &[flag(
            "--to <file>",
            "where to write it (default: ./meridian-backup-identity.txt)",
        )],
        common: &[],
        examples: &["meridian backup key"],
        anchor: "meridian-backup-key",
    },
    Command {
        name: "restore",
        group: Group::Data,
        usage: &[
            "meridian restore <file> --identity <file> [flags]",
            "meridian restore --to-time <time> [flags]",
            "meridian restore --to-archive-end [flags]",
        ],
        summary: "Restore a backup, or the database to a time",
        about: "Restore a backup into an empty namespace, or the database from its WAL archive.",
        flags: &[
            flag("--identity <file>", "the identity the backup was sealed to"),
            flag("--to-time <time>", "restore the running database to this RFC 3339 time"),
            flag("--to-archive-end", "restore it to the archive's last commit"),
            flag("--chart-version <v>", "the chart version (default: the backup's; never older)"),
            flag("--chart <ref>", "the chart (default: the published one)"),
        ],
        common: &[Namespace, Release, Timeout("15m"), Yes],
        examples: &[
            "meridian restore backup.age --identity meridian-backup-identity.txt",
            "meridian restore --to-time 2026-11-08T14:03:07Z",
        ],
        anchor: "meridian-restore",
    },
    Command {
        name: "partition split",
        group: Group::Data,
        usage: &["meridian partition split <partition> --reason <why> [flags]"],
        summary: "Halve a lake range at its midpoint",
        about: "Halve a lake range at its midpoint; its two halves continue in its writer group.",
        flags: &[REASON],
        common: &[Json, Deployment],
        examples: &["meridian partition split eq:bars --reason hot"],
        anchor: "meridian-partition-split",
    },
    Command {
        name: "partition move",
        group: Group::Data,
        usage: &["meridian partition move <partition> --to-group <n> --reason <why> [flags]"],
        summary: "Move a partition to another writer group",
        about: "Move a lake range or a book partition to another writer group, online.",
        flags: &[
            flag("--to-group <n>", "the writer group to move it to, numbered from 0"),
            REASON,
            flag("--no-wait", "answer once it has begun"),
        ],
        common: &[Timeout("2m"), Json, Deployment],
        examples: &["meridian partition move P1 --to-group 1 --reason spread"],
        anchor: "meridian-partition-move",
    },
    Command {
        name: "account move",
        group: Group::Data,
        usage: &["meridian account move <account> --to-partition <partition> --reason <why>"],
        summary: "Continue an account in another book partition",
        about: "Continue an account in another book partition, online.",
        flags: &[
            flag("--to-partition <partition>", "the book partition it continues in (P0, P1, ...)"),
            REASON,
        ],
        common: &[Json, Deployment],
        examples: &["meridian account move ACC-7 --to-partition P2 --reason together"],
        anchor: "meridian-account-move",
    },
    // Self: this binary and this machine.
    Command {
        name: "upgrade",
        group: Group::SelfCommands,
        usage: &["meridian upgrade [flags]"],
        summary: "Replace this binary with the latest release",
        about: "Replace this binary with the latest release. A deployment is upgrade-deployment.",
        flags: &[flag("--to <version>", "a named release instead of the latest")],
        common: &[],
        examples: &["meridian upgrade"],
        anchor: "meridian-upgrade",
    },
    Command {
        name: "uninstall",
        group: Group::SelfCommands,
        usage: &["meridian uninstall [flags]"],
        summary: "Revoke every sign-in and remove this binary",
        about: "Revoke every delegation this holds, then remove this binary and its authority.",
        flags: &[],
        common: &[Yes],
        examples: &["meridian uninstall"],
        anchor: "meridian-uninstall",
    },
    Command {
        name: "authority",
        group: Group::SelfCommands,
        usage: &["meridian authority"],
        summary: "Show this machine's certificate authority",
        about: "Show where this machine's certificate authority is, and how clients trust it.",
        flags: &[],
        common: &[],
        examples: &["meridian authority"],
        anchor: "",
    },
    Command {
        name: "authority trust",
        group: Group::SelfCommands,
        usage: &["meridian authority trust"],
        summary: "Trust that authority on this machine",
        about: "Trust the authority in the login keychain and apps' environment. Asked first.",
        flags: &[],
        common: &[],
        examples: &["meridian authority trust"],
        anchor: "",
    },
    Command {
        name: "authority remove",
        group: Group::SelfCommands,
        usage: &["meridian authority remove [flags]"],
        summary: "Take that authority off this machine",
        about: "Take the authority out of the login keychain and apps' environment, and delete it.",
        flags: &[],
        common: &[Yes],
        examples: &["meridian authority remove"],
        anchor: "",
    },
];

/// The first tier's width and the column its summaries start at.
#[cfg(test)]
pub const ROOT_WIDTH: usize = 80;
const ROOT_COLUMN: usize = 23;

/// A command's help: its lines at most this wide.
#[cfg(test)]
pub const COMMAND_WIDTH: usize = 100;
const FLAG_COLUMN: usize = 30;

/// `meridian` and `meridian --help`.
pub fn root() -> String {
    let mut out = String::from("Usage: meridian <command> [flags]\n");
    for group in Group::ALL {
        out.push('\n');
        out.push_str(group.heading());
        out.push('\n');
        for command in COMMANDS.iter().filter(|c| c.group == group) {
            out.push_str(&format!(
                "  {:width$}{}\n",
                command.name,
                command.summary,
                width = ROOT_COLUMN - 2
            ));
        }
        if group == Group::SelfCommands {
            out.push_str(&format!(
                "  {:width$}{}\n",
                "--version",
                "Print which release this is",
                width = ROOT_COLUMN - 2
            ));
        }
    }
    out.push_str("\nGlobal flags: -q, --quiet  -v, --verbose  -h, --help\n");
    out.push_str("Each command's flags: meridian <command> --help\n");
    out.push_str(&format!("Docs: {DOCS}\n"));
    out
}

fn flag_line(spelling: &str, says: &str) -> String {
    // A long flag is indented past where a short one's `-x, ` would be.
    let spelling = match spelling.starts_with("--") {
        true => format!("    {spelling}"),
        false => spelling.to_string(),
    };
    match spelling.len() + 2 < FLAG_COLUMN {
        true => format!("  {spelling:width$}{says}\n", width = FLAG_COLUMN - 2),
        false => format!("  {spelling}  {says}\n"),
    }
}

/// Flags kept to a command's Advanced block: they still work, and most
/// people never need them.
pub const ADVANCED: [&str; 6] = [
    "--chart <ref>",
    "--image <ref>",
    "--platform <url>",
    "--port <n>",
    "--no-doctor",
    "--release <name>",
];

/// Flags no help lists: they work, for the tests they exist for.
pub const HIDDEN: [&str; 1] = ["--plain-http"];

fn advanced(spelling: &str) -> bool {
    ADVANCED.contains(&spelling)
}

/// One command's help.
pub fn command(c: &Command) -> String {
    let mut out = format!("{}\n\nUsage:\n", c.about);
    for usage in c.usage {
        out.push_str(&format!("  {usage}\n"));
    }
    let shown: Vec<&Flag> = c
        .flags
        .iter()
        .filter(|f| !HIDDEN.contains(&f.spelling))
        .collect();
    let own: Vec<&&Flag> = shown.iter().filter(|f| !advanced(f.spelling)).collect();
    if !own.is_empty() {
        out.push_str("\nFlags:\n");
        for f in own {
            out.push_str(&flag_line(f.spelling, f.says));
        }
    }
    let common: Vec<&Common> = c
        .common
        .iter()
        .filter(|common| !advanced(common.spelling()))
        .collect();
    if !common.is_empty() {
        out.push_str("\nCommon flags:\n");
        for common in common {
            out.push_str(&flag_line(common.spelling(), &common.says()));
        }
    }
    let mut advanced_block = String::new();
    for f in shown.iter().filter(|f| advanced(f.spelling)) {
        advanced_block.push_str(&flag_line(f.spelling, f.says));
    }
    for common in c.common.iter().filter(|common| advanced(common.spelling())) {
        advanced_block.push_str(&flag_line(common.spelling(), &common.says()));
    }
    if !advanced_block.is_empty() {
        out.push_str(&format!("\nAdvanced:\n{advanced_block}"));
    }
    out.push_str("\nExamples:\n");
    for example in c.examples {
        out.push_str(&format!("  {example}\n"));
    }
    out.push_str(&format!("\nDocs: {}\n", docs_of(c)));
    out
}

fn docs_of(c: &Command) -> String {
    match c.anchor {
        "" => DOCS.to_string(),
        anchor => format!("{DOCS}#{anchor}"),
    }
}

/// The commands under a word that takes subcommands: `meridian plugin
/// --help`, a line each.
fn listing(parent: &str, under: &[&Command]) -> String {
    let mut out = format!("Usage: meridian {parent} <command> [flags]\n\n");
    let width = under
        .iter()
        .map(|c| c.name.len() - parent.len() - 1)
        .max()
        .unwrap_or(0);
    for c in under {
        out.push_str(&format!(
            "  {:width$}  {}\n",
            &c.name[parent.len() + 1..],
            c.summary
        ));
    }
    out.push_str(&format!(
        "\nEach command's flags: meridian {parent} <command> --help\nDocs: {DOCS}\n"
    ));
    out
}

/// The command a line names, the longest name that matches: `plugin key
/// setup` before `plugin key`.
pub fn find(command: &str, words: &[String]) -> Option<&'static Command> {
    let mut said = vec![command];
    said.extend(words.iter().take(2).map(String::as_str));
    (1..=said.len()).rev().find_map(|n| {
        let name = said[..n].join(" ");
        COMMANDS.iter().find(|c| c.name == name)
    })
}

/// Whether a command is one this knows, for a usage error's pointer.
pub fn known(command: &str) -> bool {
    COMMANDS
        .iter()
        .any(|c| c.name == command || c.name.starts_with(&format!("{command} ")))
}

/// `meridian <command> --help`: that command's help; a listing for a word
/// that takes subcommands; the first tier for anything else.
pub fn help_for(command: &str, words: &[String]) -> String {
    if command.is_empty() || matches!(command, "help" | "-h" | "--help") {
        return root();
    }
    if let Some(found) = find(command, words) {
        return self::command(found);
    }
    let under: Vec<&Command> = COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(&format!("{command} ")))
        .collect();
    match under.is_empty() {
        true => root(),
        false => listing(command, &under),
    }
}

#[cfg(test)]
#[path = "help/tests.rs"]
mod tests;

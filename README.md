# meridian

The command line for [Open Meridian](https://open-meridian.com), the
open-source OEMS: it brings a deployment up where you have a terminal, and
works with one that is already running. The whole install, step by step, is
meridian-core's [INSTALL.md](https://github.com/open-meridian/meridian-core/blob/main/INSTALL.md),
and every command and flag is in the
[command line reference](https://open-meridian.dev/api/cli/). This is release
0.1.25.

```
meridian doctor              # can this machine and this cluster run a deployment?
meridian up                  # install the chart, then open the wizard
meridian up --params f.yaml  # the same, answered from a file
meridian down                # uninstall it, keeping its namespace unless asked
meridian upgrade-deployment  # move it to a newer chart, in place, after checking it can
meridian connect [<address>] # sign in to a deployment (default: the local one), and keep the session
meridian sign-out            # end that session, here and at the deployment
meridian plugin new <name>   # start a plugin from the SDK's reference plugin; --role connectivity, custody, data, operations, pricing or reporting
meridian plugin check        # hold it to the framework's rules: pages, settings, SDK, tests
meridian plugin migrate      # move it to a newer SDK: its pins, each release's rewrite, then check
meridian plugin upload       # build it and put it in the deployment's catalogue
meridian plugin list         # versions uploaded, and what is launched
meridian plugin launch …     # run a version, once you approve what it asks for
meridian plugin stop <id>    # stop a launched instance
meridian plugin dev …        # run it live while you write it, on a development deployment
meridian plugin logs …       # what a plugin printed
meridian plugin events …     # what its sidecar refused it, and what else happened
meridian plugin open …       # its page: a link one browser opens, or --print
meridian upgrade             # replace this binary with the latest release
meridian uninstall           # end every session it holds, and remove it
```

Every command exits non-zero on failure, and `plugin check`, `migrate`, `dev`,
`logs`, `events` and `open` take `--json` for scripts and AI agents.

## Install

On macOS (Apple silicon or Intel) or Linux (x86_64 or arm64):

```sh
curl -fsSL https://open-meridian.com/install | sh
```

That address redirects to this repository's `install.sh`; the long form works
the same: `curl -fsSL https://raw.githubusercontent.com/open-meridian/meridian-cli/main/install.sh | sh`.

It downloads this machine's binary from the latest release, checks it against
the `.sha256` published beside it, and puts it in `~/.local/bin` -- no `sudo`.
`MERIDIAN_INSTALL_DIR` puts it elsewhere and `MERIDIAN_VERSION` picks a
release. On macOS, when you are at a terminal, it then runs `meridian authority
trust`: it makes this machine's own certificate authority (below), says what it
is for, and asks once to trust it. With no terminal to ask at, it skips that,
and `meridian up` asks instead. Then:

```
meridian --version           # which release this is
meridian upgrade             # replace it with the latest; --to <v> for another
meridian uninstall           # end every session it holds, and remove it
```

Nothing is looked up unless you ask: `meridian` never checks for a newer
release on its own. The checksum catches a broken download, not a compromised
release; signing is not built yet.

`meridian` is the way to install a deployment: `doctor` checks, `up` installs
and opens the wizard, and the deployment's own wizard does the rest.

## doctor

Answers whether this machine and this cluster can run a deployment, each answer
naming the fix rather than the symptom: a reachable cluster and the rights to
install into a namespace, Helm 3.14 or newer (`upgrade-deployment` needs its
`--reset-then-reuse-values`), a storage class for
the deployment's key, no node short of disk, the image pullable from here, the
platform reachable, and the clock within the tolerance a signed assertion
allows.

A node under disk pressure stops it: Kubernetes evicts that node's pods and
schedules none there. Free disk under 20%, or 10 GiB where that is more, is
worth knowing; on a local VM, Docker's build cache is often most of it
(`docker builder prune -a`). A launched plugin still on a sidecar the
deployment no longer runs is named too, with the stop and launch that move
it.

The clock is the one nobody can check in their head. Assertions are time-bound,
and a skewed clock fails authentication in a way that looks exactly like a bad
key.

It changes nothing, and exits non-zero when something it checked would stop an
install.

## up

```
export MERIDIAN_ENROLMENT_CODE=…       # the one-time code, from the platform
meridian up --id DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P
```

`--id` is the deployment's identifier exactly as the platform shows it, `DEP-`
included; one of any other shape is refused before anything is installed.

Runs `doctor` first — skip it with `--no-doctor` — then installs the chart with
the two values the platform gave you, waits for the dashboard, and prints the
wizard's address.

Where the cluster has an ingress controller — Rancher Desktop's Traefik, say —
it is reached through the chart's Ingress as `https://meridian.localhost`.
Every browser sends that name, and every plugin's page on a name below it, to
this machine, so the address stays after `up` ends and plugin pages work.

It is HTTPS because an agent signs in to the deployment's MCP over nothing
else. The certificate comes from this machine's own certificate authority,
which `up` makes the first time, in `~/.config/meridian/authority` with its
key readable by you alone, named for Open Meridian and this machine, valid for
ten years and able to sign names under `.localhost` and nothing else. Before
asking anything, it says what the authority is for; on macOS it then offers to
add it to your login keychain, and macOS asks for your password itself (this
never handles one), giving you two minutes to answer it. Over SSH, or anywhere
else macOS cannot show you that dialog, it asks nothing and prints the
command to run in Terminal on the Mac's own screen, and asks again next time.
Elsewhere it prints the one command that trusts it. The
deployment's certificate covers its name and `*.plugins.` below it for a year,
is kept in the Secret `<release>-tls`, and `up` and `upgrade-deployment` issue
a new one within 30 days of its end.

A browser and the Claude Code CLI read the login keychain. The Claude app, like
any app on Node, does not: it reads `NODE_EXTRA_CA_CERTS`. So on macOS the same
yes also names the root to apps started from then on (`launchctl setenv`), and
again at each login (a LaunchAgent, `~/Library/LaunchAgents/com.open-meridian.authority.plist`);
restart the Claude app afterwards. A `NODE_EXTRA_CA_CERTS` you set already is
never replaced: it offers a file holding your certificates and the root, kept
beside the root, and asks before switching to it. Elsewhere each step is
printed as a command, and nothing is changed.

`meridian authority` says where the root is and whether apps are pointed at
it; `meridian authority trust` asks again; `meridian authority remove`, and
`meridian uninstall`, undo all of it: the keychain entry, the LaunchAgent,
`NODE_EXTRA_CA_CERTS` put back as it was, and the root and its key.

`--plain-http` serves plain HTTP and makes no certificate: it is for the
cluster tests, and nothing else. `--host` names another name: under
`.localhost` it is served as above, and any other name — your firm's — is
reached over HTTPS, with the Secrets holding its certificates named in a
values file:

```yaml
# ingress.yaml
ingress:
  tls:
    secretName: meridian-tls                 # for the name
    pluginsSecretName: meridian-plugins-tls  # for *.plugins.<the name>
```

```
meridian up --id DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P --host meridian.firm.example -f ingress.yaml
```

Answer the wizard's dashboard address with it. Where there is no controller, or with `--no-ingress`, `up`
forwards a local port instead (`--port`, default 8443) and holds the forward
until you stop it.

`--development` installs it for development: it may run plugin code as it is
being written, which nobody has reviewed, and every page says so. It is an
install choice, made here or in a sandbox's own values, and never switched on
from the dashboard. It is what `plugin dev` needs; a firm's own deployment
never has it.

### Where older records go

A plugin at the edge -- a custodian's, a broker's -- keeps what its vendor sent
it, and past the window its admin sets it moves the older records to an
archive, keeps them, or deletes them. On a local or on-premises deployment the
archive is a directory on the cluster's node: a NAS export or a second disk
mounted there. Before installing, `up` asks for one, or none, the default:

```
meridian up --id DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P --archive /mnt/nas/meridian-archive
meridian up --id DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P --no-archive
```

`--archive <path>` answers it, its absolute path on the node, and `--no-archive`
declines it; given neither, `up` asks at a terminal and takes none where there
is no terminal to ask at. The answer is the chart's `pluginArchive.path`,
which allows no plugin an archive by itself: a deployment admin allows each
plugin its part of it, with a bound or none, on the plugin's Manage page. With
none, records past their window stay in each plugin's storage. In a cloud, the
archive is a bucket, named as `pluginArchive` in a values file (`-f`), and
nothing is asked. Said in two places -- a flag and a values file, say -- it is
refused rather than one quietly dropped. `upgrade-deployment` keeps it.

It drives your own `helm` and `kubectl` and prints the command it used, so
you can see exactly what it did. It embeds no Helm library: the chart is what says what
runs, and a second renderer is a second source of truth.

### Answering from a file

`--params` posts the same answers to the wizard's own endpoints, behind the same
first-run code a browser redeems. One implementation, so the scripted route and
the browser's cannot drift.

```yaml
# first-run.yaml — the wizard's answers, and no credential
db_host: postgres.internal
db_port: 5432
db_name: meridian
db_serving_role: meridian_app
db_migrating_role: meridian_migrate
backend: ldap
ldap_servers: ldaps://ldap.firm.internal:636
ldap_base_dn: ou=people,dc=firm,dc=internal
ldap_bind_dn: cn=meridian,ou=services,dc=firm,dc=internal
admin_group: meridian-admins
dashboard_url: https://meridian.firm.example
```

```
export MERIDIAN_FIRST_RUN_CODE=…
export MERIDIAN_DB_SERVING_PASSWORD=…
export MERIDIAN_DB_MIGRATING_PASSWORD=…
export MERIDIAN_LDAP_BIND_PASSWORD=…
meridian up --id DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P --params first-run.yaml
```

**The file holds no credential.** Every field the wizard asks for as a password
is read from `MERIDIAN_<FIELD>` instead, and a file naming one is refused with
the variable to use — because a file that works is a file that gets committed.

The field names are the wizard's own: what it asks for is read from the page it
serves, so a typo is refused against the real form rather than posted as an
empty answer. One more is `up`'s own, and never posted to the wizard:
`archive`, the archive's directory or `none`, answering the question above so a
scripted install is never stopped by it.

## down

```
meridian down
meridian down --delete-namespace
```

Uninstalls the release. Nothing is asked: saying `down` is the decision. The
namespace is kept, and with it the database the deployment brought and the
deployment's own key, so `meridian up` again picks both back up.

`--delete-namespace` removes the namespace too: the database and all its data
go, which nothing backs up, and so does the deployment's key, after which the
platform refuses it a new enrolment code until the key is revoked there.
Either way the deployment still exists on the platform; retiring it there is
what revokes its key. A session this machine held with it is forgotten. It
never touches the cluster itself.

Not to be confused with `meridian uninstall`, which removes this CLI.

## upgrade-deployment

```
meridian upgrade-deployment
meridian upgrade-deployment --chart-version 0.1.182 --yes
meridian upgrade-deployment -f backup.yaml
```

Moves a running deployment to a newer version of its chart, in place, with
your own cluster rights: nothing in a deployment holds a right to change the
cluster, and an upgrade is the widest change there is. Not `meridian upgrade`,
which replaces this binary.

It checks first and changes nothing if a check fails: the cluster reachable
and your rights in the namespace, Helm 3.14 or newer, no node under disk
pressure (low free disk is only reported), the release `deployed` (a `failed`
or `pending` one is refused with how to recover it), the version published and
not older than the installed one. At that version already, it
says so and exits 0. Whether the upgrade is within the skip policy, and whether
every plugin's runtime floor is met, are printed as `unknown`: neither is
built yet, because nothing is there to check against.

Then it shows the release, the version and image it is on and the ones it
moves to, and the `helm upgrade` it will run, and asks; `--yes` answers for a
script. It applies the new chart's defaults with the deployment's own values
over them:

```
helm upgrade meridian oci://ghcr.io/open-meridian/charts/meridian-runtime --version 0.1.182 \
  --namespace meridian --reset-then-reuse-values --timeout 10m
```

Never `--reuse-values`, which keeps the old chart's image tag, so every pod
restarts on the old image; and not `--wait`, which charts up to 0.1.182 fail on
their own hook. It waits itself, up to `--timeout`, for the new revision's
migration Job, every Deployment and StatefulSet to roll out, and every pod on
the image its template names. Then it deletes the finished Jobs of earlier
revisions, found by the release's label, and reports each component's image
and readiness and every container that restarted during the upgrade, with the
reason Kubernetes gives. Last, it names each launched plugin left on a sidecar
the deployment no longer runs, or launched while the rollout was under way,
with the `plugin stop` and `plugin launch` (or `plugin dev`) that move it. It
relaunches nothing itself.

`-f <file>` and `--values <file>`, Helm's own names and `up`'s, repeatable,
change the deployment's values in the same approved upgrade: each file is
given to Helm after `--reset-then-reuse-values`, so it is merged over the
deployment's own values as Helm merges them (mappings key by key, a key set to
null removed), each file over the last. The files are read before anything
changes, and one that cannot be read, or that is not a mapping, stops it; so
does a deployment whose own values cannot be read, since what the files change
could not be shown. The plan lists each path they set, change or remove,
never a value (`backup.recipient: set`), and the `helm upgrade` with its `-f`
files, before "Upgrade it?". At the version it is on already, files that
change something are an upgrade at that version; files that change nothing
leave nothing to do.

`--https` moves a local deployment, reached at a name under `.localhost`, to
HTTPS in the same approved upgrade: the certificate first, from this machine's
authority (made and trusted the first time, as `up` does), then the upgrade
with `--set ingress.tls.secretName=<release>-tls --set
dashboard.url=https://<name>`, at the version it is on if there is no newer
one. People and clients signed in at the `http://` address sign in again. A
certificate this machine wrote is renewed within 30 days of its end with or
without `--https`.

A pod left over from a restart is not waited for: one stopped for good,
`Succeeded` or `Failed`, and made by a ReplicaSet its Deployment has since
replaced. A node restart leaves one for each pod it ran, on whatever image it
had, and it never changes, so waiting for it would last until the timeout.
Pods of a launched plugin carry the release's labels and are treated the same.
The plan names them, the one confirmation covers removing them, and the cleanup
deletes them by name with the Jobs. A pod still pending, running or
terminating on the old image is waited for.

While it works it shows where it is. On a terminal, a block redrawn in place:
each step (checks, plan and confirmation, apply, migration, components,
cleanup) pending, in progress, done or failed, with its time; the total time;
the components ready, as a count and a bar; and what it waits on now, and why.
Piped or in CI, a line as each step starts and ends, and every 20 seconds one
saying what it still waits for, so a log never looks stalled. `NO_COLOR` turns
the colour off. The report at the end is the same either way.

It never prints the deployment's values, which hold its enrolment code; what it
reads from them is `image`, where the deployment is reached, and where its
archive is. Everything `up` was given, `--archive` among it, is the
deployment's own values, which `--reset-then-reuse-values` applies again, so
the archive is kept and the plan says so: `Its archive, /mnt/nas/meridian-archive
on the cluster's node, is kept`. The same steps from a firm's own pipeline
-- plain Helm, Flux or Argo CD -- are in the docs' *Upgrade a deployment*.

Where it stops, it says what to run next in `meridian` commands alone: `meridian
status` for the release's state and Helm's own words, `meridian logs migrate` for
a migration's log, `meridian logs <component> --previous` for why a container
restarted, and `meridian rollback-deployment` to return to the revision it was at.

## rollback-deployment

```
meridian rollback-deployment                    # to the last revision deployed before the newest
meridian rollback-deployment --to-revision 12
```

Returns a release Helm holds as `failed` or `pending-...`, or an upgrade that is
not wanted, to an earlier revision, with your own cluster rights. It checks first
and changes nothing if a check fails: helm and the cluster are here, and the
release's revisions are read (`--to-revision` must be one of them, and not the
newest). Then it shows both revisions, with Helm's words for the newest where it
is not deployed, and that **the database is not rolled back with it**: what a
newer revision's migration changed stays, and the older runtime may refuse a
schema it does not know, so a backup taken before the upgrade is what brings the
data back. It asks "Roll it back?" (`--yes` for a script), runs `helm rollback`
and waits, up to `--timeout`, for that revision's migration if it runs one and
every component on its template.

## backup and restore

```
meridian backup key                       # the identity backups are sealed to; prints backup.recipient
meridian backup --to ~/meridian-backups   # back up now, and copy the file here
meridian restore meridian-20261011T020000Z.meridian-backup --identity meridian-backup-identity.txt
```

`backup` runs the deployment's backup CronJob once, now: its keys and
credentials, its values and the database the chart brings, sealed to
`backup.recipient`. A deployment with no backup configured is told the
`meridian upgrade-deployment -f <file>` that sets one.

`restore` is a person's act with their own cluster rights, into a namespace
holding no deployment. It checks first and changes nothing if a check fails:
the backup opens with the identity and every member's hash holds; the
namespace holds no release of that name and nothing a deployment keeps or
would take up -- the backup's Secrets and ConfigMaps (the dashboard's public
keys, `-backup-values`), the key's Secret (`<release>-meridian-runtime-key`),
and the claims the brought database and the registry would mount and find
full. A backup volume a deleted namespace left is not a refusal: the plan says
restore deletes it, within the one approval, and its Retain policy keeps the
node's directory and the backups in it. The
chart is read at the version the backup was taken on, or `--chart-version`,
from whatever `--chart` names: one older than the backup's is refused, since
its database may hold what only a newer runtime serves, and a newer one is
said, restoring then being an upgrade. Where a dump of a database somebody
else runs is to be loaded, restore needs the right to add a container to a
pod and exec into it, and checks for it.

Then it shows the backup, the cluster and namespace, the chart and version,
what it will make and what it will do with the database, and asks "Restore
it here?"; `--yes` answers for a script, and without a terminal and without
`--yes` nothing is changed. A database somebody else runs is named, its host
and database and never its credentials, with what it means: one not in the
backup is reconnected to, so anything else running against it must be
stopped first; one whose dump the backup holds (`backup.database: true`) is
loaded by restore itself into that database, which must be empty, as the
migrating role, from a container of `backup.image` added beside the
conductor (the cluster's own path to the database, no object of its own,
gone when the conductor restarts), the URL reaching it on its standard input.

The database's URL is withheld until the database is loaded and every
table's row count matches the backup's; then the components are given it,
the conductor and the dashboard restarted, and first run gives up its rights.
The report gives the counts by store (config, book, street, lake, instrument,
dashboard), each with its tables and rows, and reads each launched plugin's
pod, for at most two minutes, for an image it could not pull: only those are
named, with `meridian plugin upload` to run, since the registry's claim can
outlive a namespace.

## connect and plugins

```
meridian connect
```

Signs in to the deployment in your browser, however it signs people in, and
asks you to let this computer act as you: a delegation, for up to 90 days,
listed on the dashboard under Connected clients, where you or a deployment
admin can revoke it. This computer keeps a ten-minute access token and a
single-use refresh token, and refreshes by itself, so nothing asks for a
browser again until the delegation lapses; you are told a week before. It
never takes a password. `meridian sign-out` revokes it, here and at the
deployment.

A session kept by an earlier meridian still works until it lapses, at most
12 hours; then `meridian connect` again.

On that delegation, as the deployment's administrator:

```
meridian plugin new my-plugin            # ./my-plugin, from the SDK's reference plugin
meridian plugin new my-prices --role data # a data plugin, from the SDK's data template
meridian plugin upload --dir my-plugin   # built with docker, from its own Dockerfile
meridian plugin list                     # versions uploaded, and what is launched
meridian plugin launch my-plugin 0.1.0 --instance my-plugin
meridian plugin stop my-plugin
```

`plugin new` writes a working plugin: its pages, each a view function and a
Jinja2 template declared with the levels it serves (a Setup page under
Manage, an Accounts page under Open and View), a `Dockerfile`,
`pyproject.toml`, and `AGENTS.md`, which teaches any coding agent the live
loop below. `CLAUDE.md` and the `develop-live` skill lead Claude Code to the
same text. Commit them with the plugin; `.dockerignore` keeps them out of its
image. From SDK 0.28.0 (CLI 0.1.45) every template also writes the plugin's
pre-push hook, `hooks/pre-push`, executable: CI-first, it runs nothing for a
branch push, whose pull request's CI is the gate, and `meridian plugin check
--run-tests` before a push straight to main. `git config core.hooksPath hooks`
turns it on in the plugin's repository, and `.dockerignore` keeps it out of
the image too.

`--role` with `connectivity`, `custody`, `data`, `operations`, `pricing` or
`reporting` writes that
role's template instead, from meridian-python's `templates/`: a whole plugin
holding the role, built on the role's common part (`meridian.roles.<role>`,
spec/plugin-pages-follow-their-role), so a person who knows the role knows the
plugin: the role's common settings in their order, its common pages and
sections in the role's order with their "not set up yet" states, its
figures, and the vendor's nuance only at the templates' blocks. The `data`
plugin puts a stand-in vendor's daily closes and bars into the lake, its
catalogue declared from code, every price parsed from the vendor's text as a
`Decimal`, with the role's Connection and Datasets pages under Manage and
their read tools, `read_connection` and `read_datasets`, each dataset said
licensed or not (contract v20); asked for an instrument another source
resolved, a custodian's say, it reads that instrument's record and serves it
by a symbol in it; its tests run the data suite, its cases about trades and
quotes, which its daily dataset never publishes, marked not presented, with
why. Replace the vendor with your own. The `custody` plugin brings a stand-in
custodian's accounts and statements in, on the role's Connections, Account
links, Statements and History pages, its tests running the custody suite. The `operations` plugin keeps opening balances,
reconciliation and breaks, on the role's Breaks, Reconciliations and Opening
balances pages, its confirmation checked by a read tool so an agent may
prepare it for a person. The `reporting` plugin values the book's positions
at the last close from the lake, and records nothing; it resolves its
reporting currency by its ISO 4217 code, its tests run the reporting suite,
and its Closes page is a read tool too, `read_report`. The `pricing` plugin
(SDK 0.31.0, contract v26) answers the analytics component's batches through
the SDK's `Pricer` over its own models, its tests running the pricing suite;
it asks for no storage, since `pricing` is no edge role, and keeps the last
batch it answered in memory. The `connectivity` plugin's tests run the
connectivity suite from SDK 0.31.0. A role with no
template of its own is refused: the reference plugin takes any role in its
`pyproject.toml`. Contract v20 renamed three roles, `dgm` to `data`, `ccm` to
`connectivity` and `match` to `confirmation`; the old names are no roles.

`upload` sends the version's declaration with it (contract v11): where
`pyproject.toml`'s `[tool.meridian]` names one, `declaration =
"<module>:<attribute>"`, it runs the SDK's `meridian-declaration` in the
image it built, with no network, and sends what it prints: the secret
settings' names, what the plugin receives and does not carry, the storage it
asks for, refused here when a plugin holding no edge role asks for storage,
and from contract v22 the algorithms its autonomous work may name
(`algorithms`, which SDK 0.26.0 writes only for a version declaring one):
every key as printed, whole. `plugin check` holds the same declaration to
that (`edge-storage`).
`plugin check --verified` holds a plugin holding a role with a conformance
suite (`custody`, `data` from contract v18, `connectivity` from contract
v22, and `pricing` from contract v26) to a test that runs the suite (`role-suite`), which `--run-tests` then
runs: a plugin holding the role is
verified for it only by passing every case, though from contract v19 the
SDK's runner lets a case about a kind of data the plugin declares it never
publishes be named not presented, with why, and from contract v20 a case
about a field its source never states (`about_field`). `role-suite` reads
the cases a test names in `not_presented` against meridian-schema's
`suites.json`: one no case of the suite has, or one every plugin of the role
passes -- the data suite's `reads-its-datasets-licences` among them, and
every case of the connectivity suite, sending nothing new on a pause and
cancelling at the venue on a stop with them -- fails it. From 0.1.43 it also
holds the test to the outcome `--run-tests` then trusts: any skip or expected
failure where a suite is run -- a marker (`pytest.mark.xfail`, strict or not,
`skip`, `skipif`, unittest's `skip` and `expectedFailure`), a call
(`pytest.xfail()`, `pytest.skip()`, `importorskip`), or a `pytestmark` -- in
the module running it or a `conftest.py` over that module fails it at its
line, since only the contract's `closed_list`, `about` and `about_field`
excuse a case; and, since the SDK's runner reports rather than raises, a
collected test (`test_*` in a `test_*.py`) must bind the run and assert it
passed in the very next statement, as the SDK's templates do --
`report = run("<role>", producers, ...)` then
`assert report.passed, report.failures` -- under no `try`, `with`, `if` or
loop. A test asserting a count or one case, catching the failure, or changing
the report first fails it at the suite's first run; tests that expect the
suite to fail may stand beside that one. Without `--verified` the rule is not checked, since a plugin not passing
its role's suite is not verified and breaks no rule: the reference plugin
given `custody`, recording a statement no vendor sent, keeps every rule.
`make check-scaffolds` makes each scaffold with the release binary and runs
its tests for real: the data template with a float in its price path fails
its suite, and so does a connectivity plugin that acknowledges a placement at
its venue after hearing a pause on itself, the suite naming
`sends-nothing-new-on-a-pause`.

`--verified` also holds a plugin holding `data`, `custody`, `operations` or
`reporting` to its role's common part (`role-pages`,
spec/plugin-pages-follow-their-role): the role's common pages declared by
their titles, with their paths where the role fixes them, their levels and,
on a plugin of several roles, their role, in the role's order and ahead of
the plugin's own pages in the same tab row; each common page's tool by the
role's name, answering the role's record (`meridian.roles.<role>`'s
`<Page>Read`), one extending it, or one of the plugin's own holding its
common fields; the role's acts by their names; and the settings in the
common order, `environment` first and `sample_data` last, the common
settings' labels kept for them, `synthetic` declared as `sample_data`. What
is held is only what is common: the data role's Connection and Datasets; a
custody plugin's Account links and Statements and its `link_account`; an
operations plugin's Breaks, Reconciliations and Opening balances and its
`confirm_opening_balance`; a reporting plugin's `reporting_currency`; and a
custody plugin keeping raw records (a `RecordKind` declared) titles its page
for them Raw records, under Open and View. The rest the templates offer stays
a candidate until a second plugin of the role needs it. A page or route the
SDK declares for the plugin counts as declared: where the plugin calls one of
the SDK's helpers that declares pages on its `Pages`, as
`custody.account_pages(pages, Source, what=...)` declares custody's Account
links, Statements and History and `link_account` (the SDK's
`custody-pages-onto-sdk` migration), the rule holds what the helper declares,
placed where the plugin calls it. What each helper declares is read from the
SDK's own source at the vendored revision: `make vendor-template` copies every
module under meridian-python's `src/meridian/roles` that decorates a page,
route or tool to `sdk-roles/`, compiled into the binary, and
`check-vendored-template` fails when it drifts. Whatever roles it
holds, every act -- a route that changes something, offered to agents --
names its check tool (`checked_by=`), a read route (`reads=True`) the plugin
declares, so an agent may prepare the act for a person to make (contract
v20). Where a tool replaces the route (`@pages.tool(replaces=...)`), the
tool is what agents are offered, and it names the check instead, unless it
or the route it replaces reads. Without
`--verified` the same findings are printed as advice, and fail nothing.
`secrets-kept` reads a secret declared as a field of a class
`@meridian.settings` makes, as it reads one declared by `Setting(...)`.

From contract v12 a deployment serves one MCP surface, where an agent a
person delegated to works as them, and a plugin's tools there are derived by
its SDK from its typed routes. `plugin check` fails a route that changes
something and declares no typed record of inputs (`params=`), so no tool is
derived from it, unless it says why it is not offered (`tool=False,
why="..."`) or a tool replaces it (`tools-cover-routes`); `--verified` holds
it as a verified plugin is held, keeping no changing route from agents.

From contract v15 a person's level is granted per role of a plugin. On a
plugin whose `[tool.meridian]` names several roles, `plugin check` fails a
page, route or setting that names no `roles=`, or a role the plugin does not
hold, and a route that changes something sending a command none of its roles
holds, by meridian-schema's `roles.json`, naming the roles that do
(`roles-declared`). A tool takes its route's roles unless it names its own. A
plugin holding one role, or none, names none, and the rule holds.

From contract v16 a plugin at the edge declares the kinds of raw record it
keeps (`RecordKind`), and the SDK declares two settings for each,
`<kind>_window_days` and `<kind>_past_window`, the same for every edge plugin.
`plugin check` fails a setting of the plugin's own taking one of those names
for a declared kind, at its line (`window-settings`), as the SDK refuses it
when the declaration is built and the sidecar at registration.

From contract v21 the work a plugin does on its own -- a scheduled sweep, a
strategy, an agent inside it -- calls the plugin's acts as its role's
service account, through `meridian.runs_as(role)`, and an account its
role's admin set to prepare only is refused an act with no check tool at run
time. `plugin check` fails a call through `runs_as` to one of the plugin's
acts that names no check tool (`checked_by=`), at the call's line
(`runs-as-checked`). It reads `async with <runs_as>(...) as act:` however
`runs_as` is reached -- `meridian.runs_as`, imported, or a name the module
binds to it -- and each `act("<tool>", ...)` in its block, the tool as
written or through a constant; a tool named some other way, or not among the
plugin's acts, it passes over.

From contract v24 every instrument is named by the platform master's ID, and
a deployment's provisional `LCL-` record, or an `INS-` record the platform
merges, is replaced by the record it became. A write naming a replaced ID is
accepted forever and recorded under that record
(spec/the-platform-master-mints-every-instrument-from-a-template, requirement
11), so `plugin check` warns of an instrument ID kept in the plugin's code --
a Python string holding an `INS-` or `LCL-` ID and nothing else, outside its
tests -- at its line (`replaced-ids-followed`): resolve the instrument from an
identifier it states, or follow the kept ID to the record it became with
`meridian.CurrentIdMap`; one written as `<map>.current("<id>")` is followed
already. A warning fails nothing, verified or not: it is printed with its
rule, file and line, and listed under `warnings` in `--json`. The check reads
meridian-schema's v28 `roles.json` and `suites.json` (dcca81b, as SDK 0.33.0
vendors them; from v24's, connectivity no longer reads the escape hatch, and
the give-up's case is marked `about: give-ups`): `pricing` is a role, not at
the edge, with its suite; v26's event group, jobs, bounded lake query and
agents' guard cases, v27's rates, liveness and RFQ quote cases, and v28's
cases are in the suites that hold them. The latency reports' cases are
required again from contract v28 (ruling 518), and each of v28's new cases is
`about` what a plugin may never publish or do (ruling 602) -- option chains,
filings, holdings, corporate actions, ratings, quotes, options, packages,
derived series, scenarios and quality states -- so a plugin touching none
names it not presented, with why; `reference`
is a role, its suite in preview and not yet held by `--verified`;
`roles-declared` knows v24's commands and the roles holding them (booking's,
`set_allocation_instruction`, `report_give_up`, the alerts',
`offer_template_fields`); and every suite's `hears-and-reads-a-replacement`
and `reads-the-template-catalogue` are cases every plugin of the role passes.

`launch` shows the roles the version asks for and runs it only once
you approve them; `--yes` approves for a script that has already read them.
Its pages are on its own name, `https://my-plugin.plugins.meridian.localhost/`,
opened from the dashboard's home by a button for each level you hold on it:
Manage (`admin`), Open (`write`) and View (`read`).

### The plugin's documentation (contract v25)

A plugin's `docs/` is what a deployment admin reads on its Documentation tab
before launching it: `overview.md` and `setup.md`, which `plugin check`
requires (`documentation-sections`), `troubleshooting.md` and `terms.md`,
which it prompts for as advice and requires of a verified plugin, and
`pages.md`. `documentation-safe` holds each section to what the tab shows as
written: UTF-8, 64 KiB a section and 256 KiB in all, no HTML, no image, no
`javascript:`, `vbscript:`, `data:` or `file:` link, no link whose text names
one host and whose target another, no character that hides text, and nothing
the screen refuses: override, acting for the reader, a claim of endorsement,
verification or regulatory readiness, posing as Open Meridian,
exfiltration, concealment, and claiming authority (as core and the platform
name them; ruling 176 and its addendum). A heading's text past one screen of the tab, a link shown as plain
text, a file in `docs/` that is not a section, and text matching the
screen's shape rules -- chat markup, answer-shaped, instruction words
(beyond the contract's allocation, trading, settlement, execution and
handling instructions), addressed to the reader (`you need to ...`, which
setup steps often say), a tool's name, a role marker (`system:`), talk of
an agent's prompt -- are advice, failing nothing: the
platform and the deployment show such a section to people marked
"Screened" and withhold it from agents, never refusing it. A plugin on an SDK before 0.29.0 is advised, never
failed. `plugin upload` sends the texts with the declaration, each held to
the digest the declaration names, and says where the version's
documentation is read; `plugin new` writes a prompt for each section, and
its CI checks with the meridian that wrote it (`MERIDIAN_VERSION`).

### Installing a listed plugin or a starter pack

```
meridian plugin install open-meridian/plugin-coinbase@1.2.0 --note "the crypto desk"
meridian plugin install open-meridian/plugin-coinbase --note "the crypto desk"
meridian plugin install --pack crypto --with plugin-coinbase --note "the crypto desk"
meridian plugin install --pack crypto --with plugin-coinbase --without plugin-public-feed \
    --note "the crypto desk" --yes
meridian plugin pack list
meridian plugin pack show crypto
meridian plugin pack update crypto --note "monthly update"
meridian plugin pack remove crypto --note "the desk closed" --yes
```

`install` puts a version the registries list into the connected deployment
as the dashboard's Browse does, as its deployment admin, and launches
nothing. `<organisation>/<plugin>@<version>` installs that version, from
`--registry` where given (community otherwise); without `@version` it
installs the one Browse shows, the newest offered release, verified first.
`--note` is required: the deployment keeps it with the install.

`--pack` installs a starter pack through the deployment, as Browse's install
sheet does: one pack install, which the deployment keeps (one record naming
the pack's revision, the members and how each went, and each member's own
install record naming its pack). A bare name is `open-meridian`'s, so
`crypto` is `open-meridian/crypto`. The deployment resolves each member to
its newest offered release at or above the pack's minimum for it; one with
none offered is Unavailable, and one the deployment won't take (its floor not
met, or a registry it doesn't take) is "Not permitted here": neither is
installed, and a pack whose required member is either is not. Members the
curator preselected are installed, and members to choose are not: `--with
<plugin>` adds one and `--without <plugin>` leaves out a preselected one,
each repeatable, by its listing or its plugin part, and a name that is not a
member is refused before anything is installed. At a terminal, each member
to choose that you did not name is asked about. A member the deployment
holds already is never moved: at or above the pack's version it counts as
installed, and below it the line says `Installed 0.3.0; 0.5.1 offered`. The
plan is shown version by version and asked about first; `--yes` approves it
without asking, and installs no member to choose that you did not name.
Where every member ticked is held already, the plan says `Nothing to
install.` and asks to record the pack as installed, so the deployment keeps
its record of the pack all the same, moving no member. The
install runs in the deployment, member by member; the command follows it, one
line a member as each ends, going on past a failure, then lists `meridian
plugin launch` for each member ready to launch and each that failed, with
why, exiting 1 when one did. A listing without `@version` reads the
platform's catalogue with your platform sign-in; `--pack` reads only the
deployment.

`plugin pack` reads and keeps the packs installed: `list` is Browse's pack
view, one line a pack (its title, asset classes, members and its state
here); `show <pack>` is its page, the setup checklist -- each member's
versions, its state with why, its connection and the step after it, what a
revision added, and the launch command for each member installed and not
launched; `update <pack>` is "Update all", each launched member moved to its
newest offered version, an installed one given it, and one asking for more
roles left for its own approval, what would move shown first; `remove
<pack>` removes only the deployment's record of the pack, every member
staying installed and running, those members shown first. `update` and
`remove` take `--note` and are asked about unless `--yes`. With `--json`
each prints the deployment's answer as its tools give it. The pack commands
and `--pack` need a deployment whose core is at contract v28 or later; an
older one is said in one line.

## The registries (contract v25)

```
meridian plugin key setup                       # git's SSH signing key, or a new one, registered
meridian plugin release 0.19.0                  # checked, the tag signed on main's head, published
meridian plugin key register ~/.ssh/release.pub --organisation acme
meridian plugin publish --registry community --tag v0.18.0 --organisation acme
meridian plugin key list --organisation acme
meridian plugin key revoke SGK-... --organisation acme --reason "copied from a build machine"
meridian plugin download acme/venue-desk@0.18.0       # on a connected machine
meridian plugin upload --bundle acme-venue-desk-0.18.0.plugin-bundle.tar
```

A plugin is named in Meridian -- its listing, what `publish` and `release` ask
the platform to build, and what `upload` puts in a deployment's catalogue --
by `[tool.meridian] name` where its `pyproject.toml` says one, else by
`[project] name` (CLI 0.1.54, ruling 971). One whose Python project is named
apart from it says so: plugin-databento's project is `plugin-databento`, so
it never shadows Databento's own client, and `name = "databento"` lists it
as `databento`. The name is a host label: lower-case letters, digits and
single hyphens.

These act through a sign-in to the platform (`--platform`), made in your
browser the first time and kept 30 days, never a password; its access tokens
live ten minutes and are good at the publishing API alone. `publish` holds
the plugin to `plugin check` first, and refuses a tag that is not `v` and the
version pyproject.toml holds, or that is not signed with an SSH key
(`git -c gpg.format=ssh tag -s v0.18.0`); then it asks the platform to build
the tag, with a token good for that plugin alone, and follows the build until
it is listed, queued for review, or refused: one line a check, cut to the
terminal's width (`--details` prints each check's full text), the build's log,
and the listing's address once it is listed. It builds, signs and pushes
nothing. Its push token is held while the build is followed, and narrowed
again only within a minute of its end; where the platform will not narrow the
sign-in's token, the browser is opened for one narrowed sign-in, once a
publish at most. From 0.1.51 (contract v27) it sends the display name the
plugin's code declares (`meridian.Interface(title=...)`, as written or
through a constant), and the builder refuses a build whose image declares
another: the refusal is said plainly, with both names. A publish while two
of the organisation's builds are running is queued, said so, and followed
until it settles; a 429 is the organisation's daily cap of publishes.

`key setup` makes the key release tags are signed with ready in one step:
git's configured SSH signing key, or one `ssh-keygen` makes (`-t ed25519`, a
desktop key; `--hardware` makes `-t ed25519-sk`, held on a FIDO security
key), ssh-keygen asking for its passphrase itself. It sets `gpg.format ssh`
and `user.signingkey` (in the plugin's repository alone where git signs with
a GPG key), and registers the public half on the organisation; somebody who
is not an Owner or Admin is told to ask one, with the line to give them.
`plugin new` offers it where git has no SSH signing key yet.

`release <version>` releases the plugin here in one command (ruling 323,
from 0.1.49): it checks, signs and publishes, and opens no pull request and
writes no file. The version bump and the CHANGELOG entry go in the change's
own pull request: it raises `pyproject.toml`'s version and adds a
`## <version>` heading to `CHANGELOG.md`, with no date and no
"(unreleased)", since the date lives on the tag and the listing. `release`
checks the checkout is on the default branch, clean and up to date with
origin; the version higher than the last tag; `pyproject.toml` already at
it; `CHANGELOG.md`'s `## <version>` heading there and undated; and that
branch's last CI run green. Each refusal says what to do next. Then it signs
`v<version>` on the branch's head, pushes it, publishes it as `publish` does
and prints the listing's address; with the tag on origin already, it
publishes that tag. GitHub is asked through its API with your own GitHub
sign-in, to read the CI: `GH_TOKEN` or `GITHUB_TOKEN`, else the credential
git keeps for github.com; nothing else is installed for it. `download` writes a plugin bundle -- the image, its
referrers, its documentation and the platform's signed key status -- only
once every digest and signature verifies against the platform's keys this
release carries; `upload --bundle` carries it into a deployment that reaches
nothing, which verifies it again. This release carries the platform's
registry-signing key (`meridian-registry/registry-signing`, version 1).
`--organisation` takes the organisation's namespace and defaults to the one
the plugin's git remote names; where that is not known, the command says to
name it.

```
meridian plugin gc
```

`plugin gc` collects the deployment's own plugin registry's garbage: the
layers retired versions left once their manifests were released. It sets the
in-cluster registry read-only (uploads wait; pulls go on, so running plugins
and launches are untouched), runs Distribution's `registry garbage-collect`
in its pod, and sets it back, after showing that and asking. A run stopped
half-way leaves it read-only, says so, and the next run finishes.

## status and logs

```
meridian status                 # the release, each component and plugin, the book's health
meridian status --json
meridian logs bor --since 10m
meridian logs bor --previous    # what it printed before its last restart
meridian logs migrate           # the newest migration Job's log
meridian logs snaptrade         # a plugin, by its instance
```

`status` says the release first -- its revision, state, chart and app version,
and Helm's own words for why where it is not `deployed`, with the
`rollback-deployment` that recovers it -- then each of the deployment's
components and, under Plugins, each plugin instance by its name: its image (a
plugin's own), how many of its pods are ready, how often they restarted and
why the last time (`OOMKilled`), and each pod's memory against the limit each
pod has, where the cluster serves metrics. Its book health says, per account,
how many journal entries the book replays since the account's last snapshot,
and when that was taken. `logs` prints what a component's pods printed: core's
components, a plugin by its instance, and the release's Jobs (`migrate`, `key`,
`first-run`, `backup`), the newest run of each; `--previous` prints what each
container printed before its last restart. The components and logs are read
through kubectl and the release through helm, and book health is the
deployment's own answer (core's `ReadBookHealthFromTerminal`, on the session
`meridian connect` keeps); nothing is changed, and a deployment admin never
runs kubectl or helm by hand. `meridian <command> --help` prints that
command's own flags.

## Moving a plugin to a newer SDK

```
meridian plugin migrate                 # to the latest release of open-meridian
meridian plugin migrate --to 0.7.0 --run-tests
```

Each release of the SDK that changes what a plugin calls carries a migration
from the release before it (decisions/025). `plugin migrate` moves the
plugin's two pins, `open-meridian==` in `pyproject.toml` and the Dockerfile's
`plugin-python:`, and `.github/workflows/check.yaml`'s `MERIDIAN_VERSION` to
this meridian's own where it names an older one, runs each step between in
the SDK's image, then `plugin check`, and says what is left by hand with its rule, file and line. The
steps run in docker, cut off from the network and given the plugin's files
on stdin; this binary alone writes the plugin's directory, once every step
has run. A base pinned by digest (`plugin-python:<v>@sha256:...`) takes the
new release's digest, read from ghcr.io as any `docker pull` reads it; where
it cannot be read, the old digest is dropped, never kept under the new tag,
and that is left by hand. It refuses changes git does not hold yet, unless `--force`, and it
never goes backwards. `make e2e-migrate` runs it, for real, over the plugins
meridian-python records its migrations for. From 0.23.0 to 0.24.0 (contract
v20) the SDK's step rewrites the renamed roles where a plugin names a role:
its `[tool.meridian]` roles, a `roles=` argument, and the role a suite is run
or an access is read for (`dgm` to `data`, `ccm` to `connectivity`, `match`
to `confirmation`); anything else naming an old role is the plugin's to
change. From 0.24.0 to 0.25.0 (contract v21) nothing a plugin calls changed
shape, so only the pins move, and the same from 0.25.0 to 0.26.0 (contract
v22) and from 0.26.0 to 0.27.0 (contract v23). From 0.27.0 to 0.28.0
(contract v24, one ID for every instrument) the SDK's step rewrites a
record's `instrument_id.startswith("LCL-")` into its `provisional` mark, the
platform's ID read from the `open_meridian` identifier into the record's
`instrument_id`, and a test's `LCL-suite-n` into `INS-suite-n`; a prefix test
or an `open_meridian` read with no record at hand is left by hand, and the
check after it warns of an ID the plugin keeps (`replaced-ids-followed`).
From 0.32.x to 0.33.0 (contract v28) the SDK's step rewrites the wire's
`SubjectRef` handed to the SDK -- `ops.SubjectRef(...)`, `operations_pb2.SubjectRef`
or one imported from the wire bindings -- into `meridian.SubjectRef(...)`
(ruling 599); one inside another wire message stays, and subjects read from a
heard message and given to a typed operation, and each role's suite test the
v28 cases reach, are left by hand.

## Developing a plugin live

On a deployment installed with `--development`, a plugin runs as you write it:
each save is running in about a second, in the same pod, with the same
sidecar and the same grants.

```
meridian up --id DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P --development
meridian connect
meridian plugin new my-plugin && cd my-plugin
meridian plugin dev --instance my-plugin
```

`plugin dev` uploads the plugin unless its version is uploaded already,
launches it live once you approve what it asks for, sends the directory, and
then sends each save. It prints what happens, each with the revision it is
about:

```
r1 sent (9 files, 0 deleted)
r1 synced (9 sent, 0 deleted)
r1 restarted
r1 ready
r2 sent (1 files, 0 deleted)
r2 crashed, exit 1
    Traceback (most recent call last):
    ...
```

Leave it running. Ctrl-C stops watching, and the instance runs on. Run it
again to pick up where it was. What it never sends: `.git`, `__pycache__`,
virtual environments, `build`, `dist`, `*.egg-info`, `.e2e` (where the
plugin's e2e copies the runtime's harness), and whatever the plugin's
`.dockerignore` names. A change to the plugin's dependencies needs a new
version: the live code runs on the image it was launched from.

From another terminal, or an agent:

```
meridian plugin open --instance my-plugin             # a link to its page, for one browser, once
meridian plugin open --instance my-plugin --print /   # the page itself, as you are served it
meridian plugin open --instance my-plugin --level open --print /   # at Open, not the first level held
meridian plugin logs --instance my-plugin --since 3   # what it printed after revision 3
meridian plugin events --instance my-plugin --follow  # sent, ready, crashed, refused, as they happen
```

`open`'s link signs one browser in to that plugin's page alone, for as long as
this session lasts. The link itself works once, within a minute, since a link
in a terminal is easily seen by somebody else. Reload the page as often as you
like; run `open` again for another browser.

A session on a plugin carries one level, as the home's buttons do: `--level
manage`, `open` or `view` (or `admin`, `write`, `read`). Without it, `open`
opens at the first level you hold, Manage before Open before View, as the
home's first button does. A page serves only the levels it is declared with,
so as a plugin's admin, `--print /` on the scaffold's Accounts page is refused
at Manage; `--level open` asks at Open, and `--print /setup` reads its Manage
page.

When it is right, raise the version in `pyproject.toml` and release it:

```
meridian plugin dev --release --instance my-plugin
```

That uploads the directory as it is, as that version, and runs it in place of
the live instance. It is then an ordinary version, in the catalogue, launched
the ordinary way.

For scripts and agents, every one of these takes `--json`, which puts results
alone on stdout (one object, or one per line for `dev` and `--follow`) and
progress on stderr. They exit 0 when done, 1 when refused or failed, 2 when
asked wrongly, and 3 when there is no session or it has lapsed, saying the
`meridian connect` to run.

## Building

```
make ci-local
```

The host needs Docker and nothing else: the toolchain is pinned inside
`Dockerfile.rust`, exactly as in meridian-core. `make install-hooks` makes
`git push` run it first.

## Releasing

A version tag, `v<version>` (the version in `Cargo.toml`), publishes a release: one
binary per target (macOS and Linux, each on x86_64 and arm64), each with its
`.sha256`, which `install.sh` and `meridian upgrade` fetch. Nothing is
published from a merge to `main`. `plugin new`'s templates are vendored from
meridian-python's `template/` and `templates/` at a pinned commit (`make vendor-template`,
which copies the role modules declaring pages to `sdk-roles/` beside them;
`check-vendored-template` fails when it drifts), so a release carries the SDK
version the new plugin pins: 0.20.0 from 0.1.35, 0.21.0 from 0.1.36, 0.22.0
(contract v18) from 0.1.37, with the `dgm` and `reporting`
templates from `templates/`, 0.23.0 (contract v19) from 0.1.38, 0.24.0
(contract v20) from 0.1.39, with the `custody`, `data`, `operations` and
`reporting` templates on their roles' common parts, and 0.25.0 (contract v21)
from 0.1.41, each scaffold's `AGENTS.md` saying how a plugin acts on its own:
as its role's service account, through `meridian.runs_as(role)`, and 0.26.0
(contract v22) from 0.1.42, each `AGENTS.md` saying how work running a
declared algorithm names it (`meridian.runs_as(role, algorithm_id=...)`) and
that a date is worked out in core (`compute_date`, `read_calendar`), and the
custody, data and reporting suites' tests meeting their v22 cases, 0.27.0
(contract v23) from 0.1.43, and 0.28.0 (contract v24) from 0.1.45, each
scaffold with the CI-first pre-push hook (`hooks/pre-push`, written
executable) and the custody and reporting suites' tests meeting their v24
cases, 0.29.0 (contract v25) from 0.1.46, and 0.30.0 (contract v25, the
SDK's role helpers) from 0.1.47, with the `connectivity` template: a venue
placed at through the SDK's `Trader`, 0.31.0 (contract v26) from 0.1.49,
with the `pricing` template and the connectivity template's suite, 0.31.1
from 0.1.50, the pricing template asking for no storage, each `AGENTS.md`
saying how a version is published to a registry with a signed release tag
(ruling 322), and the pre-push hook saying a tag push runs nothing, and
0.32.0 (contract v27) from 0.1.51, the data, reporting and connectivity
templates' suites meeting their v27 cases, the three latency reports' cases
(`about: latency`) marked not presented, with why, until core receives them
in contract v28 (ruling 518), and `sdk-roles/` carrying connectivity's page
helpers (`trading_pages`, `connection_pages`, `raw_records_page`), which
`role-pages` reads: a plugin holding data and connectivity on one
Connection, from `connection_pages`, needs no Trading page of its own.
0.1.51 also moves rustls-webpki to 0.103.16 and webpki-roots and
webpki-root-certs to 1.0.10, for RUSTSEC-2026-0381 to 0383 (ruling 509).
0.32.1 from 0.1.52 (ruling 537): the data suite's `an-exact-price` and
`a-want-recorded-against` are about closes, so a data plugin publishing
only rates names them not presented, with why, and passes `plugin check
--verified`.
0.33.0 (contract v28) from 0.1.53: each scaffold takes `meridian.SubjectRef`
(ruling 599), the data and connectivity templates' suites run the latency
reports' cases, required again (ruling 518), and name v28's new cases not
presented where their stand-ins touch none (ruling 602). 0.1.53 also installs
a starter pack through the deployment, one pack install it keeps, and adds
`meridian plugin pack list|show|update|remove` (ruling 613); both need a
deployment whose core is at contract v28.

## Licence

AGPL-3.0-or-later. See [LICENSE](LICENSE). This links meridian-core's crates,
which are AGPL, so the licence is a consequence of that rather than a
preference: a customer who runs this binary can ask for its source.

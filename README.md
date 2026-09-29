# meridian

The command line for [Open Meridian](https://open-meridian.com), the
open-source OEMS: it brings a deployment up where you have a terminal, and
works with one that is already running. The whole install, step by step, is
meridian-core's [INSTALL.md](https://github.com/open-meridian/meridian-core/blob/main/INSTALL.md).

```
meridian doctor              # can this machine and this cluster run a deployment?
meridian up                  # install the chart, then open the wizard
meridian up --params f.yaml  # the same, answered from a file
meridian down                # uninstall it, keeping its namespace unless asked
meridian upgrade-deployment  # move it to a newer chart, in place, after checking it can
meridian connect [<address>] # sign in to a deployment (default: the local one), and keep the session
meridian sign-out            # end that session, here and at the deployment
meridian plugin new <name>   # start a plugin from the SDK's reference plugin
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

Every command exits non-zero on failure, and `plugin dev`, `logs`, `events` and
`open` take `--json` for scripts and AI agents.

## Install

On macOS (Apple silicon or Intel) or Linux (x86_64 or arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/open-meridian/meridian-cli/main/install.sh | sh
```

It downloads this machine's binary from the latest release, checks it against
the `.sha256` published beside it, and puts it in `~/.local/bin` -- no `sudo`.
`MERIDIAN_INSTALL_DIR` puts it elsewhere and `MERIDIAN_VERSION` picks a
release. Then:

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
install into a namespace, Helm present and recent enough, a storage class for
the deployment's key, the image pullable from here, the platform reachable, and
the clock within the tolerance a signed assertion allows.

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
it is reached through the chart's Ingress as `http://meridian.localhost`.
Every browser sends that name, and every plugin's page on a name below it, to
this machine, so the address stays after `up` ends and plugin pages work.
`--host` names another: under `.localhost` it is plain HTTP, and any other
name — your firm's — is reached over HTTPS, with the Secrets holding its
certificates named in a values file:

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
empty answer.

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
```

Moves a running deployment to a newer version of its chart, in place, with
your own cluster rights: nothing in a deployment holds a right to change the
cluster, and an upgrade is the widest change there is. Not `meridian upgrade`,
which replaces this binary.

It checks first and changes nothing if a check fails: the cluster reachable
and your rights in the namespace, Helm 3.14 or newer, the release `deployed`
(a `failed` or `pending` one is refused with how to recover it), the version
published and not older than the installed one. At that version already, it
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
reason Kubernetes gives.

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

It never prints the deployment's values, which hold its enrolment code; the one
thing it reads from them is `image`. The same steps from a firm's own pipeline
-- plain Helm, Flux or Argo CD -- are in the docs' *Upgrade a deployment*.

## connect and plugins

```
meridian connect
```

Signs in to the deployment in your browser, however it signs people in, and
keeps a session here: 30 minutes idle, 12 hours at most. It never takes a
password. `meridian sign-out` ends it, here and at the deployment.

On that session, as the deployment's administrator:

```
meridian plugin new my-plugin            # ./my-plugin, from the SDK's reference plugin
meridian plugin upload --dir my-plugin   # built with docker, from its own Dockerfile
meridian plugin list                     # versions uploaded, and what is launched
meridian plugin launch my-plugin 0.1.0 --instance my-plugin
meridian plugin stop my-plugin
```

`plugin new` writes a working plugin: its code and page, a `Dockerfile`,
`pyproject.toml`, and `AGENTS.md`, which teaches any coding agent the live
loop below. `CLAUDE.md` and the `develop-live` skill lead Claude Code to the
same text. Commit them with the plugin; `.dockerignore` keeps them out of its
image.

`launch` shows the roles and tags the version asks for and runs it only once
you approve them; `--yes` approves for a script that has already read them.
Its page is on its own name, `http://my-plugin.plugins.meridian.localhost/`,
opened from the dashboard's home.

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
virtual environments, `build`, `dist`, `*.egg-info`, and whatever the plugin's
`.dockerignore` names. A change to the plugin's dependencies needs a new
version: the live code runs on the image it was launched from.

From another terminal, or an agent:

```
meridian plugin open --instance my-plugin             # a link to its page, for one browser, once
meridian plugin open --instance my-plugin --print /   # the page itself, as you are served it
meridian plugin logs --instance my-plugin --since 3   # what it printed after revision 3
meridian plugin events --instance my-plugin --follow  # sent, ready, crashed, refused, as they happen
```

`open`'s link signs one browser in to that plugin's page alone, for as long as
this session lasts. The link itself works once, within a minute, since a link
in a terminal is easily seen by somebody else. Reload the page as often as you
like; run `open` again for another browser.

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
`Dockerfile.rust`, exactly as in meridian-core.

## Licence

AGPL-3.0-or-later. See [LICENSE](LICENSE). This links meridian-core's crates,
which are AGPL, so the licence is a consequence of that rather than a
preference: a customer who runs this binary can ask for its source.

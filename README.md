# meridian

The command line for bringing a Meridian deployment up where you have a
terminal, and for working with one that is already running.

```
meridian doctor              # can this machine and this cluster run a deployment?
meridian up                  # install the chart, then open the wizard
meridian up --params f.yaml  # the same, answered from a file
meridian connect <address>   # sign in to a deployment, and keep the session
meridian plugin new <name>   # start a plugin from the SDK's reference plugin
meridian plugin upload       # build it and put it in the deployment's catalogue
meridian plugin launch …     # run a version, once you approve what it asks for
meridian plugin dev …        # run it live while you write it, on a development deployment
```

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

**What it is not: the way to install in a cloud.** A marketplace listing
installs the chart and the deployment's own wizard does the rest. Nothing here
is required for that, and anything this makes convenient is possible without
it. See `spec/the-cli.md` in meridian-design for what this is for and what it
deliberately leaves alone.

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
meridian up --id dep-7
```

Runs `doctor` first — skip it with `--no-doctor` — then installs the chart with
the two values the platform gave you, waits for the dashboard, and prints the
wizard's address.

Where the cluster has an ingress controller — Rancher Desktop's Traefik, say —
it is reached through the chart's Ingress as `http://meridian.localhost`
(`--host` for another name under `.localhost`). Every browser sends that name,
and every plugin's page on a name below it, to this machine, so the address
stays after `up` ends and plugin pages work. Answer the wizard's dashboard
address with it. Where there is no controller, or with `--no-ingress`, `up`
forwards a local port instead (`--port`, default 8443) and holds the forward
until you stop it.

`--development` installs it for development: it may run plugin code as it is
being written, which nobody has reviewed, and every page says so. It is an
install choice, made here or in a sandbox's own values, and never switched on
from the dashboard. It is what `plugin dev` needs; a firm's own deployment
never has it.

It drives your own `helm` and `kubectl` and prints the command it used, so you
can do the same by hand. It embeds no Helm library: the chart is what says what
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
meridian up --id dep-7 --params first-run.yaml
```

**The file holds no credential.** Every field the wizard asks for as a password
is read from `MERIDIAN_<FIELD>` instead, and a file naming one is refused with
the variable to use — because a file that works is a file that gets committed.

The field names are the wizard's own: what it asks for is read from the page it
serves, so a typo is refused against the real form rather than posted as an
empty answer.

## connect and plugins

```
meridian connect http://meridian.localhost
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

`launch` shows the roles and tags the version asks for and runs it only once
you approve them; `--yes` approves for a script that has already read them.
Its page is on its own name, `http://my-plugin.plugins.meridian.localhost/`,
opened from the dashboard's home.

## Developing a plugin live

On a deployment installed with `--development`, a plugin runs as you write it:
each save is running in about a second, in the same pod, with the same
sidecar and the same grants.

```
meridian up --id dep-7 --development
meridian connect http://meridian.localhost
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

# meridian

The command line for bringing a Meridian deployment up where you have a
terminal, and for working with one that is already running.

```
meridian doctor              # can this machine and this cluster run a deployment?
meridian up                  # install the chart, then open the wizard
meridian up --params f.yaml  # the same, answered from a file
```

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
the two values the platform gave you, waits for the dashboard, forwards a local
port to it, and prints the wizard's address. A first-run dashboard has no public
address and should not get one, so the forward is held by this process and
dropped when you stop it.

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
backend: bundled
directory: ldap
dashboard_url: https://meridian.firm.example
```

```
export MERIDIAN_FIRST_RUN_CODE=…
export MERIDIAN_DB_SERVING_PASSWORD=…
export MERIDIAN_DB_MIGRATING_PASSWORD=…
meridian up --id dep-7 --params first-run.yaml
```

**The file holds no credential.** Every field the wizard asks for as a password
is read from `MERIDIAN_<FIELD>` instead, and a file naming one is refused with
the variable to use — because a file that works is a file that gets committed.

The field names are the wizard's own: what it asks for is read from the page it
serves, so a typo is refused against the real form rather than posted as an
empty answer.

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

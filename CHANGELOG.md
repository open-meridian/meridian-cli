# Changelog

## 0.1.59

- `meridian upgrade-deployment --set key=value`, repeatable, beside `-f`, applied as Helm applies
  them. A values change is an upgrade even at the chart version already running. The plan names
  each key that changes, never its value, and withholds any value whose key names a secret. With
  Helm 4 it passes `--force-conflicts`, so the chart's counts replace the ones a first run or a
  restore set, as Helm 3 did.
- Moving a running brought database to `resilience.profile=ha`, standbys under Patroni, takes the
  chart's two upgrades. `upgrade-deployment` says so before it asks, and runs both under one
  approval, the second once the first has rolled out. If the first stops, it says which step
  stopped and what to run; run again, it finishes a half-made move. Back to `tolerant` is one
  upgrade.
- `meridian upgrade-deployment --enable-checksums` turns on Postgres's data checksums in the
  database the deployment brought. Already on: it says so and changes nothing. Otherwise it says
  the downtime, estimated from the database's size, before it asks; takes a backup first, and
  changes nothing if that fails; stops every component and the database; makes the change (under
  Patroni on member 0, the standbys then made again from it); starts everything at the counts it
  ran at; and reads the setting again.
- `meridian restore --to-time <time>`, RFC 3339 with its offset, restores the brought database in
  place from its WAL archive (`backup.walArchive`) to that time: everything stopped, the restore
  run, everything started again, under Patroni the standbys made again from member 0. Without the
  archive it is refused, saying how to turn it on. A time outside the archive fails in the
  restore's own words, leaves everything stopped and the database restorable, and the next run
  takes it from there. After a time past the archive's end, it says the latest time that can be
  restored lies before the archive's last commit, gives that commit's time where Postgres's lines
  include it, and asks for a time before it.
- `meridian restore <file>` into an ha deployment loads member 0, then brings the standbys back
  from it before it gives the database's address. Into a deployment whose external database has
  its own CA (`database.caSecret`), the restore connects with that CA; a backup without the Secret
  is refused before anything changes.
- While `restore --to-time` or `--enable-checksums` has the deployment stopped, the release's
  CronJobs (the replica comparison, the scheduled backup, the scrub) are suspended, and each is
  given back as it was when everything starts again. Neither starts while a Job of an earlier one
  still runs. A Job that fails or does not finish shows its cause from its log.
- `meridian status` shows the database's members (primary or standby, ready, how far behind), a
  member waiting for its node, which is never force-deleted, whether data checksums and
  `full_page_writes` are on, the WAL archiver's last segment and whether it is failing, the
  replica comparison's last run, whether internal TLS is on, and each store's probe. While a
  restore or `--enable-checksums` has the deployment stopped, it says which, since when, and the
  command that resumes it. `--json` keeps every key it had and adds these.
- `meridian doctor` says where a reachable database has data checksums or `full_page_writes` off,
  and points a brought one to `--enable-checksums`.

## 0.1.58

- `plugin check`'s `role-suite` reads contract v29's suites (meridian-schema ce3a6f3, as SDK 0.35.2
  vendors them): the data suite's `records-a-derived-series` and `records-a-pools-factor` can be named
  in `not_presented`, where they were refused as "no such case", and so can every read the suites now
  mark with its `about` (datasets, orders, trading instructions, replacements, templates, event
  groups and the rest), `reads-its-datasets-licences` among them. `roles.json` moves to the same
  revision.
- `roles-registered` judges a file under `e2e/` by the roles it registers itself, as the SDK's
  migration does: a stand-in reader's classes are neither held to the roles the plugin declares nor
  warned of, and its entry no longer hides a registration the plugin's own entry lacks.
- `plugin check --run-tests` (and `plugin migrate --run-tests`) runs the tests in the project's own
  environment: `uv run --frozen` for a project with a `uv.lock` and `uv` on the PATH, else the Python
  of its `.venv` or `venv`, else the `python3` on the PATH. A run that cannot import the plugin or the
  SDK says which module is missing and how to give the tests an environment, not only that they
  failed.
- `plugin migrate --to X` accepts a release the registry's JSON does not list yet but its simple
  index does (publish lag), and runs the steps in that release's own image.

## 0.1.57

- `meridian plugin migrate` gives the SDK's migration steps the plugin's `Makefile` beside its `.py`
  files, `pyproject.toml` and `Dockerfile`, so SDK 0.35.1's `core-images` finding (the Makefile's
  `RUNTIME_IMAGE` and `HARNESS_IMAGE` pins below the contract) can see it. A Makefile a step
  rewrites is written as the step left it; a plugin with none is given none. A pin to the old
  release that no step moves is still said by hand (`pin-elsewhere`).
- `plugin check`'s `replaced-ids-followed` advice follows the SDK the plugin pins: from 0.35 it
  names the role's `on_replace_merged_instrument` method (`await ids.heard(heard)`, with
  `await ids.catch_up(self)` in `on_start`), and for an earlier pin it keeps
  `receive(replace_merged_instrument=ids.heard)`. The rule and the warning are unchanged.

## 0.1.56

- The supported systems (ruling 1033), in README's "Supported systems": macOS's current major and the
  two before it (27, 26 and 15), on Apple silicon and on Intel while Apple ships macOS for Intel Macs;
  Linux on x86_64 and arm64, any distribution (static, against musl); Windows through WSL2 with the
  Linux binary. Both macOS binaries are built for macOS 15 and newer (`MACOSX_DEPLOYMENT_TARGET`
  15.0, checked with `vtool` in the release build), so macOS 14 and older no longer start them.
- Each release binary is run on every system of the window before it is published: in Ubuntu 22.04
  and 26.04, Debian 13, Rocky Linux 9 and Alpine 3.24 containers on x86_64 and arm64, and on GitHub's
  macOS 15 and 26 runners, Apple silicon and Intel. Each runs `meridian --version`, `meridian status`
  against a stand-in `kubectl` and `helm`, and `meridian plugin new` then `plugin check`
  (`tests/smoke/run.sh`). `publish` waits on every one, and the same builds and checks run on each
  pull request, publishing nothing. `make smoke-linux`, in `ci-local`, runs them on the static binary
  for the machine it runs on.
- The install script and `meridian upgrade` run a new binary once before it takes `meridian`'s place:
  a release that does not run on this system -- a macOS older than the window's floor -- installs
  nothing and leaves the binary that was there, saying why and where the supported systems are.

## 0.1.55

- `meridian plugin new` writes the SDK's 0.35.0 scaffolds (contract v29, no contract change; rulings
  1009 to 1023): each role a plugin holds is one class, `meridian.roles.<role>.Role`, and the plugin
  enters through `meridian.run(declaration=..., roles=[...], ...)` with its role's class registered
  there. The scaffolds are copied from meridian-python's main at ab749b7, where #37 merged
  (the same template trees as branch `0.35` at 2e90655).
- `plugin check`'s `template-shape` takes `meridian.run(...)`, `meridian.run_async(...)` or
  `meridian.connect(...)` (or `run`, `run_async` or `connect` imported from `meridian`) as the
  plugin's entry. It reads the call in code, so a docstring naming one no longer counts. A plugin
  calling none still fails.
- A new rule, `roles-registered` (ruling 1015). A role that `[tool.meridian] roles` declares with no
  class of it registered in `roles=[...]` fails, naming the role at the entry's line. A class
  registered for a role the plugin does not declare is a warning: the SDK does not start it, and
  logs that. A registered object is traced to its role through its imports, its assignments and
  the plugin's own classes and their bases, across modules. An object the check cannot trace, such
  as a factory's result, fails nothing. A plugin pinning an SDK before 0.35 that connects without
  `roles=` keeps every rule as before.
- `settings-declared` and `through-the-sdk` say what to write in 0.35's terms: settings among the
  declaration's, read through `on_settings`, and acts made on the role's class.

## 0.1.54

- A plugin can be named in Meridian apart from its Python project (rulings 945, 971). Set
  `[tool.meridian] name` in pyproject.toml and that name is used for the registry listing,
  `plugin publish` and `plugin release` (including the push scope), and `plugin upload`'s catalogue
  entry. Without it, `[project] name` is used as before. For example, plugin-databento keeps
  `[project] name = "plugin-databento"` and lists as `databento`.
- `meridian status` reports each pod's memory as its working set, the way Kubernetes counts it: the
  pod's cgroup usage minus its inactive file cache (ruling 948). The figure comes from each node's
  cAdvisor. The metrics API names the same figure, but some container runtimes put the whole usage
  there, file cache included. When the cAdvisor read is refused, status falls back to the metrics
  API, says so, and `--json` reports where the figure came from in `memory_read`.
- The build cache gets core's fresh-sources fix (ruling 934). Every cargo run against the target
  cache goes through `tools/fresh-sources.sh`, so a build from another checkout is never reused for
  this one. The cache now has its own id, `meridian-cli-target`, and `make check-fresh-sources`
  tests the script.
- `meridian doctor` accepts Kubernetes 1.33 and newer (rulings 849, 935, 936).
- Backups include each plugin's storage volume, and restore brings each one back file for file
  (ruling 828). The copy `--to` takes is read from the pod's own held copy, and is read again, up to
  three times, when its size or hash differs from what the Job reported.
- `meridian logs --since 1d` passes days to kubectl as hours.
- The scaffolds `plugin new` writes are re-vendored from meridian-python v0.33.3, each with its
  `AGENTS.md` (ruling 852).

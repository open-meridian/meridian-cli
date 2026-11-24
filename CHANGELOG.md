# Changelog

## Unreleased

### Added
- `meridian stop` stops a deployment's workloads, deleting nothing, and `meridian start` starts them again exactly as they were. What runs is recorded first on the database's StatefulSet (`meridian.dev/stopped`); `stop` refuses while a restore's record, a `pending-*` release or a migration, restore, rollback or act Job is there, and either, cut short, finishes when run again. While stopped, `status` says one line (`--json`: `stopped.since`), `doctor` says so without failing, and `up`, `upgrade-deployment`, `rollback-deployment` and `restore --to-time`/`--to-archive-end` refuse, pointing to `meridian start`.

### Changed
- Shorter output: two-tier help, one-line usage errors, a compact `up` and `status` (`--wide` for detail), and global `-q`/`-v`.
- `status --profile` prints the profile's advice in its text output too, a line each (`--wide` for each in full), not only in `--json`.

### Fixed
- `status` says a plugin's memory against its own container's limit, its sidecar's apart (`--wide`, `--json`'s `sidecar`), not the pod's summed.

### Deprecated
- `up --enrolment-code`, `--first-run-code`, `--shared-storage-class` and `plugin publish`/`release --details` warn with their replacement; removed next release.

Step 3d. These need a deployment whose core carries step 3d's terminal routes and probe; against
an older one each says so and changes nothing.

- `meridian partition split <partition> --reason <why>` halves a lake range at its midpoint, and
  prints the split's record: the range closed, its two halves, who and when.
- `meridian partition move <partition> --to-group <n> --reason <why>` moves a lake range or a book
  partition (`P0`, `P1`, ...) to another writer group, online. The deployment answers at once that
  it has begun; the command then follows the Resilience history until the move's record shows,
  moved or not switched with its cause (lag, timeout, digest), for `--timeout` (2m by default).
  `--no-wait` answers once it has begun. A move still under way when the time runs out is left to
  finish, and said.
- `meridian account move <account> --to-partition <partition> --reason <why>` continues an account
  in another book partition, online, and prints its record.
- Each of the three is a deployment admin's, through the session `meridian connect` keeps, as the
  Resilience page's dialogs and the dashboard's tools make them. A refusal comes through in the
  deployment's words; one that names a wait (a rollout in progress, the target lagging, a switch
  out of time) says nothing was changed and when to ask again. `--json` prints the deployment's
  answer, a refusal with its `retry_after_seconds`. Exit 0 done or begun, 1 refused or not
  switched, 2 asked wrongly, 3 without a session.
- `meridian upgrade-deployment` checks the counts before it changes anything:
  - a lower `bor.partitions` is refused while a partition past the new count holds accounts,
    naming each one and its accounts;
  - a lower `lake.databases` or `bor.databases` is refused while partitions hold data in a
    database past the new count, naming them;
  - `writerGroups` or `databases` changed in the same upgrade as the deployment's first start on
    a release that has them is refused: upgrade first, then change the count.
  What runs is read from the workloads' own settings, and what each partition holds from the
  stores' probes; where a probe does not say, a lowering is refused and says it could not be read.
- `meridian rollback-deployment` refuses a rollback that would lower a store's writer groups where
  they can only be raised (the lake in ha or with its write-ahead stream on, the book in ha), with
  the same message an upgrade gets, naming the revision. If it can't read what that revision would
  run, it refuses and says so.
- `meridian upgrade-deployment` no longer suggests rolling back after a failed upgrade that raises
  writer groups, since that would lower them again. It says to fix the cause and run the upgrade
  again.
- `meridian status` probes the members of each writer group past the first too
  (`<store>-group-<g>`), so each group's writer, its lag and its partitions are shown.
- `meridian upgrade-deployment` refuses an upgrade that leaves no store or conductor with a
  standby (`resilience.profile` to tolerant, or the last `standby` to 0) while a plugin instance
  runs with several shards or a standby pod, naming each: their leases are taken under the
  members' account, which that upgrade removes. Stop them or relaunch them unsharded first.
- `meridian status` counts a plugin's shards as what it wants ready, and its pods past them as
  standbys: `2/2 ready, 1 standby`. `--json` adds `standbys`.
- `meridian upgrade-deployment` counts a plugin as rolled out when its shards are ready and every
  one of its pods is on the new template: a standby pod, which waits unready by design, no longer
  keeps the upgrade from finishing.
- `meridian up --shared-storage-class <class>` names the ReadWriteMany storage class a plugin's
  shards and standby pod share its storage through on several nodes, the chart's
  `pluginStorage.shared.storageClassName`. A class the cluster does not have, or one a values
  file names too, is refused before anything is installed, naming the value.
- A launch the deployment refuses comes through in the deployment's own words: the bus's framing
  of a refusal (`[refusal-status ...]`) is no longer printed before them.
- `meridian plugin list` says, under a launch whose plugin's storage was copied into its shared
  storage, what the Plugins page says: `Its storage was copied from {old} into {shared}, folder
  shard-0/, on {date}. The old claim is kept.` `plugin launch` says it too once it is recorded.
- `meridian doctor` says, where plugins run: each that keeps storage and cannot take a standby
  (several nodes, no `pluginStorage.shared.storageClassName`); each running with shards or a
  standby pod and allowed a bucket archive, with the account `<namespace>/<fullname>-shards-<instance>`
  its bucket must trust and the cloud's step; and each such plugin's shards' epoch and last
  takeover, from the Resilience history on the session `meridian connect` keeps.
- New plugins start on SDK 0.37.1. `meridian plugin new --role data` writes the data template's
  `pacing.py`, `stream.py` and `tests/test_examples.py`, which it had left out.
- `meridian restore` (each form) and `meridian backup` refuse a deployment where the lake or the
  book runs more than one database (`lake.databases` or `bor.databases` above 1), before changing
  anything: each covers only a store's first database, and says which databases it would leave
  out. A backup taken where a store ran more than one database is refused at restore the same way.

### Changed

- `meridian upgrade-deployment` refuses a client-side merge over a launched plugins' Secret Helm once
  applied server-side, while plugins hold credentials, naming the plugins to stop first. (#30)
- `meridian upgrade-deployment` refuses, with nothing changed, when it can't read how Helm last applied
  the release or who manages the launched plugins' Secret. (#30)

### Fixed

- `meridian rollback-deployment` refuses a rollback that would put back a launched plugin's old
  credential or remove a live one, and names the plugins to stop first. (#29)
- `meridian upgrade-deployment` on Helm 4 no longer fails on a release Helm 3 installed: it passes
  `--force-conflicts` only where Helm last applied server-side. (#30)

### Security

- Updates `url` to 2.5.9 for RUSTSEC-2026-0592.

## 0.1.60

- `meridian restore --to-time` follows the timeline that held the time. Each restore starts a
  timeline of its own, so a second restore to a time before the first one's lies on an earlier
  timeline; the chart's restore (meridian-core's chart from step 3c) reads the archive's timeline
  history, recovers along the one that held the time, from the newest base backup on its history,
  and the CLI says which (`It recovered along timeline 1, the one that held ...`). There is no
  option to choose it. A time outside the archive is refused before the database's volume is
  touched, naming the archive's first and last times, and the hint names them back with
  `--to-archive-end` for the end itself. On a chart from before, it says before anything stops that
  the restore follows the newest timeline.
- `meridian restore --to-archive-end`: the same guards and steps as `--to-time` -- refused without
  `backup.walArchive`, saying how to turn it on; everything stopped, the restore run, everything
  started again, under Patroni the standbys made again -- to the last commit the archive holds,
  along its newest timeline. A chart from before step 3c is refused before anything stops. A stop
  it leaves is resumed by `meridian restore --to-archive-end`, as `status` says.
- `meridian plugin launch` takes `--in-flight`, `--requests-per-second` and `--answer-mib`, the
  instance's own limits at its sidecar over the deployment's `sidecar.limits`, and `--shards` and
  `--standby`, which core keeps on the launch (it still runs one pod; the pods come with step 3d).
  Each is shown with the roles before it asks, a wrong value is refused naming the field as core
  names it (`limits.answer_mib`, `shards`), and core's own refusals come through as it words them.
  After the launch it reads back what the deployment kept; a core from before contract v30 keeps
  none of them, and that is said. `meridian plugin list` shows each launch's limits, shards and
  standby.
- `meridian status` names each store member's zone and the partitions it owns or follows, beside
  its mode, readiness and lag, a long list cut on its line and whole in `--json`.
- `meridian status --profile` prints the resilience profile as the deployment computes it from its
  configuration as it is, never a preset's name: its level and what the next level lacks, each
  failure class and tier met, met by restart or not met with its reason and the change that would
  meet it and its cost, and never blind per live data type. `--json` gives the deployment's answer.
  It reads core's terminal read of the profile (from contract v30) on the session `meridian
  connect` keeps, a deployment admin's; an older core is said to be older.

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

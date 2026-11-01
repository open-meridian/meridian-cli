# Changelog

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

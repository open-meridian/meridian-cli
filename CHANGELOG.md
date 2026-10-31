# Changelog

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

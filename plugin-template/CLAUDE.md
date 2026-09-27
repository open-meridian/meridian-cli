# reference-plugin

A Meridian plugin, in Python on the open-meridian SDK. It runs beside a
sidecar in a Meridian deployment and reaches nothing else: what it may publish
and subscribe to comes from the roles `pyproject.toml` declares under
`[tool.meridian]`, once a deployment admin approves them.

- `src/reference_plugin/__main__.py` connects to the sidecar and serves the
  page; `src/reference_plugin/page.py` is the page.
- To change it and see the change running on a cluster, follow the
  `develop-live` skill (`.claude/skills/develop-live/SKILL.md`): `meridian plugin
  dev` in the background, edit, wait for `ready` at your revision, then check.
- A save changes what the plugin does, never what it is allowed to do. Roles,
  tags and dependencies take a new version, which a person approves.
- This file and `.claude/` are the person's own: `.gitignore` keeps them out of
  the repository, and `.dockerignore` out of the image and the live instance.

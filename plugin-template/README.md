# reference-plugin

A Meridian plugin, made by `meridian plugin new`.

It runs beside a sidecar in a Meridian deployment and reaches nothing else.
What it may publish and subscribe to comes from the roles `pyproject.toml`
declares, once a deployment admin approves them when it is launched, never
from its code.

Put it in a deployment that `meridian connect` signed you in to:

    meridian plugin upload
    meridian plugin launch reference-plugin 0.1.0 --instance reference-plugin

On a deployment installed for development, run it as you write it, each save
running in about a second:

    meridian plugin dev --instance reference-plugin

`CLAUDE.md` and the `develop-live` skill under `.claude/` teach Claude Code
the same loop. They are yours rather than the plugin's, so `.gitignore` keeps
them out of the repository and `.dockerignore` out of the image.

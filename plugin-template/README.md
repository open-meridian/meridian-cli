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

Its page is built on Open Meridian's plugin UI kit, which the dashboard
serves on the plugin's own host at `/.meridian/ui/`: the platform's look, and
each person's colour scheme, with no design work. Where the kit is not served
the page still works, unstyled.

`AGENTS.md` teaches any coding agent to build pages with the kit, and the same
live loop; `CLAUDE.md` and the `develop-live` skill under `.claude/` lead
Claude Code to it. Commit them with
the plugin, so whoever works on it next has them too; `.dockerignore` keeps
them out of its image.

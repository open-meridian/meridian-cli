# reference-plugin

A Meridian `reporting` plugin, made by `meridian plugin new --role
reporting`: it reads the book and the lake, and shows the positions in its
account scope at the last close, valued.

It runs beside a sidecar in a Meridian deployment and reaches nothing else.
What it may publish and subscribe to comes from the roles `pyproject.toml`
declares, once a deployment admin approves them when it is launched, never
from its code. It records nothing: what it computes is computed and shown.

Put it in a deployment that `meridian connect` signed you in to:

    meridian plugin upload
    meridian plugin launch reference-plugin 0.1.0 --instance reference-plugin

A deployment admin entitles it to the datasets it reads on the dashboard's
Data sources page. On a deployment installed for development, run it as you
write it, each save running in about a second:

    meridian plugin dev --instance reference-plugin

Hold it to the rules every Meridian plugin is built to, and run its tests:

    pip install -e . pytest
    meridian plugin check --run-tests

Its tests run the `reporting` suite the SDK carries (`tests/test_suite.py`):
the datasets it may read, a business date's closes, daily bars over a range,
the prices and bars the lake records, heard, and its reporting currency
resolved by its ISO 4217 code. Every figure stays a `Decimal`, as the lake
keeps it.

Its pages: Closes under Open (`write`) and View (`read`), each position in
the accounts the person may read, its close, the dataset it came from, the
value it makes and the change over the week, also a read tool for an agent
the person delegates to, `read_report`, answering the same rows; and Datasets
under Manage (`admin`), what it may read, with no account's data. They are built on Open
Meridian's plugin UI kit, which the dashboard serves on the plugin's own host
at `/.meridian/ui/`.

`AGENTS.md` teaches any coding agent to build pages with the kit, and the same
live loop; `CLAUDE.md` and the `develop-live` skill under `.claude/` lead
Claude Code to it. Commit them with the plugin, so whoever works on it next
has them too; `.dockerignore` keeps them out of its image.

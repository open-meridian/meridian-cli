# reference-plugin

A Meridian `dgm` plugin, made by `meridian plugin new --role dgm`: it puts a
vendor's prices into the deployment's lake.

It runs beside a sidecar in a Meridian deployment and reaches nothing of the
deployment's else. What it may publish and subscribe to comes from the roles
`pyproject.toml` declares, once a deployment admin approves them when it is
launched, never from its code. Its vendor is its own to reach: `vendor.py`
stands in for one, answering from responses written into it, so it runs
anywhere until you replace it with your vendor's client.

Put it in a deployment that `meridian connect` signed you in to:

    meridian plugin upload
    meridian plugin launch reference-plugin 0.1.0 --instance reference-plugin

A deployment admin then licenses its dataset, and entitles the plugins that
may read it, on the dashboard's Data sources page. On a deployment installed
for development, run it as you write it, each save running in about a second:

    meridian plugin dev --instance reference-plugin

Hold it to the rules every Meridian plugin is built to, and run its tests:

    pip install -e . pytest
    meridian plugin check --verified --run-tests

Its tests run the `dgm` suite the SDK carries (`tests/test_suite.py`): each
case of the role mapped to one of the vendor's responses and run through the
plugin's own conversion (`convert.py`). A plugin holding `dgm` is verified
for it only by passing every case: keep each passing as you replace the
vendor. Every price is parsed from the vendor's text as a `Decimal`, never
through a `float`; every subject and venue is resolved before a row names
it, and what does not resolve is reported.

Its catalogue (`declaration.py`) declares the dataset it serves: its key,
vendor, data types, how its rows arrive, its cadence, history, the vendor's
terms and its day. Its pages are under the dashboard's Manage (`admin`):
Connection, whether the vendor answers and what was asked of it, and
Datasets, what it serves and the standing wants it keeps current. It shows
no price: prices are the reading roles'. Each page is also a read tool for
an agent an admin delegates to, `read_connection` and `read_datasets`. They
are built on Open Meridian's plugin UI kit, which the dashboard serves on the
plugin's own host at `/.meridian/ui/`.

`AGENTS.md` teaches any coding agent to build pages with the kit, and the same
live loop; `CLAUDE.md` and the `develop-live` skill under `.claude/` lead
Claude Code to it. Commit them with the plugin, so whoever works on it next
has them too; `.dockerignore` keeps them out of its image.

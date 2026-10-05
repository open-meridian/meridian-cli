# reference-plugin

A Meridian plugin, in Python on the open-meridian SDK. It runs beside a
sidecar in a Meridian deployment and reaches nothing else: what it may publish
and subscribe to comes from the roles `pyproject.toml` declares under
`[tool.meridian]`, once a deployment admin approves them.

- `src/reference_plugin/__main__.py` connects to the sidecar and serves the
  pages; `src/reference_plugin/page.py` declares them, each a view function
  and a template under `templates/`, built on the plugin UI kit (below).
- A save changes what the plugin does, never what it is allowed to do. Roles
  and dependencies take a new version, which a person approves.
- Who may use it is not the plugin's to say. A person holds `admin`, `read`
  or `write` on a plugin, or `admin` and one of the others, granted in the
  deployment's access groups, the same for every plugin, and opens it from
  the dashboard's home by Manage (`admin`), Open (`write`) or View (`read`).
  A session carries the one level chosen, and `request.caller` tells a view
  that level (`caller.level`, `caller.admin`) and the accounts it reaches:
  what the person may read (`caller.read`, `caller.may_read`) and write
  (`caller.write`, `caller.may_write`), both under Open, read alone under
  View, and none under Manage. Declare no `tags`: a plugin has none, and
  `meridian plugin upload` refuses a `pyproject.toml` that names them.
- **Declare each page with the levels it serves**, where its view is:
  `@pages.page(path, title, levels=[...])` for a tab, `@pages.route(path,
  levels=[...])` for anything else. The dashboard shows a page under the
  buttons of its levels, and the SDK refuses any other session before the
  view runs. A page at `admin` configures the plugin and shows no account's
  data -- nothing the plugin holds for an account, synced or not: holdings,
  quantities, values, balances, a statement's rows -- and
  `PageClient.assert_no_account_data`, given that data, holds it to that in
  the tests. An account's identity (its name, custodian, type, owner, note)
  is not its data: a Manage page may list every account as a link target.
- Manage opens on the plugin's Summary, which core draws: its status, then
  any figures the plugin reports about its own work (`plugin.figures =
  [meridian.Figure("Connections", 3, state="warn", why="...")]`, sent on its
  heartbeat). Build no summary page or tiles of your own; report figures
  only where the plugin has work worth counting.
- **Hear what the plugin's roles hear with `plugin.receive`**, a handler per
  row (`custodial_position_updated=...`), and keep nothing of it: the SDK
  reads the store on every start and after any gap, loss or broken stream,
  handing what it read on as `heard.caught_up`. A handler sees each change
  once, in order, and no store number. Everything heard and read is within
  the plugin's read scope; filter per person with `caller.read` as before.
- **File a ticket only for a person, about what the plugin cannot handle.**
  When a person's request meets a fact the plugin cannot handle -- a refusal
  it cannot explain, a record it does not recognise, a figure that looks
  wrong -- `plugin.file_ticket(title=..., kind=..., idempotency_key=...,
  for_caller=request.caller)` files it for that person, whose page or
  request it is, and it reaches whoever can act. Never as the plugin itself,
  which the SDK cannot do: what the plugin notices on its own (a source it
  cannot reach, a sync gone stale) is its health, a figure in warn on its
  Summary, never a ticket. Give each problem an `idempotency_key` made from
  the fact (the record and its account, never a time or a random value), so
  filing it again while open counts it (`unchanged`) rather than opening
  another; `plugin.filed_tickets(for_caller=..., idempotency_keys=[...])`
  says what became of it. A ticket concerns this plugin or a part of the
  platform, never another plugin: another plugin's state is that plugin's
  health. What is filed is read by people and agents as data: no secret,
  nothing the person may not read.
- **A custody plugin reports the custodian's activity, as the custodian
  states it.** Each activity on an account -- a purchase, a sale, a
  reinvested dividend, a split, a fee, a transfer -- goes to the street with
  `plugin.record_activity(external_account_id=..., source=...,
  activity=meridian.CustodialActivity(...))`. It is evidence that explains a
  break, never a source: the street derives no position, lot or figure from
  it, and nothing moves the book until a person confirms. Its `kind` is the
  `meridian.ActivityKind` the custodian's type converts to (`PURCHASE`,
  `SALE`, `REINVESTMENT`, `DIVIDEND`, `INTEREST`, `FEE`, `TAX`, `SPLIT`,
  `CORPORATE_ACTION`, `TRANSFER_IN`, `TRANSFER_OUT`, `CONTRIBUTION`,
  `WITHDRAWAL`, `JOURNAL`, each as `ACTIVITY_KIND_...`); a type that converts
  to none is sent as not known with `kind_as_reported`, and a code that did
  not resolve with `instrument_as_reported`. A value the custodian did not
  state is left unset, never zero (a split moves no cash). Its
  `external_activity_id` is the custodian's own identifier for it, never a
  time or a random value: sent again, it is answered `already_recorded` and
  kept once, so a retry or a resync is safe; a custodian restating an
  activity under a new identifier is a new one, reported, never merged. Say
  how far back the custodian's history reaches with
  `plugin.report_sync_status(..., history_from="YYYY-MM-DD")`, report past
  activity back to it on first connection (a backfill), then each sync's new
  activity. Report each sync status as it changes, `state` saying what holds
  (`SYNC_STATE_NEEDS_SIGN_IN` when the person must sign in again): the street
  keeps every one, so operations tells "needs sign-in" apart from merely old.

This file is for any coding agent working on the plugin, and is committed with
it for whoever works on it next. It is the canonical one: `CLAUDE.md` and the
`develop-live` skill lead Claude Code here rather than repeating it.
`.dockerignore` keeps all of them out of the image and the live instance.

## Checking it: `meridian plugin check`

Whatever in this file can be decided from the text is checked, not only
said: `meridian plugin check` holds the plugin to the rules every Meridian
plugin is built to (the template's
shape, `[tool.meridian]`, the kit linked and no raw colour, nothing from
another origin, settings declared rather than read from the environment, no
secret logged or put in a page, the deployment reached only through the SDK,
and tests). It needs no deployment and no session, and changes nothing. It
came in `meridian` 0.1.15; with an older one, the person runs
`meridian upgrade`.

- **Run `meridian plugin check` after each change.** Exit 0: every rule
  holds. Exit 1: at least one does not. Each failure names the rule, the file
  and line, and what to write instead: fix each one as it says, then run it
  again, until it exits 0. Never work around a rule; if one seems wrong for
  this plugin, tell the person.
- **`meridian plugin check --run-tests`** also runs the tests under `tests/`
  with pytest, in the plugin's `.venv` if it has one, otherwise with the
  `python3` on the PATH, which needs the plugin and pytest installed
  (`pip install -e . pytest`). `tests/test_page.py` asks the pages under
  Manage, Open and View with `meridian.testing.PageClient`, and the operation
  they send against a stand-in for the sidecar; replace its tests as you
  replace the pages, and add one for each page and each operation.
- The plugin's CI (`.github/workflows/check.yaml`) runs the same
  `meridian plugin check --run-tests` on every push.

What the check cannot decide stays advice, below: which component fits, the
page's layout, custom properties the kit does not define, figures as strings,
and names.

## Building its pages: the kit

Build every page with Open Meridian's plugin UI kit, and nothing else for its
look. Then it looks like the rest of the platform, follows each person's
colour scheme, light or dark, and their market-direction convention, and
needs no design work. The kit is plain CSS and web components: it works from
plain HTML, as the templates write it, and from React, Vue or Svelte alike.

**It is linked for you.** Each page's template, under `templates/`, is a
Jinja2 template that extends the SDK's base template,
`{% extends "meridian/base.html" %}`, which links the kit from the path the
dashboard serves it at on this plugin's own host and draws the page's heading
and tab row (the kit drops both when the dashboard frames the page and draws
its own). A template fills three blocks: `content`, the page itself;
`head_actions`, buttons marked `data-om-action="<id>"`, which the dashboard
draws in its header; and `status`, an `<om-status data-om-header>` it draws
beside the plugin's name. `pages.render("x.html", ...)` renders it, every
value escaped, with `caller` and `level` (admin, write or read) in it; a
template adapts by `{% if level == "write" %}`, and `{% include %}` and
macros share pieces between pages. A form that posts puts `{{ csrf_input }}`
inside it (a script sends `request.csrf_token` as `X-CSRF-Token`): every
request but GET and HEAD without this plugin's token is refused before the
view runs, so another site's page cannot act here as the person. A GET
changes nothing.

Never copy the kit into the plugin, and never load it, or anything else for
the page, from another origin or a CDN.

**Which component for what:**

| For | Use |
|---|---|
| A table of records | `<om-grid row-key="…">`: set `columns`, then `setRows(rows)`; `upsert(rows)` replaces rows by key in place; figures sort exactly |
| A stream: thousands of rows, many changes a second | `<om-grid high-rate>`: only the rows in view are drawn, only changed cells are touched, and they flash up or down; `freeze-sort` stops rows jumping while streaming |
| Live data from the plugin's server | `<om-live src="events" snapshot="snapshot.json" for="grid-id">`: follows server-sent events in sequence, and reads the snapshot again after a gap or a reconnect, so nothing is missed |
| The date the figures are as of | `<om-asof>` |
| Choosing an instrument | `<om-instrument-picker src="…" asof="…">`, searching through the plugin's own server |
| A time series | `<om-chart type="line">` (or `bar`), `series` set in script |
| Several views on one page, resizable and rearrangeable | `<om-panels layout-id="…">`, a `data-panel` child per view; each person's arrangement is remembered |
| Everything else | The kit's classes: `.page`, `.page-head`, `.panel` and `.panel-body`, `.tiles`, `.tabs`, `.field`, `.filters`, `.notice`, `.badge`, `button.primary` and `.danger`, `table` and `.num`, `.empty-state` |

Give a component its data as JSON inside it, `<script
type="application/json">{{ grid | tojson }}</script>`, which needs no script of the
page's; or set it (`columns`, `rows`, `series`) as properties, in a
`customElements.whenDefined(...)` callback. The kit's README
(open-meridian/meridian-ui) documents every component, attribute and event.

**Colour.** Only the kit's custom properties, never a hex, `rgb()`, `hsl()` or
a colour's name: not in a stylesheet, an inline style, an SVG or a script. A
person chooses their scheme and their deployment's administrator may add
schemes; a raw colour is one no scheme can change and no contrast check has
seen. Prefer a class to a property, and a property to anything else:

- surfaces `--page`, `--card`, `--hover`; text `--ink`, `--ink-soft`,
  `--ink-faint`; edges `--line`, `--line-soft`, `--line-strong`
- `--accent` and `--accent-wash`; `--primary` and `--primary-ink`
- status, which never changes meaning: `--good`, `--danger`, `--warn-ink`,
  `--violet`, each with its `-wash`
- **market direction:** `--buy` for a buy, a rise or a gain, and `--sell` for a
  sell, a fall or a loss, with `--buy-wash` and `--sell-wash`. Never green and
  red, and never `--good` and `--danger`, for a price move: where a person
  reads red as up (China, Japan, Korea, Taiwan) the kit swaps buy and sell for
  them. In markup, `.buy-ink` and `.sell-ink`, `.badge.buy` and `.badge.sell`;
  in a grid, a column's `tone: "sign"`.

Spacing, radii, shadows and type are the kit's too: `var(--space-1)` to
`var(--space-10)`, `var(--radius)`, `var(--shadow)`, `var(--sans)`,
`var(--mono)`.

**No chrome, no theme code.** The dashboard frames the page and draws the
plugin's name, its tab row, the way back to the dashboard and the person. The
page draws only its content, which the base template puts inside `<main
class="page">`: no header bar, navigation, logo, sign-in or sign-out, and no
light and dark switch. The kit applies the
person's theme, which the frame hands it, with no code of the page's.

**Money is exact.** Send prices and quantities to the page as decimal strings
and show them as sent; never `parseFloat` one. The grid sorts them exactly.

**The kit's rules, which a page keeps too:** no raw colour; no custom
property the kit does not define, other than the page's own layout ones; the
page reaches only its own origin, the plugin's server, which proxies anything
further; figures are strings, never floats.

**Where the kit is not served.** Until every dashboard serves `/.meridian/ui/`,
build pages that work without it, as the templates do: put a plain `<table>`
inside `<om-grid>` (a browser shows it as it is, and the kit's grid replaces
it), give components their data as JSON inside them, and make actions plain
forms. Without the kit the page is unstyled, but everything on it works.

A short page, whole: the view, in `page.py`,

```python
COLUMNS = [
    {"key": "symbol", "label": "Instrument"},
    {"key": "quantity", "label": "Quantity", "type": "decimal", "group": True},
    {"key": "market_value", "label": "Market value", "type": "decimal", "group": True},
    {"key": "day_pnl", "label": "Day P&L", "type": "decimal", "group": True, "tone": "sign"},
]


@pages.page("/positions", "Positions", levels=["write", "read"])
def positions(request: meridian.Request) -> str:
    return pages.render("positions.html", grid={"columns": COLUMNS, "rows": []})
```

and its template, `templates/positions.html`:

```html+jinja
{% extends "meridian/base.html" %}
{% block head_actions %}
<om-live src="events" snapshot="positions.json" for="positions"></om-live>
{% endblock %}
{% block content %}
<p class="muted">Every account you may read here.</p>
<section class="panel">
  <om-grid id="positions" row-key="position_id" high-rate sort="market_value:desc" caption="Positions">
    <script type="application/json">{{ grid | tojson }}</script>
    <table><tr><th>Instrument</th><th>Quantity</th><th>Market value</th></tr></table>
  </om-grid>
</section>
{% endblock %}
```

The plugin's server answers `positions.json` with `{ "sequence": "41", "rows":
[…] }` and `events` as server-sent events, each `id: <sequence>` and `data:
{ "rows": […] }` carrying whole records; `om-live` feeds them to the grid.

## Developing it live

On a Meridian deployment installed for development, `meridian plugin dev`
runs this plugin as you write it: each saved file is sent, the plugin's process
restarts on it in the same pod, and it is running again in about a second. The
pod, its sidecar and what it is allowed to do stay as they were.

Everything here runs on the person's own session with the deployment, and is
theirs.

### Before starting

- `meridian --version` is 0.1.21 or later. Older ones have no `--level` on
  `plugin open`, before 0.1.15 no `plugin check`, and before 0.1.3 no
  `plugin dev`: the person runs `meridian upgrade`.
- The person has run `meridian connect` (with the address, for a deployment
  not on this machine). You cannot do it for them: it signs in through their
  browser. `meridian plugin list` says whether the
  session is there: it lists the catalogue, or exits **3**. Whenever any
  command exits 3, the session is missing or has lapsed. Stop, tell the person
  to run the `meridian connect` the command printed, and carry on once they
  have. There is no `meridian status`.
- The deployment was installed for development (`meridian up --development`).
  Elsewhere `plugin dev` is refused, and nothing is wrong with the plugin.

### Start the loop

The instance is called `reference-plugin` below. Use whatever the person
wants it called, and the same name in every command.

**The first time an instance is launched, the person approves what it asks
for.** Show them the `roles` in `pyproject.toml`'s `[tool.meridian]`
and ask. Only once they say yes, pass `--yes`; never pass it to get past a
question they have not answered. An instance that is live already asks
nothing.

Run it in the background, and keep it running for the whole session:

```sh
mkdir -p .meridian
meridian plugin dev --instance reference-plugin --yes --json > .meridian/dev.jsonl 2> .meridian/dev.err
```

Its output goes under `.meridian/` on purpose. `plugin dev` sends every file
in this directory that changes, except `.meridian/` and what `.dockerignore`
names. Anywhere else in this directory, its own output would be sent to the
plugin as a change, again and again. Not under your own agent's directory
(`.claude/`, `.cursor/` and the like) either: some agents guard theirs, and
writing there needs a permission a session may not have. `.meridian/` is
git-ignored, since it is this session's, not the plugin's.

`.meridian/dev.jsonl` gets one JSON object a line:

| `event` | Means |
|---|---|
| `seeded` | The live folder was filled from the plugin's image, the first time it runs live |
| `sent` | This process sent a change; `revision` is the number it was given |
| `synced` | The sidecar wrote it |
| `restarted` | The plugin's process started on that revision |
| `ready` | It connected to its sidecar again: that revision is running |
| `crashed` | It stopped with an error. `traceback` has the last of what it printed |
| `exited` | It stopped by itself, without an error |
| `refused` | The sidecar refused it something; `reason` says what |

Wait for the first `ready` before changing anything. `.meridian/dev.err` says
what it is doing, and why it stopped if it did. The first run builds and
uploads the plugin's image, which takes a minute or two.

### Change something

1. Edit and save. There is nothing to run: the save is the deploy.
2. Find the `sent` line after your save, and its `revision`, R.
3. Wait for `ready` or `crashed` at R. Nothing about R is known before then.
4. On `crashed`, read its `traceback`, fix the cause, and save again: the next
   revision replaces it. Nothing needs restarting.

Several saves close together can land as one revision. Read the newest `sent`.

### Check it

Ask the question that answers what you changed:

| To know | Run |
|---|---|
| What a page shows, as the person is served it | `meridian plugin open --instance reference-plugin --level open --print /` for Accounts under Open (`--level view` under View), and `--level manage --print /setup` for Setup under Manage: any path on the plugin, at a level it is declared with |
| What the plugin printed or logged since your change | `meridian plugin logs --instance reference-plugin --since <R-1>` |
| What the sidecar refused it, or what else happened | `meridian plugin events --instance reference-plugin --since <R-1> --json` |
| What the person sees in a browser | `meridian plugin open --instance reference-plugin --level open`: a link one browser opens once, at that level. Give it to the person, or open it in your browser pane |

`--level` is the home's button the session is opened by: `manage`, `open` or
`view`. Name it every time. A page answers 403 at a level it is not declared
with, and without `--level` the dashboard opens at the first level the person
holds, Manage before Open before View, so on a development deployment, where
they hold `admin`, `/` would be asked under Manage and refused.

`--print` exits non-zero when the plugin answers with an error, and prints what
it answered. Prefer it to a browser for checking your own work: it needs no
browser and gives the same page every time.

### What a save cannot change

- **What it is allowed to do.** A `refused` event is its grants working, not a
  bug to code around. Adding a role to `pyproject.toml` changes nothing
  live: it takes a new version, and a person approves it.
- **Its dependencies.** The live code runs on the image the instance was
  launched from. A new package in `pyproject.toml` needs a new version.

Tell the person when a change needs either, rather than looking for a way
round it.

### Release it

When the person is satisfied:

1. Run `meridian plugin check --run-tests`, and fix what fails until it
   exits 0.
2. Raise `version` in `pyproject.toml`. A version is never replaced.
3. Show the person the roles again, and get their yes.
4. Run `meridian plugin dev --release --instance reference-plugin --yes`.

That uploads this directory as that version and runs it in place of the live
instance. It is then an ordinary version in the deployment's catalogue.

### Stop

Stop the background `plugin dev`. The instance keeps running, live, as it was.
`meridian plugin stop reference-plugin` ends it, which is the person's call.

# reference-plugin

A Meridian plugin, in Python on the open-meridian SDK. It runs beside a
sidecar in a Meridian deployment and reaches nothing else: what it may publish
and subscribe to comes from the roles `pyproject.toml` declares under
`[tool.meridian]`, once a deployment admin approves them.

- `src/reference_plugin/__main__.py` connects to the sidecar and serves the
  page; `src/reference_plugin/page.py` is the page.
- A save changes what the plugin does, never what it is allowed to do. Roles,
  tags and dependencies take a new version, which a person approves.

This file is for any coding agent working on the plugin, and is committed with
it for whoever works on it next. `CLAUDE.md` and the `develop-live` skill lead
Claude Code here rather than repeating it. `.dockerignore` keeps all of them
out of the image and the live instance.

## Developing it live

On a Meridian deployment installed for development, `meridian plugin dev`
runs this plugin as you write it: each saved file is sent, the plugin's process
restarts on it in the same pod, and it is running again in about a second. The
pod, its sidecar and what it is allowed to do stay as they were.

Everything here runs on the person's own session with the deployment, and is
theirs.

### Before starting

- `meridian --version` is 0.1.3 or later. Older ones have no `plugin dev`: the
  person runs `meridian upgrade`.
- The person has run `meridian connect <address>`. You cannot do it for them:
  it signs in through their browser. `meridian plugin list` says whether the
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
for.** Show them the `roles` and `tags` in `pyproject.toml`'s `[tool.meridian]`
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
| What the page shows, as the person is served it | `meridian plugin open --instance reference-plugin --print /` (any path on the plugin) |
| What the plugin printed or logged since your change | `meridian plugin logs --instance reference-plugin --since <R-1>` |
| What the sidecar refused it, or what else happened | `meridian plugin events --instance reference-plugin --since <R-1> --json` |
| What the person sees in a browser | `meridian plugin open --instance reference-plugin`: a link one browser opens once. Give it to the person, or open it in your browser pane |

`--print` exits non-zero when the plugin answers with an error, and prints what
it answered. Prefer it to a browser for checking your own work: it needs no
browser and gives the same page every time.

### What a save cannot change

- **What it is allowed to do.** A `refused` event is its grants working, not a
  bug to code around. Adding a role or tag to `pyproject.toml` changes nothing
  live: it takes a new version, and a person approves it.
- **Its dependencies.** The live code runs on the image the instance was
  launched from. A new package in `pyproject.toml` needs a new version.

Tell the person when a change needs either, rather than looking for a way
round it.

### Release it

When the person is satisfied:

1. Run the plugin's own tests, if it has any, and fix what fails.
2. Raise `version` in `pyproject.toml`. A version is never replaced.
3. Show the person the roles and tags again, and get their yes.
4. Run `meridian plugin dev --release --instance reference-plugin --yes`.

That uploads this directory as that version and runs it in place of the live
instance. It is then an ordinary version in the deployment's catalogue.

### Stop

Stop the background `plugin dev`. The instance keeps running, live, as it was.
`meridian plugin stop reference-plugin` ends it, which is the person's call.

# reference-plugin

A Meridian `pricing` plugin, made by `meridian plugin new --role pricing`: it
answers the analytics component's batches for the pricing model ids a
deployment admin routes to it -- a value per unit for every instrument and
scenario a batch names, from exactly the market objects the batch carries.

Its whole work is the SDK's (`meridian.roles.pricing.Pricer`): each batch
answered whole, under each unit's scenario, with no sensitivity until a model
declares one (from v28); and after a restart the batches still waiting, read
from the last sequence it answered, kept in the storage the deployment grants
it. The plugin keeps what is its own: its models (`model.py`). This one
implements `quoted_price` with the built-in model's own computation, so its
answers equal the built-in implementation's exactly; replace it with yours.

It is never called by a client. Which instruments it prices is the
deployment admin's routing (the dashboard's model routing page), and
stopping it leaves a figure a compliance rule reads not evaluated, never a
silent fallback.

Put it in a deployment that `meridian connect` signed you in to:

    meridian plugin upload
    meridian plugin launch reference-plugin 0.1.0 --instance reference-plugin

Hold it to the rules every Meridian plugin is built to, and run its tests:

    pip install -e . pytest
    meridian plugin check --run-tests

Its tests run the `pricing` suite the SDK carries (`tests/test_suite.py`): a
batch answered, every scenario of a batch answered, the equity template's
test case equal to the built-in model's, no sensitivity carried, and the
batches waiting read after a restart.

Its page: Batches under Manage (`admin`), the model ids it implements, the
batches answered and any left unanswered with why; a read tool,
`read_batches`. Counts and codes only.

`AGENTS.md` teaches any coding agent to build pages with the kit, and the same
live loop; `CLAUDE.md` and the `develop-live` skill under `.claude/` lead
Claude Code to it. Commit them with the plugin, so whoever works on it next
has them too; `.dockerignore` keeps them out of its image.

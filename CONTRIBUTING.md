# Contributing

Open Meridian is open source, and built by its team: we don't accept pull
requests from outside it. The best way to help is an issue.

- **Found a bug?** Open an issue with **Report a bug**: what you did, what you
  expected, and what happened instead.
- **Have an idea?** Open an issue with **Suggest an improvement**: the problem
  you have, and what would solve it for you.

Describe rather than paste code. We build every change ourselves, from your
description.

## Your suggestions

By opening an issue or making a suggestion, you give Societal Lab Inc. a
perpetual, irrevocable, worldwide, royalty-free licence to use it for any
purpose, with no obligation to you. That lets us act on what you tell us
without any question of who owns the result.

## Building on Open Meridian

Plugins are how you extend Open Meridian, and they live in your own
repositories, not ours: the Python SDK and the plugin contract are Apache-2.0,
so a plugin you write stays yours. Start with `meridian plugin new`; see
[open-meridian.dev](https://open-meridian.dev).

## For the team

`make install-hooks` makes every push run `make pre-push`, the fast gate:
the advisories, and the stages the changed paths select (the Makefile's
`PRE_PUSH_MAP`). A change goes on a branch and lands through one pull request
per version, its summary the change log. CI on the pull request runs every
gate, and it merges when green, up to date with `main`, as a merge commit:
that keeps the branch's commit SHAs, which other repositories pin. Reproduce
a failing gate with `make ci-stage STAGE=<gate>` before pushing again;
`make ci-local` runs every gate, for when CI is down.

# meridian-cli

`meridian`, the command line that installs, upgrades and checks a deployment and
scaffolds and checks plugins. README.md says what it does; CONTRIBUTING.md how the
team works on it. `make pre-push` is the fast gate; CI runs `make ci-local`.

## Code structure

Standing rules (2026-11-20), each a check in `make check-structure`, which runs in
the pre-push gate and in CI and is red on a planted violation
(`make check-structure-selftest`). CONTRIBUTING.md states each in full.

- **S3 generated is generated:** `make check-vendored-template` (the scaffolds and
  sdk-roles are meridian-python's at SDK_REV; change them with
  `make vendor-template`, never by hand) and `make check-interims` (the register).
- **S4 interims name their end:** every stop-gap carries `INTERIM(vNN): reason`;
  `make check-interims` fails on one due at `roadmap_version`
  (tools/code-structure.toml). Moving a marker's version needs the product owner's
  word and a `Baseline-Change:` trailer naming the marker; `make check-baselines`
  fails a move without one.
- **S5 pins are immutable:** `make check-pins`. Another repository by a release or
  a full SHA, never a branch or a sibling checkout; an exception goes in
  tools/pins-allowlist.txt with its marker.
- **S9 no cluster tool it doesn't need:** `make check-shellouts`. The count of
  `"kubectl"` and `"helm"` calls in src/ (tests aside) may not grow; prefer the
  deployment's API. Each call left has a reason or an INTERIM marker.
- **S10 licences by layer:** `make check-licences`. The scaffolds and sdk-roles
  ship in plugins under Apache-2.0: no GPL or AGPL text in them.
- **S2 file size:** `make check-size` reports a hand-written file over 1,500 lines
  (a warning in v31a, failing from v31b; growth then needs a `Size-Exception:`
  trailer).
- A baseline or allowlist changes only in a commit with a `Baseline-Change:`
  trailer saying why (`make check-baselines`); baselines only go down.

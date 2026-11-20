#!/usr/bin/env python3
"""The code-structure rules' checks (standing rules adopted 2026-11-20; each
repository's CONTRIBUTING.md states them in full). One copy, the same byte for
byte in every repository that runs it; standard library only.

    python3 tools/code_structure.py interims [--write]
    python3 tools/code_structure.py pins
    python3 tools/code_structure.py shellouts
    python3 tools/code_structure.py size
    python3 tools/code_structure.py licences
    python3 tools/code_structure.py baselines --base <ref>
    python3 tools/code_structure.py --self-test

Each reads its settings from tools/code-structure.toml:

    roadmap_version   the version this repository is building (S4), e.g. "v31a"
    interims_register the register S4 writes from the markers
    pins_allowlist    S5's listed exceptions, each with its INTERIM marker
    shellouts         S9: the source globs counted, and the baseline file
    size_baseline     S2: the files over the limit today, with their lines
    size_exempt       S2: generated and vendored globs
    licence_scan      S10: the globs that ship under Apache-2.0
    notice            S10: true where NOTICE must ship beside LICENSE

S4  INTERIM(vNN): reason. A marker is `INTERIM(` then a version (v31a, v32),
    `):` and a reason. A marker whose version is the roadmap version or
    earlier fails: its stop-gap was due. The register is generated from the
    markers and a stale one fails, as a generated file does (S3).
S5  A dependency on another repository is an immutable reference: a released
    version or a full commit SHA, never a branch or a sibling checkout. Read
    in Cargo.toml (git rev, no branch or tag, no path outside the repo),
    pyproject.toml (git+ URLs at a full SHA) and .github/workflows (`uses:`
    at a version or a full SHA; a checkout of another repository at a full
    SHA or a version). An exception is listed with its INTERIM marker.
S9  The CLI's shelled-out kubectl and helm calls, counted in its source; the
    count may not grow, and a lower one lowers the baseline.
S2  A hand-written file over 1,500 lines: a new one, or a listed one grown
    past its baseline, is reported (a warning in v31a; failing from v31b,
    where growth needs a `Size-Exception:` trailer).
S10 What ships under Apache-2.0 (licence_scan) carries no GPL or AGPL
    licence text; where notice is set, NOTICE ships beside LICENSE.
    Baselines and allowlists change only in a commit with a
    `Baseline-Change:` trailer saying why, and a ratchet's baseline only goes
    down.

Exit: 0 holds, 1 broken (each finding said), 2 asked wrongly.
"""

from __future__ import annotations

import fnmatch
import re
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

LIMIT = 1500
MARKER = re.compile(r"INTERIM\((v\d+[a-z]?)\)(:\s*\S.*)?")
SHA = re.compile(r"^[0-9a-f]{40}$")
VERSION = re.compile(r"^v?\d+(\.\d+)*$")
TRAILER = "Baseline-Change:"


def version_key(version: str) -> tuple[int, str]:
    m = re.fullmatch(r"v(\d+)([a-z]?)", version)
    if not m:
        raise ValueError(f"{version!r} is not a roadmap version (v31a, v32)")
    return int(m.group(1)), m.group(2)


def tracked(root: Path) -> list[str]:
    out = subprocess.run(["git", "-C", str(root), "ls-files"], capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(f"code-structure: git ls-files: {out.stderr.strip()}")
    return [line for line in out.stdout.splitlines() if (root / line).is_file()]


def matches(path: str, globs: list[str]) -> bool:
    return any(fnmatch.fnmatch(path, glob) for glob in globs)


def text_of(root: Path, path: str) -> str | None:
    try:
        return (root / path).read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        return None


# ── S4 ──────────────────────────────────────────────────────────────────────


def markers(root: Path, files: list[str], skip: list[str]) -> tuple[list[tuple[str, str, str]], list[str]]:
    """Each marker as (version, place, reason), and each malformed one said."""
    found, malformed = [], []
    for path in files:
        if matches(path, skip):
            continue
        text = text_of(root, path)
        if text is None or "INTERIM(v" not in text:
            continue
        for n, line in enumerate(text.splitlines(), 1):
            for m in MARKER.finditer(line):
                if not m.group(2):
                    malformed.append(f"S4: {path}:{n}: INTERIM({m.group(1)}) names no reason; "
                                     f"write `INTERIM({m.group(1)}): <why, and what ends it>`")
                    continue
                reason = m.group(2)[1:].strip().rstrip("*/#-> ").strip()
                found.append((m.group(1), f"{path}:{n}", reason))
    return found, malformed


def register(found: list[tuple[str, str, str]], version: str) -> str:
    lines = [f"# Interims (S4), generated from the INTERIM(vNN) markers by "
             f"`tools/code_structure.py interims --write`. Roadmap version: {version}.",
             "# version\tplace\treason"]
    for v, place, reason in sorted(found, key=lambda f: (version_key(f[0]), f[1])):
        lines.append(f"{v}\t{place}\t{reason}")
    return "\n".join(lines) + "\n"


def check_interims(root: Path, cfg: dict, write: bool) -> list[str]:
    version = cfg["roadmap_version"]
    version_key(version)
    out = root / cfg["interims_register"]
    found, failures = markers(root, tracked(root), [cfg["interims_register"], "tools/code_structure.py"])
    for v, place, reason in found:
        if version_key(v) <= version_key(version):
            failures.append(f"S4: {place}: INTERIM({v}) is due: the roadmap version is {version}. "
                            "End the stop-gap, or move its version with the product owner's word "
                            "and a trailer saying so")
    wanted = register(found, version)
    if write:
        out.write_text(wanted, encoding="utf-8")
    elif text_of(root, cfg["interims_register"]) != wanted:
        failures.append(f"S4: {cfg['interims_register']} is not the markers' register; run "
                        "`python3 tools/code_structure.py interims --write`")
    return failures


# ── S5 ──────────────────────────────────────────────────────────────────────


def cargo_pins(path: str, text: str) -> list[str]:
    failures = []
    data = tomllib.loads(text)
    tables = []
    for key in ("dependencies", "dev-dependencies", "build-dependencies"):
        tables.append((key, data.get(key, {})))
        tables.append((f"workspace.{key}", data.get("workspace", {}).get(key, {})))
    for target, spec in data.get("target", {}).items():
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            tables.append((f"target.{target}.{key}", spec.get(key, {})))
    for where, table in tables:
        for name, dep in table.items():
            if not isinstance(dep, dict):
                continue
            if "git" in dep:
                if "branch" in dep or "tag" in dep or not SHA.match(str(dep.get("rev", ""))):
                    failures.append(f"S5: {path} [{where}] {name}: a git dependency at "
                                    f"{dep.get('branch') or dep.get('tag') or dep.get('rev') or 'its default branch'}; "
                                    "pin `rev` to a full commit SHA")
            if "path" in dep and str(dep["path"]).startswith(".."):
                failures.append(f"S5: {path} [{where}] {name}: a sibling checkout ({dep['path']}); "
                                "depend on a release or a full commit SHA")
    return failures


def pyproject_pins(path: str, text: str) -> list[str]:
    failures = []
    data = tomllib.loads(text)
    project = data.get("project", {})
    deps = list(project.get("dependencies", []))
    for extra in project.get("optional-dependencies", {}).values():
        deps += extra
    for group in data.get("dependency-groups", {}).values():
        deps += [d for d in group if isinstance(d, str)]
    for dep in deps:
        if "git+" in dep:
            ref = dep.rsplit("@", 1)[-1] if dep.count("@") >= 2 else ""
            if not SHA.match(ref.split("#")[0]):
                failures.append(f"S5: {path}: {dep}: a git dependency not at a full commit SHA")
        if "file:" in dep or re.search(r"@\s*\.\.?/", dep):
            failures.append(f"S5: {path}: {dep}: a sibling checkout")
    return failures


def workflow_pins(path: str, text: str, own: str) -> list[str]:
    failures = []
    steps = re.split(r"\n(?=\s*- )", text)
    for step in steps:
        for m in re.finditer(r"uses:\s*([^\s#'\"]+)", step):
            used = m.group(1)
            if used.startswith(("./", "docker://")):
                continue
            ref = used.rsplit("@", 1)[-1] if "@" in used else ""
            if not (SHA.match(ref) or VERSION.match(ref)):
                failures.append(f"S5: {path}: uses {used}: not a released version or a full SHA")
        repo = re.search(r"\brepository:\s*['\"]?([\w.-]+/[\w.-]+)", step)
        if repo and repo.group(1) != own:
            ref = re.search(r"\bref:\s*['\"]?([^\s'\"#]+)", step)
            value = ref.group(1) if ref else "its default branch"
            if not (SHA.match(value) or VERSION.match(value)):
                failures.append(f"S5: {path}: checks out {repo.group(1)} at {value}: not a "
                                "released version or a full SHA")
    return failures


def check_pins(root: Path, cfg: dict) -> list[str]:
    failures = []
    own = cfg["repository"]
    for path in tracked(root):
        name = Path(path).name
        text = text_of(root, path)
        if text is None:
            continue
        if name == "Cargo.toml":
            failures += cargo_pins(path, text)
        elif name == "pyproject.toml" and not matches(path, cfg.get("pins_skip", [])):
            failures += pyproject_pins(path, text)
        elif path.startswith(".github/workflows/") and name.endswith((".yml", ".yaml")):
            failures += workflow_pins(path, text, own)
    listed = cfg.get("pins_allowlist")
    allowed: list[str] = []
    if listed and (root / listed).exists():
        for n, line in enumerate((root / listed).read_text(encoding="utf-8").splitlines(), 1):
            if not line.strip() or line.startswith("#"):
                continue
            entry, _, why = line.partition(" # ")
            if not MARKER.search(why) or not MARKER.search(why).group(2):
                failures.append(f"S5: {listed}:{n}: an exception with no `INTERIM(vNN): reason`")
            allowed.append(entry.strip())
    return [f for f in failures if not any(a and a in f for a in allowed)]


# ── S9 ──────────────────────────────────────────────────────────────────────

SHELL_OUT = re.compile(r"\"(kubectl|helm)\"")


def shellout_count(root: Path, cfg: dict) -> int:
    spec = cfg["shellouts"]
    count = 0
    for path in tracked(root):
        if matches(path, spec["count"]) and not matches(path, spec.get("skip", [])):
            count += len(SHELL_OUT.findall(text_of(root, path) or ""))
    return count


def check_shellouts(root: Path, cfg: dict) -> list[str]:
    baseline = int((root / cfg["shellouts"]["baseline"]).read_text().split()[0])
    count = shellout_count(root, cfg)
    if count > baseline:
        return [f"S9: {count} shelled-out kubectl and helm calls, above the baseline of {baseline} "
                f"({cfg['shellouts']['baseline']}): call the deployment's API instead"]
    if count < baseline:
        return [f"S9: {count} shelled-out kubectl and helm calls, below the baseline of {baseline}: "
                f"lower {cfg['shellouts']['baseline']} to {count}, with a `{TRAILER}` trailer"]
    return []


# ── S2 ──────────────────────────────────────────────────────────────────────


def sizes(root: Path, cfg: dict) -> dict[str, int]:
    out = {}
    for path in tracked(root):
        if matches(path, cfg.get("size_exempt", [])):
            continue
        text = text_of(root, path)
        if text is not None:
            lines = text.count("\n")
            if lines > LIMIT:
                out[path] = lines
    return out


def read_size_baseline(root: Path, cfg: dict) -> dict[str, int]:
    held = {}
    for line in (root / cfg["size_baseline"]).read_text(encoding="utf-8").splitlines():
        if line.strip() and not line.startswith("#"):
            path, lines = line.split("\t")
            held[path] = int(lines)
    return held


def check_size(root: Path, cfg: dict) -> list[str]:
    held = read_size_baseline(root, cfg)
    warnings = []
    for path, lines in sorted(sizes(root, cfg).items()):
        if path not in held:
            warnings.append(f"S2: {path} has {lines} lines, over {LIMIT}, and is not in "
                            f"{cfg['size_baseline']}: split it")
        elif lines > held[path]:
            warnings.append(f"S2: {path} grew from {held[path]} to {lines} lines, over {LIMIT}; "
                            "from v31b this needs a `Size-Exception:` trailer")
    return warnings


# ── S10 ─────────────────────────────────────────────────────────────────────

COPYLEFT = re.compile(r"GNU (Affero )?General Public License|SPDX-License-Identifier:\s*\(?\s*A?GPL")


def check_licences(root: Path, cfg: dict) -> list[str]:
    """What licence_scan names ships under Apache-2.0, whatever this
    repository's own licence: no GPL or AGPL text in it. With notice set,
    NOTICE ships beside LICENSE."""
    failures = []
    files = tracked(root)
    if cfg.get("notice") and "NOTICE" not in files:
        failures.append("S10: no NOTICE beside LICENSE")
    for path in files:
        if path == "LICENSE" or not matches(path, cfg.get("licence_scan", [])):
            continue
        text = text_of(root, path)
        if text and COPYLEFT.search(text):
            failures.append(f"S10: {path} carries a GPL or AGPL licence, and it ships under "
                            "Apache-2.0")
    return failures


# ── Baselines change in the open ────────────────────────────────────────────


def git(root: Path, *args: str) -> str:
    out = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)
    return out.stdout if out.returncode == 0 else ""


def check_baselines(root: Path, cfg: dict, base: str) -> list[str]:
    files = [cfg["size_baseline"], cfg["interims_register"]]
    for key in ("pins_allowlist",):
        if cfg.get(key):
            files.append(cfg[key])
    if cfg.get("shellouts"):
        files.append(cfg["shellouts"]["baseline"])
    if not git(root, "rev-parse", "--verify", "--quiet", f"{base}^{{commit}}"):
        return [f"the base {base} is not a commit here, so what changed cannot be read: fetch it"]
    changed = set(git(root, "diff", "--name-only", f"{base}...HEAD").split())
    watched = [f for f in files if f in changed and f != cfg["interims_register"]]
    failures = []
    if watched and TRAILER not in git(root, "log", "--format=%B", f"{base}..HEAD"):
        failures.append(f"{', '.join(watched)} changed with no `{TRAILER}` trailer saying why")
    if cfg.get("shellouts") and cfg["shellouts"]["baseline"] in watched:
        before = git(root, "show", f"{base}:{cfg['shellouts']['baseline']}").split()
        after = (root / cfg["shellouts"]["baseline"]).read_text().split()
        if before and after and int(after[0]) > int(before[0]):
            failures.append(f"S9: the baseline rose from {before[0]} to {after[0]}; it only goes down")
    # A baseline new on this branch starts at today's figures: nothing to rise from.
    was = git(root, "show", f"{base}:{cfg['size_baseline']}")
    if cfg["size_baseline"] in watched and was:
        old = {l.split("\t")[0]: int(l.split("\t")[1]) for l in was.splitlines()
               if l.strip() and not l.startswith("#")}
        for path, lines in read_size_baseline(root, cfg).items():
            if lines > old.get(path, 0):
                failures.append(f"S2: {cfg['size_baseline']} raises {path} to {lines}; it only goes down")
    return failures


# ── self-test: each check red on a planted violation ────────────────────────


def self_test() -> int:
    cases, failed = 0, []

    def expect(name: str, cond: bool) -> None:
        nonlocal cases
        cases += 1
        if not cond:
            failed.append(name)

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        cfg = {"repository": "open-meridian/x", "roadmap_version": "v31a",
               "interims_register": "tools/interims.tsv", "size_baseline": "tools/size.tsv",
               "pins_allowlist": "tools/pins.txt", "notice": True,
               "licence_scan": ["src/*"], "size_exempt": ["gen/*"],
               "shellouts": {"count": ["src/*.rs"], "skip": ["src/*tests.rs"],
                             "baseline": "tools/shellouts.txt"}}

        def write(path: str, text: str) -> None:
            (root / path).parent.mkdir(parents=True, exist_ok=True)
            (root / path).write_text(text, encoding="utf-8")
            subprocess.run(["git", "-C", str(root), "add", path], check=True)

        write("src/a.rs", 'run("kubectl", &[]);\n// INTERIM(v33): the conductor\'s API, then gone\n')
        write("src/a_tests.rs", 'run("helm", &[]);\n')
        write("tools/shellouts.txt", "1\n")
        write("tools/size.tsv", "# path\tlines\nsrc/big.py\t1600\n")
        write("src/big.py", "x\n" * 1600)
        write("tools/pins.txt", "")
        write("NOTICE", "n\n")
        write("Cargo.toml", '[dependencies]\nc = { git = "https://x/c", rev = "' + "a" * 40 + '" }\n')
        write(".github/workflows/ci.yml", "steps:\n  - uses: actions/checkout@v4\n")
        write("tools/interims.tsv", "")
        check_interims(root, cfg, write=True)
        subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)

        expect("S4 holds on a later marker", check_interims(root, cfg, False) == [])
        expect("S9 holds at its baseline, tests not counted", check_shellouts(root, cfg) == [])
        expect("S5 holds on a SHA and a version", check_pins(root, cfg) == [])
        expect("S2 is quiet at its baseline", check_size(root, cfg) == [])
        expect("S10 holds", check_licences(root, cfg) == [])

        write("src/b.rs", "// INTERIM(v31a): due now\n// INTERIM(v40)\n")
        found = check_interims(root, cfg, False)
        expect("S4: a due marker fails", any("INTERIM(v31a) is due" in f for f in found))
        expect("S4: a marker with no reason fails", any("names no reason" in f for f in found))
        expect("S4: a stale register fails", any("is not the markers' register" in f for f in found))
        (root / "src/b.rs").unlink()
        subprocess.run(["git", "-C", str(root), "rm", "-q", "--cached", "src/b.rs"], check=True)

        write("src/c.rs", 'run("helm", &["list"]);\n')
        expect("S9: one more call fails", any("above the baseline" in f for f in check_shellouts(root, cfg)))
        write("src/c.rs", "")
        write("src/a.rs", "// INTERIM(v33): the conductor's API, then gone\n")
        expect("S9: one fewer asks to lower the baseline",
               any("below the baseline" in f for f in check_shellouts(root, cfg)))
        write("src/a.rs", 'run("kubectl", &[]);\n// INTERIM(v33): the conductor\'s API, then gone\n')

        write("Cargo.toml", '[dependencies]\nc = { git = "https://x/c", branch = "main" }\n'
                            'd = { path = "../d" }\n')
        write(".github/workflows/ci.yml", "steps:\n  - uses: o/a@main\n  - uses: actions/checkout@v4\n"
                                          "    with:\n      repository: o/b\n      ref: main\n")
        write("pyproject.toml", '[project]\ndependencies = ["m @ git+https://x/m@main"]\n')
        found = check_pins(root, cfg)
        expect("S5: a git branch fails", any("[dependencies] c" in f for f in found))
        expect("S5: a sibling checkout fails", any("sibling checkout" in f and "../d" in f for f in found))
        expect("S5: an action at a branch fails", any("uses o/a@main" in f for f in found))
        expect("S5: another repository checked out at a branch fails",
               any("checks out o/b at main" in f for f in found))
        expect("S5: a pyproject git dependency at a branch fails", any("pyproject.toml" in f for f in found))
        write("tools/pins.txt", "uses o/a@main # INTERIM(v33): o/a has no release yet\n"
                                "checks out o/b # no marker\n")
        found = check_pins(root, cfg)
        expect("S5: a listed exception with its marker is allowed",
               not any("uses o/a@main" in f for f in found))
        expect("S5: a listed exception with no marker fails", any("no `INTERIM" in f for f in found))
        write("Cargo.toml", "")
        write("pyproject.toml", "")
        write(".github/workflows/ci.yml", "")

        write("src/big.py", "x\n" * 1700)
        write("src/new.py", "x\n" * 1501)
        found = check_size(root, cfg)
        expect("S2: a listed file that grew is reported", any("grew from 1600 to 1700" in f for f in found))
        expect("S2: a new file over the limit is reported", any("src/new.py" in f for f in found))
        write("gen/huge.py", "x\n" * 5000)
        expect("S2: a generated file is exempt", not any("gen/huge.py" in f for f in check_size(root, cfg)))

        write("src/copied.py", "# SPDX-License-Identifier: AGPL-3.0-or-later\n")
        expect("S10: an AGPL file in an Apache repository fails",
               any("src/copied.py" in f for f in check_licences(root, cfg)))
        (root / "NOTICE").unlink()
        subprocess.run(["git", "-C", str(root), "rm", "-q", "--cached", "NOTICE"], check=True)
        expect("S10: a missing NOTICE fails", any("NOTICE" in f for f in check_licences(root, cfg)))

        env = ["-c", "user.name=t", "-c", "user.email=t@t"]
        write("tools/shellouts.txt", "1\n")
        subprocess.run(["git", *env, "-C", str(root), "commit", "-qm", "base"], check=True)
        write("tools/more.tsv", "")
        subprocess.run(["git", *env, "-C", str(root), "commit", "-qm", "empty"], check=True)
        cfg_new = dict(cfg, size_baseline="tools/new-size.tsv")
        write("tools/new-size.tsv", "src/big.py\t1700\n")
        subprocess.run(["git", *env, "-C", str(root), "commit", "-qm",
                        "start\n\nBaseline-Change: today's figures"], check=True)
        expect("a baseline new on the branch starts at today's figures",
               check_baselines(root, cfg_new, "HEAD~1") == [])
        write("tools/new-size.tsv", "src/big.py\t1800\n")
        subprocess.run(["git", *env, "-C", str(root), "commit", "-qm",
                        "grow\n\nBaseline-Change: grew"], check=True)
        expect("S2: a baseline entry may not rise, trailer or not",
               any("raises src/big.py to 1800" in f for f in check_baselines(root, cfg_new, "HEAD~1")))
        write("tools/shellouts.txt", "2\n")
        subprocess.run(["git", *env, "-C", str(root), "commit", "-qm", "raise it"], check=True)
        found = check_baselines(root, cfg, "HEAD~1")
        expect("a baseline changed with no trailer fails", any("no `Baseline-Change:`" in f for f in found))
        expect("a ratchet's baseline may not rise", any("only goes down" in f for f in found))
        write("tools/shellouts.txt", "0\n")
        subprocess.run(["git", *env, "-C", str(root), "commit", "-qm",
                        "lower it\n\nBaseline-Change: two calls moved to the API"], check=True)
        expect("a lowered baseline with its trailer holds", check_baselines(root, cfg, "HEAD~1") == [])
        expect("a base that is not there fails rather than passing unread",
               any("not a commit here" in f for f in check_baselines(root, cfg, "origin/nowhere")))

    for name in failed:
        print(f"FAIL  {name}", file=sys.stderr)
    if failed:
        print(f"code-structure self-test FAILED: {len(failed)} of {cases}", file=sys.stderr)
        return 1
    print(f"code-structure self-test OK: {cases} cases, each check red on its planted violation")
    return 0


def main(argv: list[str]) -> int:
    if argv[:1] == ["--self-test"]:
        return self_test()
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    root = Path(__file__).resolve().parent.parent
    cfg = tomllib.loads((root / "tools" / "code-structure.toml").read_text(encoding="utf-8"))
    command = argv[0]
    if command == "interims":
        found = check_interims(root, cfg, write="--write" in argv)
    elif command == "pins":
        found = check_pins(root, cfg)
    elif command == "shellouts":
        found = check_shellouts(root, cfg)
    elif command == "size":
        for warning in check_size(root, cfg):
            print(f"warning: {warning}", file=sys.stderr)
        print("check-size: reported (a warning until v31b)")
        return 0
    elif command == "licences":
        found = check_licences(root, cfg)
    elif command == "baselines":
        if "--base" not in argv:
            return 2
        found = check_baselines(root, cfg, argv[argv.index("--base") + 1])
    else:
        print(f"code-structure: no check {command!r}", file=sys.stderr)
        return 2
    for finding in found:
        print(finding, file=sys.stderr)
    if found:
        print(f"check-{command} FAILED: {len(found)} finding(s)", file=sys.stderr)
        return 1
    print(f"check-{command} OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

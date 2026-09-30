#!/bin/sh
# `meridian plugin migrate`, the real binary, over the plugins meridian-python
# records its migrations for (its tests/migrations), in an SDK image built
# from that checkout: `make e2e-migrate`. Each plugin is a git repository of
# its own here, migrated with --run-tests, and must come out as the tree
# meridian-python expects, with its check passing and exactly the places it
# expects left by hand.
#
# Runs inside the e2e-migrate stage of Dockerfile.rust, with the host's docker
# socket: the migrations run in an image the daemon builds and runs, given the
# plugin on stdin, so nothing here is mounted into anything.
set -eu

IMAGE="${SDK_IMAGE:?the SDK image the steps run in}"
failed=0

migrate() {
    name="$1" from="$2" to="$3" expected="$4" left="$5"
    work="/tmp/e2e/$name"
    rm -rf "$work" && mkdir -p "$work" && cp -R "/fixtures/$name-$from/." "$work"
    git -C "$work" init -q
    git -C "$work" add -A
    git -C "$work" -c user.name=e2e -c user.email=e2e@localhost commit -qm "$name at $from"

    code=0
    PYTHONPATH="$work/src" meridian plugin migrate --dir "$work" --to "$to" \
        --image "$IMAGE" --run-tests --json >"$work.json" || code=$?

    if python3 - "$work.json" "$code" "$expected" "$left" <<'PY'
import json, sys
said = json.load(open(sys.argv[1]))
code, expected, left = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
rules = sorted({b["rule"] for b in said["by_hand"]})
problems = []
if code != expected:
    problems.append(f"exited {code}, not {expected}")
if not said["check"]["passed"]:
    problems.append("plugin check failed: " + json.dumps(said["check"]["failures"]))
if rules != sorted(filter(None, left.split(","))):
    problems.append(f"left by hand {rules}, not {left}")
if problems:
    print("\n".join(problems)); sys.exit(1)
PY
    then :; else
        echo "e2e-migrate FAILED: $name, $from to $to" >&2; failed=1
    fi
    if ! diff -r -x .git -x __pycache__ -x '*.egg-info' "/fixtures/$name-$to" "$work" >&2; then
        echo "e2e-migrate FAILED: $name is not the tree meridian-python expects at $to" >&2
        failed=1
    fi
    echo "  $name: $from to $to, exit $code, $(python3 -c "import json,sys; print(len(json.load(open(sys.argv[1]))['by_hand']))" "$work.json") left by hand"
}

# desk leaves its declared tag, its registered tags and a tag's name by hand;
# meridian-snaptrade's shape leaves the base its Makefile names.
migrate desk 0.5.0 0.7.0 1 "access-tag-by-tag,identity-tags,tags-granted"
migrate snaptrade 0.6.1 0.7.0 1 "pin-elsewhere"

# Refusals, before anything changes: backwards, and a tree git does not hold.
work=/tmp/e2e/desk
code=0; meridian plugin migrate --dir "$work" --to 0.6.0 --image "$IMAGE" --force 2>/dev/null || code=$?
[ "$code" = 2 ] || { echo "e2e-migrate FAILED: backwards exited $code, not 2" >&2; failed=1; }
code=0; meridian plugin migrate --dir "$work" --to 0.7.0 --image "$IMAGE" 2>/dev/null || code=$?
[ "$code" = 2 ] || { echo "e2e-migrate FAILED: a changed tree exited $code, not 2" >&2; failed=1; }

[ "$failed" = 0 ] && echo "e2e-migrate OK: each plugin migrated as meridian-python expects, and checked"
exit "$failed"

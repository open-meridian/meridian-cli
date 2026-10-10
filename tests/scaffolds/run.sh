#!/bin/sh
# Each scaffold `meridian plugin new` writes, made by the real binary and held
# to `meridian plugin check --run-tests` against the SDK the scaffolds pin,
# installed from meridian-python at SDK_REV: `make check-scaffolds`, inside
# ci-local. Its tests run for real, the role's suite among them, which the
# check's own tests cannot: they have no Python.
#
#   - The reference plugin keeps every rule, checked as verified too.
#   - Given `custody`, as the tutorial gives it to record a statement no
#     vendor sent, it keeps every rule; checked as verified, it fails
#     role-suite, since no test of its runs the custody suite.
#   - The `dgm` and `reporting` templates keep every rule, verified, their
#     tests running their role's suite.
#   - The `dgm` with a float in its price path fails: its suite's cases,
#     run by its own tests, catch the price recorded through a float.
#
# Runs inside the scaffolds stage of Dockerfile.rust.
set -eu

cd /tmp
failed=0

say() {
    printf '%s\n' "$*"
}

# The plugin in $1, installed beside the SDK already here: the SDK is the one
# its pin names, and not on PyPI before its release.
install() {
    pip install -q --no-deps -e "$1" >/dev/null
}

# `plugin check` in $1 with the rest of the arguments; its output in $1.out.
checked() {
    dir="$1"
    shift
    (cd "$dir" && meridian plugin check "$@") >"$dir.out" 2>&1
}

passes() {
    dir="$1"
    shift
    if checked "$dir" "$@"; then
        say "ok    $dir: plugin check $* passes"
    else
        say "FAIL  $dir: plugin check $* failed:"
        cat "$dir.out"
        failed=1
    fi
}

# Fails, and says $2 in what it prints.
fails_saying() {
    dir="$1" said="$2"
    shift 2
    if checked "$dir" "$@"; then
        say "FAIL  $dir: plugin check $* passed, and should not have:"
        cat "$dir.out"
        failed=1
    elif grep -q -- "$said" "$dir.out"; then
        say "ok    $dir: plugin check $* fails, saying $said"
    else
        say "FAIL  $dir: plugin check $* failed without saying $said:"
        cat "$dir.out"
        failed=1
    fi
}

meridian plugin new reference --into reference >/dev/null
install reference
passes reference --verified --run-tests

sed -i 's/^roles = \[\]$/roles = ["custody"]/' reference/pyproject.toml
grep -q '^roles = \["custody"\]$' reference/pyproject.toml
passes reference --run-tests
fails_saying reference "it holds \`custody\`, and no test runs the custody suite" --verified

for role in dgm reporting; do
    meridian plugin new "$role-plugin" --role "$role" --into "$role-plugin" >/dev/null
    install "$role-plugin"
    passes "$role-plugin" --verified --run-tests
done

# The fixture: the dgm's vendor's JSON read with Python's floats, every price
# passed through one on its way to the lake.
convert=dgm-plugin/src/dgm_plugin/convert.py
sed -i 's/json.loads(text, parse_float=Decimal, parse_int=Decimal)/json.loads(text)/' "$convert"
if ! grep -q '= json.loads(text)$' "$convert"; then
    say "FAIL  the fixture did not put a float in the dgm's price path"
    failed=1
fi
fails_saying dgm-plugin "FAIL  tests-pass" --verified --run-tests
# And the case that names it: a price with more decimal places than a float
# holds. (Every case recording a price fails; this one is the float's.)
(cd dgm-plugin && python3 -m pytest -vv -p no:cacheprovider tests/test_suite.py) >dgm-suite.out 2>&1 || true
if grep -q "'an-exact-price': 'raised" dgm-suite.out; then
    say "ok    dgm-plugin: its suite fails an-exact-price"
else
    say "FAIL  dgm-plugin: its suite did not fail an-exact-price:"
    cat dgm-suite.out
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
say "check-scaffolds: every scaffold holds as it should"

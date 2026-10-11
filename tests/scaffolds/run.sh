#!/bin/sh
# Each scaffold `meridian plugin new` writes, made by the real binary and held
# to `meridian plugin check --run-tests` against the SDK the scaffolds pin,
# installed from meridian-python at SDK_REV: `make check-scaffolds`, inside
# ci-local. Its tests run for real, the role's suite among them, which the
# check's own tests cannot: they have no Python.
#
#   - The reference plugin keeps every rule; checked as verified, it fails
#     role-pages alone, since its two acts name no check tool (SDK 0.24.0).
#   - Given `custody`, as the tutorial gives it to record a statement no
#     vendor sent, it keeps every rule, told the custody role's common part
#     as advice; checked as verified, it fails role-suite, since no test of
#     its runs the custody suite, and role-pages, since it has none of the
#     role's common pages.
#   - The `data`, `operations` and `reporting` templates keep every rule,
#     verified, the data and reporting templates' tests running their role's
#     suite; the `custody` template keeps every rule and the role's common
#     pages, and its tests run no custody suite and its link names no check
#     tool (SDK 0.24.0), so verified it fails role-suite and role-pages on
#     those alone.
#   - The `data` template with a float in its price path fails: its suite's
#     cases, run by its own tests, catch the price recorded through a float.
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
passes reference --run-tests
fails_saying reference "/statement is an act and names no check tool" --verified
if grep -q "FAIL  role-suite\|FAIL  tests-pass" reference.out; then
    say "FAIL  reference: verified, it fails more than its acts' check tools:"
    cat reference.out
    failed=1
fi

sed -i 's/^roles = \[\]$/roles = ["custody"]/' reference/pyproject.toml
grep -q '^roles = \["custody"\]$' reference/pyproject.toml
passes reference --run-tests
if grep -q "declares no Account links page" reference.out; then
    say "ok    reference: given custody, it is told the role's common part as advice"
else
    say "FAIL  reference: given custody, it was not told the role's common part:"
    cat reference.out
    failed=1
fi
fails_saying reference "it holds \`custody\`, and no test runs the custody suite" --verified
fails_saying reference "it holds \`custody\`, and declares no Account links page" --verified

for role in data operations reporting custody; do
    meridian plugin new "$role-plugin" --role "$role" --into "$role-plugin" >/dev/null
    install "$role-plugin"
    passes "$role-plugin" --run-tests
done
for role in data operations reporting; do
    passes "$role-plugin" --verified --run-tests
done
fails_saying custody-plugin "it holds \`custody\`, and no test runs the custody suite" --verified
if ! grep -q "/links is an act and names no check tool" custody-plugin.out \
    || grep -q "declares no\|comes before" custody-plugin.out; then
    say "FAIL  custody-plugin: verified, it does not keep the custody role's common pages:"
    cat custody-plugin.out
    failed=1
fi

# The fixture: the data plugin's vendor's JSON read with Python's floats,
# every price passed through one on its way to the lake.
convert=data-plugin/src/data_plugin/convert.py
sed -i 's/json.loads(text, parse_float=Decimal, parse_int=Decimal)/json.loads(text)/' "$convert"
if ! grep -q '= json.loads(text)$' "$convert"; then
    say "FAIL  the fixture did not put a float in the data plugin's price path"
    failed=1
fi
fails_saying data-plugin "FAIL  tests-pass" --verified --run-tests
# And the case that names it: a price with more decimal places than a float
# holds. (Every case recording a price fails; this one is the float's.)
(cd data-plugin && python3 -m pytest -vv -p no:cacheprovider tests/test_suite.py) >data-suite.out 2>&1 || true
if grep -q "'an-exact-price': 'raised" data-suite.out; then
    say "ok    data-plugin: its suite fails an-exact-price"
else
    say "FAIL  data-plugin: its suite did not fail an-exact-price:"
    cat data-suite.out
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
say "check-scaffolds: every scaffold holds as it should"

#!/bin/sh
# Each scaffold `meridian plugin new` writes, made by the real binary and held
# to `meridian plugin check --run-tests` against the SDK the scaffolds pin,
# installed from meridian-python at SDK_REV: `make check-scaffolds`, inside
# ci-local. Its tests run for real, the role's suite among them, which the
# check's own tests cannot: they have no Python.
#
#   - The reference plugin keeps every rule, checked as verified too, each
#     of its acts naming its check tool.
#   - Given `custody`, as the tutorial gives it to record a statement no
#     vendor sent, it keeps every rule, told the custody role's common part
#     as advice; checked as verified, it fails role-suite, since no test of
#     its runs the custody suite, and role-pages, since it has none of the
#     role's common pages.
#   - The `custody`, `data`, `operations` and `reporting` templates keep
#     every rule, checked as verified too, on their roles' common parts; the
#     custody, data and reporting templates' tests run their role's suite.
#   - The `connectivity` template (SDK 0.30.0) keeps every rule, its tests
#     running its adapter on the SDK's rig; checked as verified, it fails
#     role-suite, since the connectivity suite is the plugin's to map to its
#     own venue.
#   - The `data` template with a float in its price path fails: its suite's
#     cases, run by its own tests, catch the price recorded through a float.
#   - A connectivity plugin that sends on a pause fails (build row 18 of
#     plans/an-order-reaches-a-venue-through-the-book): the reference plugin
#     given `connectivity`, its test running the connectivity suite with a
#     placement acknowledged at the venue after a pause on it was heard. As
#     verified it fails its tests, and the suite names the pause's case.
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
if grep -q "declares no Account links page" reference.out; then
    say "ok    reference: given custody, it is told the role's common part as advice"
else
    say "FAIL  reference: given custody, it was not told the role's common part:"
    cat reference.out
    failed=1
fi
fails_saying reference "it holds \`custody\`, and no test runs the custody suite" --verified
fails_saying reference "it holds \`custody\`, and declares no Account links page" --verified

for role in custody data operations reporting; do
    meridian plugin new "$role-plugin" --role "$role" --into "$role-plugin" >/dev/null
    install "$role-plugin"
    passes "$role-plugin" --verified --run-tests
done

meridian plugin new connectivity-plugin --role connectivity --into connectivity-plugin >/dev/null
install connectivity-plugin
passes connectivity-plugin --run-tests
fails_saying connectivity-plugin "it holds \`connectivity\`, and no test runs the connectivity suite" --verified

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

# The fixture: a connectivity plugin that hears a pause on itself, then a
# placement naming it, and acknowledges the placement at its venue anyway.
meridian plugin new pausing --into pausing >/dev/null
install pausing
sed -i 's/^roles = \[\]$/roles = ["connectivity"]/' pausing/pyproject.toml
cat >pausing/tests/test_suite.py <<'PY'
from meridian.plugin.v1 import operations_pb2 as ops
from meridian.suites import Recorder, run

PLACEMENT = "PLC-01JAE00000000000000000001"
INSTANCE = "pausing-1"


async def sends_on_a_pause(recorder: Recorder) -> None:
    pause = ops.TradingInstructionChangedEvent(
        instruction=ops.TradingInstruction(
            target=ops.TradingInstructionTarget(instance_id=INSTANCE),
            value=ops.TRADING_INSTRUCTION_VALUE_PAUSE,
        )
    )
    placement = ops.PlacementUpdatedEvent(
        placement=ops.Placement(placement_id=PLACEMENT, connectivity_instance_id=INSTANCE)
    )
    recorder.answer("TradingInstructionChanged", lambda _: pause)
    recorder.answer("PlacementUpdated", lambda _: placement)

    async def heard(_: object) -> None:
        return None

    await recorder.receive(trading_instruction_changed=heard, placement_updated=heard)
    await recorder.report_placement(
        placement_id=PLACEMENT,
        event="acknowledged",
        venue_order_id="VENUE-1",
        raw_record=ops.RawRecordRef(instance_id=INSTANCE, key="venue:report:1"),
    )


def test_the_connectivity_suite() -> None:
    report = run(
        "connectivity",
        {"sends-nothing-new-on-a-pause": sends_on_a_pause},
        instance_id=INSTANCE,
    )
    assert report.passed, report.failures
PY
fails_saying pausing "FAIL  tests-pass" --verified --run-tests
(cd pausing && python3 -m pytest -vv -p no:cacheprovider tests/test_suite.py) >pausing-suite.out 2>&1 || true
if grep -q "'sends-nothing-new-on-a-pause': 'expect\[2\]: sent 1 on ReportPlacement, and the case sends none'" pausing-suite.out; then
    say "ok    pausing: its suite fails sends-nothing-new-on-a-pause"
else
    say "FAIL  pausing: its suite did not fail sends-nothing-new-on-a-pause:"
    cat pausing-suite.out
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
say "check-scaffolds: every scaffold holds as it should"

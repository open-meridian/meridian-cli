#!/bin/sh
# The release binary's smoke check on one system of the supported window
# (ruling 1033; README, "Supported systems"): what each smoke job of
# release.yaml runs, on a macOS runner or in a Linux distribution's container,
# and what `make smoke-linux` runs here.
#
#   sh tests/smoke/run.sh <version> <target>
#
# with the binary under test first on PATH as `meridian`. It checks that:
#
#   - `meridian --version` runs, and says this version and this target;
#   - `meridian status` reads a deployment, through a stand-in kubectl and
#     helm (tests/smoke/stub) that answer the reads it makes for one with a
#     single component, and says the release and the component;
#   - `meridian plugin new` writes the reference plugin (the template
#     vendored from meridian-python, compiled into the binary), and
#     `meridian plugin check` holds it to every rule.
#
# Nothing needs a network, a cluster, Python or anything else installed: the
# binary is static on Linux, and a stock system has /bin/sh. Prints one line
# per check, and exits non-zero if any failed.

set -u

version="$1"
target="$2"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

check() {
    if [ "$1" = 0 ]; then
        printf 'ok      %s\n' "$2"
    else
        printf 'FAILED  %s\n' "$2"
        failures=$((failures + 1))
    fi
}

holds() { # holds <text> <needle>
    case "$1" in *"$2"*) echo 0 ;; *) echo 1 ;; esac
}

# Where this ran, for the job's log: the system, its release and its kernel.
if [ -r /etc/os-release ]; then
    system="$(sed -n 's/^PRETTY_NAME=//p' /etc/os-release | tr -d '"')"
elif command -v sw_vers >/dev/null 2>&1; then
    system="macOS $(sw_vers -productVersion)"
else
    system="$(uname -s)"
fi
printf 'smoke   meridian %s (%s) on %s, kernel %s %s, %s\n' \
    "$version" "$target" "$system" "$(uname -s)" "$(uname -r)" "$(uname -m)"

# ── --version ─────────────────────────────────────────────────────────────

said="$(meridian --version 2>&1)"
status=$?
check $status "meridian --version runs: $said"
check "$(holds "$said" "meridian $version ($target; meridian-core ")" "and says $version for $target"

# ── status, against the stand-in deployment ──────────────────────────────

before=$failures
# HOME and XDG_CONFIG_HOME empty of sessions: no deployment is connected, so
# book health is said not to be read, and nothing is asked of a deployment.
said="$(PATH="$here/stub:$PATH" HOME="$work" XDG_CONFIG_HOME="$work/config" meridian status -n smoke 2>&1)"
status=$?
check $status "meridian status reads the stand-in deployment"
check "$(holds "$said" "Release: meridian revision 3, deployed")" "and says its release, deployed"
check "$(holds "$said" "bor")" "and its component"
check "$(holds "$said" "1/1 ready")" "and that its one pod is ready"
if [ "$failures" != "$before" ]; then
    printf '%s\n' "$said"
fi

# ── plugin new, then plugin check ─────────────────────────────────────────

said="$(cd "$work" && meridian plugin new smoke-plugin --into smoke-plugin 2>&1)"
status=$?
check $status "meridian plugin new writes the reference plugin"
said="$(cd "$work/smoke-plugin" 2>/dev/null && meridian plugin check 2>&1)"
status=$?
check $status "meridian plugin check holds it to every rule"
if [ $status != 0 ]; then
    printf '%s\n' "$said"
fi

if [ "$failures" != 0 ]; then
    printf 'smoke FAILED: %s check(s) on %s\n' "$failures" "$system"
    exit 1
fi
printf 'smoke OK on %s\n' "$system"

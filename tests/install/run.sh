#!/bin/sh
# shellcheck disable=SC2016 # the shims below write their own $1, unexpanded
# The install script and `meridian upgrade` and `uninstall`, against a
# stand-in for GitHub's releases (stand_in.py), in a container: what
# `make check-install` runs. The real meridian is /usr/local/bin/meridian;
# the releases it is upgraded to are the stand-in's scripts. `uname` and
# `sysctl` are shimmed to try every platform's choice from one container.
#
# Prints one line per check and exits non-zero if any failed.

set -u

RELEASES="http://127.0.0.1:8765/releases"
SCRIPT=/w/install.sh
failures=0

check() {
    if [ "$1" = 0 ]; then
        printf 'ok: %s\n' "$2"
    else
        printf 'FAILED: %s\n' "$2"
        failures=$((failures + 1))
    fi
}

holds() { # holds <text> <needle>
    case "$1" in *"$2"*) echo 0 ;; *) echo 1 ;; esac
}

python3 /w/tests/install/stand_in.py 8765 &
for _ in 1 2 3 4 5 6 7 8 9 10; do
    curl -fsS -o /dev/null "http://127.0.0.1:8765/releases/tag/v9.9.9" 2>/dev/null && break
    sleep 1
done

# ── The install script ────────────────────────────────────────────────────

said="$(MERIDIAN_RELEASES="$RELEASES" MERIDIAN_INSTALL_DIR=/tmp/latest sh "$SCRIPT" 2>&1)"
status=$?
check $status "the script installs the latest"
check "$(holds "$("/tmp/latest/meridian" --version 2>&1)" "meridian 9.9.9")" "and it is v9.9.9: $(/tmp/latest/meridian --version 2>&1)"
check "$(holds "$said" "/tmp/latest is not on your PATH")" "and says how to put it on PATH"
check "$([ "$(stat -c %a /tmp/latest/meridian)" = 755 ] && echo 0 || echo 1)" "executable by anybody, written by its owner"
check "$([ -z "$(find /tmp/latest -name '.meridian-*')" ] && echo 0 || echo 1)" "nothing left beside it"

said="$(MERIDIAN_RELEASES="$RELEASES" MERIDIAN_INSTALL_DIR=/tmp/named MERIDIAN_VERSION=9.9.9 sh "$SCRIPT" 2>&1)"
status=$?
check $status "a named release installs"

said="$(MERIDIAN_RELEASES="$RELEASES" MERIDIAN_INSTALL_DIR=/tmp/broken MERIDIAN_VERSION=6.6.6 sh "$SCRIPT" 2>&1)"
status=$?
check "$([ $status != 0 ] && [ ! -e /tmp/broken/meridian ] && echo 0 || echo 1)" "a download that is not its checksum is refused, and nothing installed"
check "$(holds "$said" "not what its checksum says")" "and it says so: $said"

said="$(MERIDIAN_RELEASES="$RELEASES" MERIDIAN_INSTALL_DIR=/tmp/none MERIDIAN_VERSION=1.2.3 sh "$SCRIPT" 2>&1)"
status=$?
check "$([ $status != 0 ] && echo 0 || echo 1)" "a release that is not there is refused: $said"

said="$(MERIDIAN_RELEASES="http://mirror.firm.example/releases" MERIDIAN_INSTALL_DIR=/tmp/plain sh "$SCRIPT" 2>&1)"
status=$?
check "$([ $status != 0 ] && echo 0 || echo 1)" "plain http from elsewhere is refused: $said"

# Every platform's binary, from one container: uname and sysctl shimmed.
mkdir -p /tmp/shim
for case in "Linux x86_64 - x86_64-unknown-linux-musl" \
            "Linux aarch64 - aarch64-unknown-linux-musl" \
            "Darwin arm64 - aarch64-apple-darwin" \
            "Darwin x86_64 0 x86_64-apple-darwin" \
            "Darwin x86_64 1 aarch64-apple-darwin"; do
    # shellcheck disable=SC2086 # split into its four words, on purpose
    set -- $case
    printf '#!/bin/sh\ncase "$1" in -s) echo %s ;; -m) echo %s ;; esac\n' "$1" "$2" >/tmp/shim/uname
    printf '#!/bin/sh\necho %s\n' "$3" >/tmp/shim/sysctl
    chmod 755 /tmp/shim/uname /tmp/shim/sysctl
    said="$(PATH="/tmp/shim:$PATH" MERIDIAN_RELEASES="$RELEASES" MERIDIAN_INSTALL_DIR="/tmp/p-$4" sh "$SCRIPT" 2>&1)"
    check "$(holds "$said" "for $4")" "$1 on $2 (arm64 under Rosetta: $3) gets $4"
    # No terminal here: nothing is asked, and one line says `up` will ask.
    asks="$(holds "$said" "\`meridian up\` asks to trust")"
    if [ "$1" = Darwin ]; then
        check "$asks" "with nobody to ask on macOS, trusting the authority is left to up, and said"
    else
        check "$([ "$asks" != 0 ] && echo 0 || echo 1)" "on Linux nothing is said about trusting it"
    fi
    check "$([ "$(holds "$said" "stand-in asked")" != 0 ] && echo 0 || echo 1)" "and authority trust is not run"
done

# At a terminal on macOS: `meridian authority trust` is run, its answers read
# from the terminal rather than from the pipe the script came down.
printf '#!/bin/sh\ncase "$1" in -s) echo Darwin ;; -m) echo arm64 ;; esac\n' >/tmp/shim/uname
said="$(script -qec "PATH=/tmp/shim:$PATH MERIDIAN_RELEASES=$RELEASES MERIDIAN_INSTALL_DIR=/tmp/tty sh $SCRIPT </dev/null" /dev/null </dev/null 2>&1)"
check "$(holds "$said" "stand-in asked: authority trust, its input a terminal: yes")" "at a terminal on macOS, authority trust is run at the terminal: $said"

printf '#!/bin/sh\ncase "$1" in -s) echo MINGW64_NT-10.0 ;; -m) echo x86_64 ;; esac\n' >/tmp/shim/uname
said="$(PATH="/tmp/shim:$PATH" MERIDIAN_RELEASES="$RELEASES" MERIDIAN_INSTALL_DIR=/tmp/windows sh "$SCRIPT" 2>&1)"
status=$?
check "$([ $status != 0 ] && echo 0 || echo 1)" "a platform with no binary is refused, naming those there are: $said"

# ── meridian upgrade ──────────────────────────────────────────────────────

mkdir -p /tmp/u && cp /usr/local/bin/meridian /tmp/u/meridian
before="$(/tmp/u/meridian --version)"
said="$(MERIDIAN_RELEASES="$RELEASES" /tmp/u/meridian upgrade --to "$(/usr/local/bin/meridian --version | awk '{ print $2 }')" 2>&1)"
status=$?
check "$(holds "$said" "nothing to do")" "upgrade to the release it is says there is nothing to do: $said"

said="$(MERIDIAN_RELEASES="$RELEASES" /tmp/u/meridian upgrade --to 6.6.6 2>&1)"
status=$?
check "$([ $status != 0 ] && [ "$(/tmp/u/meridian --version)" = "$before" ] && echo 0 || echo 1)" "a release that is not its checksum is refused, and the binary left: $said"

said="$(su tester -s /bin/sh -c "MERIDIAN_RELEASES=$RELEASES /tmp/u/meridian upgrade" 2>&1)"
status=$?
check "$([ $status != 0 ] && [ "$(/tmp/u/meridian --version)" = "$before" ] && echo 0 || echo 1)" "where it cannot write, it refuses before fetching and leaves the binary: $said"

said="$(MERIDIAN_RELEASES="$RELEASES" /tmp/u/meridian upgrade 2>&1)"
status=$?
check $status "upgrade fetches the latest: $said"
check "$(holds "$(/tmp/u/meridian --version 2>&1)" "meridian 9.9.9")" "and it is the binary now: $(/tmp/u/meridian --version 2>&1)"

# ── meridian uninstall ────────────────────────────────────────────────────

mkdir -p /tmp/v && cp /usr/local/bin/meridian /tmp/v/meridian
export XDG_CONFIG_HOME=/tmp/config
mkdir -p /tmp/config/meridian/sessions
# A terminal session an older CLI kept, which no deployment honours since
# contract v15: forgotten here, and said, with nothing asked of the deployment.
printf '{"address":"http://127.0.0.1:9","session":"not-a-real-one","subject":"local|ada","expires_at":"2026-09-26T12:00:00Z"}' \
    >/tmp/config/meridian/sessions/127.0.0.1_9.json
# And a delegation's pair with another nobody can reach (decisions/029).
printf '{"address":"http://127.0.0.1:8","subject":"local|ada","expires_at":"2026-12-25T00:00:00Z","client_id":"mdc_x","access_token":"mda_x","access_expires_at_s":1,"refresh_token":"mdr_x","expires_at_s":1798156800}' \
    >/tmp/config/meridian/sessions/127.0.0.1_8.json

# Off macOS, `authority trust` makes the root in this CLI's own directory and
# prints each step as a command, changing nothing else.
said="$(NODE_EXTRA_CA_CERTS=/etc/firm.pem /tmp/v/meridian authority trust </dev/null 2>&1)"
status=$?
check $status "authority trust on Linux: $said"
check "$([ -s /tmp/config/meridian/authority/root.pem ] && echo 0 || echo 1)" "and the root is made"
check "$(holds "$said" "update-ca-certificates")" "it prints the command that trusts it"
check "$(holds "$said" "cat /etc/firm.pem /tmp/config/meridian/authority/root.pem > /tmp/config/meridian/authority/node-extra-ca-certs.pem")" "one NODE_EXTRA_CA_CERTS already named is kept, in a file holding both"
check "$(holds "$said" "export NODE_EXTRA_CA_CERTS=/tmp/config/meridian/authority/node-extra-ca-certs.pem")" "and the command naming that file to apps"
check "$([ ! -e /tmp/config/meridian/authority/node-extra-ca-certs.pem ] && echo 0 || echo 1)" "a file it only printed is not made"
said="$(/tmp/v/meridian authority 2>&1)"
check "$(holds "$said" "The Claude app, like any app on Node, reads NODE_EXTRA_CA_CERTS")" "authority says what the Claude app reads"

said="$(/tmp/v/meridian uninstall </dev/null 2>&1)"
status=$?
check "$([ $status != 0 ] && [ -e /tmp/v/meridian ] && [ -e /tmp/config/meridian/sessions/127.0.0.1_9.json ] && echo 0 || echo 1)" "with nobody to ask and no --yes, nothing is removed"
check "$(holds "$said" "the session an older meridian kept for http://127.0.0.1:9, forgotten here")" "and it listed the older session it would forget"

said="$(/tmp/v/meridian uninstall --yes 2>&1)"
status=$?
check $status "uninstall --yes: $said"
check "$([ ! -e /tmp/v/meridian ] && echo 0 || echo 1)" "the binary is gone"
check "$([ ! -e /tmp/config/meridian ] && echo 0 || echo 1)" "and every session with it, and its directory"
check "$(holds "$said" "remove the NODE_EXTRA_CA_CERTS line")" "the authority goes, with the command that stops naming it to apps"
check "$(holds "$said" "Forgotten the session an older meridian kept for http://127.0.0.1:9.")" "an older meridian's session is forgotten here, and said"
check "$(holds "$said" "revoke this computer's delegation from Connected clients on http://127.0.0.1:8")" "an unreachable deployment's delegation is forgotten here, and how to revoke it said"

echo
if [ "$failures" != 0 ]; then
    echo "install FAILED: $failures"
    exit 1
fi
echo "install OK"

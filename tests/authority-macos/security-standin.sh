#!/bin/bash
# The stand-in for a person answering macOS's password dialog, on a runner.
#
# `meridian authority trust` adds its root to the login keychain with
# `security add-trusted-cert -r trustRoot -k <login keychain>`, and macOS asks
# the person at the screen for their password before it changes their trust
# settings. A runner has nobody at the screen, so run.sh first lets the real
# step wait on that dialog and checks the CLI went no further. Then, for the
# rest of the run,
# this file is put first on PATH as `security`: the CLI is the shipped
# binary, unchanged, asking for exactly what it asks a person's Mac for
# (recorded in $MERIDIAN_E2E_STANDIN/calls), and the trust is put where a
# runner can put it without a dialog -- the System keychain, with sudo.
#
# It handles the two calls the CLI makes and passes anything else to the
# real security. Not shipped, not reachable from the binary: it exists only
# in run.sh's scratch directory, for the length of a run.

set -euo pipefail

real=/usr/bin/security
system=/Library/Keychains/System.keychain
state="${MERIDIAN_E2E_STANDIN:?run.sh sets MERIDIAN_E2E_STANDIN}"

# Each sudo call, killed after a minute and its outcome recorded, so a dialog
# waiting where nobody can answer it is a failure the CLI reports, not a hang.
standin() {
    local status=0
    perl -e 'alarm shift @ARGV; exec @ARGV or die "exec: $!"' 60 sudo "$real" "$@" \
        2>>"$state/calls" || status=$?
    printf '  -> sudo security %s: status %s\n' "$*" "$status" >>"$state/calls"
    return "$status"
}

case "${1:-}" in
    add-trusted-cert)
        # add-trusted-cert -r trustRoot -k <login keychain> <root>
        printf '%s\n' "security $*" >>"$state/calls"
        root="${!#}"
        sha1="$(openssl x509 -noout -fingerprint -sha1 -in "$root" | sed 's/.*=//; s/://g')"
        cp "$root" "$state/$sha1.pem"
        standin add-trusted-cert -d -r trustRoot -k "$system" "$root"
        ;;
    delete-certificate)
        # delete-certificate -t -Z <SHA-1> <login keychain>
        printf '%s\n' "security $*" >>"$state/calls"
        sha1=""
        previous=""
        for argument in "$@"; do
            if [ "$previous" = -Z ]; then sha1="$argument"; fi
            previous="$argument"
        done
        # The certificate out of the System keychain. Its admin trust setting
        # stays, naming a certificate no keychain holds: on a runner,
        # `remove-trusted-cert -d` hung in run 37205612295 and was hung up on
        # in 37206379234, and with the root gone nothing can chain to it.
        standin delete-certificate -Z "$sha1" "$system"
        rm -f "$state/$sha1.pem"
        ;;
    *)
        exec "$real" "$@"
        ;;
esac

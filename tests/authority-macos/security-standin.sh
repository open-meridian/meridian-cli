#!/bin/bash
# The stand-in for a person answering macOS's password dialog, on a runner.
#
# `meridian authority trust` adds its root to the login keychain with
# `security add-trusted-cert -r trustRoot -k <login keychain>`, and macOS asks
# the person at the screen for their password before it changes their trust
# settings. A runner has nobody at the screen, so run.sh first lets the real
# step be refused and checks the CLI says so. Then, for the rest of the run,
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

case "${1:-}" in
    add-trusted-cert)
        # add-trusted-cert -r trustRoot -k <login keychain> <root>
        printf '%s\n' "security $*" >>"$state/calls"
        root="${!#}"
        sha1="$(openssl x509 -noout -fingerprint -sha1 -in "$root" | sed 's/.*=//; s/://g')"
        cp "$root" "$state/$sha1.pem"
        exec sudo "$real" add-trusted-cert -d -r trustRoot -k "$system" "$root"
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
        if [ -f "$state/$sha1.pem" ]; then
            sudo "$real" remove-trusted-cert -d "$state/$sha1.pem" || true
            rm -f "$state/$sha1.pem"
        fi
        exec sudo "$real" delete-certificate -Z "$sha1" "$system"
        ;;
    *)
        exec "$real" "$@"
        ;;
esac

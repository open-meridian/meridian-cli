#!/bin/bash
# `meridian authority trust` on a real Mac, as install.sh runs it at a
# terminal, then `meridian uninstall`; and again where NODE_EXTRA_CA_CERTS
# already named another file (ruled 2026-10-04, task
# kernel/a-development-deployment-serves-https). What the authority-macos
# workflow runs on a macOS runner, and `make e2e-authority-macos` on a Mac.
#
# It changes this machine: the keychain, launchd's environment for apps and
# a LaunchAgent, and puts each back. So it refuses to start without
# MERIDIAN_E2E_CHANGES_THIS_MAC=yes, and refuses where NODE_EXTRA_CA_CERTS is
# named already or the LaunchAgent is there, rather than touch a person's own.
#
# The install is the real install.sh, downloading a published release
# (MERIDIAN_VERSION, or the latest), run under expect at a pseudo-terminal
# answering yes, so its /dev/tty path is the one exercised.
#
# The login keychain (MERIDIAN_E2E_KEYCHAIN):
#   standin  the default on GitHub Actions. Adding a trust setting to a
#            person's login keychain waits on macOS's password dialog, which
#            nobody can answer on a runner. So the real step runs first and is
#            refused, and the run checks the CLI reports that and goes no
#            further; then security-standin.sh stands in for the person, put
#            first on PATH as `security`, trusting the root in the System
#            keychain with sudo instead. The shipped binary has no test-only
#            path: everything it does is what it does on a person's Mac.
#   login    the default elsewhere. The real login keychain, and the person
#            running it answers macOS's dialog.
# Off a runner, the CLI's files and the binary go in a scratch directory
# (XDG_CONFIG_HOME, MERIDIAN_INSTALL_DIR), never the person's own; on a
# runner, the defaults, ~/.config/meridian and ~/.local/bin.
#
# The HTTPS checks need a certificate from the root for a name under
# .localhost. The CLI issues one only into a deployment's Secret, so openssl
# signs one here with the root's key: test-only, in a scratch directory.
#
# Prints one line per check and exits non-zero if any failed.

set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
label=com.open-meridian.authority
plist="$HOME/Library/LaunchAgents/$label.plist"
login_keychain="$HOME/Library/Keychains/login.keychain-db"
system_keychain=/Library/Keychains/System.keychain
name="authority-e2e.localhost"
failures=0

die() {
    printf 'authority-macos: %s\n' "$*" >&2
    exit 2
}

check() { # check <status> <what>
    if [ "$1" = 0 ]; then
        printf 'ok: %s\n' "$2"
    else
        printf 'FAILED: %s\n' "$2"
        failures=$((failures + 1))
    fi
}

ok() { "$@" >/dev/null 2>&1 && echo 0 || echo 1; }
not() { "$@" >/dev/null 2>&1 && echo 1 || echo 0; }
holds() { case "$(cat "$1" 2>/dev/null)" in *"$2"*) echo 0 ;; *) echo 1 ;; esac; }
lacks() { case "$(cat "$1" 2>/dev/null)" in *"$2"*) echo 1 ;; *) echo 0 ;; esac; }
is() { [ "$1" = "$2" ] && echo 0 || echo 1; }
named() { launchctl getenv NODE_EXTRA_CA_CERTS 2>/dev/null || true; }
loaded() { launchctl list "$label"; }
in_system_keychain() { security find-certificate -a -Z "$system_keychain" 2>/dev/null | grep -q "$1"; }
sha1_of() { openssl x509 -noout -fingerprint -sha1 -in "$1" | sed 's/.*=//; s/://g'; }

# ── Whether to run at all ───────────────────────────────────────────────────

[ "$(uname -s)" = Darwin ] || die "this is a macOS test"
[ "${MERIDIAN_E2E_CHANGES_THIS_MAC:-}" = yes ] ||
    die "this changes this Mac's keychain, launchd's environment for apps and its LaunchAgents, and puts them back. Set MERIDIAN_E2E_CHANGES_THIS_MAC=yes to run it."
for tool in expect node openssl curl launchctl security; do
    command -v "$tool" >/dev/null 2>&1 || die "this needs $tool"
done

on_runner=no
[ "${GITHUB_ACTIONS:-}" = true ] && on_runner=yes
keychain="${MERIDIAN_E2E_KEYCHAIN:-$([ $on_runner = yes ] && echo standin || echo login)}"
case "$keychain" in
    standin | login) ;;
    *) die "MERIDIAN_E2E_KEYCHAIN is standin or login, not $keychain" ;;
esac

[ -z "$(named)" ] ||
    die "NODE_EXTRA_CA_CERTS names $(named) to apps already; this would not touch it. Unset it to run this."
[ ! -e "$plist" ] || die "$plist is there already; this would not touch it"
! loaded >/dev/null 2>&1 || die "launchd has $label loaded already; this would not touch it"

scratch="$(mktemp -d "${TMPDIR:-/tmp}/meridian-authority-e2e.XXXXXX")"
if [ $on_runner = no ]; then
    export XDG_CONFIG_HOME="$scratch/config"
    export MERIDIAN_INSTALL_DIR="$scratch/bin"
fi
authority="${XDG_CONFIG_HOME:-$HOME/.config}/meridian/authority"
root="$authority/root.pem"
combined="$authority/node-extra-ca-certs.pem"
bin="${MERIDIAN_INSTALL_DIR:-$HOME/.local/bin}/meridian"
[ ! -e "$authority" ] || die "$authority is there already; this would not touch it"
[ ! -e "$bin" ] || die "$bin is there already; this would not touch it"

export MERIDIAN_E2E_STANDIN="$scratch/standin"
mkdir -p "$MERIDIAN_E2E_STANDIN" "$scratch/shim"
cp "$here/security-standin.sh" "$scratch/shim/security"
chmod 755 "$scratch/shim/security"
plain_path="$PATH"
if [ "$keychain" = standin ]; then trusted_path="$scratch/shim:$PATH"; else trusted_path="$PATH"; fi

server=""
# Whatever a failed run left, put back: the LaunchAgent, a NODE_EXTRA_CA_CERTS
# naming this run's files, the root in a keychain.
cleanup() {
    [ -n "$server" ] && kill "$server" 2>/dev/null
    if [ -e "$bin" ] && [ -e "$authority" ]; then
        PATH="$trusted_path" "$bin" authority remove --yes >/dev/null 2>&1
    fi
    if [ -e "$plist" ]; then
        launchctl unload "$plist" 2>/dev/null
        rm -f "$plist"
    fi
    case "$(named)" in "$scratch"/* | "$authority"/*) launchctl unsetenv NODE_EXTRA_CA_CERTS ;; esac
    for left in "$MERIDIAN_E2E_STANDIN"/*.pem; do
        [ -e "$left" ] || continue
        sudo security remove-trusted-cert -d "$left" 2>/dev/null
        sudo security delete-certificate -Z "$(sha1_of "$left")" "$system_keychain" >/dev/null 2>&1
    done
    [ -e "$bin" ] && [ $on_runner = no ] && rm -f "$bin"
    rm -rf "$scratch"
}
trap cleanup EXIT

printf 'authority-macos: keychain %s, installing %s, in %s\n' \
    "$keychain" "${MERIDIAN_VERSION:-the latest release}" "$scratch"

# install <transcript> <seconds> <PATH>: the real install.sh at a terminal.
install() {
    PATH="$3" expect -f "$here/answer-yes.exp" "$1" "$2" sh "$repo/install.sh"
    local status=$?
    printf '%s\n' "--- $1" && cat "$1" && printf '%s\n' "---"
    return $status
}

asked() { cat "$1.asked" 2>/dev/null || echo none; }

# A certificate from the root, for <name>, signed by openssl with the root's
# key (test-only; the CLI issues its own only into a deployment's Secret).
leaf() { # leaf <name> <out-prefix>
    openssl genrsa -out "$2-key.pem" 2048 2>/dev/null &&
        openssl req -new -key "$2-key.pem" -subj "/CN=$1" -out "$2.csr" 2>/dev/null &&
        printf '%s\n' \
            "basicConstraints=critical,CA:FALSE" \
            "keyUsage=critical,digitalSignature,keyEncipherment" \
            "extendedKeyUsage=serverAuth" \
            "subjectAltName=DNS:$1" \
            "authorityKeyIdentifier=keyid" \
            "subjectKeyIdentifier=hash" >"$2.ext" &&
        openssl x509 -req -in "$2.csr" -CA "$root" -CAkey "$authority/root-key.pem" \
            -set_serial "0x$(openssl rand -hex 8)" -days 30 -sha256 -extfile "$2.ext" \
            -out "$2.pem" 2>/dev/null
}

port=""
# serve: an HTTPS server on 127.0.0.1 alone, for $name, from the root as it is now.
# Called directly, never in $(...), so the server it starts is this shell's.
serve() {
    [ -n "$server" ] && kill "$server" 2>/dev/null && wait "$server" 2>/dev/null
    rm -f "$scratch/port"
    leaf "$name" "$scratch/leaf" || return 1
    node "$here/https.js" serve "$scratch/leaf.pem" "$scratch/leaf-key.pem" "$scratch/port" &
    server=$!
    for _ in $(seq 1 50); do
        [ -s "$scratch/port" ] && port="$(cat "$scratch/port")" && return 0
        sleep 0.2
    done
    return 1
}

node_get() { # node_get [NODE_EXTRA_CA_CERTS]
    if [ -n "${1:-}" ]; then
        NODE_EXTRA_CA_CERTS="$1" node "$here/https.js" get "https://$name:$port/"
    else
        env -u NODE_EXTRA_CA_CERTS node "$here/https.js" get "https://$name:$port/"
    fi
}

system_curl() {
    /usr/bin/curl -fsS --max-time 10 --resolve "$name:$port:127.0.0.1" "https://$name:$port/"
}

macos_trusts() { # macos_trusts <cert> <name>: macOS's own evaluation, for TLS at that name
    security verify-cert -q -c "$1" -p ssl -s "$2"
}

# ── 1. Installed at a terminal, answering yes ──────────────────────────────

if [ "$keychain" = standin ]; then
    echo
    echo "== The login keychain's dialog, with nobody to answer it"
    t="$scratch/1-refused.txt"
    install "$t" 180 "$plain_path"
    status=$?
    check "$(is "$status" 0)" "install.sh finishes though the keychain step does not (status $status; 124 is a dialog waiting for a person)"
    check "$(holds "$t" "Installed meridian")" "install.sh installed the release"
    check "$(holds "$t" "Trust it now?")" "the CLI asked, at the terminal install.sh gave it"
    check "$(is "$(asked "$t")" 1)" "and asked once (answered $(asked "$t"))"
    check "$(holds "$t" "It was not added: security said no")" "the keychain step was refused, and the CLI said so"
    check "$(holds "$t" "To trust it, run:")" "and said how to do it by hand"
    check "$(holds "$t" "security add-trusted-cert -r trustRoot -k ~/Library/Keychains/login.keychain-db")" "naming the login keychain command"
    check "$(lacks "$t" "Apps started from now on")" "and pointed no apps at a root it could not trust"
    check "$(not test -e "$plist")" "no LaunchAgent"
    check "$(is "$(named)" "")" "NODE_EXTRA_CA_CERTS still names nothing"
    check "$(not test -e "$authority/asked")" "and it is not marked as asked, so it asks again"

    echo
    echo "== Before anything trusts the root"
    serve
    check $? "an HTTPS server for $name on 127.0.0.1, its certificate from the root"
    check "$(not node_get)" "Node refuses it: $(node_get)"
    check "$(not system_curl)" "macOS's curl refuses it"
    check "$(not macos_trusts "$scratch/leaf.pem" "$name")" "macOS does not trust it"
fi

echo
echo "== Trusted, the $keychain keychain"
t="$scratch/1-trusted.txt"
install "$t" 300 "$trusted_path"
status=$?
check "$(is "$status" 0)" "install.sh at a terminal (status $status)"
check "$(holds "$t" "Trust it now?")" "the CLI asked"
check "$(is "$(asked "$t")" 1)" "once (answered $(asked "$t"))"
check "$(holds "$t" "Added to your login keychain, trusted for HTTPS.")" "and trusted it"
if [ "$keychain" = standin ]; then
    check "$(holds "$MERIDIAN_E2E_STANDIN/calls" "add-trusted-cert -r trustRoot -k $login_keychain $root")" \
        "asking for the login keychain, as on a person's Mac; the stand-in put it in the System keychain"
    check "$(ok in_system_keychain "$(sha1_of "$root")")" "the root is in the System keychain (the stand-in)"
fi
check "$(holds "$t" "Apps started from now on are pointed at it with NODE_EXTRA_CA_CERTS=$root")" "and pointed apps at it"

echo
echo "== What it left"
check "$(ok openssl x509 -noout -in "$root")" "the root: $root"
check "$(holds "$root" "BEGIN CERTIFICATE")" "a certificate"
check "$(is "$(stat -f %Lp "$authority/root-key.pem" 2>/dev/null)" 600)" "its key, readable by its owner alone (mode $(stat -f %Lp "$authority/root-key.pem" 2>/dev/null))"
check "$(ok test -e "$plist")" "the LaunchAgent: $plist"
check "$(holds "$plist" "<string>$root</string>")" "naming the root"
check "$(ok loaded)" "loaded: launchctl list $label"
launchctl print "gui/$(id -u)/$label" >/dev/null 2>&1 &&
    echo "   (launchctl print gui/$(id -u)/$label shows it too)"
check "$(is "$(named)" "$root")" "launchctl getenv NODE_EXTRA_CA_CERTS: $(named)"
check "$(ok test -e "$authority/asked")" "marked as asked"

echo
echo "== HTTPS from the root"
serve
check $? "an HTTPS server for $name on 127.0.0.1, its certificate from the root"
check "$(not node_get)" "Node without NODE_EXTRA_CA_CERTS refuses it: $(node_get)"
check "$(ok node_get "$(named)")" "Node with what launchd names reaches it: $(node_get "$(named)")"
check "$(ok macos_trusts "$scratch/leaf.pem" "$name")" "macOS trusts the certificate for $name (security verify-cert)"
check "$(ok system_curl)" "macOS's curl reaches it: $(system_curl 2>&1)"
leaf authority-e2e.example.com "$scratch/outside"
check "$(not macos_trusts "$scratch/outside.pem" authority-e2e.example.com)" "and macOS refuses one from the root for a name outside .localhost: the name constraint holds"

echo
echo "== meridian uninstall"
t="$scratch/1-uninstall.txt"
sha1="$(sha1_of "$root")"
PATH="$trusted_path" "$bin" uninstall --yes >"$t" 2>&1
status=$?
printf '%s\n' "--- $t" && cat "$t" && printf '%s\n' "---"
check "$(is "$status" 0)" "it finishes (status $status)"
check "$(holds "$t" "Removed it from your login keychain.")" "out of the keychain"
check "$(holds "$t" "Removed $plist.")" "the LaunchAgent removed"
check "$(holds "$t" "NODE_EXTRA_CA_CERTS no longer names it to apps.")" "and NODE_EXTRA_CA_CERTS unset"
check "$(not test -e "$authority")" "the root and its key are gone"
check "$(not test -e "$plist")" "the LaunchAgent is gone"
check "$(not loaded)" "launchd has it no longer"
check "$(is "$(named)" "")" "NODE_EXTRA_CA_CERTS names nothing"
check "$(not test -e "$bin")" "the binary is gone"
if [ "$keychain" = standin ]; then
    check "$(not in_system_keychain "$sha1")" "the root is out of the System keychain (the stand-in)"
fi
check "$(not system_curl)" "and macOS's curl refuses the deployment it signed for again"

# ── 2. Where NODE_EXTRA_CA_CERTS named another file already ───────────────

echo
echo "== NODE_EXTRA_CA_CERTS named another file already"
theirs="$scratch/theirs.pem"
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$scratch/theirs-key.pem" -out "$theirs" \
    -subj "/CN=A root the person named already" -days 30 2>/dev/null
launchctl setenv NODE_EXTRA_CA_CERTS "$theirs"
check "$(is "$(named)" "$theirs")" "launchd names $theirs to apps"

t="$scratch/2-trusted.txt"
install "$t" 300 "$trusted_path"
status=$?
check "$(is "$status" 0)" "install.sh at a terminal (status $status)"
check "$(holds "$t" "NODE_EXTRA_CA_CERTS already names $theirs, and this never replaces it.")" "the CLI said it would not replace theirs"
check "$(holds "$t" "Make that file, and name it to apps instead of $theirs?")" "and asked to name a file holding both"
check "$(is "$(asked "$t")" 2)" "two questions, both answered yes (answered $(asked "$t"))"
check "$(holds "$t" "Apps started from now on are pointed at it with NODE_EXTRA_CA_CERTS=$combined")" "and pointed apps at it"
check "$(is "$(cat "$combined" 2>/dev/null)" "$(cat "$theirs" "$root" 2>/dev/null)")" "the file holds theirs and the root: $combined"
check "$(is "$(named)" "$combined")" "launchctl getenv NODE_EXTRA_CA_CERTS: $(named)"
check "$(holds "$authority/node-extra-ca-certs.before" "$theirs")" "what it named before is recorded"
check "$(holds "$plist" "<string>$combined</string>")" "the LaunchAgent names the file holding both"
check "$(ok loaded)" "loaded: launchctl list $label"
serve
check $? "an HTTPS server from the new root"
check "$(ok node_get "$(named)")" "Node with the file holding both reaches it: $(node_get "$(named)")"

t="$scratch/2-remove.txt"
PATH="$trusted_path" "$bin" authority remove --yes >"$t" 2>&1
status=$?
printf '%s\n' "--- $t" && cat "$t" && printf '%s\n' "---"
check "$(is "$status" 0)" "meridian authority remove --yes (status $status)"
check "$(holds "$t" "Apps are pointed at $theirs again with NODE_EXTRA_CA_CERTS, as before.")" "it said theirs is back"
check "$(is "$(named)" "$theirs")" "NODE_EXTRA_CA_CERTS names theirs again: $(named)"
check "$(not test -e "$authority")" "the root, its key and the file holding both are gone"
check "$(not test -e "$plist")" "the LaunchAgent is gone"
check "$(not loaded)" "launchd has it no longer"

PATH="$trusted_path" "$bin" uninstall --yes >/dev/null 2>&1
check "$(not test -e "$bin")" "meridian uninstall removes the binary"
check "$(is "$(named)" "$theirs")" "and leaves theirs named"
launchctl unsetenv NODE_EXTRA_CA_CERTS

echo
if [ "$failures" -gt 0 ]; then
    echo "authority-macos: $failures checks FAILED"
    exit 1
fi
echo "authority-macos: every check passed (keychain: $keychain)"

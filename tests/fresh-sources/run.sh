#!/bin/sh
# tools/fresh-sources.sh (ruling 934), held to what it promises: a source's
# time is when the target cache first saw its content, whatever time the
# checkout gave it; content the cache has not seen is new as of the run; a
# build the record did not see makes every source new; the command's status
# is the script's. A stand-in workspace, and `stat` for the command.
set -eu

fail() {
	echo "fresh-sources: FAILED: $*" >&2
	exit 1
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
mkdir -p src plugin-template sdk-roles/custody target/debug/.fingerprint
printf '[package]\nname = "x"\n' >Cargo.toml
: >Cargo.lock
echo 'fn main() {}' >build.rs
echo 'fn main() { println!("one"); }' >src/main.rs
echo 'template' >plugin-template/AGENTS.md
echo 'role' >sdk-roles/custody/pages.py

at() { stat -c %Y "$1"; }

# A first run: no record, so a build it did not see, and every source new.
said=$(fresh-sources true 2>&1)
case $said in *"holds a build this did not record"*) ;; *) fail "a first run: $said" ;; esac
first=$(at src/main.rs)
[ "$(at sdk-roles/custody/pages.py)" = "$first" ] || fail "sdk-roles is not among the sources"
[ "$(at plugin-template/AGENTS.md)" = "$first" ] || fail "plugin-template is not among the sources"
[ "$(at build.rs)" = "$first" ] || fail "build.rs is not among the sources"

# Another checkout of the same content, its files newer: given back the time
# the cache first saw them, so the build in the cache is fresh for it.
sleep 1
touch src/main.rs build.rs
said=$(fresh-sources true 2>&1)
case $said in *"0 of 6 sources changed"*) ;; *) fail "the same content again: $said" ;; esac
[ "$(at src/main.rs)" = "$first" ] || fail "the same content was not given its first-seen time"

# A checkout with a change, its file older than the cache's last build: new
# as of now, so cargo builds it again.
echo 'fn main() { println!("two"); }' >src/main.rs
touch -d '@1000000000' src/main.rs
said=$(fresh-sources true 2>&1)
case $said in *"1 of 6 sources changed"*) ;; *) fail "a changed source: $said" ;; esac
[ "$(at src/main.rs)" -gt "$first" ] || fail "a changed source kept an old time"
[ "$(at build.rs)" = "$first" ] || fail "an unchanged source lost its first-seen time"

# A build the record did not see (a fingerprint newer than its end): every
# source new.
sleep 1
touch target/debug/.fingerprint/stranger
said=$(fresh-sources true 2>&1)
case $said in *"holds a build this did not record"*) ;; *) fail "a stranger's build: $said" ;; esac
[ "$(at build.rs)" -gt "$first" ] || fail "a stranger's build left a source its old time"

# The command's status is the script's, and nothing is said on stdout.
if fresh-sources false 2>/dev/null; then fail "a failed command passed"; fi
[ -z "$(fresh-sources true 2>/dev/null)" ] || fail "it said something on stdout"

echo "fresh-sources OK"

#!/bin/sh
# `fresh-sources <command...>`: the command (a cargo build) run with each
# workspace source's time set from what the shared target cache last saw of it.
#
# meridian-core's tools/fresh-sources.sh (its 7c239fbe, change B), given to the
# CLI by ruling 934: the same script, reading this crate's sources (`sources`,
# below) and guarding its target cache, meridian-cli-target. Core's account of
# why follows.
#
# The cargo target cache (the cache mount meridian-core-target) is one for
# every checkout on this machine: every worktree, every agent's clone. Cargo
# decides a workspace crate is fresh by time alone -- no source newer than
# what it built -- and a checkout's times are when it was checked out. So a
# worktree whose changed file was older than another worktree's last build of
# the same crate was given that other build, and the gate passed or failed on
# code that was not the branch's (change-a, 2026-10-30, two runs lost).
#
# So the times are set here, from content: target/.fresh-sources records each
# source's SHA-256 and when that content was first seen. A source whose content
# differs from the record is new as of now; every source is given the time its
# content was first seen. A crate built from other content was built before
# that time, and is built again; one built from this content was built after
# it, and is fresh, whichever checkout's times the files arrived with. Nothing
# is kept beyond the record, so the gate's disk is what it was.
#
# A build this did not wrap -- an older checkout's, or one stopped before it
# finished -- leaves units the record knows nothing of. One is found by a
# fingerprint newer than the end of the last wrapped build, and then every
# source counts as new: the workspace's own crates are built again (never
# their dependencies), which is always correct.
#
# Run in the workspace's root (/w), under a lock held until the command ends,
# so two builds sharing the cache cannot interleave a record and a compile.
# Says nothing on stdout, which a caller may be collecting from cargo.
set -eu

record=target/.fresh-sources
ended=target/.fresh-sources.ended
mkdir -p target
exec 9>target/.fresh-sources.lock
flock 9

now=$(date +%s.%N)
current=$(mktemp)
fresh=$(mktemp)
kept=$(mktemp)
trap 'rm -f "$current" "$fresh" "$kept"' EXIT

foreign=
if [ -f "$record" ] && [ -f "$ended" ]; then
	# The .fingerprint directories are cargo's own (target/<profile>/ and
	# target/<triple>/<profile>/), named without a space.
	# shellcheck disable=SC2044
	for prints in $(find target -maxdepth 3 -type d -name .fingerprint); do
		if [ -n "$(find "$prints" -newer "$ended" -print -quit)" ]; then
			foreign=yes
			break
		fi
	done
else
	foreign=yes
fi
if [ -n "$foreign" ]; then
	echo "fresh-sources: the target cache holds a build this did not record; the workspace's crates are built again" >&2
	: >"$record"
fi

# What the crate is built from: its manifest, lock and build script, its code,
# and what build.rs and include_str! compile into the binary (the scaffolds
# and the SDK's role modules), whose times cargo reads as it reads the code's.
sources="Cargo.toml Cargo.lock build.rs src plugin-template plugin-templates sdk-roles"
for path in $sources; do
	[ -e "$path" ] && find "$path" -type f
done | LC_ALL=C sort | tr '\n' '\0' | xargs -0 -r sha256sum >"$current"

# record: <sha256> TAB <first seen> TAB <path>; sha256sum: <sha256>, two
# spaces, <path>. Sources not in this build are kept as they were.
awk -v now="$now" -v kept="$kept" '
	FILENAME == ARGV[1] {
		split($0, f, "\t"); hash[f[3]] = f[1]; seen[f[3]] = f[2]; next
	}
	{
		h = substr($0, 1, 64); p = substr($0, 67); here[p] = 1
		t = (p in hash && (hash[p] "") == (h "")) ? seen[p] : now
		print h "\t" t "\t" p
	}
	END { for (p in hash) if (!(p in here)) print hash[p] "\t" seen[p] "\t" p >kept }
' "$record" "$current" >"$fresh"

changed=$(awk -F'\t' -v now="$now" '($2 "") == now' "$fresh" | wc -l | tr -d ' ')
cut -f2 "$fresh" | LC_ALL=C sort -u | while read -r at; do
	awk -F'\t' -v at="$at" '($2 "") == at { print $3 }' "$fresh" | tr '\n' '\0' | xargs -0 -r touch -m -d "@$at"
done
cat "$fresh" "$kept" >"$record"
echo "fresh-sources: $changed of $(wc -l <"$current" | tr -d ' ') sources changed since the target cache last saw them" >&2

set +e
"$@"
status=$?
set -e
touch "$ended"
exit "$status"

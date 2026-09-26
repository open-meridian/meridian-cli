#!/bin/sh
# Install meridian, the Open Meridian CLI (spec/the-cli, ruling 8):
#
#   curl -fsSL https://raw.githubusercontent.com/open-meridian/meridian-cli/main/install.sh | sh
#
# Downloads this machine's binary from the latest GitHub release, checks it
# against the .sha256 published beside it, and puts it in ~/.local/bin. No
# sudo, and nothing outside that directory. Afterwards `meridian upgrade`
# moves it to a newer release and `meridian uninstall` removes it.
#
# Settings, all optional:
#   MERIDIAN_INSTALL_DIR  where it goes (default: ~/.local/bin)
#   MERIDIAN_VERSION      a release, e.g. 0.2.0 (default: the latest)
#   MERIDIAN_RELEASES     a mirror of the releases, over https
#
# The checksum catches a broken download, not a compromised release: both
# come from the same place. Signing is a gap the spec names.
#
# Everything is inside main(), called on the last line, so a download cut off
# part way runs nothing.

set -eu

main() {
    releases="${MERIDIAN_RELEASES:-https://github.com/open-meridian/meridian-cli/releases}"
    releases="${releases%/}"
    dir="${MERIDIAN_INSTALL_DIR:-${HOME}/.local/bin}"

    case "$releases" in
        https://*) ;;
        http://127.0.0.1:* | http://localhost:* | http://127.0.0.1/* | http://localhost/*) ;;
        *) fail "MERIDIAN_RELEASES is ${releases}: releases come over https://" ;;
    esac
    command -v curl >/dev/null 2>&1 || fail "this needs curl"

    target="$(target_of "$(uname -s)" "$(uname -m)")"
    name="meridian-${target}"

    if [ -n "${MERIDIAN_VERSION:-}" ]; then
        tag="v${MERIDIAN_VERSION#v}"
    else
        # GitHub answers /releases/latest with a redirect to its tag: a page,
        # not the API, so no rate limit a shared address could run out of.
        landed="$(curl -fsSL -o /dev/null -w '%{url_effective}' "${releases}/latest")" ||
            fail "could not reach ${releases}"
        case "$landed" in
            */releases/tag/*) tag="${landed##*/releases/tag/}" ;;
            *) fail "${releases} has no release published yet" ;;
        esac
    fi

    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    say "Downloading meridian ${tag} for ${target}"
    curl -fsSL -o "${work}/${name}" "${releases}/download/${tag}/${name}" ||
        fail "there is no ${name} in ${tag}"
    curl -fsSL -o "${work}/${name}.sha256" "${releases}/download/${tag}/${name}.sha256" ||
        fail "${tag} has no checksum for ${name}"

    expected="$(awk '{ print $1; exit }' "${work}/${name}.sha256")"
    actual="$(sha256_of "${work}/${name}")"
    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        fail "${name} is not what its checksum says; nothing was installed"
    fi

    mkdir -p "$dir" || fail "could not make ${dir}"
    chmod 755 "${work}/${name}"
    # Beside it first, then renamed: never half a binary where meridian was.
    mv "${work}/${name}" "${dir}/.meridian-install-$$" ||
        fail "could not write ${dir}; set MERIDIAN_INSTALL_DIR to a directory you own"
    mv "${dir}/.meridian-install-$$" "${dir}/meridian"

    say "Installed $("${dir}/meridian" --version) at ${dir}/meridian"
    case ":${PATH}:" in
        *":${dir}:"*) ;;
        *) say "${dir} is not on your PATH. Add it, in your shell's profile:
  export PATH=\"${dir}:\$PATH\"" ;;
    esac
}

# The release binary for an OS and a machine, as `uname` names them.
target_of() {
    case "$1/$2" in
        Linux/x86_64 | Linux/amd64) echo x86_64-unknown-linux-musl ;;
        Linux/aarch64 | Linux/arm64) echo aarch64-unknown-linux-musl ;;
        Darwin/arm64) echo aarch64-apple-darwin ;;
        Darwin/x86_64)
            # A shell under Rosetta says x86_64 on Apple silicon; the native
            # binary is the one to have.
            if [ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ]; then
                echo aarch64-apple-darwin
            else
                echo x86_64-apple-darwin
            fi
            ;;
        *) fail "there is no meridian built for $1 on $2. There are: Linux on x86_64 and arm64, macOS on Apple silicon and Intel" ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        fail "this needs sha256sum or shasum to check the download"
    fi
}

say() {
    printf '%s\n' "$*"
}

fail() {
    printf 'meridian install: %s\n' "$*" >&2
    exit 1
}

main "$@"

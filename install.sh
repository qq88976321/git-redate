#!/bin/sh
# git-redate installer.
#
# Downloads the prebuilt binary for this platform from a GitHub release,
# verifies its sha256 checksum, and installs it. Once the binary is on
# PATH, git dispatches `git redate ...` to it as a subcommand.
#
# The canonical one-liner:
#
#   curl -fsSL https://github.com/qq88976321/git-redate/releases/latest/download/install.sh | sh
#
# This script is published as an asset of every release, so the copy you
# run always knows the asset layout of the release it shipped with.
#
# Everything lives inside main(), which is called on the very last line:
# a truncated download therefore cannot execute a partial script.

set -eu

REPO="qq88976321/git-redate"
BIN="git-redate"
DEFAULT_BASE_URL="https://github.com/qq88976321/git-redate"
DEFAULT_INSTALL_DIR="${HOME}/.local/bin"

say() {
    printf '%s\n' "$*"
}

err() {
    printf 'install.sh: %s\n' "$*" >&2
}

die() {
    err "$*"
    exit 1
}

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

usage() {
    cat <<EOF
Install the ${BIN} binary for this platform.

USAGE:
    install.sh [OPTIONS]

OPTIONS:
    --version <tag>   Install this release instead of the latest (e.g. v0.1.0)
    --to <dir>        Install into <dir> instead of ${DEFAULT_INSTALL_DIR}
    -h, --help        Print this help

ENVIRONMENT:
    GIT_REDATE_VERSION       Same as --version
    GIT_REDATE_INSTALL_DIR   Same as --to
    GIT_REDATE_BASE_URL      Release download root (default
                             ${DEFAULT_BASE_URL});
                             used to test this installer against a local
                             file:// tree

Flags take precedence over the environment.

Rather read the script before running it? Fetch it first:

    curl -fsSL https://github.com/${REPO}/releases/latest/download/install.sh -o install.sh
    less install.sh
    sh install.sh
EOF
}

# Sets TARGET to the rust target triple whose release asset fits this host.
detect_target() {
    kernel=$(uname -s)
    machine=$(uname -m)

    case "${machine}" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) die "unsupported architecture: ${machine} (supported: x86_64, aarch64)" ;;
    esac

    case "${kernel}" in
        # The Linux builds are statically linked against musl, so one binary
        # per architecture runs on every distribution whatever its glibc
        # version is. That is why no libc detection happens here.
        Linux) TARGET="${arch}-unknown-linux-musl" ;;
        Darwin) TARGET="${arch}-apple-darwin" ;;
        *) die "unsupported operating system: ${kernel} (supported: Linux, Darwin)" ;;
    esac
}

# curl_to <url> <dest>
curl_to() {
    if [ -n "${HTTPS_ONLY}" ]; then
        curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 -o "$2" "$1"
    else
        curl -fsSL -o "$2" "$1"
    fi
}

# Sets VERSION from the tag that `releases/latest` redirects to. Uses the
# redirect rather than the REST API, which is rate limited per IP.
resolve_latest_version() {
    latest_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "${BASE_URL}/releases/latest") ||
        die "could not reach ${BASE_URL}/releases/latest (pass --version <tag> to skip this lookup)"
    VERSION="${latest_url##*/}"
    case "${VERSION}" in
        v[0-9]*) ;;
        *) die "could not read a version tag out of ${latest_url}; pass --version <tag>" ;;
    esac
}

# verify_checksum <dir> <sidecar name>
#
# The sidecar is named `<archive>.sha256`, not `<archive>.tar.gz.sha256`, and
# its single line names the .tar.gz without a path, so the check has to run
# from the directory holding both files.
verify_checksum() {
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$1" && sha256sum -c "$2" >/dev/null 2>&1) ||
            die "checksum verification failed for ${ARCHIVE}.tar.gz"
    else
        (cd "$1" && shasum -a 256 -c "$2" >/dev/null 2>&1) ||
            die "checksum verification failed for ${ARCHIVE}.tar.gz"
    fi
}

# install_binary <path to the extracted binary>
install_binary() {
    mkdir -p "${INSTALL_DIR}" || die "could not create ${INSTALL_DIR}"

    staged="${INSTALL_DIR}/.${BIN}.tmp.$$"
    cp "$1" "${staged}" || die "could not write to ${INSTALL_DIR}"
    chmod 755 "${staged}"

    # One rename into place: atomic on the destination filesystem, and it
    # replaces a copy that is currently running without "text file busy".
    mv -f "${staged}" "${INSTALL_DIR}/${BIN}" || {
        rm -f "${staged}"
        die "could not install into ${INSTALL_DIR}"
    }
}

# Warns about the two ways a successful install can still look broken.
report_path() {
    case ":${PATH}:" in
        *":${INSTALL_DIR}:"*)
            shadow=$(command -v "${BIN}" 2>/dev/null || true)
            if [ -n "${shadow}" ] && [ "${shadow}" != "${INSTALL_DIR}/${BIN}" ]; then
                say ""
                say "NOTE: ${shadow} comes earlier on your PATH and will shadow this"
                say "      install. Remove it, or reorder your PATH."
            fi
            ;;
        *)
            say ""
            say "NOTE: ${INSTALL_DIR} is not on your PATH, so git cannot find the"
            say "      subcommand yet. Add it to your shell profile:"
            say ""
            say "    export PATH=\"${INSTALL_DIR}:\$PATH\""
            ;;
    esac
}

main() {
    VERSION="${GIT_REDATE_VERSION:-}"
    INSTALL_DIR="${GIT_REDATE_INSTALL_DIR:-${DEFAULT_INSTALL_DIR}}"
    BASE_URL="${GIT_REDATE_BASE_URL:-${DEFAULT_BASE_URL}}"

    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                [ $# -ge 2 ] || die "--version requires a tag, e.g. --version v0.1.0"
                VERSION="$2"
                shift 2
                ;;
            --version=*)
                VERSION="${1#*=}"
                shift
                ;;
            --to)
                [ $# -ge 2 ] || die "--to requires a directory"
                INSTALL_DIR="$2"
                shift 2
                ;;
            --to=*)
                INSTALL_DIR="${1#*=}"
                shift
                ;;
            -h | --help)
                usage
                return 0
                ;;
            *)
                die "unknown option: $1 (try --help)"
                ;;
        esac
    done

    [ -n "${INSTALL_DIR}" ] || die "--to requires a non-empty directory"

    need_cmd curl
    need_cmd tar
    need_cmd uname
    command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 ||
        die "need sha256sum or shasum to verify the download"

    # Only restrict the transport for the real, https download root; the
    # file:// root used by the installer's own test would be rejected.
    HTTPS_ONLY=""
    case "${BASE_URL}" in
        https://*) HTTPS_ONLY=1 ;;
    esac

    detect_target
    [ -n "${VERSION}" ] || resolve_latest_version

    ARCHIVE="${BIN}-${TARGET}"
    archive_url="${BASE_URL}/releases/download/${VERSION}/${ARCHIVE}.tar.gz"
    sha_url="${BASE_URL}/releases/download/${VERSION}/${ARCHIVE}.sha256"

    workdir=$(mktemp -d 2>/dev/null || mktemp -d -t git-redate) ||
        die "could not create a temporary directory"
    trap 'rm -rf "${workdir}"' EXIT INT TERM

    say "downloading ${BIN} ${VERSION} for ${TARGET}"
    curl_to "${archive_url}" "${workdir}/${ARCHIVE}.tar.gz" ||
        die "download failed: ${archive_url}
  ${VERSION} may not ship an asset for ${TARGET}; see ${BASE_URL}/releases"
    curl_to "${sha_url}" "${workdir}/${ARCHIVE}.sha256" ||
        die "download failed: ${sha_url}"

    verify_checksum "${workdir}" "${ARCHIVE}.sha256"

    tar -xzf "${workdir}/${ARCHIVE}.tar.gz" -C "${workdir}" ||
        die "could not extract ${ARCHIVE}.tar.gz"
    [ -f "${workdir}/${BIN}" ] ||
        die "${ARCHIVE}.tar.gz does not contain ${BIN}"

    install_binary "${workdir}/${BIN}"

    installed=$("${INSTALL_DIR}/${BIN}" --version 2>/dev/null) ||
        die "installed ${INSTALL_DIR}/${BIN} but it does not run on this machine"
    say "installed ${installed} to ${INSTALL_DIR}/${BIN}"

    report_path

    say ""
    say "Try it:  git redate --help"
}

main "$@"

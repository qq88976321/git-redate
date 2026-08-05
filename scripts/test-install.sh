#!/bin/sh
# Offline round trip for install.sh.
#
# CI cannot exercise the install path before a release exists, and even
# after that it would depend on GitHub being reachable. So this builds the
# real musl asset, lays out a release tree exactly as
# taiki-e/upload-rust-binary-action names it, points install.sh at that tree
# over file://, and asserts the three outcomes that matter: a clean install,
# a refused tampered archive, and a clear error on a platform with no asset.
#
# Run with `just test-install`. Needs the x86_64-unknown-linux-musl target
# (`rustup target add x86_64-unknown-linux-musl`).

set -eu

TARGET="x86_64-unknown-linux-musl"
TAG="v0.0.0-test"
BIN="git-redate"
ARCHIVE="${BIN}-${TARGET}"

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "${root}"

work=$(mktemp -d)
trap 'rm -rf "${work}"' EXIT INT TERM

failures=0

pass() {
    printf 'ok   - %s\n' "$1"
}

fail() {
    printf 'FAIL - %s\n' "$1" >&2
    failures=$((failures + 1))
}

# lay_out_release <base dir> - build a fake release download tree.
lay_out_release() {
    dir="$1/releases/download/${TAG}"
    mkdir -p "${dir}"
    cp "target/${TARGET}/release/${BIN}" "${dir}/${BIN}"
    (
        cd "${dir}"
        tar -czf "${ARCHIVE}.tar.gz" "${BIN}"
        rm -f "${BIN}"
        # Same shape the action produces: the sidecar is `<archive>.sha256`,
        # not `<archive>.tar.gz.sha256`, and it names the tarball with no path.
        sha256sum "${ARCHIVE}.tar.gz" >"${ARCHIVE}.sha256"
    )
}

printf 'building the %s asset\n' "${TARGET}"
cargo build --release --target "${TARGET}" --locked

crate_version=$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)

# --- case 1: a clean install into a directory that does not exist yet ------
lay_out_release "${work}/good"
out=$(
    GIT_REDATE_BASE_URL="file://${work}/good" \
        sh install.sh --version "${TAG}" --to "${work}/bin/nested" 2>&1
) || fail "case 1: install.sh exited non-zero: ${out}"

if [ -x "${work}/bin/nested/${BIN}" ]; then
    pass "case 1: binary installed and executable"
else
    fail "case 1: ${work}/bin/nested/${BIN} is missing or not executable"
fi

reported=$("${work}/bin/nested/${BIN}" --version)
if [ "${reported}" = "${BIN} ${crate_version}" ]; then
    pass "case 1: installed binary reports '${reported}'"
else
    fail "case 1: expected '${BIN} ${crate_version}', got '${reported}'"
fi

case "${out}" in
    *"is not on your PATH"*) pass "case 1: warns that the install dir is not on PATH" ;;
    *) fail "case 1: missing the PATH warning, got: ${out}" ;;
esac

# --- case 2: a tampered archive must be refused ----------------------------
lay_out_release "${work}/tampered"
printf 'tampered' >>"${work}/tampered/releases/download/${TAG}/${ARCHIVE}.tar.gz"

if out=$(
    GIT_REDATE_BASE_URL="file://${work}/tampered" \
        sh install.sh --version "${TAG}" --to "${work}/bin-tampered" 2>&1
); then
    fail "case 2: install.sh succeeded on a tampered archive"
else
    case "${out}" in
        *"checksum verification failed"*) pass "case 2: tampered archive refused" ;;
        *) fail "case 2: aborted but not on the checksum, got: ${out}" ;;
    esac
fi

if [ -e "${work}/bin-tampered/${BIN}" ]; then
    fail "case 2: a binary was installed despite the checksum mismatch"
else
    pass "case 2: nothing was installed"
fi

# --- case 3: platform mapping, via a uname that claims to be an arm Mac ----
mkdir -p "${work}/shim"
cat >"${work}/shim/uname" <<'SHIM'
#!/bin/sh
case "$1" in
    -s) echo Darwin ;;
    -m) echo arm64 ;;
    *) echo Darwin ;;
esac
SHIM
chmod 755 "${work}/shim/uname"

if out=$(
    PATH="${work}/shim:${PATH}" GIT_REDATE_BASE_URL="file://${work}/good" \
        sh install.sh --version "${TAG}" --to "${work}/bin-darwin" 2>&1
); then
    fail "case 3: install.sh succeeded with no matching asset"
else
    case "${out}" in
        *"${BIN}-aarch64-apple-darwin.tar.gz"*)
            pass "case 3: resolved the arm64 macOS asset name and reported it"
            ;;
        *) fail "case 3: error did not name the darwin asset, got: ${out}" ;;
    esac
fi

# --- case 4: an unsupported kernel is rejected before any download ---------
cat >"${work}/shim/uname" <<'SHIM'
#!/bin/sh
case "$1" in
    -s) echo Plan9 ;;
    -m) echo x86_64 ;;
    *) echo Plan9 ;;
esac
SHIM

if out=$(
    PATH="${work}/shim:${PATH}" GIT_REDATE_BASE_URL="file://${work}/good" \
        sh install.sh --version "${TAG}" --to "${work}/bin-plan9" 2>&1
); then
    fail "case 4: install.sh succeeded on an unsupported kernel"
else
    case "${out}" in
        *"unsupported operating system: Plan9"*) pass "case 4: unsupported kernel rejected" ;;
        *) fail "case 4: wrong error, got: ${out}" ;;
    esac
fi

printf '\n'
if [ "${failures}" -eq 0 ]; then
    printf 'install.sh round trip: all checks passed\n'
else
    printf 'install.sh round trip: %s check(s) failed\n' "${failures}" >&2
    exit 1
fi

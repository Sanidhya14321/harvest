#!/bin/sh
# shellcheck disable=SC2016,SC2310,SC2312 # the harness embeds scripts and reads substitutions on purpose

set -eu

ROOT=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd)
INSTALLER=${ROOT}/install.sh
TEST_ROOT=$(mktemp -d 2>/dev/null || mktemp -d -t omp-tests)
FIXTURES=${TEST_ROOT}/fixtures
FAKE_BIN=${TEST_ROOT}/bin
DOWNLOAD_LOG=${TEST_ROOT}/downloads.log
ORIGINAL_PATH=${PATH}
ROOT_INSTALLATION=

cleanup() {
    if [ -n "${ROOT_INSTALLATION}" ]; then
        rm -f /usr/local/bin/omp /usr/local/bin/harvest
    fi
    rm -rf "${TEST_ROOT}"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "${FIXTURES}" "${FAKE_BIN}"
: >"${DOWNLOAD_LOG}"

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

assert_contains() {
    needle=$1
    file=$2
    if ! grep -F "${needle}" "${file}" >/dev/null 2>&1; then
        cat "${file}" >&2
        fail "expected '${needle}' in ${file}"
    fi
}

assert_equals() {
    expected=$1
    actual=$2
    [ "${expected}" = "${actual}" ] || fail "expected '${expected}', got '${actual}'"
}

sha256() {
    file=$1
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "${file}" | awk '{print $1}'
    else
        shasum -a 256 "${file}" | awk '{print $1}'
    fi
}

prepare_release() {
    asset=$1
    contents=$2
    printf '%s' "${contents}" >"${FIXTURES}/${asset}"
    printf '%s  dist/%s\n' "$(sha256 "${FIXTURES}/${asset}")" "${asset}" >"${FIXTURES}/SHA256SUMS.txt"
}

cat >"${FAKE_BIN}/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
    -s) printf '%s\n' "$OMP_TEST_OS" ;;
    -m) printf '%s\n' "$OMP_TEST_ARCH" ;;
    *) printf '%s %s\n' "$OMP_TEST_OS" "$OMP_TEST_ARCH" ;;
esac
EOF

cat >"${FAKE_BIN}/curl" <<'EOF'
#!/bin/sh
output=
url=
effective=0
timeout=0
speed_limit=0
speed_time=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        -o | --output)
            output=$2
            shift 2
            ;;
        --max-time)
            [ "$2" = 30 ] || exit 2
            timeout=1
            shift 2
            ;;
        --speed-limit)
            [ "$2" = 1000 ] || exit 2
            speed_limit=1
            shift 2
            ;;
        --speed-time)
            [ "$2" = 30 ] || exit 2
            speed_time=1
            shift 2
            ;;
        -w | --write-out)
            [ "$2" = '%{url_effective}' ] || exit 2
            effective=1
            shift 2
            ;;
        http://* | https://*)
            url=$1
            shift
            ;;
        *)
            shift
            ;;
    esac
done
[ -n "$output" ] && [ -n "$url" ] || exit 2
if [ "$effective" = 1 ]; then
    [ "$timeout" = 1 ] || exit 2
else
    [ "$speed_limit" = 1 ] && [ "$speed_time" = 1 ] || exit 2
fi
printf '%s\n' "$url" >>"$OMP_TEST_DOWNLOAD_LOG"
if [ "$effective" = 1 ]; then
    [ "$url" = https://github.com/Sanidhya14321/harvest/releases/latest ] || exit 2
    printf '%s' "${OMP_TEST_LATEST_URL-https://github.com/Sanidhya14321/harvest/releases/tag/v1.2.3}"
else
    case "$url" in
        */latest/download/*) exit 2 ;;
    esac
    cp "$OMP_TEST_FIXTURES/${url##*/}" "$output"
fi
EOF
cat >"${FAKE_BIN}/ldd" <<'EOF'
#!/bin/sh
printf '%s\n' "${OMP_TEST_LIBC:-glibc}"
EOF
cat >"${FAKE_BIN}/sysctl" <<'EOF'
#!/bin/sh
printf '%s\n' "${OMP_TEST_TRANSLATED:-0}"
EOF
chmod +x "${FAKE_BIN}/ldd" "${FAKE_BIN}/sysctl"
chmod +x "${FAKE_BIN}/uname" "${FAKE_BIN}/curl"

run_installer() {
    test_home=$1
    test_os=$2
    test_arch=$3
    shift 3
    mkdir -p "${test_home}"
    env \
        HOME="${test_home}" \
        PATH="${FAKE_BIN}:${ORIGINAL_PATH}" \
        SHELL="${OMP_TEST_SHELL-/bin/sh}" \
        OMP_TEST_OS="${test_os}" \
        OMP_TEST_ARCH="${test_arch}" \
        OMP_TEST_FIXTURES="${FIXTURES}" \
        OMP_TEST_DOWNLOAD_LOG="${DOWNLOAD_LOG}" \
        OMP_NO_MODIFY_PATH=1 \
        sh "${INSTALLER}" "$@"
}

printf 'test: installs a pinned Linux x86_64 release\n'
case_dir=${TEST_ROOT}/linux-x86
bin_dir=${case_dir}/bin
prepare_release omp-linux-x64 'linux x86 binary'
run_installer "${case_dir}/home" Linux x86_64 --version 1.2.3 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1
assert_equals 'linux x86 binary' "$(cat "${bin_dir}/omp")"
[ -x "${bin_dir}/omp" ] || fail "installed Unix binary is not executable"
assert_contains 'https://github.com/Sanidhya14321/harvest/releases/download/v1.2.3/omp-linux-x64' "${DOWNLOAD_LOG}"
assert_contains 'verified SHA-256 checksum' "${case_dir}.out"

printf 'test: selects the latest macOS Apple Silicon release\n'
case_dir=${TEST_ROOT}/macos-arm
bin_dir=${case_dir}/bin
prepare_release omp-darwin-arm64 'macOS ARM binary'
before=$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')
run_installer "${case_dir}/home" Darwin arm64 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1
assert_equals 'macOS ARM binary' "$(cat "${bin_dir}/omp")"
assert_contains 'https://github.com/Sanidhya14321/harvest/releases/download/v1.2.3/omp-darwin-arm64' "${DOWNLOAD_LOG}"
assert_equals "$((before + 3))" "$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')"

printf 'test: selects the Linux ARM64 release\n'
case_dir=${TEST_ROOT}/linux-arm
bin_dir=${case_dir}/bin
prepare_release omp-linux-arm64 'Linux ARM binary'
run_installer "${case_dir}/home" Linux aarch64 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1
assert_equals 'Linux ARM binary' "$(cat "${bin_dir}/omp")"
assert_contains 'https://github.com/Sanidhya14321/harvest/releases/download/v1.2.3/omp-linux-arm64' "${DOWNLOAD_LOG}"

printf 'test: selects native musl, Rosetta and Windows ARM64 assets\n'
for routing in 'Linux x86_64 musl 0 omp-linux-musl-x64 omp' 'Linux aarch64 musl 0 omp-linux-musl-arm64 omp' 'Darwin x86_64 glibc 1 omp-darwin-arm64 omp' 'MINGW64_NT aarch64 glibc 0 omp-windows-arm64.exe omp.exe'; do
    set -- ${routing}
    case_dir=${TEST_ROOT}/route-$5
    prepare_release "$5" "binary for $5"
    OMP_TEST_LIBC=$3 OMP_TEST_TRANSLATED=$4 run_installer "${case_dir}/home" "$1" "$2" --version 1.2.3 --bin-dir "${case_dir}/bin" >"${case_dir}.out" 2>&1
    assert_equals "binary for $5" "$(cat "${case_dir}/bin/$6")"
    assert_contains "releases/download/v1.2.3/$5" "${DOWNLOAD_LOG}"
done

printf 'test: installs and upgrades both aliases with an unsupported shell\n'
case_dir=${TEST_ROOT}/alias-upgrade
bin_dir=${case_dir}/bin
prepare_release omp-linux-x64 'first binary'
run_installer "${case_dir}/home" Linux x86_64 --version 1.2.3 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1
assert_equals 'first binary' "$(cat "${bin_dir}/harvest")"
prepare_release omp-linux-x64 'second binary'
run_installer "${case_dir}/home" Linux x86_64 --version 1.2.3 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1
assert_equals 'second binary' "$(cat "${bin_dir}/omp")"
assert_equals 'second binary' "$(cat "${bin_dir}/harvest")"

printf 'test: an unrelated harvest alias prevents replacing either command\n'
printf 'user-owned command' >"${bin_dir}/different"
mv "${bin_dir}/different" "${bin_dir}/harvest"
prepare_release omp-linux-x64 'third binary'
if run_installer "${case_dir}/home" Linux x86_64 --version 1.2.3 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1; then
    fail 'unrelated harvest alias was overwritten'
fi
assert_equals 'second binary' "$(cat "${bin_dir}/omp")"
assert_equals 'user-owned command' "$(cat "${bin_dir}/harvest")"

printf 'test: rejects invalid release redirects before fetching assets\n'
for latest_url in 'https://example.com/releases/tag/v1.2.3' 'https://github.com/Sanidhya14321/harvest/releases/tag/v1.2.3-beta.1'; do
    before=$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')
    if OMP_TEST_LATEST_URL="${latest_url}" run_installer "${TEST_ROOT}/invalid-latest/home" Linux x86_64 --bin-dir "${TEST_ROOT}/invalid-latest/bin" >"${TEST_ROOT}/invalid-latest.out" 2>&1; then
        fail "invalid latest redirect unexpectedly succeeded"
    fi
    assert_equals "$((before + 1))" "$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')"
    assert_contains 'invalid latest release' "${TEST_ROOT}/invalid-latest.out"
done
unset OMP_TEST_LATEST_URL

printf 'test: rejects invalid checksum records before fetching the binary\n'
case_dir=${TEST_ROOT}/invalid-checksum
mkdir -p "${case_dir}/bin"
printf 'existing binary' >"${case_dir}/bin/omp"
for checksum_case in duplicate missing malformed; do
    prepare_release omp-linux-x64 'new binary'
    case "${checksum_case}" in
        duplicate)
            cp "${FIXTURES}/SHA256SUMS.txt" "${FIXTURES}/duplicate"
            cat "${FIXTURES}/duplicate" >>"${FIXTURES}/SHA256SUMS.txt"
            ;;
        missing) printf '%064d  omp-linux-arm64\n' 0 >"${FIXTURES}/SHA256SUMS.txt" ;;
        malformed) printf 'invalid  omp-linux-x64\n' >"${FIXTURES}/SHA256SUMS.txt" ;;
        *) fail "unsupported checksum fixture" ;;
    esac
    before=$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')
    if run_installer "${case_dir}/home" Linux x86_64 --version 1.2.3 --bin-dir "${case_dir}/bin" >"${case_dir}.out" 2>&1; then
        fail "${checksum_case} checksum unexpectedly succeeded"
    fi
    assert_equals "$((before + 1))" "$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')"
    assert_equals 'existing binary' "$(cat "${case_dir}/bin/omp")"
    assert_contains 'release checksum' "${case_dir}.out"
done

printf 'test: rejects a checksum mismatch without replacing an installation\n'
case_dir=${TEST_ROOT}/bad-checksum
bin_dir=${case_dir}/bin
mkdir -p "${bin_dir}"
printf 'existing binary' >"${bin_dir}/omp"
printf 'tampered binary' >"${FIXTURES}/omp-linux-x64"
printf '%064d  omp-linux-x64\n' 0 >"${FIXTURES}/SHA256SUMS.txt"
if run_installer "${case_dir}/home" Linux x86_64 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1; then
    fail "checksum mismatch unexpectedly succeeded"
fi
assert_equals 'existing binary' "$(cat "${bin_dir}/omp")"
assert_contains 'checksum verification failed' "${case_dir}.out"

printf 'test: installs aliases without executing downloaded completion code\n'
case_dir=${TEST_ROOT}/no-completions
home_dir=${case_dir}/home
bin_dir=${case_dir}/bin
prepare_release omp-linux-x64 '#!/bin/sh
exit 99'
OMP_TEST_SHELL=/bin/bash run_installer "${home_dir}" Linux x86_64 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1
[ -f "${bin_dir}/harvest" ] || fail "harvest alias was not installed"
[ ! -e "${home_dir}/.local/share/bash-completion" ] || fail "retired completions were installed"

printf 'test: installs harvest when SHELL is empty\n'
case_dir=${TEST_ROOT}/empty-shell
home_dir=${case_dir}/home
bin_dir=${case_dir}/bin
prepare_release omp-linux-x64 '#!/bin/sh
exit 99'
OMP_TEST_SHELL='' run_installer "${home_dir}" Linux x86_64 --bin-dir "${bin_dir}" >"${case_dir}.out" 2>&1
[ -f "${bin_dir}/harvest" ] || fail "harvest was not installed when SHELL was empty"
[ ! -e "${home_dir}/.local/share/bash-completion" ] || fail "retired completions were installed"

printf 'test: updates a shell profile once for the default bin directory\n'
case_dir=${TEST_ROOT}/path-update
home_dir=${case_dir}/home
prepare_release omp-linux-x64 '#!/bin/sh
exit 0'
mkdir -p "${home_dir}"
for run in 1 2; do
    env \
        HOME="${home_dir}" \
        PATH="${FAKE_BIN}:${ORIGINAL_PATH}" \
        SHELL=/bin/zsh \
        OMP_TEST_OS=Linux \
        OMP_TEST_ARCH=x86_64 \
        OMP_TEST_FIXTURES="${FIXTURES}" \
        OMP_TEST_DOWNLOAD_LOG="${DOWNLOAD_LOG}" \
        sh "${INSTALLER}" --bin-dir "${home_dir}/.local/bin" >"${case_dir}.${run}.out" 2>&1
done
assert_equals '1' "$(grep -c "^export PATH=\"\\\$HOME/.local/bin:\\\$PATH\"\$" "${home_dir}/.zshrc")"
assert_contains 'Added by the Omp installer' "${home_dir}/.zshrc"

printf 'test: rejects unsupported systems before downloading\n'
case_dir=${TEST_ROOT}/unsupported
before=$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')
if run_installer "${case_dir}/home" FreeBSD x86_64 --bin-dir "${case_dir}/bin" >"${case_dir}.out" 2>&1; then
    fail "unsupported operating system unexpectedly succeeded"
fi
after=$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')
assert_equals "${before}" "${after}"
assert_contains 'unsupported operating system: FreeBSD' "${case_dir}.out"

printf 'test: rejects unsafe version values before downloading\n'
case_dir=${TEST_ROOT}/invalid-version
before=$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')
if run_installer "${case_dir}/home" Linux x86_64 --version 'v1/../../invalid' --bin-dir "${case_dir}/bin" >"${case_dir}.out" 2>&1; then
    fail "invalid release version unexpectedly succeeded"
fi
after=$(wc -l <"${DOWNLOAD_LOG}" | tr -d ' ')
assert_equals "${before}" "${after}"
assert_contains 'invalid release version' "${case_dir}.out"

if [ "${OMP_TEST_ROOT_DEFAULT:-0}" = 1 ]; then
    printf 'test: root installation is immediately available on PATH\n'
    assert_equals '0' "$(id -u)"
    [ ! -e /usr/local/bin/omp ] || fail "/usr/local/bin/omp already exists"
    [ ! -e /usr/local/bin/harvest ] || fail "/usr/local/bin/harvest already exists"
    case_dir=${TEST_ROOT}/root-default
    home_dir=${case_dir}/home
    prepare_release omp-linux-x64 '#!/bin/sh
set -eu
case "$1" in
    completions)
        [ "$2" = bash ] && [ "$3" = --install ] && [ "$4" = --automatic ]

        ;;
    --version) printf "omp test\\n" ;;
    *) exit 2 ;;
esac'
    ROOT_INSTALLATION=1
    env \
        HOME="${home_dir}" \
        PATH="/usr/local/bin:${FAKE_BIN}:${ORIGINAL_PATH}" \
        SHELL= \
        OMP_TEST_OS=Linux \
        OMP_TEST_ARCH=x86_64 \
        OMP_TEST_FIXTURES="${FIXTURES}" \
        OMP_TEST_DOWNLOAD_LOG="${DOWNLOAD_LOG}" \
        sh "${INSTALLER}" >"${case_dir}.out" 2>&1
    assert_contains 'installed to /usr/local/bin/omp' "${case_dir}.out"
    assert_equals 'omp test' "$(PATH=/usr/local/bin omp --version)"
    assert_equals 'omp test' "$(PATH=/usr/local/bin harvest --version)"
fi

if [ -n "${OMP_TEST_BINARY:-}" ]; then
    printf 'test: installed candidate rejects completion installation without bootstrap\n'
    case_dir=${TEST_ROOT}/retired-bootstrap
    mkdir -p "${case_dir}/bin"
    cp "${OMP_TEST_BINARY}" "${case_dir}/bin/omp"
    if env HOME="${case_dir}/home" XDG_DATA_HOME="${case_dir}/data" XDG_CONFIG_HOME="${case_dir}/config" \
        XDG_STATE_HOME="${case_dir}/state" SHELL=/bin/fish PATH="${case_dir}/bin:${ORIGINAL_PATH}" \
        "${case_dir}/bin/omp" completions bash --install --automatic >"${case_dir}/stdout" 2>"${case_dir}/stderr"; then
        fail "retired completions unexpectedly succeeded"
    fi
    [ ! -s "${case_dir}/stdout" ] || fail "retired command emitted a script"
    assert_contains 'retired' "${case_dir}/stderr"
    [ ! -e "${case_dir}/data" ] || fail "retired command created completion data"
    [ ! -e "${case_dir}/config" ] || fail "retired command created config"
    [ ! -e "${case_dir}/bin/harvest" ] || fail "retired command created a wrapper"
    printf 'test: version does not bootstrap shell files\n'
    env HOME="${case_dir}/first-home" XDG_DATA_HOME="${case_dir}/first-data" XDG_CONFIG_HOME="${case_dir}/first-config" \
        XDG_STATE_HOME="${case_dir}/first-state" SHELL=/bin/bash PATH="${case_dir}/bin:${ORIGINAL_PATH}" \
        "${case_dir}/bin/omp" --version >/dev/null
    [ ! -e "${case_dir}/first-data" ] || fail "version bootstrapped retired completions"
fi

printf 'All shell installer tests passed.\n'

#!/bin/sh
# Run real provider/tool/session contracts under the oldest promised GNU libc.
set -eu

if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo "usage: sh rust-agent/tests/portable-gnu.sh SHIPPING_BINARY" >&2
  exit 2
fi

HARVEST_PORTABLE_DOCKER=$(command -v docker)
# Architecture manifests from the pinned CentOS 7 multi-platform index
# sha256:be65f488b7764ad3638f236b7b515b3678369a5124c47b8d32916d6487418ea4.
case "${HARVEST_PORTABLE_PLATFORM:-$(uname -m)}" in
  x86_64|amd64|linux/amd64)
    HARVEST_PORTABLE_PLATFORM=linux/amd64
    HARVEST_PORTABLE_IMAGE=centos@sha256:dead07b4d8ed7e29e98de0f4504d87e8880d4347859d839686a31da35a3b532f
    ;;
  aarch64|arm64|linux/arm64)
    HARVEST_PORTABLE_PLATFORM=linux/arm64
    HARVEST_PORTABLE_IMAGE=centos@sha256:73f11afcbb50d8bc70eab9f0850b3fa30e61a419bc48cf426e63527d14a8373b
    ;;
  *) echo "unsupported GNU runtime platform" >&2; exit 2 ;;
esac
if [ -n "${HARVEST_PORTABLE_EMULATOR:-}" ] && [ ! -f "$HARVEST_PORTABLE_EMULATOR" ]; then
  echo "the requested user-mode emulator must be a regular file" >&2
  exit 2
fi
HARVEST_PORTABLE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/omp-portable.XXXXXX")
HARVEST_PORTABLE_USER="$(id -u):$(id -g)"
export HARVEST_PORTABLE_DOCKER HARVEST_PORTABLE_IMAGE HARVEST_PORTABLE_ROOT HARVEST_PORTABLE_USER
export HARVEST_PORTABLE_PLATFORM
trap 'rm -rf "$HARVEST_PORTABLE_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

cp "$1" "$HARVEST_PORTABLE_ROOT/omp"
mkdir "$HARVEST_PORTABLE_ROOT/work"
if [ -n "${HARVEST_PORTABLE_EMULATOR:-}" ]; then
  cp "$HARVEST_PORTABLE_EMULATOR" "$HARVEST_PORTABLE_ROOT/emulator"
  chmod 755 "$HARVEST_PORTABLE_ROOT/emulator"
fi

cat > "$HARVEST_PORTABLE_ROOT/guest" <<'GUEST'
#!/bin/sh
# Host networking lets the isolated binary reach the test's loopback-only model.
# Only this disposable fixture directory is mounted into the old runtime.
case "$(pwd -P)" in
  "$HARVEST_PORTABLE_ROOT"/*) workdir=$(pwd -P) ;;
  *) workdir="$HARVEST_PORTABLE_ROOT/work" ;;
esac
if [ -f "$HARVEST_PORTABLE_ROOT/emulator" ]; then
  set -- "$HARVEST_PORTABLE_ROOT/emulator" "$@"
fi
exec "$HARVEST_PORTABLE_DOCKER" run --rm --platform "$HARVEST_PORTABLE_PLATFORM" \
  --network "${HARVEST_PORTABLE_NETWORK:-host}" --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  --user "$HARVEST_PORTABLE_USER" \
  --mount "type=bind,src=$HARVEST_PORTABLE_ROOT,dst=$HARVEST_PORTABLE_ROOT" \
  --workdir "$workdir" \
  -e PATH=/nonexistent -e OMP_SKIP_INTEGRATIONS -e HOME \
  -e HARVEST_CODING_AGENT_DIR -e HARVEST_AGENT_DIR \
  -e XDG_DATA_HOME -e XDG_STATE_HOME -e USERPROFILE \
  "$HARVEST_PORTABLE_IMAGE" "$@"
GUEST
cat > "$HARVEST_PORTABLE_ROOT/run" <<'RUN'
#!/bin/sh
exec "$HARVEST_PORTABLE_ROOT/guest" "$HARVEST_PORTABLE_ROOT/omp" "$@"
RUN
chmod 755 "$HARVEST_PORTABLE_ROOT/run" "$HARVEST_PORTABLE_ROOT/guest"

libc=$(HARVEST_PORTABLE_NETWORK=none "$HARVEST_PORTABLE_ROOT/guest" /usr/bin/getconf GNU_LIBC_VERSION)
test "$libc" = 'glibc 2.17'

TMPDIR="$HARVEST_PORTABLE_ROOT/work" \
  HARVEST_TEST_BINARY="$HARVEST_PORTABLE_ROOT/run" \
  cargo xtask test -p harvest-omp --test cli --all-features

#!/bin/sh
# Run real provider/tool/session contracts under the oldest promised GNU libc.
set -eu

if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
  echo "usage: sh rust-agent/tests/portable-gnu.sh SHIPPING_BINARY" >&2
  exit 2
fi

HARVEST_PORTABLE_DOCKER=$(command -v docker)
HARVEST_PORTABLE_IMAGE=centos@sha256:be65f488b7764ad3638f236b7b515b3678369a5124c47b8d32916d6487418ea4
HARVEST_PORTABLE_ROOT=$(mktemp -d "${TMPDIR:-/tmp}/omp-portable.XXXXXX")
HARVEST_PORTABLE_USER="$(id -u):$(id -g)"
export HARVEST_PORTABLE_DOCKER HARVEST_PORTABLE_IMAGE HARVEST_PORTABLE_ROOT HARVEST_PORTABLE_USER
trap 'rm -rf "$HARVEST_PORTABLE_ROOT"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

cp "$1" "$HARVEST_PORTABLE_ROOT/omp"
mkdir "$HARVEST_PORTABLE_ROOT/work"

libc=$("$HARVEST_PORTABLE_DOCKER" run --rm --network none --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  "$HARVEST_PORTABLE_IMAGE" /usr/bin/getconf GNU_LIBC_VERSION)
test "$libc" = 'glibc 2.17'

cat > "$HARVEST_PORTABLE_ROOT/run" <<'RUN'
#!/bin/sh
# Host networking lets the isolated binary reach the test's loopback-only model.
# Only this disposable fixture directory is mounted into the old runtime.
case "$(pwd -P)" in
  "$HARVEST_PORTABLE_ROOT"/*) workdir=$(pwd -P) ;;
  *) workdir="$HARVEST_PORTABLE_ROOT/work" ;;
esac
exec "$HARVEST_PORTABLE_DOCKER" run --rm --network host --read-only \
  --cap-drop ALL --security-opt no-new-privileges \
  --user "$HARVEST_PORTABLE_USER" \
  --mount "type=bind,src=$HARVEST_PORTABLE_ROOT,dst=$HARVEST_PORTABLE_ROOT" \
  --workdir "$workdir" \
  -e PATH=/nonexistent -e OMP_SKIP_INTEGRATIONS -e HOME \
  -e HARVEST_CODING_AGENT_DIR -e HARVEST_AGENT_DIR \
  -e XDG_DATA_HOME -e XDG_STATE_HOME -e USERPROFILE \
  "$HARVEST_PORTABLE_IMAGE" "$HARVEST_PORTABLE_ROOT/omp" "$@"
RUN
chmod 755 "$HARVEST_PORTABLE_ROOT/run"

TMPDIR="$HARVEST_PORTABLE_ROOT/work" \
  HARVEST_TEST_BINARY="$HARVEST_PORTABLE_ROOT/run" \
  cargo xtask test -p harvest-omp --test cli --all-features

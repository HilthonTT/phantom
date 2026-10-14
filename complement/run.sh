#!/bin/sh
# Builds the phantom Complement image and runs Complement against it.
#
#   complement/run.sh [go test arguments...]
#
# With no arguments the whole suite runs; pass e.g. `-run TestLogin` to narrow
# it. Needs Docker and Go. Complement is cloned into target/complement on the
# first run (or set COMPLEMENT_DIR to a checkout of your own). Results land in
# target/complement-results: the `go test -json` stream, which summary.sh
# turns into a pass/fail table, and a JUnit file when gotestsum is installed.
#
# Exits non-zero when any test fails, which today is many of them; CI runs it
# without blocking on that.

set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
image=${COMPLEMENT_BASE_IMAGE:-complement-phantom:latest}
suite=${COMPLEMENT_DIR:-$root/target/complement}
results=$root/target/complement-results

docker build -f "$root/complement/Dockerfile" -t "$image" "$root"

if [ ! -d "$suite" ]; then
    git clone --depth 1 https://github.com/matrix-org/complement "$suite"
fi

mkdir -p "$results"
export COMPLEMENT_BASE_IMAGE="$image"

# Complement starts a container per test deployment; an hour covers the whole
# suite on a CI runner with room to spare. `-count=1` because a cached pass
# says nothing about an image that has since been rebuilt.
set -- -count=1 -timeout 1h "$@" ./tests/...

cd "$suite"
if command -v gotestsum >/dev/null 2>&1; then
    gotestsum --format testname \
        --jsonfile "$results/results.json" \
        --junitfile "$results/junit.xml" \
        -- "$@"
else
    # pipefail is not POSIX, so the status of `go test` is carried out of the
    # pipeline by hand; without it a failing run would report success.
    status=0
    { go test -json "$@" || echo $? >"$results/status"; } | tee "$results/results.json" >/dev/null
    [ -f "$results/status" ] && status=$(cat "$results/status") && rm -f "$results/status"
    exit "$status"
fi

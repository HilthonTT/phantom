#!/bin/sh
# Summarises a Complement run from its `go test -json` stream as Markdown: the
# totals, then every top-level test by outcome.
#
#   complement/summary.sh [target/complement-results/results.json]
#
# CI appends the output to the job summary. Needs jq.

set -eu

command -v jq >/dev/null 2>&1 || { echo "summary.sh needs jq" >&2; exit 1; }

json=${1:-$(cd "$(dirname "$0")/.." && pwd)/target/complement-results/results.json}

# Only top-level tests are counted: a subtest's outcome is already folded into
# its parent's, and counting both would make one failure look like several.
outcomes=$(jq -r 'select(.Test != null and (.Test | contains("/") | not)
    and (.Action == "pass" or .Action == "fail" or .Action == "skip"))
    | "\(.Action) \(.Test)"' "$json" | sort -u)

count() { printf '%s\n' "$outcomes" | grep -c "^$1 " || true; }

echo "## Complement"
echo
echo "| Passed | Failed | Skipped |"
echo "| ---: | ---: | ---: |"
echo "| $(count pass) | $(count fail) | $(count skip) |"

for outcome in pass fail; do
    echo
    echo "<details><summary>${outcome}ed</summary>"
    echo
    printf '%s\n' "$outcomes" | sed -n "s/^$outcome /- /p"
    echo
    echo "</details>"
done

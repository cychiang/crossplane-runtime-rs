#!/usr/bin/env bash
# baseline.sh <label>: builds the Go baseline, brings the cluster up, installs
# the baseline and runs the scenarios into parity/.work/runs/<label>/.
set -euo pipefail
dir=$(dirname "$0")
[ $# -eq 1 ] || { echo "usage: $0 <label>" >&2; exit 1; }
xpkg=$("$dir/make-go-baseline.sh" | tail -1)
"$dir/up.sh"
"$dir/install-provider.sh" provider-baseline "$xpkg"
"$dir/run.sh" "$1"

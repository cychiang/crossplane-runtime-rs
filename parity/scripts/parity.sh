#!/usr/bin/env bash
# parity.sh: the whole Go-against-Rust comparison in one command.
#
# Builds the Go baseline and the Rust template provider, brings the cluster
# up, records a Go run (parity-go) and then a Rust run (parity-rust) of every
# scenario, and diffs them with the allowance file. Prints the differ's report
# and exits with its status: 0 equal, 1 different, 2 bad input. Exits 1 before
# diffing when the Go run itself fails a scenario, since two runs failing the
# same step would compare equal. The Rust provider stays installed.
#
# TEMPLATE_DIR names the provider-template-rs checkout (default: a sibling of
# this repository).
#
# On macOS the script holds the machine awake with caffeinate. A sleep moves
# the wall clock past the 30s creation grace period, so a run that slept
# compares as DIFFERENT for reasons neither provider controls.
set -euo pipefail
if [ -z "${PARITY_AWAKE:-}" ] && command -v caffeinate >/dev/null; then
  PARITY_AWAKE=1 exec caffeinate -ims "$0" "$@"
fi
source "$(dirname "$0")/lib.sh"
dir=$(dirname "$0")
template=$(cd "${TEMPLATE_DIR:-$PARITY/../../provider-template-rs}" && pwd)
[ -x "$template/scripts/build-xpkg.sh" ] || die "no provider-template-rs at $template (set TEMPLATE_DIR)"
start=$SECONDS

log "building the Go baseline"
go_xpkg=$("$dir/make-go-baseline.sh" | tail -1)
log "building the Rust provider"
rust_xpkg=$("$template/scripts/build-xpkg.sh" | tail -1)

"$dir/up.sh"

"$dir/install-provider.sh" provider-baseline "$go_xpkg"
"$dir/run.sh" parity-go || die "the Go run failed a scenario; see $WORK/runs/parity-go/chainsaw.log"

"$dir/install-provider.sh" provider-template-rs "$rust_xpkg"
"$dir/run.sh" parity-rust || log "the Rust run failed a scenario; the diff shows where"

set +e
"$dir/diff.sh" parity-go parity-rust --allow "$PARITY/allow/go-vs-rust.jsonl"
rc=$?
set -e
log "parity.sh finished in $((SECONDS - start))s with differ exit $rc"
exit "$rc"

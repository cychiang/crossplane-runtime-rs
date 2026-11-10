#!/usr/bin/env bash
# Runs the Go facts DIVERGENCES.md cites against the baseline's own module versions: the event
# correlator's repeat patch, the reference resolver's apply body through controller-runtime's
# typed client, and whether the baseline binary accepts --kubeconfig. Needs no cluster.
set -euo pipefail
source "$(dirname "$0")/lib.sh"

BASELINE=${GO_BASELINE:-$WORK/go-baseline}
[ -f "$BASELINE/go.mod" ] || die "no Go baseline at $BASELINE (run make-go-baseline.sh or set GO_BASELINE)"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
cp "$BASELINE/go.sum" "$TMP/"
sed '1s#.*#module gochecks#' "$BASELINE/go.mod" > "$TMP/go.mod"
printf '\nrequire github.com/example/provider-baseline v0.0.0\nreplace github.com/example/provider-baseline => %s\n' "$BASELINE" >> "$TMP/go.mod"
cp -R "$PARITY/go-checks/." "$TMP/"
(cd "$TMP" && GOFLAGS=-mod=mod go test -v ./...)

log "baseline binary with --kubeconfig"
(cd "$BASELINE" && go build -o "$TMP/provider" ./cmd/provider)
"$TMP/provider" --help 2>&1 | grep -q -- --kubeconfig && die "--help lists --kubeconfig"
set +e
out=$(env -u KUBECONFIG "$TMP/provider" --kubeconfig=/nonexistent 2>&1)
code=$?
set -e
printf '%s (exit %d)\n' "$out" "$code"
[ "$code" -ne 0 ] && grep -q "unknown long flag '--kubeconfig'" <<<"$out" || die "expected kingpin to reject --kubeconfig"

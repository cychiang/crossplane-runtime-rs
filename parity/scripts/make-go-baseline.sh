#!/usr/bin/env bash
# Regenerates the Go baseline provider into parity/.work/go-baseline, lays the
# overlay over it, builds it, and (unless SKIP_XPKG=1) builds the xpkg.
# Prints the xpkg path on the last line.
set -euo pipefail
source "$(dirname "$0")/lib.sh"

GEN=${XP_PROVIDER_GEN:-/Users/cychiang/Projects/github.com/cychiang/xp-provider-gen/bin/xp-provider-gen}
OUT=$WORK/go-baseline

[ -x "$GEN" ] || die "generator not found at $GEN (set XP_PROVIDER_GEN)"

rm -rf "$OUT"
mkdir -p "$OUT"
(
  cd "$OUT"
  "$GEN" init --domain=example.com --repo=github.com/example/provider-baseline \
    --git-name=baseline --git-email=noreply@example.com
  "$GEN" create api --group=sample --version=v1alpha1 --kind=MyType
)

cp -R "$PARITY/go-baseline/overlay/." "$OUT/"
(
  cd "$OUT"
  gofmt -l internal | { ! grep .; } || die "overlay is not gofmt-clean"
  go build ./...
  go vet ./internal/...
)
log "baseline builds: $OUT"

[ "${SKIP_XPKG:-0}" = 1 ] && exit 0

arch=$(go env GOARCH)
(
  cd "$OUT"
  # Only the node's architecture is needed on kind; VERSION is fixed so the
  # xpkg file name is stable across regenerations.
  make build PLATFORMS="linux_$arch" VERSION=v0.0.0-parity
)
xpkg=$(ls "$OUT"/_output/xpkg/linux_"$arch"/*.xpkg)
log "xpkg built"
echo "$xpkg"

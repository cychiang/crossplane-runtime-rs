#!/usr/bin/env bash
# run.sh [image]: how late a delayed wake-up fires, on Linux, for a tokio sleep (the Rust work
# queue's timer) and for controller-runtime's priority queue AddAfter+Get (Go's). Cross-builds
# both for linux/arm64 and runs them in any image with /bin/sh, outside the kind cluster.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
source "$here/../scripts/lib.sh"
image=${1:-rancher/klipper-lb:v0.4.17}
BASELINE=${GO_BASELINE:-$WORK/go-baseline}
[ -f "$BASELINE/go.mod" ] || die "no Go baseline at $BASELINE (set GO_BASELINE)"

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
(cd "$here/rs" && CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld \
  cargo +1.99.1 build --release -q --target aarch64-unknown-linux-musl --target-dir "$out/target")
cp "$out/target/aarch64-unknown-linux-musl/release/timer-probe" "$out/tokio"
mkdir "$out/go"
cp "$BASELINE/go.sum" "$here/go/main.go" "$out/go/"
sed '1s#.*#module timerprobe#' "$BASELINE/go.mod" > "$out/go/go.mod"
(cd "$out/go" && GOFLAGS=-mod=mod CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build -o "$out/pq" .)
docker run --rm -v "$out:/p" "$image" sh -c 'for i in 1 2 3; do /p/tokio 60; /p/pq; done'

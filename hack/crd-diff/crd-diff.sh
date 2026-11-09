#!/usr/bin/env bash
# Usage: hack/crd-diff/crd-diff.sh [<go-crd-dir> [<rust-crds-json-stream>]]
#
# Structural diff of the CRDs the Rust provider generates against the Go baseline's CRDs.
# Exits non-zero on any structural difference, or when a CRD exists on one side only.
# Descriptions are excluded from the comparison and counted separately.
#
# <go-crd-dir> holds controller-gen YAML; it defaults to the regenerated Go baseline. Each file
# is converted to JSON at run time with `kubectl patch --local`, which reads the file and
# contacts no cluster, so no converted fixture is committed. Without <rust-crds-json-stream>
# the Rust CRDs come from `cargo run -p provider-sample --example crds`.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
go_dir=${1:-$root/parity/.work/go-baseline/package/crds}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

shopt -s nullglob
yamls=("$go_dir"/*.yaml)
if [ ${#yamls[@]} -eq 0 ]; then
    echo "crd-diff: no *.yaml in $go_dir" >&2
    exit 2
fi
for f in "${yamls[@]}"; do
    kubectl patch --local --type merge -p '{}' -o json -f "$f" >>"$work/go.json"
done

if [ $# -ge 2 ]; then
    cp "$2" "$work/rust.json"
else
    cargo run --quiet --manifest-path "$root/Cargo.toml" -p provider-sample --example crds >"$work/rust.json"
fi

python3 -I "$root/hack/crd-diff/diff.py" "$work/go.json" "$work/rust.json"

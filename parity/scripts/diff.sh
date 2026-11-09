#!/usr/bin/env bash
# diff.sh <labelA> <labelB> [trace-diff args...]
#
# Compares two recorded runs in parity/.work/runs with trace-diff. Exit 0 when
# they are equal under the normalization rules in parity/README.md, 1 when they
# differ, 2 on bad input.
set -euo pipefail
source "$(dirname "$0")/lib.sh"

[ $# -ge 2 ] || die "usage: $0 <labelA> <labelB> [--allow <file>] [--layer <layer>] [--disable-rule <rule>]"
a=$WORK/runs/$1
b=$WORK/runs/$2
shift 2
[ -d "$a" ] || die "no run $a"
[ -d "$b" ] || die "no run $b"
cargo build -q --release --manifest-path "$PARITY/trace-diff/Cargo.toml"
exec "$PARITY/trace-diff/target/release/trace-diff" "$a" "$b" "$@"

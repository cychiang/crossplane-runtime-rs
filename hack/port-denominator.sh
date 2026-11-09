#!/usr/bin/env bash
# Usage: port-denominator.sh <go-test-inventory.tsv>
# Drops legacy (cluster-scoped) cases, which are out of scope, and prints the rest.
set -euo pipefail
awk -F'\t' 'NR==1 || !($2 ~ /legacy/ || $4 ~ /Legacy/ || $5 ~ /Legacy/)' "$1"

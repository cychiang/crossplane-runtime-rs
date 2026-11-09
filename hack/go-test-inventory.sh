#!/usr/bin/env bash
# Usage: go-test-inventory.sh <crossplane-runtime checkout at the target tag> <pkg dir>...
# Prints TSV: package, file, line, test func, case name. One row per table case.
set -euo pipefail
root=$1; shift
printf 'package\tfile\tline\ttest\tcase\n'
for pkg in "$@"; do
  for f in "$root/$pkg"/*_test.go; do
    [ -e "$f" ] || continue
    awk -v pkg="$pkg" -v file="$(basename "$f")" '
      /^func Test[A-Za-z0-9_]*\(/ { t=$2; sub(/\(.*/, "", t); seen=0 }
      /^\t\t"[^"]+": *\{/ && t != "" { c=$0; sub(/^\t\t"/, "", c); sub(/".*/, "", c); printf "%s\t%s\t%d\t%s\t%s\n", pkg, file, NR, t, c; seen=1 }
      /^}/ && t != "" { if (!seen) printf "%s\t%s\t%d\t%s\t%s\n", pkg, file, NR, t, "(no table)"; t="" }
    ' "$f"
  done
done

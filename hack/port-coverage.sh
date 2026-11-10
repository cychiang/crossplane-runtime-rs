#!/usr/bin/env bash
# Usage: hack/port-coverage.sh [--complete] [--list-missing]
#
# Counts how much of docs/phase0/port-denominator.tsv the Rust tests cover.
#
# A Go case is ported when a Rust test carries its marker:
#
#     // go: <package> <TestFunc>/<case>      (or `<TestFunc>` alone when the row has no case)
#
# directly above either a `#[test]` function or a `Case {` table row. A table row must have
# `name: "<case>"` as its first field, so the marker and the row cannot drift apart. A marker
# under `#[ignore]` counts as ignored, not ported.
#
# A Go case is skipped when docs/port-skips.tsv lists it with a reason.
#
# docs/port-extra.tsv lists Go cases outside the denominator that are ported as behavioural
# specs for code the runtime owns (client-go's queue, controller-runtime's controller). Their
# markers and skips follow the same rules; they are counted on a second summary line and never in
# the denominator's numbers.
#
# Exits non-zero when a marker or skip names something the denominator does not have, a marker
# is attached to nothing, a table row's name disagrees with its marker, a case is marked twice
# or both ported and skipped, or a skip has no reason. With --complete it also exits non-zero
# while any case is neither ported nor skipped.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
exec python3 -I "$root/hack/port-coverage.py" "$root" "$@"

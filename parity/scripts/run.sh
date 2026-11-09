#!/usr/bin/env bash
# run.sh <label>
#
# Runs the scenarios against the installed provider and collects the three
# parity artifacts into parity/.work/runs/<label>/:
#   chainsaw-report.json  chainsaw's report
#   audit.jsonl           the provider service account's writes, one per line
#   calls.jsonl           the fake external service's call log
# plus provider.json, provider.log, chainsaw.log and audit-raw.jsonl (every
# audit event of the run window, for debugging).
set -euo pipefail
source "$(dirname "$0")/lib.sh"

[ $# -eq 1 ] || die "usage: $0 <label>"
label=$1
[[ "$label" =~ ^[A-Za-z0-9._-]+$ ]] || die "label must match [A-Za-z0-9._-]+"
[ -f "$WORK/provider.json" ] || die "no provider installed; run install-provider.sh"
cluster_exists || die "cluster $CLUSTER is not up; run up.sh"
sa=$(jq -r .serviceAccount "$WORK/provider.json")
rev=$(jq -r .revision "$WORK/provider.json")
kc -n "$XP_NS" get "deploy/$rev" >/dev/null || die "installed revision $rev has no deployment; rerun install-provider.sh"

out=$WORK/runs/$label
rm -rf "$out"
mkdir -p "$out"
cp "$WORK/provider.json" "$out/"

kc delete --raw "$EXT_PROXY/_faults" >/dev/null
kc delete --raw "$EXT_PROXY/_calls" >/dev/null

start=$(wc -c < "$AUDIT_LOG")
set +e
chainsaw test --test-dir "$PARITY/scenarios" --config "$PARITY/scenarios/chainsaw.yaml" \
  --kube-context "kind-$CLUSTER" \
  --report-format JSON --report-path "$out" --report-name chainsaw-report \
  2>&1 | tee "$out/chainsaw.log"
rc=${PIPESTATUS[0]}
set -e

# Let the provider finish writes triggered by the last cleanup before the
# window closes.
sleep 5
end=$(wc -c < "$AUDIT_LOG")

tail -c +"$((start + 1))" "$AUDIT_LOG" | head -c "$((end - start))" > "$out/audit-raw.jsonl"
jq -c --arg user "system:serviceaccount:$XP_NS:$sa" '
  select(.user.username == $user and (.verb | IN("create", "update", "patch", "delete", "deletecollection")))
  | {
      verb,
      apiGroup: .objectRef.apiGroup,
      resource: .objectRef.resource,
      subresource: .objectRef.subresource,
      namespace: .objectRef.namespace,
      name: .objectRef.name,
      username: .user.username,
      userAgent,
      code: .responseStatus.code,
      requestObject,
      requestReceivedTimestamp
    }' "$out/audit-raw.jsonl" > "$out/audit.jsonl"

kc get --raw "$EXT_PROXY/_calls" > "$out/calls.jsonl"
kc -n "$XP_NS" logs "deploy/$rev" > "$out/provider.log" 2>&1 || true

log "chainsaw exit $rc; $(wc -l < "$out/audit.jsonl" | tr -d ' ') provider writes; $(wc -l < "$out/calls.jsonl" | tr -d ' ') external calls"
log "artifacts in $out"
exit "$rc"

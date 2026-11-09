#!/usr/bin/env bash
# Deletes the parity cluster. Run artifacts and the audit log stay in .work.
set -euo pipefail
source "$(dirname "$0")/lib.sh"

if cluster_exists; then
  kind delete cluster --name "$CLUSTER" --kubeconfig "$KUBECONFIG"
else
  log "cluster $CLUSTER does not exist"
fi
rm -f "$WORK/provider.json"

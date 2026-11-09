# Sourced by the parity scripts. Everything targets one named kind cluster
# through a private kubeconfig, never the user's default context.
PARITY=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
WORK=$PARITY/.work
CLUSTER=${PARITY_CLUSTER:-xprs-parity}
export KUBECONFIG=$WORK/kubeconfig
export PATH=$WORK/bin:$PATH

NODE_IMAGE=kindest/node:v1.36.4@sha256:099e049362a1526b2db71494e1947aae99bd16290d7c895f2b7ea312e3cbfaed
CROSSPLANE_CHART_VERSION=2.4.3
CHAINSAW_VERSION=v0.2.15
XP_NS=crossplane-system
EXT_NS=parity-external
EXT_IMAGE=parity/fake-external:dev
EXT_PROXY=/api/v1/namespaces/$EXT_NS/services/http:fake-external:8080/proxy
AUDIT_DIR=$WORK/cluster/audit
AUDIT_LOG=$AUDIT_DIR/kube-apiserver-audit.log

mkdir -p "$WORK/bin"

log() { printf '==> %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

cluster_exists() { kind get clusters 2>/dev/null | grep -x "$CLUSTER" >/dev/null; }

kc() { kubectl --kubeconfig "$KUBECONFIG" "$@"; }

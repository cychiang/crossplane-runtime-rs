#!/usr/bin/env bash
# Brings up the parity cluster: kind with audit logging, Crossplane core, and
# the fake external service. Safe to rerun; each step converges.
set -euo pipefail
source "$(dirname "$0")/lib.sh"

if [[ "$(chainsaw version 2>/dev/null || true)" != *"Version: $CHAINSAW_VERSION"* ]]; then
  log "installing chainsaw $CHAINSAW_VERSION into $WORK/bin"
  GOBIN=$WORK/bin go install "github.com/kyverno/chainsaw@$CHAINSAW_VERSION"
fi

mkdir -p "$AUDIT_DIR" "$WORK/cluster"
if cluster_exists; then
  log "cluster $CLUSTER exists"
  kind export kubeconfig --name "$CLUSTER" --kubeconfig "$KUBECONFIG"
else
  sed -e "s|@AUDIT_POLICY@|$PARITY/cluster/audit-policy.yaml|" -e "s|@AUDIT_DIR@|$AUDIT_DIR|" \
    "$PARITY/cluster/kind.yaml" > "$WORK/cluster/kind.yaml"
  log "creating cluster $CLUSTER"
  kind create cluster --name "$CLUSTER" --config "$WORK/cluster/kind.yaml" --kubeconfig "$KUBECONFIG" --wait 120s
fi
NODE=$CLUSTER-control-plane
docker exec "$NODE" grep -q -- "--audit-policy-file=/etc/kubernetes/policies/audit-policy.yaml" \
  /etc/kubernetes/manifests/kube-apiserver.yaml || die "kube-apiserver runs without the audit policy"
[ -f "$AUDIT_LOG" ] || die "no audit log at $AUDIT_LOG"

# The package cache is a hostPath on the node, so install-provider.sh can drop
# packages in with docker cp and they survive a Crossplane pod restart.
docker exec "$NODE" sh -c 'mkdir -p /var/parity-xpkg-cache && chmod 0777 /var/parity-xpkg-cache'
kc apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $XP_NS
---
apiVersion: v1
kind: PersistentVolume
metadata:
  name: parity-package-cache
spec:
  capacity: {storage: 1Gi}
  accessModes: [ReadWriteOnce]
  storageClassName: ""
  persistentVolumeReclaimPolicy: Retain
  hostPath: {path: /var/parity-xpkg-cache, type: Directory}
  claimRef: {namespace: $XP_NS, name: package-cache}
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: package-cache
  namespace: $XP_NS
spec:
  accessModes: [ReadWriteOnce]
  storageClassName: ""
  volumeName: parity-package-cache
  resources: {requests: {storage: 1Gi}}
EOF

if helm list -n "$XP_NS" -o json | jq -e --arg c "crossplane-$CROSSPLANE_CHART_VERSION" \
    'any(.[]; .name == "crossplane" and .status == "deployed" and .chart == $c)' >/dev/null; then
  log "crossplane chart $CROSSPLANE_CHART_VERSION already deployed"
else
  log "installing crossplane chart $CROSSPLANE_CHART_VERSION"
  helm upgrade --install crossplane crossplane --repo https://charts.crossplane.io/stable \
    --version "$CROSSPLANE_CHART_VERSION" -n "$XP_NS" \
    --set packageCache.pvc=package-cache --wait --timeout 5m
fi
kc -n "$XP_NS" rollout status deploy/crossplane --timeout=180s

tag=$(cat "$PARITY"/fake-external/{go.mod,*.go,Dockerfile} | shasum -a 256 | cut -c1-12)
image=${EXT_IMAGE%:*}:$tag
if ! docker image inspect "$image" >/dev/null 2>&1; then
  log "building $image"
  docker build -q -t "$image" "$PARITY/fake-external" >/dev/null
fi
kind load docker-image "$image" --name "$CLUSTER" >/dev/null
kc apply -f - <<EOF
apiVersion: v1
kind: Namespace
metadata:
  name: $EXT_NS
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: fake-external
  namespace: $EXT_NS
spec:
  replicas: 1
  selector: {matchLabels: {app: fake-external}}
  template:
    metadata:
      labels: {app: fake-external}
    spec:
      containers:
      - name: fake-external
        image: $image
        imagePullPolicy: Never
        ports: [{containerPort: 8080}]
        readinessProbe:
          httpGet: {path: /healthz, port: 8080}
          periodSeconds: 2
---
apiVersion: v1
kind: Service
metadata:
  name: fake-external
  namespace: $EXT_NS
spec:
  selector: {app: fake-external}
  ports: [{name: http, port: 8080, targetPort: 8080}]
EOF
kc -n "$EXT_NS" rollout status deploy/fake-external --timeout=120s
kc get --raw "$EXT_PROXY/healthz" >/dev/null || die "fake-external is not reachable through the API server proxy"
log "up: cluster $CLUSTER, kubeconfig $KUBECONFIG"

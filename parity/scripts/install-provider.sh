#!/usr/bin/env bash
# install-provider.sh <name> <xpkg file> [runtime image]
#
# Installs a provider package on the parity cluster without a registry, the
# same way the generator's local.xpkg.deploy target does: the package goes into
# Crossplane's package cache and the Provider uses packagePullPolicy: Never.
# The runtime image defaults to the one embedded in the xpkg. Any other
# Provider is removed first, because both parity providers own the same CRDs.
set -euo pipefail
source "$(dirname "$0")/lib.sh"

[ $# -ge 2 ] || die "usage: $0 <name> <xpkg file> [runtime image]"
name=$1
xpkg=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")
image=${3:-}
[ -f "$xpkg" ] || die "no xpkg at $xpkg"
cluster_exists || die "cluster $CLUSTER is not up; run up.sh"
NODE=$CLUSTER-control-plane

hex=$(shasum -a 256 "$xpkg" | cut -d' ' -f1)
digest=sha256:$hex
source_ref=xpkg.crossplane.internal/dev/$name

for other in $(kc get providers.pkg.crossplane.io -o name); do
  [ "$other" = "provider.pkg.crossplane.io/$name" ] && continue
  log "removing $other"
  kc delete "$other" --wait --timeout=180s
done

if [ -z "$image" ]; then
  image=parity/$name:${hex:0:12}
  if ! docker image inspect "$image" >/dev/null 2>&1; then
    id=$(docker load -i "$xpkg" | sed -n 's/^Loaded image ID: //p')
    [ -n "$id" ] || die "docker load did not report an image ID for $xpkg"
    docker tag "$id" "$image"
  fi
fi
log "loading runtime image $image"
kind load docker-image "$image" --name "$CLUSTER" >/dev/null

# Crossplane 2.2 and later look a cached package up by its friendly ID;
# earlier versions by <source>@<digest>. Write both, as local.xpkg.mk does.
friendly=$(printf '%.50s-%.12s' "$source_ref" "$digest" | sed 's/[^a-z0-9]/-/g' | cut -c1-63 | sed 's/-*$//')
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
crossplane xpkg extract --from-xpkg "$xpkg" -o "$tmp/pkg.gz"
docker exec "$NODE" mkdir -p /var/parity-xpkg-cache/xpkg.crossplane.internal/dev
docker cp "$tmp/pkg.gz" "$NODE:/var/parity-xpkg-cache/$friendly.gz" >/dev/null
docker cp "$tmp/pkg.gz" "$NODE:/var/parity-xpkg-cache/xpkg.crossplane.internal/dev/$name@$digest.gz" >/dev/null
docker exec "$NODE" chmod -R a+rX /var/parity-xpkg-cache

kc apply -f - <<EOF
apiVersion: pkg.crossplane.io/v1beta1
kind: DeploymentRuntimeConfig
metadata:
  name: parity-$name
spec:
  deploymentTemplate:
    spec:
      selector: {}
      template:
        spec:
          containers:
          - name: package-runtime
            image: $image
            args: ["--debug"]
---
apiVersion: pkg.crossplane.io/v1
kind: Provider
metadata:
  name: $name
spec:
  package: $source_ref@$digest
  packagePullPolicy: Never
  skipDependencyResolution: true
  runtimeConfigRef:
    name: parity-$name
EOF

log "waiting for $name to become Healthy"
kc wait "provider.pkg.crossplane.io/$name" --for=condition=Healthy --timeout=300s

rev=$(kc get providerrevisions.pkg.crossplane.io -l "pkg.crossplane.io/package=$name" \
  -o jsonpath='{range .items[?(@.spec.desiredState=="Active")]}{.metadata.name}{end}')
kc -n "$XP_NS" rollout status "deploy/$rev" --timeout=180s
sa=$(kc -n "$XP_NS" get "deploy/$rev" -o jsonpath='{.spec.template.spec.serviceAccountName}')
[ -n "$sa" ] || die "cannot find the service account of revision $rev"
jq -n --arg name "$name" --arg xpkg "$xpkg" --arg digest "$digest" --arg image "$image" \
  --arg revision "$rev" --arg serviceAccount "$sa" \
  '{name: $name, xpkg: $xpkg, digest: $digest, image: $image, revision: $revision, serviceAccount: $serviceAccount}' \
  > "$WORK/provider.json"
log "installed $name revision $rev, service account $sa"

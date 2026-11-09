# Parity harness

The parity harness runs one set of chainsaw scenarios against a provider on a
pinned kind cluster. It collects three artifacts per run. A Go run and a Rust
run are then compared. The Go half exists first, so its run is the baseline.

## Commands

```bash
parity/scripts/baseline.sh go-1         # all four steps below in one command
parity/scripts/make-go-baseline.sh      # regenerate, overlay and build the Go provider; prints the xpkg path
parity/scripts/up.sh                    # kind cluster, Crossplane core, fake external service
parity/scripts/install-provider.sh provider-baseline \
  parity/.work/go-baseline/_output/xpkg/linux_arm64/provider-baseline-v0.0.0-parity.xpkg
parity/scripts/run.sh go-1              # scenarios, then artifacts into parity/.work/runs/go-1/
parity/scripts/down.sh                  # delete the cluster
```

Every script is safe to rerun. Everything runs against the kind cluster
`xprs-parity` through `parity/.work/kubeconfig`. The scripts never use the
default kubeconfig. Set `PARITY_CLUSTER` to change the cluster name.

## Pins

| Item | Value |
|---|---|
| kind node | `kindest/node:v1.36.4@sha256:099e049362a1526b2db71494e1947aae99bd16290d7c895f2b7ea312e3cbfaed` |
| Crossplane | Helm chart `crossplane` 2.4.3 |
| chainsaw | v0.2.15, installed into `parity/.work/bin` |
| Go baseline | `xp-provider-gen init` and `create api --group=sample --version=v1alpha1 --kind=MyType` |

## Layout

- `fake-external/` is the external system. It is an HTTP service built from the Go standard library only.
- `go-baseline/overlay/` holds the user-owned files that replace the scaffold's stubs. The scaffold itself is not checked in. `make-go-baseline.sh` regenerates it into `.work/go-baseline/`.
- `cluster/` holds the kind config, which `up.sh` renders with absolute host paths, and the audit policy.
- `scenarios/` holds provider-agnostic chainsaw tests. A Rust run uses the same YAML. Only the installed package changes.
- `scripts/` holds the commands above. `lib.sh` holds the shared pins and paths.

## Fake external service contract

The service runs as `fake-external` in namespace `parity-external`, on port 8080. Each thing is
`{"configurableField": "<string>"}`, keyed by name.

| Request | Result |
|---|---|
| `GET /things/{name}` | 200 with the thing, or 404 |
| `PUT /things/{name}` | Creates (201) or replaces (200) the thing and returns it. A body that is not a thing gets 400 |
| `DELETE /things/{name}` | 204, or 404 when absent |
| `GET /_calls` | The call log as JSON lines: `seq`, `method`, `path`, `body` (compacted JSON, omitted when empty), `status`. The log has no timestamps |
| `DELETE /_calls` | Clears the log. `seq` restarts at 1 |
| `POST /_faults` | `{"method": "PUT", "name": "x", "count": 3, "status": 500}` fails the next `count` thing requests with that method (and name, when given) |
| `GET /_faults`, `DELETE /_faults` | Lists or clears pending faults |

Only `/things/` requests are logged. A request failed by a fault is logged with its injected
status. Scenarios and scripts reach the service through the API server's service proxy at
`/api/v1/namespaces/parity-external/services/http:fake-external:8080/proxy`. They do not need a port-forward.

## Provider contract

The Rust provider must implement exactly this. The Go overlay is the reference.

**Endpoint.** The flag `--external-endpoint` (env `EXTERNAL_ENDPOINT`) defaults to
`http://fake-external.parity-external.svc.cluster.local:8080`. The ProviderConfig and its credentials are resolved
as usual, but the client ignores them.

**Name.** The thing is named by the MR's `crossplane.io/external-name`. The runtime's default initializer
sets that to `metadata.name` before the first Observe. Scenario MR names are unique across namespaces,
because the thing name ignores the namespace.

**Requests.** Each request uses `Content-Type: application/json`. A PUT body is `{"configurableField": spec.forProvider.configurableField}`.

| Method | Call | Then |
|---|---|---|
| Observe | `GET /things/{name}`, including while the MR is being deleted | 404 returns `ResourceExists: false`. 200 sets `status.atProvider.configurableField` to the thing's value, sets `Available`, and returns `ResourceExists: true` with `ResourceUpToDate` set to whether the value equals the spec |
| Create | `PUT` | Sets `Creating` before the call. On success sets `status.atProvider.configurableField` from the response |
| Update | `PUT` | On success sets `status.atProvider.configurableField` from the response |
| Delete | `DELETE` | Sets `Deleting` before the call. 204 and 404 both succeed |

No connection details are returned. A status code the table does not expect becomes the error
`<METHOD> /things/<name>: status <code>`. Example: `PUT /things/a: status 500`. The runtime wraps
it, so a failed create shows `create failed: PUT /things/a: status 500` on the Synced condition.

**Runtime config.** `install-provider.sh` runs the package with a DeploymentRuntimeConfig. The config sets
the runtime image and the args `["--debug"]`. Any other flag keeps the scaffold's default. For example, `--poll` is 1m.

## Installing a provider

`install-provider.sh <name> <xpkg> [runtime image]` installs without a registry. It uses the same
trick as the generator's `local.xpkg.deploy`. It extracts the package into Crossplane's package cache,
then creates the Provider with `packagePullPolicy: Never`. The cache is a hostPath PVC on the kind node, so it
survives a Crossplane restart. The package ref is `xpkg.crossplane.internal/dev/<name>@sha256:<sha256 of the xpkg file>`,
so a rebuilt xpkg gets a new revision. The runtime image defaults to the image embedded in the
xpkg. The script runs `docker load` on the xpkg and tags the result. The script deletes every other Provider first,
because the Go and Rust providers serve the same CRDs. It writes the revision and its service account to
`.work/provider.json`, which `run.sh` filters on.

## Run artifacts

`run.sh <label>` clears the call log and faults. It runs `scenarios/` with `--parallel 1` and writes into
`parity/.work/runs/<label>/`:

- `chainsaw-report.json` is chainsaw's JSON report.
- `audit.jsonl` holds the writes (create, update, patch, delete) made by the installed provider's service account, one JSON object per line. Each line has `verb`, `apiGroup`, `resource`, `subresource`, `namespace`, `name`, `username`, `userAgent`, `code`, `requestObject` and `requestReceivedTimestamp`. Nothing in it is normalized yet.
- `calls.jsonl` is the fake service's call log.
- `audit-raw.jsonl` holds every audit event in the run window. The window is the byte range of the audit log between chainsaw's start and 5s after it ends.
- `provider.log`, `chainsaw.log` and `provider.json` are kept for debugging.

The audit policy (`cluster/audit-policy.yaml`) logs `crossplane-system` service-account writes at
`Request` level for the provider's groups (`sample.example.com`, `example.com`), Events, Secrets, Leases
and `protection.crossplane.io`. It drops reads and control-plane housekeeping. It keeps other writes at `Metadata`
level, so chainsaw's own steps stay visible as markers.

## Scenarios

| Test | What it proves |
|---|---|
| `lifecycle` | Create reaches Ready and PUTs the thing. A spec change PUTs the new value. Deleting the MR DELETEs the thing |
| `pause` | With `crossplane.io/paused` set, Synced is `ReconcilePaused` and a spec change does not reach the thing. Resuming applies it |
| `observe-only` | `managementPolicies: [Observe]` on a seeded thing reaches Ready without a PUT. Deleting the MR leaves the thing |
| `create-failure` | Three injected 500s on create give `ReconcileError`. The fourth try succeeds, and the call log has exactly three failed PUTs |

Each test runs in its own fixed namespace with its own Secret and namespaced ProviderConfig. Each manifest
sets `providerConfigRef.kind: ProviderConfig`, because the CRD default `ClusterProviderConfig` does not exist in this
provider.

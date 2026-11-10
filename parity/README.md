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
parity/scripts/diff.sh go-1 go-2        # compare two runs; exit 0 when equal under the rules below
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
| `GET /_calls` | The call log as JSON lines: `seq`, `method`, `path`, `body` (compacted JSON, omitted when empty), `status`, `fault` (true when an injected fault answered), `userAgent`, and `forwardedFor` (the `X-Forwarded-For` header, omitted when empty). The log has no timestamps |
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
| Observe | `GET /things/{name}`, including while the MR is being deleted | 404 returns `ResourceExists: false`. 200 sets `status.atProvider.configurableField` to the thing's value, sets `Available`, and returns `ResourceExists: true` with `ResourceUpToDate` set to whether the value equals the spec, and the connection detail `configurableField` set to the value |
| Create | `PUT` | Sets `Creating` before the call. On success sets `status.atProvider.configurableField` from the response |
| Update | `PUT` | On success sets `status.atProvider.configurableField` from the response |
| Delete | `DELETE` | Sets `Deleting` before the call. 204 and 404 both succeed |

Observe returns one connection detail, `configurableField`, holding the thing's value. Create, Update
and Delete return none. The runtime publishes it to the Secret named by `spec.writeConnectionSecretToRef`
when that is set. A status code the table does not expect becomes the error
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

## Comparing runs

`trace-diff` (`parity/trace-diff/`, a standalone crate) compares two run directories:

```bash
trace-diff <run-A> <run-B> [--allow <file>] [--layer chainsaw|audit|events|calls]... [--disable-rule <rule>]...
```

It exits 0 when the runs are equal under the rules below, 1 with a report when they differ, and 2 on bad input.
Each layer prints how much it compared, so an empty diff of empty data shows up. `--disable-rule` turns one
normalization rule off, which shows what it erases. A Go-vs-Rust diff means something only because the Go-vs-Go diffs
of the recorded runs are empty.

### Layers

- **chainsaw.** The same set of tests, the same status per test, and the same ordered steps with the same status.
- **audit.** Successful writes other than Events, grouped by `(apiGroup, resource, namespace, normalized name)` and
  ordered by `requestReceivedTimestamp`. Within a group the ordered sequence of `(verb, subresource, normalized
  requestObject)` must match. The verb is compared for every group, so update against patch on an MR, ProviderConfig
  or usage fails unless an allowance names it. `username` and `userAgent` are not compared.
- **failed writes.** Non-2xx writes are listed per run and never compared. Go's 409s come from informer-cache staleness
  and land on a different object in each run: go-1 had two ProviderConfig 409s, go-2 one, go-fresh one MR status 409,
  go-a one MR update 409, go-b none, go-c one MR status 409, go-fresh-b one ProviderConfig 409.
- **events.** Keyed by `(involved object, type, reason)`. The set of keys and each key's set of normalized messages
  must match. Counts, event names and timestamps are client-go correlator output and are not compared. A patch is
  joined to its create by event name.
- **calls.** Per thing, the ordered sequence of PUTs, DELETEs and fault-answered requests must match exactly,
  including who made each call. The provider's non-faulted GETs are compared as a set of status codes, because their
  count follows poll timing. The service always writes the `userAgent` field, even when empty, so a run without it is an old recording. A call is the scenario's when it carries `X-Forwarded-For`, which the API server's
  service proxy adds and the provider's direct calls lack. In go-a every `kubectl` call had it and no provider call did.
  The scenarios' GETs are left out. Their PUTs and DELETEs stay in the sequence, labelled `scenario`.

### Normalization rules

Each rule is a named function in `trace-diff/src/normalize.rs` or `audit.rs` with its own unit tests. The evidence
counts come from `trace-diff` on go-1 against go-2 with only that rule off.

| Rule | Erases | Why |
|---|---|---|
| `drop-resource-version` | `metadata.resourceVersion` | It is the cluster-wide etcd revision at read time, so it depends on every write the cluster ever took. 92 differences when off. |
| `drop-managed-fields` | `metadata.managedFields` | It is the server's bookkeeping, echoed back in an update body. Between runs the entries swap places (the provider's and chainsaw's entries trade index), which gives 5 differences when off (7 for go-2 against go-fresh). Its manager names derive from the User-Agent, so Go and Rust differ there by design (parity spec section 7, rows 10 and 11). |
| `mask-timestamps` | Every RFC3339 string value, replaced by `<ts>` | `creationTimestamp`, `deletionTimestamp`, `lastTransitionTime` and the `external-create-*` annotation values are wall-clock times. The key stays, so a missing annotation or a missing `deletionTimestamp` still shows. 92 differences when off. |
| `tokenize-uids` | Each UID, replaced by `<uid Kind namespace/name>` for the object it belongs to, in any string value and in object names | UIDs are random per object. A ProviderConfigUsage is named after its MR's UID, so its group key changes every run. The token names the owner rather than a counter, so one extra object does not renumber the rest. 99 differences when off. |
| `wrapped-error-tail` | In condition and event messages, the text after a chain of crossplane-runtime wrap prefixes (`create failed: `, `connect failed: `, `cannot update managed resource: ` and the others listed in `RUNTIME_WRAP_PREFIXES`, taken from the v2.4.3 source), replaced by `<wrapped error>` | Decision of 2026-11-09T22:18:55Z: runtime-authored text is exact and a wrapped client error is compared by its prefix. A tail of the provider-contract shape `<METHOD> /things/<name>: status <code>` stays exact, because this README fixes that text. A tail the runtime creates itself (`RUNTIME_TAILS`, such as `observe failed: external resource does not exist`) also stays exact. A message with no runtime prefix stays exact. It changes nothing in the Go runs, whose only wrapped tail is contract text. It exists for the Rust side, and fabricated messages test it. |
| `collapse-repeated-rounds` | In ProviderConfig groups only, a block of writes that repeats the block right before it, after failed writes are dropped | In every group, consecutive identical writes collapse (the settled rule, block length 1). go-fresh's `parity-lifecycle` ProviderConfig took one extra `(update, update status)` pair 2ms after an identical pair. That is an extra ProviderConfig reconcile from a usage event (parity spec section 7, row 19), and workqueue coalescing decides how many run. With the rule off, go-1 against go-fresh gives 2 differences. It is limited to ProviderConfigs because on an MR the repeated rounds are retries: on the create-failure MR it would cut 15 writes to 11 in every run and hide the retry count, so MR rounds stay compared. This rule extends the settled collapse rule and can be dropped by deleting it. |

Not normalized, because no Go-vs-Go diff needed it: `generation`, `observedGeneration`, finalizers, condition order,
annotation keys, and the write order within an object. Revision and service-account names appear only in `username`,
which is not compared, so they need no rule.

### Allowances

`--allow <file>` takes JSON lines. Blank lines and lines starting with `#` are skipped. Each line names one
difference and must give a reason:

```json
{"layer": "audit", "key": "sample.example.com/mytypes *", "kind": "verb", "a": "* update", "b": "* patch", "reason": "..."}
```

`layer` is `chainsaw`, `audit`, `events` or `calls`. `kind` is `missing` (only in A), `extra` (only in B), `changed`
or `verb`. `key`, `a` and `b` are globs over the report's key and A and B columns, where `*` matches anything, and
an omitted `a` or `b` matches anything. The report lists each allowance as used or `UNUSED`.

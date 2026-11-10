# Parity harness

The parity harness runs one set of chainsaw scenarios against a provider on a
pinned kind cluster. It collects three artifacts per run. A Go run and a Rust
run are then compared. The Go half exists first, so its run is the baseline.

## How to reproduce the parity result

```bash
parity/scripts/parity.sh
```

It needs Docker, kind, kubectl, helm, jq, Go, the `crossplane` CLI, the `xp-provider-gen` binary (`XP_PROVIDER_GEN`)
and a provider-template-rs checkout beside this repository (`TEMPLATE_DIR`). From nothing it builds the Go baseline
and the Rust template xpkg, brings up the kind cluster, installs the Go provider and records run `parity-go`, installs
the Rust provider and records run `parity-rust`, then runs `trace-diff` on the two with `allow/go-vs-rust.jsonl`. It
prints the differ's report and exits with its status: 0 when the runs are equal apart from the allowances, 1 when they
differ. It exits 1 without diffing when the Go run fails a scenario. The cluster stays up with the Rust provider
installed; `parity/scripts/down.sh` deletes it. From a deleted cluster, with warm Go, Cargo and Docker build caches, it
takes about 21 minutes (1279 to 1382s over four runs) on an arm64 Mac, most of it the two runs of fifteen scenarios.

The machine must stay awake for the whole run. A sleep moves the wall clock past the 30s creation grace period, so
the run that slept skips its `PendingExternalResource` events and their writes and compares as DIFFERENT. One run
that crossed an idle sleep, a lid-close sleep and many maintenance sleeps took 7230s and lost two such events on
the Go side. On macOS `parity.sh` runs under `caffeinate`, which prevents idle sleep but not a lid close, and then
reads the power log: if the machine slept during the run it prints `INVALID` and exits 3, whatever the diff said.
Elsewhere, keep the machine awake yourself, and treat a run much longer than usual as suspect.

A Go-against-Rust diff means something only because Go-against-Go diffs are empty. To check that, record more Go
runs and diff them with no allowance file, as described under Commands.

## Commands

```bash
parity/scripts/parity.sh                # the whole Go-against-Rust comparison, see above
parity/scripts/baseline.sh go-1         # the next four steps in one command
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
| `update-failure` | Three injected 500s on the update PUT give `ReconcileError` with `update failed: ...`. The retry applies the change (B46, then B48) |
| `delete-failure` | Three injected 500s on DELETE keep the finalizer, with `Ready=False/Deleting` and `delete failed: ...`. The fourth try deletes the thing and the MR (B23, B24, B29) |
| `observe-failure` | Three injected 500s on GET before anything exists give `observe failed: ...`. The retry creates the thing (B19, then B41) |
| `connection-secret` | `writeConnectionSecretToRef` publishes the connection detail to a Secret the MR controls. A spec change patches the new value in. Deleting the MR removes the Secret through its owner reference (the publisher at the B30 and B47 sites; B48 with update details) |
| `no-update` | Policies without Update: a spec change is observed (`observedGeneration` 2) but the thing keeps its value. Delete still deletes it (B45, B24, B29) |
| `no-delete` | Policies without Delete: deleting the MR leaves the created thing in place (B41, then B10) |
| `policy-edges` | On a missing thing, `[Observe]` reports `observe failed: external resource does not exist` (B20), `[Observe, Update]` reports success without creating (B44), and `[Delete]` is rejected as unsupported before any call (B05) |
| `create-pending` | A simulation: an MR created with `external-create-pending` already set is refused with `cannot determine creation result ...` and nothing is called (B13). Removing only that annotation triggers no reconcile, because the event filter ignores it. A label change does, and the create goes ahead (B41). The test waits 5s before removing the annotation: in go-f the refusal's status write got a 409, which Go turns into a requeue (spec X1), and that requeue landed after the removal and created the thing on its own. Deleting soon after the create also reaches B21 |
| `providerconfig-in-use` | Deleting a ProviderConfig an MR uses leaves it with its finalizer, `users: 1` and `Terminating=True/InUse` (PC4). Deleting the MR releases and deletes it (PC6) |
| `default-providerconfig-ref` | An MR without `providerConfigRef` gets the CRD default `ClusterProviderConfig`, which this provider lacks, so Connect fails with `connect failed: unsupported provider config kind: ClusterProviderConfig` (B17). Pointing it at the namespaced ProviderConfig fixes it |
| `poll-interval` | With `crossplane.io/poll-interval: 5s`, a value changed behind the provider's back is put back within 30s, well inside the default 1m poll (B43, then B48) |

Branch ids (B, PC) are rows of `docs/phase0/02-parity-spec.md` sections 1.3 and 4.1. The spec change while paused
is covered by `pause`. The `crossplane.io/reconcile-requested-at` annotation is not tested, because the baseline
`MyType` cannot record reconcile tokens (spec section 3), so the annotation only triggers an ordinary reconcile.
Faults match by method and name, so a PUT fault injected after the thing exists fails only the update. A scenario
never reads a thing through the proxy while a GET fault is pending on it, because that read would consume the fault.

Cleanup starts as soon as a test ends, with no delay. Once an MR is gone, its usage goes too and the ProviderConfig is
reconciled twice within about 15ms: one round writes `users: 0`, and the status change triggers a second round that
writes the same again. Cleanup then deletes the ProviderConfig. When the delete lands inside the second round, that
round's update or status write fails with 409 and its other write is never sent (spec 4.1 PC7 and PC8). Failed writes
are not compared, so that run has a write fewer. `scripts/pc-settle.py <run>` prints, per run, the smallest gap from
the provider's last ProviderConfig write to the delete, and the tests with a ProviderConfig 409.

The race is possible on both stacks, and its odds follow the provider's reconcile latency. Go's smallest gap is 31 to
33ms (go-h to go-j). A Rust provider that opened a new HTTP connection on every reconcile finished the 30s
grace-period chain about 40ms later than Go (`scripts/finalizer-lag.py`), its gap fell to 0 to 1ms, and 2 of 3 runs
differed. A 2s `delayBeforeCleanup` covered that until the cause was fixed. With one shared connection pool the
Rust gap is 12 to 29ms, and runs rust-nodelay-1 to 3 are each equal to go-h, go-i and go-j. If a run ever differs by
one missing ProviderConfig write after a 409, rerun it and check `pc-settle.py`.

Each test runs in its own fixed namespace with its own Secret and namespaced ProviderConfig. Each manifest except
`default-providerconfig-ref` sets `providerConfigRef.kind: ProviderConfig`, because the CRD default
`ClusterProviderConfig` does not exist in this provider.

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
  go-a one MR update 409, go-b none, go-c one MR status 409, go-fresh-b one ProviderConfig 409. With the fifteen scenarios,
  go-e had one MR status 409, go-g two ProviderConfig 409s, go-h three MR status and one ProviderConfig 409, go-i four
  MR status 409s and go-j one MR status 409. In go-f a 409 also changed the outcome of a scenario: see `create-pending`.
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
an omitted `a` or `b` matches anything. `detail` limits a `changed` allowance to the changed paths the report lists
under a difference. It is one glob or a list of globs, and every changed path must match one of them, so a
difference that also changes anything else is not allowed. The report lists each allowance as used or `UNUSED`.

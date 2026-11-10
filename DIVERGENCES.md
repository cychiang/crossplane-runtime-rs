# Divergences from the Go stack

Every known difference between a provider built on this runtime and the Go baseline
(crossplane-runtime v2.4.3, controller-runtime v0.24.1, client-go v0.36). Most rows come from the
units' notes in `docs/design/core.md`; the ones whose Evidence names Go source or a run were
checked against current code at integration. The Go-against-Rust trace diff compares
four layers (`parity/README.md`): the audit trace of writes, core/v1 Events, calls to the fake
external service, and chainsaw steps. The "Observable" column names the layer a row can show up
in, or metrics, or none.

Not listed: places where the port differs from a Go mechanism but behaves the same on the wire
(a missing `nil`, a scheme lookup, a mock pattern). Those are in `docs/port-skips.tsv` and the
"Implementation reconciliation" notes of `docs/design/core.md`.

| What differs | Why | Observable | Evidence |
|---|---|---|---|
| The API User-Agent reports the provider's version and commit `unknown` (`<argv0>/<version> (<os>/<arch>) kubernetes/unknown`). Go reports client-go's build version and commit (`.../v0.0.0 (linux/arm64) kubernetes/$Format`). | Forced: a Rust binary has no client-go version to report. | Audit `userAgent` and the `managedFields` manager name derived from it. The differ drops `managedFields` and does not compare `userAgent`. | `docs/design/core.md` unit 1.4 and unit 1.9 rest; `decisions.tsv` line 49; `crates/crossplane-runtime/src/manager.rs` `user_agent` |
| Calls to the external service carry no User-Agent. Go's sample client sends `Go-http-client/1.1`. | Deliberate: hyper sets none, and naming Go's client would be false. | External calls `userAgent` field. Not compared; the differ tells provider calls from scenario calls by `X-Forwarded-For`. | `parity/.work/runs/go-a/calls.jsonl`; `examples/provider-sample/src/provider/client.rs` |
| A transport failure in the sample's HTTP client reads `Get "<url>": client error (Connect): ...`. Go's reads `Get "<url>": dial tcp ...`. | Forced: the cause text comes from hyper, not Go's `net` package. The `<Method> "<url>": ` frame is Go's `url.Error`. | Condition and event messages when the service is unreachable. The `wrapped-error-tail` rule replaces the tail after a runtime prefix. | `examples/provider-sample/src/provider/client_test.rs` `an_unreachable_service_names_the_request`; `parity/README.md` normalization rules |
| `controller_runtime_*` and `workqueue_*` metrics are not served. The endpoint serves only what a provider registers (`crossplane_managed_resource_*` and the managed reconciler's). | Not ported; no parity layer compares metrics. | Metrics. | `docs/design/core.md` unit 1.9 rest; `decisions.tsv` line 48 |
| When an event repeats (count above 1) but the correlator produced no patch, Rust creates it. Go's `recordEvent` sends `Patch` with the nil patch and creates only if that finds no event. | Not recorded in the unit's note. | Audit trace of Event writes: a PATCH in Go, a create in Rust, in that case only. Events are compared by (object, type, reason) and message. | `docs/design/core.md` unit 1.9 rest; client-go `tools/record/event.go:330-341`; `crates/crossplane-runtime/src/event.rs` `record_event` |
| A null `eventTime` is left out of the Event body. Go sends `"eventTime":null`. | Forced: k8s-openapi writes an `Option` field only when it is `Some` (its `core/v1/event.rs:292`). | Event request bodies only. The audit policy keeps no bodies. | `docs/design/core.md` unit 1.9 rest |
| The applicators' merge-patch body always carries `apiVersion` and `kind`. Go marshals a typed object whose TypeMeta may be empty. | Forced: the body comes from `Object::to_json`, which always writes both. | Audit request body of the connection Secret patch: Go's body has neither field. The stored object is the same (checked live: the same merge patch with and without the two fields gave identical Secrets, `managedFields` included). Allowed by `parity/allow/go-vs-rust.jsonl`. | `docs/design/core.md` unit 1.5; `decisions.tsv` line 47; `parity/scripts/diff.sh go-h rust-4`: 3 differences on the `connection-secret` Secret patches, each `.apiVersion: (absent) -> "v1"` and `.kind: (absent) -> "Secret"` |
| `meta::remove_finalizer` removes every copy of a finalizer. Go's `RemoveFinalizer` removes from the slice it is ranging over, so with a duplicate it panics or leaves a copy, depending on position. Inferred, not run: controller-runtime's panic recovery turns the panic into a reconcile error. | The Rust function uses `retain`; no note says whether copying Go was considered. | Audit trace, only for an object with a duplicate finalizer. | Go `pkg/meta/meta.go:191-200`; `crates/crossplane-core/src/meta.rs` `remove_finalizer`. Go's loop, run at integration: `[a b a]` and `[a a]` panic with `slice bounds out of range`, `[a a b]` leaves `[a b]`. No test pins it. |
| With leader election on, no namespace given and no in-cluster namespace file, the Rust manager fails in `Manager::start`. Go fails in `ctrl.NewManager`, which builds the resource lock. Same error text for a missing namespace file. | Follows from building the elector in `start`, not in `new`. | None: the process exits before any reconcile either way. | controller-runtime `pkg/manager/manager.go:406-420` and `pkg/leaderelection/leader_election.go:84-88`; `crates/crossplane-runtime/src/manager.rs` `elector`, called from `run` |
| Work queue priorities are not ported. The queue keeps controller-runtime's per-object priority-queue behaviour (one entry per key, an add cancels a pending delay, shutdown does not drain) but orders ready keys only by arrival. | Deliberate: priorities only order different objects. | Interleaving of different objects' writes in the audit trace. The differ compares per-object sequences. | `docs/port-skips.tsv` (priorityqueue and `TestSource` priority rows); `docs/design/core.md` unit 1.9 queue half; `decisions.tsv` line 43 |
| The informer cache has no `ErrCacheNotStarted`. A read before `Manager::start` starts the informer instead of failing. | The note says an informer starts on first use, before or after start; it gives no further reason. | None: a provider never reads before start. | `docs/design/core.md` unit 1.9 rest |
| A change log entry for a failed Create, Update or Delete has no `additionalDetails`. Go returns details beside the error and logs them. | Forced: a Rust `Result` carries a value or an error, not both. | Change logs, off in the baseline unless `--enable-changelogs` is set. | `docs/design/core.md` unit 1.6 |
| The MR metric recorder observes nothing for an object with no creation or deletion timestamp. Go observes the seconds since year one. | The Rust recorder skips a missing timestamp; the note gives no further reason. | Metrics, only for objects without timestamps; objects from the API server always have `creationTimestamp`. | `docs/design/core.md` unit 1.6 |
| A Rust provider cannot install `transform_strip_crd_schema` in its cache, since the cache has no public options. A Go provider can, through `cache.Options`. | The baseline does not install it, so the Rust cache matches the baseline by keeping whole CRDs. | None: memory use only. | `docs/design/core.md` units 1.7 and 1.8 and "Integration"; baseline `cmd/provider/main.go:95-97` |
| The sample's `main.rs` reads only the environment variables Go names with `Envar(...)`. Go's kingpin app also sets `DefaultEnvars()`, which derives a variable for every flag. | Recorded in the scaffold notes; not ported. | None unless a deployment sets a flag through a derived variable, which the Rust binary would ignore. | `docs/design/core.md` "Implementation reconciliation"; baseline `cmd/provider/main.go:57`; `examples/provider-sample/src/main.rs` |
| Legacy cluster-scoped managed resources, the legacy usage-kind probe (`legacyPCU`), the legacy Secret publisher, composition wrapper clients and scheme helpers are not ported. | Deliberate scope: modern namespaced resources only. | None for the baseline, whose kinds are namespaced. | `decisions.tsv` lines 8, 11, 17; `docs/port-skips.tsv` |
| The body of a metadata update (PUT to the main resource) carries no `status` until the object has one. Go marshals its zero-value status struct, so its body has `"status":{}` for a ProviderConfig and `"status":{"atProvider":{"configurableField":""}}` for the sample `MyType`. | Forced: kube's derived kinds hold status as an `Option`, written only when present. Go's status is a non-pointer struct. | Audit request body of the first metadata updates of an object. The stored object is the same: the API server ignores `status` on a write to the main resource when the CRD has a status subresource (checked live: a replace carrying `status.users: 42` stored `status: {}`). Allowed by `parity/allow/go-vs-rust.jsonl`. | `parity/scripts/diff.sh go-a rust-1`: 13 differences, all of this form, and none with the allowance. |

Write verbs. `decisions.tsv` line 13 asks for every case where Go updates and Rust patches. There
is none: the non-test patch call sites are the patching applicator (a merge patch, as Go's
`APIPatchingApplicator`), the reference resolver (a server-side apply, as Go's) and the event
sink (a strategic merge patch of a repeated event, as client-go's), in
`crates/crossplane-runtime/src/{resource/applicator.rs,reconciler/managed/collaborators.rs,event.rs}`.

## Unverified

- **The reference resolver's apply-patch body.** Rust builds it in the kind's wire shape, with
  `apiVersion`, `kind` and `metadata.name`. Go's ported test expects `{"name":"resolved"}`, but
  that comes from Go's fake, which has no TypeMeta (`docs/design/core.md` unit 1.6,
  `TestPrepareJSONMerge`). Whether a real Go provider's body differs is not known. The baseline
  kind has no references, so the resolver never patches in a parity run. A trace from a
  provider with a reference field would settle it.
- **Pointing the manager at a chosen kubeconfig.** `RestConfig::infer` reads only the
  environment (`KUBECONFIG`, then in-cluster config). The Go baseline calls `ctrl.GetConfig`,
  which also reads a `--kubeconfig` flag registered on Go's `flag` set. The baseline parses its
  flags with kingpin, so whether that flag is ever parsed is not known. Running the baseline
  binary with `--kubeconfig` would settle it.
- **Requeue delays run long.** Over the 1, 2, 4, 8 and 16s backoff that holds a deleted MR
  inside the creation grace period, the Rust provider's `PendingExternalResource` event writes
  came 7.8, 3.0, 4.2, 6.2 and 19.7ms later than the delay, the Go provider's 3.9, 1.2, 0.9, 0.6 and 0.8ms
  (`parity-poll-interval` in runs rust-4 and go-h, from `audit-raw.jsonl`). The work queue
  sleeps with `tokio::time::sleep_until` on the exact deadline and the backoff arithmetic matches
  client-go, so the cause is not known. No compared layer shows it, but it moved Rust's last
  ProviderConfig writes about 35ms later at the end of a test, into the cleanup race that the
  2s `delayBeforeCleanup` in `parity/scenarios/chainsaw.yaml` now removes (rust-5 and rust-6).

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
| An event that reaches the sink already at count above 1, with no cached observation, is created by Rust. Go's `recordEvent` PATCHes it with a nil patch (an empty body) and creates only if that finds no event. | Not ported: no recorder produces such an event. Both start an event at count 1 (client-go v0.36.3 `tools/record/event.go:504`, `events_cache.go:283`; Rust `make_event`, `Correlator::aggregate`). Count passes 1 only in `eventObserve`'s cached-observation branch, which builds the patch too (`events_cache.go:338-353`, Rust `Correlator::observe`). Go's patch is nil there only if `CreateTwoWayMergePatch` fails, which no recorded event was seen to do. Porting would add a zero-length strategic merge body for a branch nothing reaches. | None: every repeat a recorder sends is a PATCH with the same three fields (`count`, `lastTimestamp`, `message`) on both sides. | `parity/scripts/go-checks.sh` (`TestRepeatAlwaysHasPatch`: counts 1, 2, 3, a patch on each repeat); client-go v0.36.3 `tools/record/event.go:330-341`; `crates/crossplane-runtime/src/event.rs` `record_event`; `event_test.rs` (the `expect_update` table) |
| A null `eventTime` is left out of the Event body. Go sends `"eventTime":null`. | Forced: k8s-openapi writes an `Option` field only when it is `Some` (its `core/v1/event.rs:292`). | Event request bodies only. The audit policy keeps no bodies. | `docs/design/core.md` unit 1.9 rest |
| The applicators' merge-patch body always carries `apiVersion` and `kind`. Go marshals a typed object whose TypeMeta may be empty. | Forced: the body comes from `Object::to_json`, which always writes both. | Audit request body of the connection Secret patch: Go's body has neither field. The stored object is the same (checked live: the same merge patch with and without the two fields gave identical Secrets, `managedFields` included). Allowed by `parity/allow/go-vs-rust.jsonl`. | `docs/design/core.md` unit 1.5; `decisions.tsv` line 47; `parity/scripts/diff.sh go-h rust-4`: 3 differences on the `connection-secret` Secret patches, each `.apiVersion: (absent) -> "v1"` and `.kind: (absent) -> "Secret"` |
| `meta::remove_finalizer` removes every copy of a finalizer. Go's `RemoveFinalizer` removes from the slice it is ranging over, so with a duplicate it panics or leaves a copy, depending on position. Inferred, not run: controller-runtime's panic recovery turns the panic into a reconcile error. | The Rust function uses `retain`; no note says whether copying Go was considered. | Audit trace, only for an object with a duplicate finalizer. | Go `pkg/meta/meta.go:191-200`; `crates/crossplane-core/src/meta.rs` `remove_finalizer`. Go's loop, run at integration: `[a b a]` and `[a a]` panic with `slice bounds out of range`, `[a a b]` leaves `[a b]`. No test pins it. |
| With leader election on, no namespace given and no in-cluster namespace file, the Rust manager fails in `Manager::start`. Go fails in `ctrl.NewManager`, which builds the resource lock. Same error text for a missing namespace file. | Follows from building the elector in `start`, not in `new`. | None: the process exits before any reconcile either way. | controller-runtime `pkg/manager/manager.go:406-420` and `pkg/leaderelection/leader_election.go:84-88`; `crates/crossplane-runtime/src/manager.rs` `elector`, called from `run` |
| Work queue priorities are not ported. The queue keeps controller-runtime's per-object priority-queue behaviour (one entry per key, an add cancels a pending delay, shutdown does not drain) but orders ready keys only by arrival. | Deliberate: priorities only order different objects. | Interleaving of different objects' writes in the audit trace. The differ compares per-object sequences. | `docs/port-skips.tsv` (priorityqueue and `TestSource` priority rows); `docs/design/core.md` unit 1.9 queue half; `decisions.tsv` line 43 |
| The informer cache has no `ErrCacheNotStarted`. A read before `Manager::start` starts the informer instead of failing. | The note says an informer starts on first use, before or after start; it gives no further reason. | None: a provider never reads before start. | `docs/design/core.md` unit 1.9 rest |
| A change log entry for a failed Create, Update or Delete has no `additionalDetails`. Go returns details beside the error and logs them. | Forced: a Rust `Result` carries a value or an error, not both. | Change logs, off in the baseline unless `--enable-changelogs` is set. | `docs/design/core.md` unit 1.6 |
| The MR metric recorder observes nothing for an object with no creation or deletion timestamp. Go observes the seconds since year one. | The Rust recorder skips a missing timestamp; the note gives no further reason. | Metrics, only for objects without timestamps; objects from the API server always have `creationTimestamp`. | `docs/design/core.md` unit 1.6 |
| A Rust provider cannot install `transform_strip_crd_schema` in its cache, since the cache has no public options. A Go provider can, through `cache.Options`. | The baseline does not install it, so the Rust cache matches the baseline by keeping whole CRDs. | None: memory use only. | `docs/design/core.md` units 1.7 and 1.8 and "Integration"; baseline `cmd/provider/main.go:95-97` |
| With `KUBECONFIG` unset, `RestConfig::infer` (kube's `Config::infer`) loads `~/.kube/config` before in-cluster config, and falls back to in-cluster config after any kubeconfig error. Go's `ctrl.GetConfig` tries in-cluster config first when `KUBECONFIG` is unset, and falls back to it only for an empty or missing kubeconfig; a malformed one is an error. | Follows from kube's `Config::infer`; no note says whether Go's order was considered. | None in a provider pod: `KUBECONFIG` is unset and there is no home kubeconfig, so both use in-cluster config. A binary run outside a cluster with a home kubeconfig uses it on both sides. | controller-runtime v0.24.1 `pkg/client/config/config.go:115-154`; client-go v0.36.3 `tools/clientcmd/merged_client_builder.go:104-124`; kube-client 5.0.0 `src/config/mod.rs:237-254`; `crates/crossplane-runtime/src/manager.rs` `RestConfig::infer` |
| Legacy cluster-scoped managed resources, the legacy usage-kind probe (`legacyPCU`), the legacy Secret publisher, composition wrapper clients and scheme helpers are not ported. | Deliberate scope: modern namespaced resources only. | None for the baseline, whose kinds are namespaced. | `decisions.tsv` lines 8, 11, 17; `docs/port-skips.tsv` |
| The body of a metadata update (PUT to the main resource) carries no `status` until the object has one. Go marshals its zero-value status struct, so its body has `"status":{}` for a ProviderConfig and `"status":{"atProvider":{"configurableField":""}}` for the sample `MyType`. | Forced: kube's derived kinds hold status as an `Option`, written only when present. Go's status is a non-pointer struct. | Audit request body of the first metadata updates of an object. The stored object is the same: the API server ignores `status` on a write to the main resource when the CRD has a status subresource (checked live: a replace carrying `status.users: 42` stored `status: {}`). Allowed by `parity/allow/go-vs-rust.jsonl`. | `parity/scripts/diff.sh go-a rust-1`: 13 differences, all of this form, and none with the allowance. |

Write verbs. `decisions.tsv` line 13 asks for every case where Go updates and Rust patches. There
is none: the non-test patch call sites are the patching applicator (a merge patch, as Go's
`APIPatchingApplicator`), the reference resolver (a server-side apply, as Go's) and the event
sink (a strategic merge patch of a repeated event, as client-go's), in
`crates/crossplane-runtime/src/{resource/applicator.rs,reconciler/managed/collaborators.rs,event.rs}`.

Checked equal, not listed above. `parity/scripts/go-checks.sh` runs each against the baseline's module versions, with no cluster.

- **The reference resolver's apply body.** A Go provider's resolver sends `apiVersion`, `kind` and the changed fields, with no
  `metadata` unless resolution changed it. The cached `Get` sets the GVK (controller-runtime v0.24.1
  `pkg/cache/internal/cache_reader.go:104`), and the client restores it after every typed write (`pkg/client/client.go:267`), so
  an earlier `Update` in the same reconcile (`NameAsExternalName`) does not clear it. `prepareJSONMerge` blanks the existing
  object's GVK, so both type fields reach the patch. `TestAfterCachedGet` and `TestAfterTypedUpdate` run the real
  `APISimpleReferenceResolver` through controller-runtime's typed client against a recording HTTP server. Both bodies are
  `{"apiVersion":...,"kind":"MyType","spec":{...}}`. The Rust body has the same shape (`api_test.rs`
  `resolve_references_applies_the_go_client_body`). The `metadata.name` in the ported `TestPrepareJSONMerge` is there
  only because that case changes the name.
- **`--kubeconfig`.** Neither binary takes it. `ctrl.GetConfig` registers it on Go's `flag.CommandLine`, which the baseline
  never parses: kingpin parses `os.Args`, prints `error: unknown long flag '--kubeconfig', try --help` and exits 1, and
  `--help` does not list the flag. Matching the baseline therefore needs no flag in the Rust template. A provider that wants one
  would parse it itself and build its config from that path.

## Explained

- **A delayed requeue fires about 0.7ms later than Go's.** The provider removes an MR's
  finalizer after the 30s creation grace period, at the end of a chain of rate-limited requeues,
  so the delay from the scenario's delete to that update sums every requeue's reconcile time
  (`parity/scripts/finalizer-lag.py`). Medians: Go 31012 to 31013ms (go-h to go-l); Rust 31046
  to 31057ms (rust-5 to rust-8, minimum 31040); Rust with one shared HTTP connection pool
  31018ms (rust-pool-1, all 10 samples 31015 to 31029). The template had built a new pool on
  every connect, so every reconcile opened a new connection to the external service; Go's
  `http.Client` reuses `http.DefaultTransport`. Why a new connection cost several ms (DNS
  search-path lookups, TCP connect) was inferred, not measured. About 6ms of excess over the
  chain was left, explained below. The excess lag had pushed Rust's last ProviderConfig writes into
  the cleanup window: without a cleanup delay, the gap from the last ProviderConfig
  write to cleanup's delete was 10 to 34ms at its smallest per Go run (go-e to go-j) and as little as 0 to 1ms on
  Rust (rust-4 to rust-6), which had ProviderConfig 409s in five tests against one in go-h
  (`parity/scripts/pc-settle.py` and the audit). The single event intervals once used here
  (1 to 20ms) were too few samples to show the change.
  With the shared pool and no cleanup delay, rust-nodelay-1 to 3 are each equal to go-h, go-i and
  go-j (9 of 9), with a smallest gap of 12 to 29ms, so the scenarios run without the delay.
  The rest of the excess is the timer. The chain is six reconciles: one on the delete, then
  requeues after 1, 2, 4, 8 and 16s. Between the provider's writes, the 2, 4, 8 and 16s gaps run
  a median 0.9 to 1.5ms over nominal on Go (go-h to go-l) and 1.6 to 2.4ms on Rust (rust-pool-1,
  rust-nodelay-1 to 3); the 1s gap, which also carries the first reconcile, is 1003.7 to 1004.8ms
  on Go and 1004.8 to 1005.5ms on Rust. The delete-to-first-write interval is level (3.2 to
  3.7ms on both), so the excess comes once per delayed wake-up, not from one slow reconcile. tokio rounds every sleep deadline
  up to the end of a millisecond (tokio 1.54.1 `runtime/time/source.rs:17-19`). Go's priority
  queue waits on Go's runtime timer (`time.Tick`, controller-runtime v0.24.1
  `pkg/controller/priorityqueue/priorityqueue.go:335`). Measured on Linux, in a container on the Docker daemon the kind cluster uses
  (`parity/timer-probe/run.sh`, 3 rounds): a tokio `sleep_until` 50ms plus a sub-millisecond
  offset away fires a median 1.40 to 1.57ms late, and controller-runtime's `AddAfter` then `Get`
  0.62 to 0.77ms late. That is about 0.7ms a wake, about 3.5ms over five wakes. The Rust queue
  adds nothing to tokio's sleep: `probe_add_after_overshoot` in `workqueue_test.rs` (macOS,
  release) gives a median 2.64ms for `add_after` then `get` against 2.65ms for a bare sleep.
  No gap shows a rate-limiter error: each is its nominal 1, 2, 4, 8 or 16s plus about a millisecond.
  Not changed: matching Go would mean waking early on tokio's timer and finishing the wait on a
  finer clock, for 0.7ms a requeue that no parity layer shows. A smaller per-reconcile cost was
  seen in the run's own log: under `--debug` the Rust filter was `debug` for every crate, so
  hyper, tower and kube's client logged each request (24472 lines in rust-nodelay-1 against 590
  in go-k), 10 to 15us apart, roughly 0.1ms a reconcile, inferred from the timestamps. Go's
  `--debug` never traces requests, since client-go logs through klog at its default verbosity.
  Fixed: `--debug` now keeps the transport crates at `info` (`logging.rs`,
  `debug_logs_the_provider_at_debug_but_not_each_http_request`).

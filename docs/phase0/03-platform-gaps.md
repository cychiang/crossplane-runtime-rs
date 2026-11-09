# Phase 0.3: controller-runtime and client-go behaviors versus kube-rs

This document is reference material for the Rust port. It lists each behavior a Go Crossplane provider gets from controller-runtime and client-go. For each behavior it states what the Go side does, what `kube` and `kube-runtime` offer, the gap, and options to close it. Every recommendation is a proposal. Items marked **User decision** are the user's call.

## Sources and citation keys

Citations use `key:path:line`. Each key maps to one pinned source tree.

| Key | Tree | Upstream URL for the same lines |
|---|---|---|
| `xpr` | crossplane-runtime v2.4.3 snapshot in the session scratchpad (`xpr-v2.4.3/`) | https://github.com/crossplane/crossplane-runtime/blob/v2.4.3/ |
| `base` | xp-provider-gen scaffold `provider-baseline` in the session scratchpad | none (generated locally) |
| `cr` | controller-runtime v0.24.1 in the Go module cache, the version the baseline binary links (`base:go.mod:17`) | https://github.com/kubernetes-sigs/controller-runtime/blob/v0.24.1/ |
| `cg` | client-go v0.36.3 in the Go module cache (`base:go.mod:16`) | https://github.com/kubernetes/client-go/blob/v0.36.3/ |
| `kp` | kingpin v2.4.0 in the Go module cache | https://github.com/alecthomas/kingpin/blob/v2.4.0/ |
| `kube` | kube-rs 5.0.0 source tarball | https://github.com/kube-rs/kube/blob/5.0.0/ |
| `xp` | Crossplane core v2.4.3 source tarball | https://github.com/crossplane/crossplane/blob/v2.4.3/ |

All trees were read on 2026-11-09. The baseline links controller-runtime v0.24.1 and client-go v0.36.3, not the v0.23.1 and v0.35.3 that the runtime's own `go.mod` pins (`xpr:go.mod:28,32`). The Go provider under test runs v0.24.1, so this document cites v0.24.1. See `04-version-pins.md` for what the mismatch means.

The Go provider's wiring lives in `base:cmd/provider/main.go:55-163`, `base:internal/controller/mytype/wiring.go:47-94`, and `base:internal/controller/config/config.go:39-56`.

## Manager lifecycle and signal handling

**Go side.** `ctrl.NewManager` builds the manager (`base:cmd/provider/main.go:92`). `mgr.Start(ctrl.SetupSignalHandler())` runs it (`base:cmd/provider/main.go:162`). The signal handler cancels the context on the first SIGINT or SIGTERM and calls `os.Exit(1)` on the second (`cr:pkg/manager/signals/signal.go:27-45`, `cr:pkg/manager/signals/signal_posix.go:26`). On stop, runnables get a 30 second graceful shutdown window (`cr:pkg/manager/internal.go:57`). Losing the leader lease skips graceful shutdown and fails the manager with "leader election lost" (`cr:pkg/manager/internal.go:627-638`). The manager also starts informers, waits for cache sync (2 minute default `CacheSyncTimeout`, `cr:pkg/controller/controller.go:248-250`), and runs runnables added after start, which the safe-start gate depends on.

**kube-rs.** There is no manager. `Controller::shutdown_on_signal` stops one controller on SIGINT or SIGTERM and forces exit on a second signal (`kube:kube-runtime/src/controller/mod.rs:1480`). `graceful_shutdown_on` takes any future as the trigger (`kube:kube-runtime/src/controller/mod.rs:1458`). Each `Controller` owns its own watcher and store.

**Gap.** Nothing coordinates several controllers, the metrics server, the state-metrics ticker, the gate, and leader election under one shutdown. Nothing bounds shutdown time.

**Options.**
1. Write a small `Manager` in `crossplane-runtime-rs`. It holds a `tokio_util::sync::CancellationToken`, a `JoinSet` of runnables, a 30 second shutdown deadline, and an "add after start" method. Each controller gets `graceful_shutdown_on(token.cancelled())`.
2. Use `shutdown_on_signal` on each controller and let `main` join them. This has no shared deadline and no late-add.

**Recommendation.** Option 1. The safe-start gate needs late-add, so a manager exists either way.

## Cached client versus direct API reads

**Go side.** `mgr.GetClient()` reads structured objects from the informer cache. It writes to the API server directly (`cr:pkg/cluster/cluster.go:206-213`). Unstructured objects bypass the cache by default (`cr:pkg/cluster/cluster.go:207-209`, `cr:pkg/client/client.go:242-262`). The first `Get` or `List` on a new type starts a cluster-wide informer for that type, which shows up as one `list` and one `watch` per type. Every runtime read goes through this client. Examples are the managed reconciler's `Get` (`xpr:pkg/reconciler/managed/reconciler.go:934`), the ProviderConfig reconciler's `Get` and `List` (`xpr:pkg/reconciler/providerconfig/reconciler.go:157,187`), the usage tracker's `Get` before `Update` (`xpr:pkg/resource/api.go:117`), the connector's ProviderConfig and Secret reads (`base:internal/provider/connector.go:128,140`), and the state-metrics `List` (`xpr:pkg/statemetrics/mr_state_metrics.go:104`).

**kube-rs.** `Api::get` and `Api::list` always call the API server. A `reflector::Store` gives cached reads for one type, and `Controller::store()` exposes the store a controller already keeps (`kube:kube-runtime/src/controller/mod.rs:808`). Nothing creates stores lazily per type.

**Gap.** Reads cost real requests in Rust unless the port routes them through stores. Stale reads matter more than the request count. Go writes often follow a stale cache read, so Go sees `409 Conflict` responses and requeues (`xpr:pkg/errors/reconcile.go:29-35`). A Rust port that reads fresh from the API server will see fewer conflicts. The audit log records those failed writes with their response code. A Go `Create` of a ProviderConfigUsage that is not yet in the cache would return `409 AlreadyExists` (inferred from the Get-then-Create path, not observed). A fresh Rust read never makes that request.

**Audit-log consequence.** The audit log sees only real requests. Cached Go reads are invisible, and direct Rust reads are visible. That is why the comparison in the last section covers writes only. Failed writes still differ, because they come from read staleness.

**Options.**
1. Give the runtime a `CachedClient` that reads from per-type `reflector::Store`s, starts them lazily, and writes through `Api`. This matches Go read staleness most closely.
2. Use direct reads everywhere and exclude non-2xx writes from the audit diff.
3. Use stores only for the reconciled kind and the ProviderConfigUsage kind, and use direct reads for Secrets and ProviderConfigs.

**Recommendation.** Option 1 for the kinds the runtime itself reads. A cluster-wide Secret watch is what Go does today (inferred from lazy informer start, `cr:pkg/cluster/cluster.go:210-213`), so the Rust side should do the same to keep RBAC and audit shape equal. **User decision.** Whether failed writes with code 409 count in the diff.

## Watches, resync, and the poll-interval requeue

**Go side.** Each managed kind has one controller. `For(&MyType{})` watches it with the `DesiredStateChanged` predicate (`base:internal/controller/mytype/wiring.go:88-93`). The predicate passes generation changes, label changes, and annotation changes, except the two create annotations (`xpr:pkg/resource/predicates.go:55-69`). Status-only writes and informer resyncs match none of the three predicates, so they do not trigger a reconcile. The cache resync period is `--sync`, default 1h (`base:cmd/provider/main.go:61,95-97`). The MR controller filters resync events. The ProviderConfig controller has no predicate (`base:internal/controller/config/config.go:48-52`), so resync triggers it. Drift detection uses requeue instead. When the external resource is up to date, the reconciler returns `RequeueAfter: pollIntervalHook(effectivePollInterval)` (`xpr:pkg/reconciler/managed/reconciler.go:1514`). It returns the same after a successful external update (`xpr:pkg/reconciler/managed/reconciler.go:1586`). The default poll is `--poll`, 1m (`base:cmd/provider/main.go:62`). A per-object poll-interval annotation can override it, down to a minimum (`xpr:pkg/reconciler/managed/reconciler.go:867-875`). The ProviderConfig controller also watches ProviderConfigUsage and maps each one to its ProviderConfig (`xpr:pkg/resource/enqueue_handlers.go:47-76`).

**kube-rs.** `watcher` does list and watch with no periodic resync. `reconcile_all_on` takes a stream and reconciles every object in the store on each item (`kube:kube-runtime/src/controller/mod.rs:1347`). `predicate_filter` is a stable `WatchStreamExt` method (`kube:kube-runtime/src/utils/watch_ext.rs:153`). `Controller::for_stream`, which accepts a filtered stream, needs the `unstable-runtime-stream-control` feature (`kube:kube-runtime/src/controller/mod.rs:637-638`, `kube:kube-runtime/Cargo.toml:15-18`). `reconcile_all_on` is not gated. It takes a function that hashes an object, and it passes an object when the hash changes. Built-in predicates hash generation, resource version, labels, annotations, or finalizers (`kube:kube-runtime/src/utils/predicate.rs:319-344`). `Controller::watches` maps other kinds to the main kind (`kube:kube-runtime/src/controller/mod.rs:1086`). `Action::requeue(d)` schedules a later run (`kube:kube-runtime/src/controller/mod.rs:109`).

**Gap.** Without a predicate, every Rust status write triggers a new reconcile, and the Rust side would loop. The built-in annotation predicate cannot ignore specific keys. Predicates only exist on watch streams, so the `For` stream must be built with `Controller::for_stream`, which is behind an unstable feature. There is no 1h resync for the ProviderConfig controller.

**Options.**
1. Port `DesiredStateChanged` as one hash function over generation, labels, and annotations minus the two ignored keys. Feed it through `predicate_filter` and `Controller::for_stream`. Add `reconcile_all_on(interval(sync))` to the ProviderConfig controller only.
2. Use stable `Controller::new` and apply the same hash check at the top of the reconciler. It keeps the last hash per object and returns the previous `Action` without any write when the hash is unchanged. The reconcile still runs, but the write trace matches.
3. Skip predicates and let the reconciler detect no-op runs. This changes the write trace.

**Recommendation.** Option 1, accepting the unstable feature for Phase 1, with option 2 as the fallback if the feature breaks on a kube upgrade. **User decision.** Whether an `unstable-*` kube feature is acceptable. Note one semantic difference. Go predicates compare the old and new object. kube predicates compare against a cache of hashes with a 1 hour TTL (`kube:kube-runtime/src/utils/predicate.rs:139-152`). Both pass the first sighting of an object.

## Workqueue, rate limiting, and concurrency

**Go side.** There are four layers.
1. Client-side throttle. `LimitRESTConfig` sets QPS to `5 * rps` and Burst to `10 * rps` (`xpr:pkg/ratelimiter/default.go:48-57`). With `--max-reconcile-rate=10` that is QPS 50 and burst 100 (`base:cmd/provider/main.go:65,92`).
2. Global reconcile bucket. `NewGlobal(10)` is a token bucket of 10 per second with burst 100 (`xpr:pkg/ratelimiter/default.go:32-34`). `ratelimiter.Reconciler` wraps every reconciler. When the bucket is empty it returns `RequeueAfter: d` with no error (`xpr:pkg/ratelimiter/reconciler.go:54-98`).
3. Per-item backoff. `ForControllerRuntime` sets an exponential per-item limiter from 1s to 60s (`xpr:pkg/ratelimiter/default.go:42-44`, `xpr:pkg/controller/options.go:83-91`). Without it, controller-runtime would default to 5ms to 1000s (`cr:pkg/controller/controller.go:252-258`).
4. Queue semantics. controller-runtime v0.24.1 uses its priority queue by default (`cr:pkg/controller/controller.go:260-271`). The reconcile handler has four outcomes (`cr:pkg/internal/controller/controller.go:482-513`). An error re-adds the item rate-limited. `RequeueAfter` forgets the backoff and adds after the delay. `Requeue: true` re-adds rate-limited without an error. Success forgets the backoff. `MaxConcurrentReconciles` is 10, the same flag as the rate (`base:cmd/provider/main.go:125`). `RecoverPanic` is true (`xpr:pkg/controller/options.go:84-89`).

**kube-rs.** The reconciler returns `Result<Action, E>`. On error, the user's `error_policy` returns an `Action` (`kube:kube-runtime/src/controller/mod.rs:93-125,230-232`). There is no per-item failure counter. The scheduler deduplicates by object and keeps the earlier run time (`kube:kube-runtime/src/scheduler.rs:77-90`). `Config::concurrency` defaults to 0, which means unbounded (`kube:kube-runtime/src/controller/mod.rs:436-446`). `Config::debounce` exists (`kube:kube-runtime/src/controller/mod.rs:419-432`). `trigger_backoff` is a watch reconnect backoff, not a per-item backoff (`kube:kube-runtime/src/controller/mod.rs:795-803`, `kube:kube-runtime/src/watcher.rs:1022-1034`). kube-client has no client-side QPS limiter (inferred from source search with no match for qps or rate limit in `kube:kube-client/src`).

**Gap.** Rust has no equivalent of `Requeue: true`, no per-item exponential backoff, no "forget on success", no global bucket, no client QPS throttle, and no default concurrency limit. These decide when writes happen, so they decide the audit trace timing.

**Options.**
1. Port the four layers into the runtime. The per-item limiter is a `HashMap<ObjectRef, u32>` failure counter that resets on success or `RequeueAfter`. `error_policy` and a requeue-rate-limited result read it. The global bucket is a shared token bucket checked before the inner reconcile, with the same `RequeueAfter` behavior. The client throttle is a tower layer, such as `governor`, added through `ClientBuilder`. Set `concurrency(10)`.
2. Write a custom queue that drives `applier` directly and keeps Go's queue semantics. This is more work and closer to the priority queue.

**Recommendation.** Option 1. Model the reconcile outcome as a Rust enum with four variants (error, requeue-after, requeue-rate-limited, done), so the runtime cannot drop one by accident. The priority queue's ordering is not ported. Initial-list items get lower priority in Go. This affects only ordering across objects. The audit diff compares per-object sequences, so cross-object order does not matter.

## Reconcile timeout and context cancellation

**Go side.** The managed reconciler caps each run at `timeout + 30s` and gives the external client `timeout`, default 1m (`xpr:pkg/reconciler/managed/reconciler.go:49-50,927-931`). The ProviderConfig reconciler uses 2m (`xpr:pkg/reconciler/providerconfig/reconciler.go:43,153`). controller-runtime adds no timeout by default (`cr:pkg/config/controller.go:89-91`). Cancellation is cooperative. Code checks `ctx`, and the HTTP client aborts in-flight requests.

**kube-rs.** No reconcile timeout. `tokio::time::timeout` drops the future at its next `.await` point.

**Gap.** A dropped future cannot run its own cleanup. A Go reconcile that times out mid-external-call still returns, records an event, and writes status with the remaining 30 seconds of grace. A dropped Rust future does nothing after the drop.

**Options.**
1. Pass a `CancellationToken` or deadline into `ExternalClient` calls. Wrap only the external call in `timeout(1m)`. Keep the 30s grace for the status write after it. This mirrors the two Go contexts.
2. Wrap the whole reconcile in `timeout(90s)` and accept lost status writes on timeout.

**Recommendation.** Option 1.

## Leader election

**Go side.** Leader election is off by default (`--leader-election`, env `LEADER_ELECTION`, `base:cmd/provider/main.go:59`). When on, it uses a Lease named `crossplane-leader-election-provider-baseline`, a 60s lease duration, a 50s renew deadline, and the default 2s retry period (`base:cmd/provider/main.go:106-110`, `cr:pkg/manager/internal.go:56`). The namespace comes from the in-cluster service account file (`cr:pkg/leaderelection/leader_election.go:84-87,136-147`). Losing the lease ends the process (`cr:pkg/manager/internal.go:627-638`). Crossplane core v2.4.3 sets no leader election flag or env on the provider Deployment. A case-insensitive search for "leader" in `xp:internal/controller/pkg/runtime/*.go` finds only a comment (`xp:internal/controller/pkg/runtime/runtime.go:386`).

**kube-rs.** kube-runtime 5.0.0 has no leader election module (`kube:kube-runtime/src/lib.rs`). `kube-lease-manager` 0.13.1 supports kube 5. It updates the Lease with server-side apply and reports state on a tokio watch channel (https://github.com/alex-karpenko/kube-lease-manager README, read 2026-11-09).

**Gap.** Lease write shape differs. client-go updates the Lease, and kube-lease-manager applies it. Exit-on-loss behavior is the caller's job.

**Options.**
1. Use `kube-lease-manager` and exit the process on loss.
2. Write about 150 lines that port client-go's update-based lease loop. This gives an identical audit shape.
3. Defer it. The e2e runs one replica with leader election off.

**Recommendation.** Option 3 for parity, then option 1. Exclude `coordination.k8s.io/leases` from the audit diff in all cases. **User decision.** Whether leader election is in Phase 1 scope.

## Event recording

**Go side.** The reconcilers use `mgr.GetEventRecorderFor(name)` (`base:internal/controller/mytype/wiring.go:55`, `base:internal/controller/config/config.go:55`). That is the deprecated core/v1 path (`cr:pkg/manager/internal.go:263-264`, `cr:pkg/internal/recorder/recorder.go:113-117,215`). `event.APIRecorder` calls `AnnotatedEventf` with an `external-name` annotation (`xpr:pkg/event/event.go:93-106`, `xpr:pkg/reconciler/managed/reconciler.go:944`). client-go aggregates and deduplicates. It keeps an LRU of 4096 entries, aggregates after 10 similar events within 600 seconds, and rate-limits each object to a burst of 25 and then 1 per 300 seconds (`cg:tools/record/events_cache.go:36-48,435-447`). Repeats become `PATCH` on the existing Event with a higher `count` (`cg:tools/record/event.go:330-340`). Writes retry up to 12 times, 10 seconds apart (`cg:tools/record/event.go:39-41`).

**kube-rs.** `kube_runtime::events::Recorder` writes `events.k8s.io/v1` Events (`kube:kube-runtime/src/events.rs:4-5`). It caches events for 6 minutes and turns repeats into a merge `PATCH` that bumps `series.count` (`kube:kube-runtime/src/events.rs:21,311-347`). It has no spam filter. Generated events carry no annotations (`kube:kube-runtime/src/events.rs:260-292`).

**Gap.** This is the biggest deployment gap. Crossplane core grants providers `events` only in the core API group and `coordination.k8s.io` (`xp:internal/controller/rbac/provider/roles/roles.go:73-78`). A search for `events.k8s.io` in `xp:internal/controller/rbac` finds nothing. So the stock kube `Recorder` gets `403 Forbidden` under Crossplane-managed RBAC (inferred from the RBAC rules, not run). It also drops the external-name annotation, uses a different count field, and uses a different aggregation window.

**Options.**
1. Write a core/v1 recorder in the runtime that ports client-go's correlator. It keeps the same keys, the same 10-in-600s aggregation, the same spam filter, the annotations, and `PATCH` on repeat.
2. Use kube's `Recorder` and ask Crossplane core for `events.k8s.io` RBAC. This is out of the port's control.
3. Write a minimal core/v1 recorder with no aggregation. It is simple, but the audit trace differs after the 10th repeat.

**Recommendation.** Option 1. It is the only option that works under stock Crossplane RBAC and matches the audit trace.

## Update, patch, server-side apply, field managers, and user-agent

**Go side.** The runtime uses full `Update` (PUT) for metadata and spec. Examples are the finalizer add and remove (`xpr:pkg/resource/api.go:165-183`), the external-name initializer (`xpr:pkg/reconciler/managed/api.go:76`), critical annotations (`xpr:pkg/reconciler/managed/api.go:288`), the ProviderConfig finalizer (`xpr:pkg/reconciler/providerconfig/reconciler.go:230,241`), and the usage tracker (`xpr:pkg/resource/api.go:104-130`). Status goes through `Status().Update` (`xpr:pkg/reconciler/managed/reconciler.go:978,1005`). One SSA patch exists. The reference resolver applies with field owner `managed.crossplane.io/api-simple-reference-resolver` and force (`xpr:pkg/reconciler/managed/api.go:40-42,262`). No default field owner is set, so for PUT the API server derives the managedFields manager from the user-agent. client-go's user-agent is `<basename of argv[0]>/<version> (<os>/<arch>) kubernetes/<commit>` (`cg:rest/config.go:515-536`). For the baseline image, argv[0] is `crossplane-provider-baseline-provider` (`base:cluster/images/provider-baseline/Dockerfile:6,9`).

**kube-rs.** `Api::replace` is PUT (`kube:kube-client/src/api/core_methods.rs:501`). `replace_status` and `patch_status` exist (`kube:kube-client/src/api/subresource.rs:491,523`). `Patch::Apply` with `PatchParams::apply(manager).force()` is SSA. kube-client 5.0.0 sets no User-Agent header. A search for "agent" in `kube:kube-client/src` finds nothing. `Config::headers` adds headers to every request (`kube:kube-client/src/config/mod.rs:197`, `kube:kube-client/src/client/builder.rs:359`).

**Gap.** Without a User-Agent, the audit `userAgent` is empty and the managedFields manager name differs (inferred, not observed). kube's built-in finalizer helper uses JSON patch with a `test` op (`kube:kube-runtime/src/finalizer.rs:133,166,223`). The Go side uses PUT, so the helper would change the write verb.

**Options.**
1. Mirror Go verb for verb. Use `replace` wherever Go uses `Update`, `replace_status` for `Status().Update`, and SSA with the same field owner for the reference resolver. Set `User-Agent` to `<binary>/<version> (linux/<arch>) kubernetes/<commit>` through `Config::headers`. Do not use `kube_runtime::finalizer`.
2. Use patches where they are cleaner, and treat update and patch as equal in the diff.

**Recommendation.** Option 1. **User decision.** The exact user-agent string. It can copy Go's binary name, or use a Rust name and let the diff normalize it.

## Optimistic concurrency and conflict retry

**Go side.** PUT carries `resourceVersion`, so a stale write gets 409. The managed reconciler turns any conflict into `Requeue: true` with no error (`xpr:pkg/errors/reconcile.go:29-35`, `xpr:pkg/reconciler/managed/reconciler.go:922`). It does not emit a warning event for conflicts (`xpr:pkg/reconciler/managed/reconciler.go:1387`). Critical annotations retry with `retry.DefaultRetry`. On conflict they re-read and re-add the annotations (`xpr:pkg/reconciler/managed/api.go:281-298`).

**kube-rs.** `replace` sends whatever `resourceVersion` the object holds, so 409 works the same. kube-client retries 429, 503, and 504 by default (`kube:kube-client/src/config/mod.rs:198-199,224`, `kube:kube-client/src/client/builder.rs:357`). There is no `retry.OnError` helper.

**Gap.** Small. Port `SilentlyRequeueOnConflict` and `retry.DefaultRetry`, which are 5 steps, 10ms, factor 1.0, and jitter 0.1 (`cg:util/retry/util.go:28-33`). kube's default retry layer repeats writes on 429 and 503. client-go also retries those (behavior unverified for v0.36.3). Both show up as duplicate audit entries.

**Recommendation.** Port both helpers. Keep `default_retry` on. The noise-floor run shows whether retries appear at all on kind.

## Status subresource writes

**Go side.** `Status().Update` is `PUT .../status`, which changes only status (`xpr:pkg/reconciler/managed/reconciler.go:1005`). The CRDs enable the status subresource (`base:package/crds/sample.example.com_mytypes.yaml:206-207`). A full `Update` after late-init resets in-memory status, so the reconciler re-applies the reconcile-request token just before the status write (`xpr:pkg/reconciler/managed/reconciler.go:999-1006`).

**kube-rs.** `replace_status` sends the whole object to `/status` (`kube:kube-client/src/api/subresource.rs:523`).

**Gap.** None in the API. The risk is ordering. Rust must reproduce "spec PUT, then status PUT with the in-memory status re-applied". kube returns the server object from `replace`, and that object's status is the stored one. Go's `Update` also overwrites the in-memory object with the response, which is why the token is re-applied. The Rust port must keep the desired status in a separate variable across the spec write.

## Finalizers

**Go side.** `finalizer.managedresource.crossplane.io` (`xpr:pkg/reconciler/managed/reconciler.go:47`) and `in-use.crossplane.io` on ProviderConfigs (`xpr:pkg/reconciler/providerconfig/reconciler.go:41`). Add and remove are PUTs (`xpr:pkg/resource/api.go:165-183`). Remove ignores NotFound.

**kube-rs.** `kube_runtime::finalizer` wraps a reconcile into apply and cleanup events and writes JSON patches (`kube:kube-runtime/src/finalizer.rs:133-223`).

**Gap.** Different verb and a different control-flow shape. Crossplane's reconciler decides itself when to add and remove the finalizer (`xpr:pkg/reconciler/managed/reconciler.go:1057,1298,1340`).

**Recommendation.** Port `APIFinalizer` as PUT. Do not use the kube helper.

## Owner references and garbage collection

**Go side.** ProviderConfigUsage objects get a controller owner reference to their MR with `blockOwnerDeletion: true` (`xpr:pkg/resource/providerconfig.go:191`, `xpr:pkg/meta/meta.go:122-127`). The PCU name is the MR UID (`xpr:pkg/resource/providerconfig.go:188`). The API server's garbage collector deletes the PCU after the MR is gone. The provider does nothing for that.

**kube-rs.** `OwnerReference` comes from k8s-openapi. `Resource::controller_owner_ref` builds one (`kube:kube-core/src/resource.rs:151`).

**Gap.** None in behavior. The owner reference must carry the same `apiVersion` and `kind`. Typed kube objects always know their GVK.

## Unstructured and dynamic objects

**Go side.** In the provider closure, `pkg/resource/unstructured` matters for two things. Interfaces carry claim and composite references (`xpr:pkg/resource/interfaces.go:28,40-41,163`). `AsProtobufStruct` converts an object for change logs (`xpr:pkg/resource/resource.go:478-500`). The `WrapperClient` (`xpr:pkg/resource/unstructured/client.go:45-160`) unwraps typed wrappers around `*unstructured.Unstructured`. Unstructured reads bypass the cache (see the cached client section).

**kube-rs.** `kube::core::DynamicObject` with `ApiResource`, and `serde_json::Value`.

**Gap.** Small. Only the changelog snapshot needs a JSON-to-protobuf-Struct conversion. `prost-types::Struct` stores numbers as `f64`, the same as Go `structpb`.

**Recommendation.** Defer the `WrapperClient` port, because no provider-path code calls it. Port `AsProtobufStruct` with the changelog client.

## CRD generation

**Go side.** controller-gen v0.20.0 generates the CRDs (`base:package/crds/sample.example.com_mytypes.yaml:6`) from markers. The baseline uses these markers and extensions:
- Print columns READY, SYNCED, EXTERNAL-NAME, and AGE with a JSONPath filter (`base:apis/sample/v1alpha1/mytype_types.go:60-63`).
- A status subresource (`base:apis/sample/v1alpha1/mytype_types.go:64`).
- Namespaced scope, categories, and a short name (`base:apis/sample/v1alpha1/mytype_types.go:65`).
- An enum (`base:apis/v1alpha1/types.go:41`, `base:package/crds/sample.example.com_mytypes.yaml:83-89`).
- Defaults `['*']` and `{kind: ClusterProviderConfig, name: default}` (`base:package/crds/sample.example.com_mytypes.yaml:69-71,92-95`).
- `x-kubernetes-list-type: map` with `x-kubernetes-list-map-keys: [type]` on conditions (`base:package/crds/sample.example.com_mytypes.yaml:183-185`).
- `storageversion` and `priority=1` print columns (`base:apis/v1alpha1/types.go:57,61`).

The baseline uses no CEL rules or scale subresource. The crossplane/apis types may carry markers this pass did not read.

**kube-rs.** `#[derive(CustomResource)]` supports `printcolumn`, `shortname`, `category`, `scale`, `status`, `selectable`, `storage`, `served`, `deprecated`, annotations, labels, and CEL `validation` (`kube:kube-derive/src/lib.rs:182-259`). `x_kube(validation)` adds `x-kubernetes-validations` to fields (`kube:kube-derive/src/lib.rs:465-476`). `merge_strategy_property` and `ListMerge::Map(keys)` emit `x-kubernetes-list-type: map` and `x-kubernetes-list-map-keys` (`kube:kube-core/src/cel/mod.rs:415-470`). The structural rewriter turns `#[serde(flatten)]` maps into `x-kubernetes-preserve-unknown-fields` and strips `uniqueItems` (`kube:kube-core/src/schema.rs:21,369-374`). It emits `nullable: true` for `Option` fields (`kube:kube-core/src/schema.rs:51,78`). Defaults come from schemars `#[schemars(default)]`, enums from Rust enums, and formats from Rust integer types.

**Gap.** Expected differences against controller-gen:
- `nullable: true` on every `Option`. controller-gen emits none for `omitempty` pointers.
- Integer formats. `i64` gives `int64`, but `u32` and `usize` give non-Kubernetes formats.
- Default literals. schemars serializes the Rust `Default` value, which may print fields controller-gen omits.
- Description text, from doc comments versus Go comments.
- Field order. This is cosmetic.
- The `metadata` and `apiVersion` stanzas.

**Options for the e2e.**
1. Ship the Go CRDs byte-for-byte in both packages for the differential e2e. Defaulting and validation then cannot differ. Run CRD-generation parity as a separate gate.
2. Ship the Rust-generated CRDs and accept that schema differences may change apiserver defaulting.

**Recommendation.** Option 1 for the behavior diff. Add a separate CRD gate. The gate generates the Rust CRDs to YAML, normalizes both sides with a script (sort keys, drop `description`, drop `controller-gen.kubebuilder.io/version`), and runs `dyff between --ignore-order-changes go.yaml rust.yaml`. The gate passes on an empty diff or on a reviewed allowlist of `nullable` and format entries. A second check runs `kubectl apply --dry-run=server` on each CRD and diffs the stored objects. The API server normalizes defaults there, which catches structural-schema rejections.

**Observation.** The MR default `providerConfigRef.kind` is `ClusterProviderConfig` (`base:package/crds/sample.example.com_mytypes.yaml:94`). The baseline ships no ClusterProviderConfig CRD (`base:package/crds/`). The chainsaw test sets `kind: ProviderConfig` explicitly (`base:test/behavior/mytype-pause/chainsaw-test.yaml:31-33`). Any shared test that omits the kind fails on both sides the same way.

## Deepcopy and scheme registration

**Go side.** `zz_generated.deepcopy.go` and `AddToScheme` (`base:cmd/provider/main.go:114-115`). The managed reconciler builds a new object from the GVK through the scheme and panics if the kind is missing (`xpr:pkg/reconciler/managed/reconciler.go:884-891`).

**kube-rs.** `Clone` replaces deepcopy. Generics over `K: Resource` replace the scheme.

**Gap.** None. The port removes this layer. `newManaged` becomes `K::default()` or a constructor closure.

## Metrics

**Go side.** The controller-runtime metrics server listens on `:8080` at `/metrics` by default (`cr:pkg/metrics/server/server.go:41-45`). Crossplane core declares container port 8080 named `metrics` and adds optional scrape annotations (`xp:internal/controller/pkg/runtime/runtime.go:42-52,375-380`, `xp:internal/controller/pkg/runtime/runtime_provider.go:245`). Default metric families:
- `controller_runtime_reconcile_total`, `controller_runtime_reconcile_errors_total`, `controller_runtime_terminal_reconcile_errors_total`, `controller_runtime_reconcile_panics_total`, `controller_runtime_reconcile_time_seconds`, `controller_runtime_max_concurrent_reconciles`, `controller_runtime_active_workers`, and `controller_runtime_reconcile_timeouts_total` (`cr:pkg/internal/controller/metrics/metrics.go:33-89`).
- `workqueue_depth`, `workqueue_adds_total`, `workqueue_queue_duration_seconds`, `workqueue_work_duration_seconds`, `workqueue_unfinished_work_seconds`, `workqueue_longest_running_processor_seconds`, and `workqueue_retries_total` (`cr:pkg/metrics/workqueue.go:18-28`).
- `rest_client_requests_total` (`cr:pkg/metrics/client_go_adapter.go:35`).
- `leader_election_master_status` and `leader_election_slowpath_total` (`cr:pkg/metrics/leaderelection.go:14,19`).
- Go runtime and process collectors (assumed from the standard prometheus client registration, not read).

The runtime adds `crossplane_managed_resource_first_time_to_reconcile_seconds`, `..._first_time_to_readiness_seconds`, `..._deletion_seconds`, and `..._drift_seconds` with a `gvk` label (`xpr:pkg/reconciler/managed/metrics.go:32,60-85`). It also adds `crossplane_managed_resource_exists`, `..._ready`, and `..._synced` gauges (`xpr:pkg/statemetrics/mr_state_metrics.go:46-59`). The state recorder lists every 5s (`base:cmd/provider/main.go:63`, `xpr:pkg/statemetrics/mr_state_metrics.go:135-145`).

**kube-rs.** None.

**Gap.** Everything. The names and labels are part of the observable contract.

**Options.**
1. Use `prometheus` 0.14 with the same names, buckets, and labels. Serve it on `:8080/metrics` with `axum` or plain `hyper`.
2. Use `prometheus-client` (OpenMetrics). Its text output differs in small ways, such as `_total` handling (label unverified).

**Recommendation.** Option 1. Port the `crossplane_*` families and the `controller_runtime_*` and `workqueue_*` families the runtime can produce. Skip the Go runtime metrics. Metrics are outside the audit diff. A scrape-and-compare-names check is a cheap extra gate.

## Health and readiness probes

**Go side.** `HealthProbeBindAddress` is unset (`base:cmd/provider/main.go:92-111`). An empty address disables the probe server (`cr:pkg/manager/manager.go:473-477`). Crossplane core sets no probes on the provider container. A search for `Probe` in `xp:internal/controller/pkg/runtime/runtime.go` and `runtime_defaults.go` finds none.

**Gap.** None. The Rust binary needs no probe endpoint for parity.

## Logging

**Go side.** `--debug`/`-d` selects zap development mode, a console encoder at debug level (`base:cmd/provider/main.go:58,76`, `cr:pkg/log/zap/zap.go:52-55,171-180`). Without `--debug`, controller-runtime logs go to a discard sink (`base:cmd/provider/main.go:78-87`). Runtime `Debug` is logr `V(1)` (`xpr:pkg/logging/logging.go:75-90`). The local deploy passes `--debug` (`base:build/makelib/local.xpkg.mk:64`).

**kube-rs.** kube uses `tracing`.

**Gap.** Format only. Logs are not compared.

**Recommendation.** Use `tracing-subscriber`. `--debug` gives the pretty format at `debug`, and the default gives JSON at `info`.

## Flags, environment, TLS, and webhooks

**Go side.** kingpin with `DefaultEnvars()` (`base:cmd/provider/main.go:57`). Every flag without an explicit env name gets `<APP>_<FLAG>`, uppercased, with non-alphanumerics turned into `_` (`kp:flags.go:37-43`, `kp:app.go:17,309-314,701-703`). APP is the binary basename, so `--poll` reads `CROSSPLANE_PROVIDER_BASELINE_PROVIDER_POLL` (inferred from `base:cluster/images/provider-baseline/Dockerfile:6`). The explicit env names are `LEADER_ELECTION`, `ENABLE_MANAGEMENT_POLICIES`, `ENABLE_CHANGE_LOGS`, and `CHANGELOGS_SOCKET_PATH` (`base:cmd/provider/main.go:59-69`). `main.go` has no TLS, webhook, or certificate flags. Crossplane core always injects `POD_NAMESPACE`, `PROVIDER_NAME`, `REVISION_NAME`, and `REVISION_UID`. It adds a webhook port and `WEBHOOK_TLS_CERT_DIR` only when a TLS server secret exists (`xp:internal/controller/pkg/runtime/runtime_provider.go:205-264`).

**Recommendation.** Use `clap` with the `env` feature. Keep the same long flags, short flags (`-d`, `-l`, `-s`), defaults, and explicit env names. Derive the prefixed env names from the binary name, as kingpin does. Ignore the injected env vars. Leave TLS and webhooks out of Phase 1.

## gRPC ChangeLogs client

**Go side.** Off by default (`base:cmd/provider/main.go:68,142-155`). When on, it dials `unix:///var/run/changelogs/changelogs.sock` with insecure credentials. `SendChangeLog` sends one `ChangeLogEntry` with a timestamp, provider version, GVK, name, external name, operation, a `google.protobuf.Struct` snapshot, an optional error, and additional details (`xpr:apis/changelogs/proto/v1alpha1/changelog.proto:26-88`). The call uses a 10s timeout and `WaitForReady(true)` (`xpr:pkg/reconciler/managed/changelogger.go:33,85-121`).

**kube-rs.** Not applicable. Use `tonic` 0.14 with `tonic-prost-build` codegen from the same proto, and connect over a `tokio::net::UnixStream` through `Endpoint::connect_with_connector`.

**Gap.** tonic has no per-call `WaitForReady`. A lazy channel with a retry until the 10s deadline gives close behavior.

**Recommendation.** Port it in Phase 1 behind the same flag. It is off in the e2e unless a changelog sidecar is configured. **User decision.** Whether the e2e enables change logs with a sidecar that records entries for a second diff.

## Feature flags and the safe-start gate

**Go side.** `feature.Flags` is a set of named flags (`xpr:pkg/feature/features.go:22,27`). The gate is `gate.Gate[GVK]`. `Register(fn, gvks...)` runs `fn` once, when all listed GVKs are set true (`xpr:pkg/gate/gate.go:25-106`). The `crd-gate` controller watches every CRD and sets the gate for each served version of an Established CRD that is not being deleted (`xpr:pkg/reconciler/customresourcesgate/setup.go:32-45`, `xpr:pkg/reconciler/customresourcesgate/reconciler.go:46-82`). The MR controller registers through the gate (`base:internal/controller/mytype/wiring.go:37-44`). The ProviderConfig controller does not (`base:internal/controller/register.go:33-36`). On the core side, `safe-start` in `package/crossplane.yaml` (`base:package/crossplane.yaml:21-22`) has three effects:
- Core grants get, list, and watch on CRDs (`xp:internal/controller/rbac/provider/roles/roles.go:180-185`).
- MRDs start inactive (`xp:internal/controller/pkg/revision/reconciler.go:652-653`).
- Core scales the Deployment to zero until the first MRD is active (`xp:internal/controller/pkg/runtime/runtime.go:259-283`).

The Helm chart's default activation policy activates `*` (`xp:cluster/charts/crossplane/values.yaml:60-61`).

**kube-rs.** None. A plain `Controller` over CRDs is easy to write.

**Gap.** Starting a controller after the manager has started. The manager section covers it.

**Recommendation.** Port `Gate<T>` as a mutex around a list of `(FnOnce, HashSet<T>)`. Port `crd-gate` as a `Controller<CustomResourceDefinition>`. The gated callback spawns the MR controller into the manager. The CRD watch is cluster-wide in both runtimes, so its list and watch are read-only and out of the write diff.

## Container image and xpkg packaging

**Go side.** The image is `distroless/static` with the binary at `/usr/local/bin/crossplane-provider-baseline-provider`, run as user 65532 (`base:cluster/images/provider-baseline/Dockerfile:1-9`). `crossplane xpkg build --embed-runtime-image <image> --package-root package --examples-root ...` embeds the image when `crossplane.yaml` has `kind: Provider` (`base:build/makelib/xpkg.mk:76-82`). The local deploy loads the xpkg into kind and creates a `DeploymentRuntimeConfig`. That config overrides the `package-runtime` container image and passes `--debug`. It then creates a `Provider` with `packagePullPolicy: Never` (`base:build/makelib/local.xpkg.mk:55-66`). `crossplane.yaml` declares the `safe-start` capability and a controller image (`base:package/crossplane.yaml:20-24`).

**Gap for Rust.** The binary must run on `distroless/static`, so build a static binary for `x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl`, with `rustls` and no OpenSSL. The alternative is `distroless/cc` with a glibc build. Everything else is reusable. The Rust package gets the same `crossplane.yaml` with a different `metadata.name` and image, and the same CRDs (see CRD generation).

**Recommendation.** Use a musl static build on `distroless/static`. Copy the package directory and `xpkg build` invocation unchanged.

## Unit-test seam

**Go side.** Table tests inject `test.MockClient`, a struct of function fields, one per client method (`xpr:pkg/test/fake.go:274-290`). The managed reconciler tests use it about 244 times (search count across `xpr:pkg/reconciler/managed/reconciler_legacy_test.go` and `reconciler_modern_test.go`). The case inventory lists 473 cases (`docs/phase0/go-test-inventory.tsv`).

**kube-rs.** `Client` is a concrete type over any tower `Service<Request<Body>>` (`kube:kube-client/src/client/mod.rs:87,153-160`). kube's own tests use `tower_test::mock::pair` (`kube:kube-client/src/client/mod.rs:645-652`). `tower-test` 0.4.0 dates from 2021.

**Options.**
1. Use a trait seam. Define `trait KubeApi { async fn get(..); async fn replace(..); async fn replace_status(..); .. }` in the runtime, with one production implementation over `kube::Api` and a mock struct of boxed closures. That struct is the Go `MockClient` almost line for line. Go table tests port case by case, with the same inputs and the same expected calls.
2. Use a tower mock service. Tests script HTTP request and response pairs. This tests kube's serialization too, but each Go case grows into URL and JSON assertions. That is a poor fit for 473 cases.
3. Use an envtest-style real API server. This means a kind cluster or a downloaded `kube-apiserver` and `etcd`. It is high fidelity, slow, and cannot inject errors such as "Update returns 409 once".

**Recommendation.** Option 1 for the ported table tests, because it keeps the case-for-case mapping the inventory needs. Use option 3 only for a few integration tests of the trait's production implementation. The trait is the boundary, so business logic never sees `kube::Api` directly.

## Audit-trace comparison specification

This section specifies the differential write-trace check. It runs the same chainsaw suite once against the Go provider and once against the Rust provider. Each run uses a fresh kind cluster, the same Crossplane core version, and the same node image.

### Enable audit logging on kind

kind passes kube-apiserver flags through `kubeadmConfigPatches` and mounts the policy with `extraMounts` (https://github.com/kubernetes-sigs/kind/blob/main/site/content/docs/user/auditing.md, read 2026-11-09). The baseline creates its cluster with no config (`base:build/makelib/controlplane.mk:25`), so the e2e target must pass `--config`.

```yaml
# kind-audit.yaml
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
- role: control-plane
  kubeadmConfigPatches:
  - |
    kind: ClusterConfiguration
    apiServer:
      extraArgs:
        audit-log-path: /var/log/kubernetes/kube-apiserver-audit.log
        audit-policy-file: /etc/kubernetes/policies/audit-policy.yaml
        audit-log-maxsize: "500"
      extraVolumes:
      - {name: audit-policies, hostPath: /etc/kubernetes/policies, mountPath: /etc/kubernetes/policies, readOnly: true, pathType: DirectoryOrCreate}
      - {name: audit-logs, hostPath: /var/log/kubernetes, mountPath: /var/log/kubernetes, readOnly: false, pathType: DirectoryOrCreate}
  extraMounts:
  - {hostPath: ./audit-policy.yaml, containerPath: /etc/kubernetes/policies/audit-policy.yaml, readOnly: true}
```

The kind guide uses the map form of `extraArgs`. kubeadm v1beta4 uses a list of name and value pairs. Whether kind 0.33.0 accepts the map form for node v1.35 or v1.36 is unverified. Smoke-test it first with `docker exec <node> grep audit /etc/kubernetes/manifests/kube-apiserver.yaml`.

```yaml
# audit-policy.yaml
apiVersion: audit.k8s.io/v1
kind: Policy
omitStages: [RequestReceived]
rules:
- level: None
  verbs: [get, list, watch]
- level: None
  resources: [{group: coordination.k8s.io, resources: [leases]}]
- level: Request
  userGroups: ["system:serviceaccounts:crossplane-system"]
  verbs: [create, update, patch, delete, deletecollection]
- level: Metadata
  verbs: [create, update, patch, delete, deletecollection]
```

The kind sample policy logs at `Metadata` level, which drops request bodies. Writes need `Request` level. Use `RequestResponse` only if the user picks the semantic option for update versus patch below. The `Metadata` rule keeps chainsaw's own writes as markers for step boundaries.

### Select the provider's writes

Filter on `user.username` with the prefix `system:serviceaccount:crossplane-system:provider-`. Do not filter on `userAgent`, which differs by design. Crossplane names the service account after the provider revision (`xp:internal/controller/pkg/runtime/runtime.go:307-323`), so the suffix is random. Normalize it to `<provider-sa>`.

### Fields to keep

For each event keep:
- `verb`
- `objectRef.apiGroup`, `objectRef.resource`, `objectRef.subresource`, `objectRef.namespace`, and `objectRef.name`
- `user.username` (normalized)
- `userAgent` (reported, not compared)
- `responseStatus.code`
- `requestObject` for create, update, and patch
- `requestReceivedTimestamp`, used only to order events within one object

### Normalize

- Names: replace generated names (`generateName` results, PCU names equal to MR UIDs, and Event names `<obj>.<hex>`) with stable tokens in order of first appearance per run.
- UIDs: map each UID to `uid-N` by first appearance. Apply the map to `ownerReferences[].uid`, PCU names, and `resourceRef`.
- Delete `metadata.resourceVersion`, `metadata.managedFields`, `metadata.creationTimestamp`, `metadata.generation`, and `metadata.selfLink`.
- Replace every RFC3339 timestamp with `<ts>`. That covers `lastTransitionTime`, `crossplane.io/external-create-*` annotations, and `lastHandledReconcileAt` token values.
- Sort map keys. Sort `status.conditions` by `type`.

### Compare

1. Group events by `(apiGroup, resource, namespace, normalized name)`.
2. Within each group, compare the ordered sequence of `(verb, subresource, code, normalized requestObject)`.
3. Poll-driven status writes repeat once per poll interval, so their count scales with run time. **User decision.** Pick one of three rules. Collapse consecutive identical writes into one entry with a count and ignore the count, compare counts within a tolerance, or cut each run at a fixed chainsaw step marker.
4. Report events in their own layer, keyed by `(regarding object, reason, type)`, with the count and message. Core v1 and events.k8s.io bodies differ in shape even when the recorder matches.

### Does update versus patch count as a difference

**User decision.** Options:
1. Strict. The verb is part of the key. A Rust `patch` where Go sent `update` fails. This fits a verbatim port, and the recommended design (mirror Go verbs) passes it.
2. Semantic. Log `RequestResponse`, compare `responseObject` after normalization, and treat `update` and `patch` as equal when the stored object matches. This is more tolerant and costs larger logs.
3. Split. Strict for the MR, ProviderConfig, and PCU kinds. Semantic for Events.

Recommendation is option 3.

### Measure the noise floor first

Before any Rust run, run the Go provider twice on two fresh clusters and diff the traces with the same tool. Every difference left after normalization is noise. Typical sources are the cache-driven 409s, retry duplicates, and poll counts. Either the normalizer handles a noise source or an allowlist names it, with the reason. The Rust comparison counts only differences above that floor. The Go-against-Go diff must come out empty under the final rules before a Go-against-Rust result means anything.

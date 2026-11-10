# Core design

The contract for Phase 1. The scaffold in `crates/` and `examples/provider-sample` compiles
against it; every `todo!()` is a body one plan unit fills in without changing a signature. A
signature change is a deviation: record it under "Implementation reconciliation" first.

## Problem

Port the provider-facing part of crossplane-runtime v2.4.3 so a Rust provider behaves like a Go
one under the differential harness in `parity/`. Three constraints make the shape non-obvious.
Go tables replace collaborators with closures and compare whole objects, so the seam between the
reconciler and the API server must be one mockable trait, and 377 Go cases must port case for
case. kube-rs has no controller-runtime: the work queue, the informer cache, predicates, the
event recorder and the rate limiters are ours (`docs/phase0/03-platform-gaps.md`). And the CRDs
must match controller-gen's output structurally, which kube-rs and schemars do not produce
unaided.

## Usage (caller's view)

The usage is the compiled example, which mirrors the Go baseline file for file. Read these
instead of a prose copy that could drift:

| Provider author writes | File | Go baseline file |
|---|---|---|
| The kind | `examples/provider-sample/src/apis/sample_v1alpha1.rs` | `apis/sample/v1alpha1/mytype_types.go` |
| ProviderConfig kinds | `examples/provider-sample/src/apis/v1alpha1.rs` | `apis/v1alpha1/types.go` |
| The external client | `examples/provider-sample/src/controller/mytype/external.rs` | `internal/controller/mytype/external.go` |
| The connector | `examples/provider-sample/src/provider/connector.rs` | `internal/provider/connector.go` |
| Controller wiring | `examples/provider-sample/src/controller/mytype/wiring.rs`, `controller/config.rs` | `wiring.go`, `config/config.go` |
| The binary | `examples/provider-sample/src/main.rs` | `cmd/provider/main.go` |
| A table test of their own kind | `examples/provider-sample/tests/mytype_reconciler.rs` | (none) |
| A ported Go table | `crates/crossplane-runtime/src/reconciler/managed/reconciler_modern_test.rs` | `reconciler_modern_test.go` |

What an author sees: two imports (`crossplane_core` for types, traits and derives,
`crossplane_runtime` for the machinery); `#[derive(CustomResource, Managed)]` on the spec;
`impl ExternalClient<MyType> for External` with plain `async fn` taking `&mut MyType`; one
registration call, `controller::new_managed_by(mgr)...complete(reconciler)`.

## Shape

Four crates, dependencies pointing one way:

- `crossplane-core`: core/v2 wire types (`v2`), `errors`, `meta`, the object traits (`Object`,
  `Kind`, `Managed`, `ProviderConfig`, `ProviderConfigUsage`) and the `Client` seam. No tokio, no
  HTTP (`cargo tree -p crossplane-core` shows neither).
- `crossplane-derive`: the angryjet replacement. Emits `::crossplane_core::...` paths.
- `crossplane-test`: closure `MockClient`, fake kinds, `assert_object_eq`, `assert_result`.
  Depends on core only.
- `crossplane-runtime`: everything that runs. Dev-depends on `crossplane-test`.

Load-bearing decisions:

- **The seam is `dyn Client`** (`crossplane-core/src/client.rs`), object safe over `dyn Object`,
  with typed `get_as::<K>` and `list_as::<K>` on top. `get` returns the object. The reconciler,
  every collaborator and provider code reach the API server only through it; `kube` appears in
  exactly one private module, `crossplane-runtime/src/kube_client.rs`, which is also the only
  place a `kube::Error` becomes an `Error` (per boundary-discipline).
- **Kinds need no `Default`.** The runtime never constructs an empty kind, so required spec
  fields stay required. Go zero values are `Default` on the common types; kubebuilder defaults
  live only in the schema (`ManagementPolicies` is empty by default and carries `["*"]` as a
  schema default).
- **`Reconciler<M>` is generic over the kind.** Collaborators (`Initializer<M>` and the rest) and
  the external client receive `&mut M`. `ExternalConnector<M>` and `ExternalClient<M>` are native
  `async fn` traits; the object-safe mirrors `DynConnector`/`DynExternal` are crate private.
- **`ReconcilerBuilder` is Go's option list.** Each collaborator a Go table replaces is a builder
  method with a `*Fn` closure adapter next to its trait.
- **`Timing`** holds the five durations with Go's values (`reconciler.go:49-53`; min poll is
  unset in `NewReconciler`, so zero).
- **`Error`** implements `std::error::Error`, carries a `StatusReason` through `wrap`, displays
  as Go's `"<message>: <cause>"`, and compares the way `test.EquateErrors` does.
- **`Outcome`** (`Done`, `Requeue`, `RequeueAfter`) and `Result<Outcome>` are the whole
  reconcile result; `controller/driver.rs` maps them to queue operations in one pure function,
  `queue_op`, tested against controller-runtime's switch.
- **The driver runs over a plain stream of keys**, so the queue and worker loop are testable
  without kube. `cache`, `kube_client`, `controller::workqueue` and `controller::driver` are
  private; three `compile_fail,E0603` doc tests in `crossplane-runtime/src/lib.rs` keep them so.
- **`RestConfig`** has one constructor, `RestConfig::infer(binary, version)`, and one consumer,
  `Manager::new`, so the QPS throttle and the Go-style User-Agent cannot be skipped.
- **One registration path.** `controller::new_managed_by(mgr)` is the only way to start a
  controller; predicates receive whole old and new objects.

Interface depth: the public surface an author touches is two traits to implement, one derive, one
builder and one registration chain. Behind it sit the informer cache, the queue, rate limiting,
the two-deadline timeout, conflict handling, finalizers, usage tracking and the 48 return
branches of the parity spec.

Deliberately absent: a scheme, cluster-scoped (legacy) kinds, a prelude, any re-export of core
from the runtime (core re-exports only its own derives at the crate root), a public queue or
informer, a blanket `From<kube::Error>`.

### Module map

Each unit owns the listed files. Units do not share files except where noted.

| Rust module | Go package | Plan unit |
|---|---|---|
| `crossplane-core/src/v2/{condition,policies,resource,schema}.rs`, `duration.rs` | `crossplane/apis` `core/v2` | 1.2 |
| `crossplane-core/src/errors.rs`, `meta.rs` | `pkg/errors`, `pkg/meta` | 1.3 |
| `crossplane-runtime/src/{feature,event,logging,clock}.rs` | `pkg/feature`, `pkg/event`, `pkg/logging` | 1.3 |
| `crossplane-core/src/resource/kinds.rs` (`mark_conditions` and its tables only; the file is 1.4's) | `pkg/conditions` | 1.3 |
| `crossplane-core/src/client.rs`, `resource/{object,kinds}.rs` | `pkg/resource` interfaces, controller-runtime `client` | 1.4 |
| `crossplane-test/src/{lib,fake}.rs` | `pkg/test`, `pkg/resource/fake` | 1.4 |
| `crossplane-runtime/src/kube_client.rs` | controller-runtime `client` | 1.4 |
| `crossplane-runtime/src/resource/{applicator,connection,credentials,enqueue,finalizer,late_init,predicates,tracker}.rs` | `pkg/resource` | 1.5 |
| `crossplane-runtime/src/reconciler/managed/{mod,collaborators,external,policy,changelog,metrics}.rs` and its tests | `pkg/reconciler/managed` | 1.6 |
| `crossplane-runtime/src/reconciler/providerconfig.rs` | `pkg/reconciler/providerconfig` | 1.7 |
| `crossplane-runtime/src/gate.rs`, `reconciler/customresourcesgate.rs` | `pkg/gate`, `pkg/reconciler/customresourcesgate` | 1.8 |
| `crossplane-runtime/src/controller/{mod,workqueue,driver}.rs`, `cache.rs`, `manager.rs`, `ratelimiter.rs`, `statemetrics.rs`, `reconciler/mod.rs` | `pkg/controller`, `pkg/ratelimiter`, `pkg/statemetrics`, controller-runtime | 1.9 |
| `crossplane-derive/src/lib.rs` | crossplane-tools angryjet | 1.10 |
| `crossplane-runtime/src/reference.rs` (not created yet; `Managed::resolve_references` is its hook) | `pkg/reference` | 1.11 |

Shared files, and who may touch them:

- `event.rs`: 1.3 ports the recorder tables, 1.9 fills the API recorder and correlator.
- `reconciler/mod.rs`: owned by 1.9, but `silently_requeue_on_conflict` and its Go test
  (`pkg/errors` `TestSilentlyRequeueOnConflict`) belong to 1.3.
- `lib.rs` and `mod.rs` files only declare modules and re-export. Any unit may add a `mod` line
  for a file it creates (a test file, `reference.rs`); none may change what is public.
- `Cargo.toml` files: a unit adds only the dependencies its own files need.

Run these units in the order of `docs/plan.md`; the pairs above are sequential, not parallel.

## Synthesis decision

Base: candidate-opus, because its ported Go cases ran, its CRDs diffed to zero and its timeout
and grace code was real. All three properties are kept and rerun (see "Verification").

Grafts:

1. Crate layering, from fable. Landed. Replaces opus's `test-support` feature and
   `crossplane-apis` crate. The closure adapters for collaborators stay in the runtime next to
   their traits.
2. From fable, landed: private `cache`, `kube_client` and queue; `Error` as above with one
   private conversion; a `RateLimiter` trait (`when`, `forget`; `num_requeues` left out because
   nothing calls it); predicates over whole objects; the `replace`/`replace_status`/`patch` verb
   mapping; single-path `RestConfig`; Go zero values as `Default`.
3. `get`: fable's returned object. Decided by experiment. Opus's in-place
   `get(key, &mut dyn Object)` needs `M::default()` before the call; with `Default` removed from
   `MyTypeSpec` and `MyTypeParameters` the opus sketch fails with
   `E0277: the trait bound MyType: Default is not satisfied`. In this scaffold `MyType` has no
   `Default` and both ported cases plus the provider-side case pass. For the same reason the mock
   helper is `new_mock_get_fn(|| object)`, and `ProviderConfigUsage::from_usage(metadata, usage)`
   replaces `U::default()` in the tracker.
4. External client: opus's native `async fn` with crate-private erasure, typed by kind as in
   fable. Landed.
5. `Timing` from sonnet, with values from the Go source: `reconcileGracePeriod` 30s (line 49),
   `reconcileTimeout` 1m (50), `defaultPollInterval` 1m (52), `defaultGracePeriod` 30s (53). The
   key-stream driver from haiku. Both landed.
6. Red flags in the base, fixed. Removed from the public surface: the `queue` module with
   `WorkQueue` and `ItemExponentialBackoff`, `Manager::kube_client()`, `Manager::informer()` and
   `SharedInformer`. The second registration path that went was constructing a `Controller`
   directly and adding it to the manager; `Controller` is now private and `new_managed_by` is the
   only entry.

Rejected: none of the six. Dropped along the way as unused: the `governor`, `thiserror`,
`json-patch` and `jiff` dependencies, and any prelude or re-export layer.

## Tradeoffs accepted

- We accept one JSON round trip per typed read (`get_as` goes through `dyn Object`) in exchange
  for an object-safe seam that closures can mock.
- We accept boxed futures on collaborator traits in exchange for storing them as trait objects;
  only the two traits authors implement are native `async fn`.
- We accept `Reconciler<M>` being monomorphized per kind in exchange for no downcasts in provider
  code.
- We accept `controller_gen_compat` rewriting the schema after generation in exchange for a zero
  structural CRD diff without forking kube-derive.
- We accept owning a queue and informer in exchange for controller-runtime's requeue semantics,
  which `kube::runtime::Controller` does not have.
- `clippy::type_complexity` is allowed workspace wide (the `*Fn` adapters each hold one boxed
  closure), and `crossplane-runtime` had `#![allow(dead_code)]` while bodies were `todo!()`.
  Removed at integration.

## Alternatives considered

- In-place `get(key, &mut dyn Object)` (Go's signature). Smaller diff from Go tables, but forces
  `Default` onto every provider spec type. Lost on the experiment above.
- An object-safe `dyn Managed` with interface assertions, as Go has. One reconciler type for all
  kinds, but every provider method starts with a downcast and the error path Go calls
  `errNotMyType`. Lost on interface depth.
- `kube::runtime::Controller` as the driver. Far less code to own, but its scheduler cannot
  express controller-runtime's rate-limited requeue versus forget, which the parity spec pins.
- A single crate with a `test-support` feature (the base). Fewer crates, but the mock and fakes
  then compile against a second configuration of every type.

## Implementation reconciliation

Deviations of the scaffold from the synthesis brief:

- Bodies that are real but have no test yet. They were kept because something run here executes
  them or because the base had them real, and the unit that owns each must test or replace it:
  the kube client verb mapping (1.4), the derive
  expansion and `controller_gen_compat` (exercised by crd-diff only; 1.10 and 1.2).
- The provider's HTTP client (`examples/provider-sample/src/provider/client.rs`) was `todo!()`.
  Filled at integration over hyper-util's legacy client (already in the tree through kube), with
  Go's status handling, contract error texts, 10s timeout and `json.Encoder` trailing newline.
  A transport error reads `Get "<url>": <cause chain>` like Go's `url.Error`; the cause text is
  hyper's, not Go's `net` package's.
- `main.rs` mirrors only the environment variables Go names explicitly, not kingpin's
  `DefaultEnvars`.
- `hack/port-coverage.sh` gained two flags the brief did not ask for: `--complete` (fail while
  anything is missing) and `--list-missing`.
- Units 1.2 and 1.3 (core/v2, errors, meta, conditions, feature, event, logging):
  - `port-denominator.tsv` had no `core/v2` rows. The 31 cases of `condition_test.go` were added
    with `hack/go-test-inventory.sh`, so the denominator is now 408. `AIsNil` and `BIsNil` are
    skipped: a nil `*ConditionedStatus` cannot be built, so `ConditionedStatus::equal` takes two
    references.
  - `Condition::equal` was strict about `observedGeneration`. Go ignores it when either side is
    zero (`IdenticalWithMissingObservedGeneration` forced the fix).
  - Go's empty `Status` has no `ConditionStatus` value. Cases that leave it unset use `Unknown` on
    both sides; no expectation depends on it.
  - `external_create_succeeded_during` takes `now: Timestamp` where Go calls `time.Since`. The
    scaffold had no such function. The reconciler passes its `Clock`.
  - Empty `labels`, `annotations` and `finalizers` become `None` after a removal. Go leaves an
    empty collection that `omitempty` drops from the JSON, so `RemoveFinalizer/FinalizerExists`
    expects `None` here.
  - `Error::cause` was added for `errors.Cause`. `Wrapf` has no Rust function; its cases run
    `wrap_err` over a `format!` message.
  - `TestOGConditionSetMark` is an integration test in `crossplane-runtime/tests/conditions.rs`
    because core cannot dev-depend on `crossplane-test`. The feature, event and logging tests are
    integration tests too, so `event.rs` and `reconciler/mod.rs` (unit 1.9's) carry no test blocks
    from unit 1.3 beyond `silently_requeue_on_conflict`'s.
  - Not ported because only legacy or composite resources use them: `CompositeDeletePolicy`,
    `resource_cluster.go`, `ObservedStatus` accessors, `TypedReference` GVK helpers.
  - `duration::parse` rejects negative durations; the Go function accepts them and `GetPollInterval`
    drops them, so the result is the same.

Unit 1.4 (client seam, test support):

- `KubeClient::uncached(kube)` reads the API server directly (Go `client.New`); `KubeClient::new`
  keeps the cache seam for 1.9. `kube_client::connect(config, user_agent)` builds the one
  `kube::Client` and sends the User-Agent as a default header (kube has no `UserAgent` field);
  `RestConfig::into_kube_client` should call it. `RestConfig` itself is still 1.9's `todo!()`.
  Go's baseline sends `crossplane-provider-baseline-provider/v0.0.0 (linux/arm64) kubernetes/$Format`
  (audit log): client-go's `DefaultKubernetesUserAgent` (`rest/config.go:530`) uses
  `version.Get()`, the client-go build version, cut at the first `-`, and the commit cut to
  seven characters, not the provider's version.
- The runtime's `kube` gained the `ring` feature: `rustls-tls` alone selects no crypto provider
  and the first connect panics.
- `convert_error` follows apimachinery's `IsNotFound`/`IsConflict`/`IsForbidden`/`IsInvalid`:
  the reason when it is a known one, else the HTTP code.
- `ObjectType::group_kind` and its `Debug` form match Go's `GroupKind`/`GroupVersionKind`
  `String()` for the core group (`ConfigMap`, `/v1, Kind=ConfigMap`).
- `Managed`, `ProviderConfig` and `ProviderConfigUsage` gained the setters and getters of Go's
  interfaces (and of `ObservedStatus` on `Managed`) as provided methods, so derives are
  unchanged.
- `assert_object_eq` (and the new `assert_conditions_eq`) follow Go's `Condition.Equal`, which
  ignores `observedGeneration` when either side is zero. `Condition::equal` in `v2/condition.rs`
  (unit 1.2) does not, so (inferred from reading both, not run) `set_conditions` replaces a
  condition Go would keep.
- Go test patterns with no direct counterpart: `NewMockGetFn(err, fns...)` that fills the object
  and fails (`pkg/reconciler/managed/api_test.go:570` `UpdateConflictGetError` expects the caller's
  object filled after a failed get; a returned-object `get` cannot express it); `NewMockGetFn(nil)`
  where the Go object passed in is the caller's copy (the applicators), so the case must build
  that object in `new_mock_get_fn`; `fake.Manager` (builders take the client); `fake.GVK` and
  `fake.SchemeWith` (`ObjectType::of`); `NewMockClient()` (no in-scope table uses it);
  `fake.Managed` (use `fake::ModernManaged`); `NewMockPatchFn(nil, fn)` and raw `MockList` /
  `MockGet` closures (a boxed closure in the `MockClient` field, which also sees the
  `ListOptions` and `ObjectType`); `mockSimpleReferencer` with `MockResolveReferences`
  (`pkg/reconciler/managed/api_test.go:373`, local to that test, so its port defines a local kind
  overriding `Managed::resolve_references`). The embeds `Manageable`,
  `TypedProviderConfigReferencer`, `LocalConnectionSecretWriterTo` and `ObservedStatus` are the
  `Managed` accessors; `fake.MockLocalConnectionSecretOwner` is any `Managed` kind with
  `write_connection_secret_to_reference`. None, legacy: `fake.LegacyManaged`,
  `LegacyProviderConfigUsage`, `LegacyProviderConfigReferencer`,
  `RequiredProviderConfigReferencer`, `Orphanable`, `ConnectionSecretWriterTo`,
  `MockConnectionSecretOwner`. None, composition: `fake.Composite`, `Composed`,
  `CompositeClaim`.
- Open question 7: Update overwriting the in-memory object is now tested at the seam
  (`crossplane-test/tests/support.rs`, `new_mock_response_fn`) and against a real API server
  (`kube_client_test.rs`: a status set in memory is gone after the PUT). No Go table case pins it.

Unit 1.9, queue half (work queue, driver, rate limiters):

- The work queue follows controller-runtime v0.24.1's priority queue, not client-go's classic
  queue, because the baseline runs the priority queue (`cr:pkg/controller/controller.go:252-271`;
  `UsePriorityQueue` is unset in the baseline and in `xpr:pkg/controller/options.go:83-91`). The
  two differ per object, so this is not only ordering. The priority queue holds one entry per
  key (`priorityqueue.go:219-277`). A plain `Add` of a waiting key makes it ready and cancels the
  delay (`:239-240,261-264`), where client-go's delaying queue keeps the heap entry and adds the
  key again when it fires (`cg:util/workqueue/delaying_queue.go:298-307,330-335`). So a watch
  event during a `RequeueAfter` wait gives one reconcile, not two, and a `RequeueAfter` returned
  by a reconcile that an event arrived during is absorbed. `ShutDown` stops `Get` at once and
  queued keys are dropped (`priorityqueue.go:424-428,475-484`); client-go's `ShutDown` lets `Get`
  drain the queue first. The scaffold's `dirty`/`processing`/heap `State` is replaced by
  `items`/`ready`/`waiting`/`locked`, Go's own fields. Priorities are not ported.
- `queue_op` maps `RequeueAfter(Duration::ZERO)` to `Forget`: Go tests `RequeueAfter > 0`
  (`cr:pkg/internal/controller/controller.go:495`), so a zero delay is plain success, not an
  immediate requeue.
- `ControllerOptions::default()` now sets `recover_panic: true`. An unset `RecoverPanic` counts
  as true in controller-runtime (`controller.go:199`); the baseline sets it true anyway.
- A recovered panic becomes `Error::new("panic: <message> [recovered]")`, Go's format
  (`controller.go:203`). With `recover_panic` off, `driver::run` resumes the panic, as an
  unrecovered panic in a Go worker ends the process.
- `ratelimiter::Reconciler::new` keeps its `name` parameter but does not use it. Go keys the
  limiter by `name + request`; `RateLimiter` is keyed by `Request`, which is the same for the
  global bucket because a bucket ignores the item.
- client-go's `MaxOfRateLimiter`, `ItemFastSlowRateLimiter` and `WithMaxWaitRateLimiter` are
  not ported. `NewController` is the exponential limiter alone (`xpr:pkg/ratelimiter/default.go:42-44`;
  its doc comment saying it takes the maximum with another limiter is stale), and MaxOf appears
  only in `DefaultControllerRateLimiter`, which controller-runtime uses only with the priority
  queue off. The bucket applies only through the global wrapper.
- The `governor` dependency stays dropped. The bucket is a few lines hand-written over
  `golang.org/x/time/rate`'s arithmetic, so Go's grant-and-go-into-debt `Reserve` (open
  question 2) is copied exactly instead of mapped onto another crate's model.
- `hack/port-coverage.py` reads a second listing, `docs/port-extra.tsv`, of Go cases outside the
  denominator that are ported as the spec for code the runtime owns (client-go workqueue,
  controller-runtime controller and priority queue). Their markers and skips follow the same
  rules and are counted on a second summary line, never in the denominator's.

Units 1.7 and 1.8 (ProviderConfig reconciler, safe-start gate):

- `customresourcesgate::Reconciler` holds `Arc<dyn gate::SetGate<GroupVersionKind>>`, not
  `Arc<Gate<..>>`. Go's `TestReconcile` swaps in a `MockGate` that records `Set` calls, and a
  concrete `Gate` cannot be observed that way (`set` returns "changed" for an absent key whether
  it was set true or false). `Gate` implements the one-method `SetGate`, so `setup` still passes
  `o.gate.clone()` and `controller::Options` is unchanged.
- `TransformStripCRDSchema` is `customresourcesgate::transform_strip_crd_schema(&mut dyn Object)`.
  It mutates in place and ignores any kind but a CRD, where Go returns the object. Nothing calls
  it, by decision at integration: Go's `Setup` installs no transform (`setup.go` only builds the
  controller) and the baseline's `cache.Options` sets only `SyncPeriod`
  (`cmd/provider/main.go:95-97`), so the baseline caches whole CRDs. Wiring a transform hook
  into the cache would change only memory use, never a request, and would make the Rust cache
  differ from the baseline's. A provider cannot install it either, since the cache has no public
  options; add a per-kind transform hook when a provider needs one.
- Gate callbacks run on `spawn_blocking` when a tokio runtime is current and on a new thread
  otherwise, because a callback typically registers controllers and the caller of `register` or
  `set` may be either.
- The ProviderConfig reconciler ended the whole run at `TIMEOUT`. Fixed at integration: each
  call is bounded by the one deadline (see "Integration"). The legacy usage kind probe
  (`legacyPCU`) is not ported: the label selector always carries the kind label.
- `Setup` with a nil gate (G3) has no Rust counterpart: `Options::gate` is a non-optional `Arc`.
- Extra tests with no Go case, in `providerconfig_test.rs`: verb order and bodies (get, list,
  delete, PUT, status PUT on every pass), the list labels, the Terminating condition, and the
  `UsageAccounting` warning events (type, reason, message) through a recording recorder. In
  `gate_test.rs`: `Set`'s changed result, a key flipping back before a later register, a released
  registration never rerunning, and a multi-task concurrency test.
Unit 1.5 (`pkg/resource`):

- `crossplane-core/src/errors.rs` gained `ErrorKind` (`NotControllable`, `NotAllowed`,
  `MissingReference`), `Error::with_kind` and `Error::kind`. Go's `IsNotControllable`,
  `IsNotAllowed` and `IsMissingReference` are type assertions on the error itself, so `kind` does
  not look through a wrap, and `Error`'s equality now also compares it (`test.EquateErrors`
  compares types). Callers: the managed worker's secret publisher needs
  `resource::is_not_allowed` (`pkg/reconciler/managed/api.go:118,177`); the tracker uses it here.
- `ApplyOption`: `ControllersMustMatch` was a scaffold misnomer (the Go function of that name is a
  reference-selector helper) and is now `MustBeControllableBy(uid)`, which the tracker needs and
  whose message embeds the UID. `Custom(fn(&mut current, &desired) -> Result)` covers
  `ApplyOptionError` and `UpdateFn` (`ApplyOption::update_fn`). `ApplyOption::run(current,
  desired)` is the one evaluator both applicators use and Go tables call. Kept:
  `ConnectionSecretMustBeControllableBy(uid)` and `AllowUpdateIf` (over JSON). Not ported:
  `StoreCurrentRV` (no caller outside tests).
- New `Applicator` trait, `ApplyFn`, `ApplicatorWithRetry` and `Backoff` (Go `wait.Backoff`,
  `retry.OnError`; jitter is random without a `rand` dependency). Both applicators keep their
  inherent `apply` so callers need no import. `ProviderConfigUsageTracker::with_applicator`
  mirrors Go tables setting the tracker's `c`; `new(client)` is unchanged. Without it
  `TestTrackModern/NopUpdate` (which calls the options on a hand-built current usage) cannot be
  ported.
- The tracker passes `MustBeControllableBy(uid)` (the scaffold TODO said `ControllersMustMatch`)
  and a typed `Custom` option for "reference changed", not a JSON path, so a derive that moves the
  field cannot make every reconcile write. Its `resourceRef` has no UID, as in Go.
- `common_credential_extractor`: `InjectedIdentity` is an error ("no extraction handler registered
  for source: InjectedIdentity"), only `None` yields nothing. The scaffold TODO said otherwise.
  `extract_env(lookup, ..)`, `extract_fs(..)` and `extract_secret(client, ..)` are public as in Go.
- `local_connection_secret_for` keeps the scaffold signature. `data` is `None` when empty, which
  serialises as Go's `omitempty` empty map. The expected owner reference in the ported case uses
  the Rust fake's group and kind (`fake.crossplane.io/v1`, `ModernManaged`), not Go's
  `MockOwnerGVK`, because the kind comes from the type. `get_external_tags` likewise lowers
  `ModernManaged.fake.crossplane.io`.
- Added files: `apply_option.rs`, `helpers.rs` (`Ignore`, `IgnoreAny`, `IsAPIErrorWrapped`,
  `IsConditionTrue`, `GetExternalTags`, `FirstNAndSomeMore`), `reference.rs`
  (`ReferenceStatus*`). `AttributeReferencer` is not ported (legacy reference resolution).
- Applicators send the merge-patch body from `Object::to_json`, which always has `apiVersion`
  and `kind`; Go marshals a typed object whose TypeMeta may be empty. Not observed against the
  parity trace.
- No `pkg/resource` case needed the fill-and-fail `NewMockGetFn`, so `crossplane-test` is
  unchanged (`SecretFailureGet`'s closure ignores the object).
- Skipped rows are in `docs/port-skips.tsv`: `GetKind` and `MustCreateObject` (scheme),
  `ConnectionSecretFor` (cluster-scoped owner), `NotProviderConfigReferencer` (type switch), and
  the two nil-error rows.
Unit 1.9, rest (RestConfig, cache, manager, builder, event recorder, statemetrics):

- `kube_client::connect` takes a third argument, `Throttle`, a tower layer that is client-go's
  token bucket (QPS 5, burst 10). Watches skip it, as client-go's `Watch` has no rate limiter
  (`cg:rest/request.go:758-765` vs `:1054`). `RestConfig::infer(binary, version)` and
  `into_kube_client` are the only way to a client, so every request carries the throttle.
- The User-Agent has client-go's shape with Go's OS and architecture names. Its version is the
  provider's, not client-go's build version, and the commit is always `unknown`: a Rust binary
  has no client-go version to report. Live: `crossplane_runtime-<hash>/v0.0.0 (darwin/arm64)
  kubernetes/unknown` in the audit log.
- `ApiRecorder` feeds a bounded channel of 1000 and drops with an error log when full, Go's
  `maxQueuedEvents` broadcaster queue. The correlator, spam filter and sink retry are client-go's
  `tools/record` (28 of its tests ported, in `docs/port-extra.tsv`). Two wire differences: with
  no patch a record is created directly where Go calls `Patch` with a nil patch first, and a
  null `eventTime` is left out where Go sends `"eventTime":null`.
- The cache has no `ErrCacheNotStarted`: an informer starts on the first read or subscription,
  before or after `Manager::start`. Go's cache refuses reads before start; a provider never reads
  before start, so nothing observable changes.
- `controller_runtime_*` and `workqueue_*` metrics are not ported. The metrics endpoint serves
  only what a provider registers (`crossplane_managed_resource_*` and the managed reconciler's).
- The manager's elector is written here, client-go's `tryAcquireOrRenew` over a PUT with the
  resourceVersion, rather than `kube-lease-manager`, which judges expiry from the server's
  `renewTime` against the local clock and renews without a resourceVersion
  (`leader_election.rs` module doc).
- The builder's live test used test-local copies of `DesiredStateChanged` and of a
  usage-shaped handler while unit 1.5 was unfinished. At integration it runs the real
  `resource::desired_state_changed` and `EnqueueRequestForProviderConfig`, watching the
  baseline's `example.com` ProviderConfigUsage.
- `#![allow(dead_code)]` stays in `crossplane-runtime/src/lib.rs`: the remaining `todo!()`s are
  other units' (1.5, 1.6, 1.7, 1.8).

Unit 1.6 (managed reconciler):

- Deadlines. The scaffold dropped the whole run at the outer deadline. Now each call is bounded
  on its own, as a Go context bounds it: external calls (connect, observe, create, update,
  delete) by `timeout`, every Kubernetes and collaborator call by `timeout + grace`. A call past
  its deadline fails with `context deadline exceeded` and the reconcile carries on into the
  branch that handles the error, so an external hang still gets its event and status write.
  `Deadlines` and `bounded` in `reconciler/managed/mod.rs`.
- Shape. `Reconciler::run` handles get, pause and the reconcile-request token; `Run` holds the
  per-reconcile state and has one method per phase. A phase returns `ControlFlow`: continue, or
  break with the branch's result. `report` is Go's branch tail (event, conditions, status
  write), `failed` adds the conflict short-circuit, `finish` stamps the token and writes status.
  B04 has no code: `PolicyResolver::validate` cannot return a conflict.
- Status reset. Every full-object write replaces the in-memory object with the server's
  response: the initializer, the finalizer, the create-pending annotation, the critical
  annotations and late-init. The parity spec describes this for late-init only (S5). It is why
  the first reconcile of an observe-only resource writes `Synced` without `Ready`: the finalizer
  PUT returns the stored, empty status. Measured in the Go audit trace and traced to
  controller-runtime's decoder, which zeroes the target before decoding. Pinned by
  `adding_the_finalizer_drops_the_conditions_observe_set`.
- Change logs. tonic and prost are plain dependencies, not behind a cargo feature as the
  scaffold comment said. The three proto messages are hand-written prost structs
  (`changelog/proto.rs`), so no build needs protoc; `request_encodes_as_protoc_does` pins them
  to bytes protoc produced from the Go repository's `.proto`. `ChangeLogServiceClient` is the
  seam Go's generated client interface gives. `UnixChangeLogClient` dials lazily and redials
  every 100ms while the socket is unreachable, which stands in for `grpc.WaitForReady(true)`;
  the logger's send timeout bounds it. Only a failed dial is redialed. An error the sidecar
  sends, or a connection lost while the call is in flight, is returned at once, so an entry is
  never sent twice.
  `ChangeLogEntry.snapshot` is a JSON object, not any JSON value.
- Go returns `AdditionalDetails` beside an error from Create, Update and Delete and logs them.
  A Rust `Result` carries one or the other, so a failed operation's entry has no details.
- `ApiLocalSecretPublisher::with_applicator` exists for the ported table, as Go's test builds
  the struct directly.
- `MrMetricRecorder::with_clock` is not in Go. An object with no creation or deletion timestamp
  observes nothing, where Go would observe the seconds since year one.
- `effective_poll_interval`: the minimum floors the annotation only, as in Go.
- `TestRetryingCriticalAnnotationUpdater/UpdateConflictGetError`: Go's mock Get fills the
  object and then fails, and the case expects the filled object. `Client::get` returns an object
  or an error, so the ported case expects the object untouched. `crossplane-test` is unchanged.
- `TestPrepareJSONMerge`: Go's fake has no TypeMeta and inlines ObjectMeta, so Go expects
  `{"name":"resolved"}`. The Rust patch is in a real kind's wire shape, with `apiVersion`,
  `kind` and `metadata.name`.
- `TestAPISecretPublisher` (4 rows) is skipped: the legacy publisher.
- Tests with no Go counterpart are in `reconciler_effects_test.rs` (effect order per branch,
  the conflict table, deadlines, the annotation updater's retries), `metrics.rs` and the second
  half of `changelogger_test.rs` (wire encoding, Unix socket transport against a tonic server).

Integration (the merged units run together):

- `#![allow(dead_code)]` is gone from `crossplane-runtime/src/lib.rs`. What it hid, and why each
  went where it did: `Elector::identity` was dead (the identity is passed in and only read
  inside the elector) and is deleted. Test-only, now `#[cfg(test)]`: `KubeClient::uncached` with
  `Reads::Direct`, `get_direct` and `list_direct` (no in-scope Go path uses an uncached reader;
  the baseline never calls `GetAPIReader` or `client.New`), `RestConfig::user_agent` (the
  header is sent from the field; tests read it back), `Manager::metrics_address` (tests bind
  port 0), and `ReconcilerBuilder::connection_publisher` (Go's `withLocalConnectionPublishers`
  is unexported and only its tests call it). None was a seam left unwired.
- The ProviderConfig reconciler bounds each call by `TIMEOUT` from the start, Go's single
  `context.WithTimeout`, instead of dropping the run. A Get past it returns `cannot get
  ProviderConfig: context deadline exceeded`; a List past it takes the List branch (warning
  event, wait 30s). `bounded` moved from the managed reconciler to the private
  `reconciler/deadline.rs`; `Deadlines`, the managed reconciler's two-context pair, stays there.
- `e2e_test.rs` runs the units together against a live cluster: a Manager, a managed controller
  wired as `wiring.rs` wires one (rate-limited reconciler, `desired_state_changed`, the API
  recorder), a per-process `xprs-e2e-<pid>.example.com` CRD it installs and removes, and an
  in-memory external system. It asserts through an uncached client: finalizer, external-name
  and create annotations, `Synced`/`Ready` with Go's reasons, core/v1 Events with Go's reasons,
  messages and source component, a spec change reaching external update, the pause annotation
  stopping observes with `ReconcilePaused`, unpausing applying the change made while paused, and
  deletion calling external delete and removing the finalizer. What the first run showed:
  - It lives inside the crate, not in `examples/provider-sample/tests`, because `RestConfig` has
    only `infer`, which reads `KUBECONFIG` from the environment, and `unsafe_code` is forbidden,
    so a test cannot point it at a chosen kubeconfig. A provider that wants Go's `--kubeconfig`
    flag cannot either. Public API left unchanged; a `RestConfig::from_kubeconfig` would close
    it.
  - A delete within `creationGracePeriod` (30s) of a successful create waits it out: Observe
    reports not-exists, so the Pending branch (`R:1218`, before the deletion branch at `R:1231`)
    records `PendingExternalResource` and requeues until the grace period ends. Go does the same.
    The test sets a 3s grace period through `creation_grace_period`.
  - With a 1s poll, the `desired_state_changed` filter is not observable: polls pick up a spec
    change either way. The builder live test is what pins the filter.
  - Repeated identical events (the second `UpdatedExternalResource`) show as one Event with a
    higher count, the correlator's patch, as in the Go traces.

## Verification

Rerun these; each was run on the scaffold.

- `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets`: clean.
- `cargo test --workspace`: `modern_reconciler` (GetManagedError, ExternalCreatePending),
  `queue_ops_match_controller_runtime`, the provider-side `get_managed_error`, three
  compile-fail doc tests. Nothing ignored.
- Mutation: dropping `Condition::creating()` from the create-pending branch of
  `reconciler/managed/mod.rs` fails the case with Go's reason text.
- `hack/crd-diff/crd-diff.sh`: 0 structural differences for all three CRDs; descriptions are
  excluded and counted. Go YAML is converted at run time with `kubectl patch --local`, which
  contacts no cluster. Removing the `managementPolicies` schema default makes it exit 1.
- `hack/port-coverage.sh`: 2 of 377 ported.
- Unit 1.9 queue half: the `controller::workqueue`, `controller::driver` and `ratelimiter` tests,
  including a multi-thread stress test (6 producers, 8 workers, 4 keys) asserting no overlapping
  reconcile of one key and no lost add. Timing tests run on tokio's paused clock.
- Unit 1.9 rest: unit tests in `cache_test.rs`, `event_test.rs`, `manager_test.rs`,
  `statemetrics_test.rs`; live tests in `live_test.rs`, skipped unless `XPRS_TEST_KUBECONFIG`
  names a cluster (`XPRS_TEST_AUDIT_LOG` adds the audit checks). They create only `xprs-*`
  namespaces.

## Open questions and risks

None of these is resolved by the scaffold.

1. Answered (unit 1.9). Neither client-go's dirty and processing sets nor its delaying heap is
   what the baseline runs; it runs controller-runtime's priority queue, and its per-object
   behaviour differs from client-go's (see "Implementation reconciliation", unit 1.9). The Rust
   queue holds Go's priority-queue state minus priorities: a key-to-entry map, ready items by
   insertion counter, waiting items by ready time, and a locked set. Each part is needed: the
   map gives one entry per key, the locked set keeps a held key from a second worker, the
   waiting order gives earliest-deadline wakeups. Mutations that hand out a locked key, keep a
   delay beside a plain add, or let the later delay win each fail ported cases.
2. Answered (unit 1.9). `BucketRateLimiter.When` is `Limiter.Reserve().Delay()`
   (`cg:util/workqueue/default_rate_limiters.go:68-70`). `Reserve` is `reserveN(now, 1,
   InfDuration)` (`golang.org/x/time@v0.15.0/rate/rate.go:207-209,227-230`), which always grants when
   `n <= burst`, stores the bucket's tokens even when negative and returns the time to wait
   (`rate.go:350-377`); nothing cancels the reservation. So the token is consumed when the
   wrapper skips the reconcile and returns `RequeueAfter`. The wrapper then records the request
   as limited and lets it through without a second `When` when it returns
   (`xpr:pkg/ratelimiter/reconciler.go:65-98`), so each pass costs exactly one token. The Rust
   bucket implements exactly this, in Go's float arithmetic (`advance`, `durationFromTokens`).
   With rate 0 and burst 0 it returns Go's `InfDuration`.
3. Is the reconcile-request default right? `Managed::TRACKS_RECONCILE_REQUESTS` is off unless
   the derive is told otherwise, which matches the Go baseline; on is what a user of
   `status.lastHandledReconcileAt` expects.
4. Answered (unit 1.9). Yes, with the last known object. client-go v0.36 replaces the store
   atomically on a relist and sends a delete for every vanished key before the list's updates
   (`processReplacedAllInfo`, `cg:tools/cache/controller.go:756-800`); the Rust store does the
   same on kube's `Init..InitDone`, tested in `cache_test.rs`. Each subscriber's queue is
   unbounded, as client-go's `processorListener` buffer is (`shared_informer.go:1223-1228`), so a
   slow controller loses no event. Cached reads also bring back Go's 409 pattern: a write made
   from a cached read can conflict with the installed provider's newer status write, which the
   builder live test hit and retries.
5. Answered (unit 1.9). Implemented as client-go's. Observable: the first event is a create,
   repeats within ten minutes are a patch of `count`, `lastTimestamp` and `message` (live: 12
   identical events gave 1 create and 11 patches in the audit log, the Go baseline's shape).
   Aggregation into "(combined from similar events)" needs 10 distinct messages with one reason
   in ten minutes, and the spam filter 25 events per object in a burst. The audit policy keeps
   no request bodies, so whether a parity run reaches either is not known from the traces.
6. Driver side answered (unit 1.9): the driver adds no reconcile timeout, because
   controller-runtime's `ReconciliationTimeout` is unset in the baseline; the managed reconciler
   owns the timeout. What the driver guarantees is that a key is released on every exit path
   (normal return, recovered panic, unrecovered panic, the worker's future dropped
   mid-reconcile), through a drop guard as Go defers `Done` (`controller.go:432`).
   `a_dropped_reconcile_releases_its_key` tests the dropped case. Reconciler side answered
   (unit 1.6): the reconciler no longer drops the run. Each call fails on its own deadline and
   the remaining branches run, as in Go (see "Implementation reconciliation", unit 1.6).
7. Answered (unit 1.6). No Go case pins it; the parity trace does. See "Status reset" under
   unit 1.6. `adding_the_finalizer_drops_the_conditions_observe_set` and the
   `new_mock_response_fn` cases cover it.
8. Is the JSON bridge in `get_as` and `assert_object_eq` cheap enough at poll rates, or does the
   cache need a typed fast path?
9. `controller_gen_compat` strips `nullable` globally. Is there a field where controller-gen
   does emit it?
10. Is owning the queue and informer worth it against adapting `kube::runtime`, once the queue
    cases are ported and the cost is known?

## Next implementation step

Unit 1.2: port the 31 condition cases against `crossplane-core/src/v2/condition.rs`, each with a
`// go:` marker.

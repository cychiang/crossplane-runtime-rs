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
  closure), and `crossplane-runtime` has `#![allow(dead_code)]` while bodies are `todo!()`. The
  second must go when unit 1.9 closes.

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
  the kube client verb mapping (1.4), the reconcile timeout and grace wrapper and the phase
  skeleton of `Run` beyond the two ported branches (1.6), `policy.rs` (1.6), the driver `work`
  loop (1.9; `queue_op` is tested), the derive expansion and `controller_gen_compat` (exercised
  by crd-diff only; 1.10 and 1.2).
- The provider's HTTP client (`examples/provider-sample/src/provider/client.rs`) is `todo!()`;
  it lands with the template in unit 2.3.
- `main.rs` mirrors only the environment variables Go names explicitly, not kingpin's
  `DefaultEnvars`.
- `hack/port-coverage.sh` gained two flags the brief did not ask for: `--complete` (fail while
  anything is missing) and `--list-missing`.

Unit 1.4 (client seam, test support):

- `KubeClient::uncached(kube)` reads the API server directly (Go `client.New`); `KubeClient::new`
  keeps the cache seam for 1.9. `kube_client::connect(config, user_agent)` builds the one
  `kube::Client` and sends the User-Agent as a default header (kube has no `UserAgent` field);
  `RestConfig::into_kube_client` should call it. `RestConfig` itself is still 1.9's `todo!()`.
  Go's baseline sends `crossplane-provider-baseline-provider/v0.0.0 (linux/arm64) kubernetes/$Format`
  (audit log), that is client-go's build version, not the provider's.
- The runtime's `kube` gained the `ring` feature: `rustls-tls` alone selects no crypto provider
  and the first connect panics.
- `convert_error` follows apimachinery's `IsNotFound`/`IsConflict`/`IsForbidden`/`IsInvalid`:
  the reason when it is a known one, else the HTTP code.
- `ObjectType::group_kind` and its `Debug` form match Go's `GroupKind`/`GroupVersionKind`
  `String()` for the core group (`ConfigMap`, `/v1, Kind=ConfigMap`).
- `Managed`, `ProviderConfig` and `ProviderConfigUsage` gained the setters and getters of Go's
  interfaces as provided methods, so derives are unchanged.
- `assert_object_eq` (and the new `assert_conditions_eq`) follow Go's `Condition.Equal`, which
  ignores `observedGeneration` when either side is zero. `Condition::equal` in `v2/condition.rs`
  (unit 1.2) does not, so `set_conditions` replaces a condition Go would keep.
- Go test patterns with no direct counterpart: `NewMockGetFn(err, fns...)` that fills the object
  and fails (`pkg/reconciler/managed/api_test.go:570` `UpdateConflictGetError` expects the caller's
  object filled after a failed get; a returned-object `get` cannot express it); `NewMockGetFn(nil)`
  where the Go object passed in is the caller's copy (the applicators), so the case must build
  that object in `new_mock_get_fn`; `fake.Manager` (builders take the client); `fake.GVK` and
  `fake.SchemeWith` (`ObjectType::of`); `NewMockClient()` (no in-scope table uses it);
  `fake.Managed` (use `fake::ModernManaged`). The legacy and composition fakes are out of scope.
- Open question 7: Update overwriting the in-memory object is now tested at the seam
  (`crossplane-test/tests/support.rs`, `new_mock_response_fn`) and against a real API server
  (`kube_client_test.rs`: a status set in memory is gone after the PUT). No Go table case pins it.

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

## Open questions and risks

None of these is resolved by the scaffold.

1. Nobody has implemented the work queue. Does `controller/workqueue.rs` need client-go's exact
   dirty and processing sets and its delaying heap, or is a smaller structure observably the same
   under the harness?
2. `GlobalRateLimiter`: Go's bucket limiter reserves a token and returns the delay. Which
   reserve semantics does the Rust limiter need for `when` to return the same delays?
3. Is the reconcile-request default right? `Managed::TRACKS_RECONCILE_REQUESTS` is off unless
   the derive is told otherwise, which matches the Go baseline; on is what a user of
   `status.lastHandledReconcileAt` expects.
4. Deletions missed across a relist: does the informer synthesize delete events for objects that
   vanished while the watch was down, as client-go's `DeletedFinalStateUnknown` does?
5. The event correlator is unimplemented. Which of client-go's aggregation and spam-filter rules
   are observable in the parity traces?
6. At the outer deadline the reconciler drops the whole run and returns a deadline error. Go lets
   each later Kubernetes call fail on the expired context and carries on through the remaining
   branches. Is the difference observable (status writes, events), and if so which side changes?
7. No ported case exercises Update overwriting the in-memory object (parity spec S5). Which Go
   case pins it, and should `late_initialize` wait for that case before it is written?
8. Is the JSON bridge in `get_as` and `assert_object_eq` cheap enough at poll rates, or does the
   cache need a typed fast path?
9. `controller_gen_compat` strips `nullable` globally. Is there a field where controller-gen
   does emit it?
10. Is owning the queue and informer worth it against adapting `kube::runtime`, once the queue
    cases are ported and the cost is known?

## Next implementation step

Unit 1.2: port the 31 condition cases against `crossplane-core/src/v2/condition.rs`, each with a
`// go:` marker.

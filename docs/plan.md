# Plan

Goal: a Rust port of the provider-facing part of crossplane-runtime v2.4.3,
a Rust provider template built on it, and proof from outside the Rust code
that both behave like the Go stack. Decisions and their reasons are in
`decisions.tsv`. Phase 0 documents are in `docs/phase0/`.

## Definition of done

All of these hold, each checked by a committed script:

1. Every row of `docs/phase0/port-denominator.tsv` maps to a passing Rust
   test or to a recorded skip with a reason (`hack/port-coverage.sh`).
2. The CRDs the Rust template generates match the Go baseline's CRDs
   structurally, apart from entries in `DIVERGENCES.md`.
3. On kind (Kubernetes 1.36, Crossplane core v2.4.3) the Go baseline and the
   Rust template both install as xpkg packages and reach `Healthy`.
4. The same chainsaw suite passes against both.
5. The audit-trace diff of the two runs is empty after normalization, apart
   from entries in `DIVERGENCES.md`. A Go-against-Go diff is empty first.

## Rigor

High for the reconciler, the client seam and the e2e harness: these decide
whether the parity claim means anything. Each gets a ported Go test table or
a differential check before the next unit starts. Lower for leaf packages
with small tables (`meta`, `conditions`, `errors`): port the table, run it,
move on.

## Phases

Each unit ends in a check and a commit.

### Phase 1: runtime port (this repository)

| Unit | Go source | Check |
|---|---|---|
| 1.0 Workspace scaffold: toolchain pin, lints, CI, coverage script | none | `cargo test`, `cargo clippy`, coverage script reports 0 of N |
| 1.1 Design bake-off: core types, client seam, reconcile outcome, controller driver | spec sections 1, 6 | design doc with type sketches that compile |
| 1.2 Common types and CRD schema fragments | `crossplane/apis` `core/v2` | 31 ported condition cases; schema fragments diff against Go CRDs |
| 1.3 Leaf packages: errors, meta, conditions, feature, event, logging | same-named packages | ported tables |
| 1.4 Client seam, test support, fake managed types | `pkg/test`, `pkg/resource/fake` | used by 1.5 onward |
| 1.5 `resource`: finalizer, usage tracker, late initializer, predicates, enqueue | `pkg/resource` | ported tables |
| 1.6 Managed reconciler: policies, connection publisher, critical annotations, change logs, metrics | `pkg/reconciler/managed` | 121 ported cases |
| 1.7 ProviderConfig reconciler | `pkg/reconciler/providerconfig` | ported table |
| 1.8 Safe-start gate | `pkg/gate`, `pkg/reconciler/customresourcesgate` | ported tables |
| 1.9 Driver: rate limiting, requeue outcomes, predicates, events recorder, leader election, metrics endpoint | `pkg/ratelimiter`, `pkg/controller`, `pkg/statemetrics`, controller-runtime behaviour in `03-platform-gaps.md` | ported tables plus tests written from the gaps document |
| 1.10 Derive macro replacing angryjet | crossplane-tools | compile tests on the fake types |
| 1.11 Reference resolution | `pkg/reference` | 98 ported cases |

### Phase 2: reference pair

| Unit | Check |
|---|---|
| 2.1 Fake external service: a small HTTP server both providers call, which logs every call | its own tests |
| 2.2 Go baseline from xp-provider-gen, with `external.go` calling the fake service so create and delete paths run | `make build`, chainsaw pause test passes |
| 2.3 `provider-template-rs` with the same group, kinds and external logic | builds, CRD diff |
| 2.4 xpkg packaging for the Rust binary | `crossplane xpkg build` output installs |

The stock scaffold's external client never reaches Create or Delete, because
the default initializer sets the external name before the first Observe
(inferred from source, `02-parity-spec.md`). Both providers therefore get the
same richer external client. The fake service's call log is a third oracle
beside chainsaw and the audit trace.

### Phase 3: differential e2e

| Unit | Check |
|---|---|
| 3.1 kind config with audit logging and pinned node image | cluster comes up, audit file has provider writes |
| 3.2 Trace normalizer and differ | unit tests on recorded traces |
| 3.3 Go-against-Go noise floor | diff empty |
| 3.4 Chainsaw scenarios: create, update, pause, policies, delete, failure paths | pass on Go |
| 3.5 Go-against-Rust | chainsaw equal, trace diff empty or explained |

### Later

Experimental Rust target in xp-provider-gen, once Phase 3 passes.

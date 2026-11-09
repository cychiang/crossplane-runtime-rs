# Phase 0.4: proposed version pins

This document proposes the pins for the Rust port and the differential e2e. All values were checked on 2026-11-09 against the source URL in the same row. Each pin is a proposal. Items marked **User decision** are the user's call.

## Crossplane core

| Item | Value | Source |
|---|---|---|
| Core release built on crossplane-runtime v2.4.3 | v2.4.3 only. The v2.4.3 source tarball confirms `crossplane-runtime/v2 v2.4.3` at go.mod line 12 | https://raw.githubusercontent.com/crossplane/crossplane/v2.4.3/go.mod |
| Its Kubernetes libraries | `k8s.io/api`, `apimachinery`, and `client-go` v0.35.3. `apiextensions-apiserver` v0.35.0 | same go.mod |
| Its controller-runtime | v0.23.1 | same go.mod |
| Its Go toolchain | go 1.26.10 | same go.mod |
| Neighbouring tags | v2.4.2 pins runtime v2.4.2. v2.4.0 and v2.4.1 pin v2.4.0. v2.3.7 pins v2.3.5 | `https://raw.githubusercontent.com/crossplane/crossplane/<tag>/go.mod` |
| Latest stable core | v2.4.3, released 2026-10-22. v2.5.0-rc.0 is a prerelease | https://api.github.com/repos/crossplane/crossplane/releases/latest |
| Helm chart | `crossplane` 2.4.3, appVersion 2.4.3, repo `https://charts.crossplane.io/stable` | https://charts.crossplane.io/stable/index.yaml |
| Generator default | `CROSSPLANE_VERSION ?= 2.4.3` | provider-baseline `hack/xp-provider-gen.mk:56` |

**Proposal.** Install Crossplane core v2.4.3 with Helm chart 2.4.3 in both e2e runs. It is the only core release that links the same runtime version as the providers under test. It is also the generator's default.

## Kubernetes and kind

| Item | Value | Source |
|---|---|---|
| kind | v0.33.0, released 2026-08-26. Local is v0.33.0 | https://api.github.com/repos/kubernetes-sigs/kind/releases/latest |
| Generator pin | `KIND_VERSION ?= v0.33.0` | provider-baseline `build/makelib/k8s_tools.mk:32` |
| kind node 1.35 | `kindest/node:v1.35.8@sha256:07b2536e30b803ed61d1677a79df6115f798ce64c80f9e22f6ed45afd09323c0` | https://github.com/kubernetes-sigs/kind/releases/tag/v0.33.0 |
| kind node 1.36 | `kindest/node:v1.36.4@sha256:099e049362a1526b2db71494e1947aae99bd16290d7c895f2b7ea312e3cbfaed` | same release notes |
| kind node 1.37 (kind default) | `kindest/node:v1.37.0@sha256:a1ed56cfb0e7b93589bdf97c8cd566405a265939e3620fc4f5de89adff580ae5` | same release notes |
| Upstream latest patches | 1.35.10, 1.36.6, and 1.37.2. kind images lag at 1.35.8 and 1.36.4 | https://dl.k8s.io/release/stable-1.35.txt, https://dl.k8s.io/release/stable-1.36.txt, https://dl.k8s.io/release/stable.txt |

## The 0.35 versus 0.36 mismatch, and which Kubernetes to run

crossplane-runtime v2.4.3 pins k8s.io 0.35.3 and controller-runtime v0.23.1 (`xpr-v2.4.3/go.mod:25-32`). Crossplane core v2.4.3 pins the same. The xp-provider-gen generator pins k8s.io v0.36.3 and controller-runtime v0.24.1 (`xp-provider-gen/pkg/versions/dependencies.yaml:57-64`). So the Go baseline links the newer pair (`provider-baseline/go.mod:14-17`). Go's minimal version selection takes the higher version, so the runtime code runs against controller-runtime v0.24.1 in the baseline binary. The runtime's own tests never ran on that version. Two behaviors in `03-platform-gaps.md` come from v0.24.1. One is the default priority queue (`controller-runtime@v0.24.1/pkg/controller/controller.go:252-271`). The other is the event recorder path (`pkg/internal/recorder/recorder.go:113-117`).

The e2e today runs neither version. `controlplane.mk` creates the cluster with no `--image` (`provider-baseline/build/makelib/controlplane.mk:25`), so kind v0.33.0 uses its default node, v1.37.0.

What this means:
- The Go-versus-Rust diff is valid on any one Kubernetes version, as long as both runs use the same node image digest.
- The Kubernetes version decides which API server behavior both providers meet. Examples are defaulting, managedFields, and the events API.
- client-go and kube both tolerate one minor of version skew. A 0.35 or 0.36 client against a 1.35, 1.36, or 1.37 server is supported (Kubernetes version skew policy, not re-read this session).

**Options.** **User decision.**
1. Run `kindest/node:v1.35.8`. It matches the runtime and core pins. Set k8s-openapi feature `v1_35`.
2. Run `kindest/node:v1.36.4`. It matches what the Go baseline binary links. Set k8s-openapi feature `v1_36`.
3. Run `kindest/node:v1.37.0`, the generator's current behavior. Set k8s-openapi feature `v1_37`.

A fourth axis is independent of the node choice. The Go baseline could pin k8s.io back to 0.35.3 and controller-runtime to v0.23.1, so the oracle matches the runtime's tested pins.

**Recommendation.** Option 1, and leave the Go baseline on the generator's 0.36.3 and v0.24.1. The oracle should be what a real generated provider runs, and the generator pins those versions. v1.35 is the API version that the runtime and core were built against. Pin the node by digest in a kind config. Do not rely on kind's default.

## Rust crates

All versions come from `https://crates.io/api/v1/crates/<name>` (`max_stable_version` and the version's `rust_version`). "Not declared" means the crate publishes no `rust-version`.

| Crate | Latest stable | MSRV | Notes |
|---|---|---|---|
| `kube` | 5.0.0 (2026-11-01) | 1.89.0 | Features `runtime`, `derive`, `client`, `rustls-tls`, and `unstable-runtime` if the predicate design in `03-platform-gaps.md` uses `Controller::for_stream`. Requires `k8s-openapi ^0.29.0` (https://crates.io/api/v1/crates/kube/5.0.0/dependencies). Tested against Kubernetes 1.33 to 1.37 (https://github.com/kube-rs/kube/releases/tag/5.0.0) |
| `kube-runtime` | 5.0.0 | 1.89.0 | No leader election module (https://raw.githubusercontent.com/kube-rs/kube/5.0.0/kube-runtime/src/lib.rs) |
| `kube-derive`, `kube-client`, `kube-core` | 5.0.0 | 1.89.0 | Pulled in by `kube` |
| `k8s-openapi` | 0.29.0 (2026-10-28) | not declared | Version features `v1_33` to `v1_37`, plus `latest` and `schemars`. `latest` means `v1_37`, so set the feature explicitly to match the node image |
| `schemars` | 1.2.3 | 1.74 | `kube-core` takes `schemars = "1.0.0"` as an optional dependency behind `derive` (https://github.com/kube-rs/kube/blob/5.0.0/kube-core/Cargo.toml#L25 and the workspace `Cargo.toml` line 75). Use 1.x |
| `json-patch` | 4.2.0 (2026-04-30) | not declared | 5.0.0 exists, but the kube 5.0.0 workspace pins `json-patch = "4"` (https://github.com/kube-rs/kube/blob/5.0.0/Cargo.toml#L60). `kube::api::Patch::Json` takes the 4.x type, so a direct 5.x dependency gives a second, incompatible type. Pin 4.2 |
| `tokio` | 1.54.1 | 1.85 | `full` or `rt-multi-thread,macros,signal,time,net` |
| `tokio-util` | 0.7.20 | 1.85 | `CancellationToken` for the manager |
| `tonic` | 0.14.9 | 1.88 | ChangeLogs client over a Unix socket |
| `tonic-prost-build` and `tonic-prost` | 0.14.9 | (tonic family) | Prost codegen moved to these crates in 0.14 |
| `prost` | 0.14.4 | 1.85 | `prost-types` for `Struct` and `Timestamp` |
| `prometheus` | 0.14.0 (2025-03-27) | 1.81 | Proposed. Same text format and naming model as Go `client_golang` |
| `prometheus-client` | 0.25.2 | not declared | Alternative, OpenMetrics |
| `metrics` and `metrics-exporter-prometheus` | 0.24.6 and 0.18.3 | 1.71.1 | Alternative facade. Not proposed, because histogram bucket control is indirect |
| `tracing` | 0.1.44 | 1.65 | kube already uses it |
| `tracing-subscriber` | 0.3.23 | 1.65 | `env-filter`, `json` |
| `clap` | 4.6.7 | 1.85 | `derive`, `env` (kingpin env parity) |
| `thiserror` | 2.0.21 | 1.77 | Runtime library errors |
| `anyhow` | 1.0.104 | 1.68 | Provider binary `main` only |
| `serde` | 1.0.229 | 1.56 | |
| `serde_json` | 1.0.154 | 1.71 | |
| `axum` | 0.8.9 | 1.80 | `/metrics` on `:8080`. `hyper` 1.12.0 (MSRV 1.63) is the lighter option |
| `tower` | 0.5.3 | 1.64 | Client middleware (throttle layer) |
| `governor` | 0.10.4 (2025-12-16) | not declared | Token bucket for the client QPS throttle and the global reconcile bucket |
| `tower-test` | 0.4.0 (2021) | not declared | Stale. Only needed if the tower-mock test seam is chosen |
| `kube-lease-manager` | 0.13.1 (2026-11-02) | 1.89.0 | Depends on `kube ^5.0.0` and `k8s-openapi ^0.29.0`. Uses SSA on the Lease (https://github.com/alex-karpenko/kube-lease-manager README) |
| `kube-leader-election` | 0.44.0 (2026-06-16) | | Depends on `kube ^4`. Not usable with kube 5 |

## Test and packaging tools

None of these are installed locally. The commands below are for the user to run. This session installed nothing.

| Tool | Version | Install | Source |
|---|---|---|---|
| chainsaw | v0.2.15 (2026-05-06). The generator pins 0.2.15 (`provider-baseline/build/makelib/k8s_tools.mk:82`) | `brew tap kyverno/chainsaw https://github.com/kyverno/chainsaw && brew install kyverno/chainsaw/chainsaw`, or `go install github.com/kyverno/chainsaw@v0.2.15`, or the release binary. Plain `brew install chainsaw` installs an unrelated tool | https://api.github.com/repos/kyverno/chainsaw/releases/latest, https://kyverno.github.io/chainsaw/latest/quick-start/install/ |
| uptest | v2.2.0 (2025-10-10). The generator pins v2.2.0 (`k8s_tools.mk:86`) | Release binary `uptest_<os>-<arch>` from GitHub releases. The generator Makefile downloads it into its tools dir | https://api.github.com/repos/crossplane/uptest/releases/latest |
| uptest's chainsaw | Its v2.2.0 go.mod requires chainsaw `v0.2.13-0.20250116043056-57a42010852a`. It runs chainsaw as a library or a forked CLI. Whether the release binary embeds chainsaw is unverified | https://raw.githubusercontent.com/crossplane/uptest/v2.2.0/go.mod |
| dyff | v1.12.0 (2026-04-26) | `brew install homeport/tap/dyff`, or `go install github.com/homeport/dyff/cmd/dyff@v1.12.0`, or the release tarball | https://api.github.com/repos/homeport/dyff/releases/latest, and the README `go install` path (https://github.com/homeport/dyff/blob/main/README.md) |
| crossplane CLI | v2.5.0 (2026-08-21). Local is v2.5.0. The generator pins v2.5.0 (`k8s_tools.mk:51`). Since v2.3.0 the CLI ships from the separate `crossplane/cli` repo at `cli.crossplane.io` (`k8s_tools.mk:54-57`), so its version is independent of core | https://api.github.com/repos/crossplane/cli/releases/latest, and `https://cli.crossplane.io/stable/v2.5.0/bin/darwin_arm64/crossplane` returned HTTP 200 |

The chainsaw version that uptest v2.2.0 links (0.2.13 pre-release) differs from the standalone chainsaw (0.2.15) that `make test-behavior` runs (`hack/xp-provider-gen.mk:95-99`). Both runs use the same pair, so the differential result does not depend on that difference.

## Rust toolchain and MSRV

| Item | Value | Source |
|---|---|---|
| Latest stable Rust | 1.99.1 (`rustc 1.99.1 (58cf8f9c3 2026-10-13)`). Local matches | https://static.rust-lang.org/dist/channel-rust-stable.toml |
| Highest MSRV among proposed crates | 1.89.0 (`kube` 5.0.0, `kube-lease-manager` 0.13.1). `tonic` 0.14.9 needs 1.88 | crates.io API rows above |
| kube MSRV policy | Trails the latest stable by about 2 versions, best effort | https://kube.rs/rust-version/ |
| kube main branch | Already declares 1.95 (unreleased) | https://raw.githubusercontent.com/kube-rs/kube/main/Cargo.toml |

**Options.** **User decision.**
1. Declare `rust-version = "1.89"`. It is the real floor today. Pin `rust-toolchain.toml` to `1.99.1` for reproducible builds. CI runs a second job on 1.89.
2. Declare `rust-version = "1.95"`. This anticipates the next kube release. It loses nothing, because nobody consumes the runtime on an older toolchain yet.

**Recommendation.** Option 1. Raise the MSRV when kube raises it, in the same PR as the kube bump.

## Pins proposed for the e2e, in one place

- Crossplane core v2.4.3 through Helm chart 2.4.3.
- kind v0.33.0 with `kindest/node:v1.35.8@sha256:07b2536e30b803ed61d1677a79df6115f798ce64c80f9e22f6ed45afd09323c0` (**User decision**, see the options above).
- k8s-openapi 0.29.0 with feature `v1_35`, and kube 5.0.0.
- chainsaw v0.2.15, uptest v2.2.0, dyff v1.12.0, and crossplane CLI v2.5.0.
- Rust 1.99.1 toolchain with MSRV 1.89.

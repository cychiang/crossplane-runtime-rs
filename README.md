# crossplane-runtime-rs

A Rust port of the provider-facing part of
[crossplane-runtime](https://github.com/crossplane/crossplane-runtime) v2.4.3:
the managed resource and ProviderConfig reconcilers, their default
collaborators, the controller driver and the manager. A provider written on it
behaves like a Go provider built on crossplane-runtime, and the parity harness
checks that from outside the Rust code.
[provider-template-rs](../provider-template-rs) is a provider template built on
it.

## Crates

| Crate | What it holds |
|---|---|
| `crates/crossplane-core` | core/v2 wire types, errors, meta helpers, the object traits and the Kubernetes client seam. No async runtime, no HTTP |
| `crates/crossplane-runtime` | Reconcilers, default collaborators, the controller driver and the manager |
| `crates/crossplane-derive` | Derives that replace angryjet's generated method sets |
| `crates/crossplane-test` | Go `pkg/test` and `pkg/resource/fake`: the closure MockClient, fake kinds and comparison helpers |
| `examples/provider-sample` | A sample provider used by the integration tests and the CRD diff |

## Tests

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
XPRS_TEST_KUBECONFIG=<kubeconfig> cargo test --workspace   # without it the cluster tests skip
hack/port-coverage.sh --complete    # every Go test case maps to a Rust test or a recorded skip
hack/crd-diff/crd-diff.sh           # the sample's CRDs against the Go baseline's
```

## Parity proof

`parity/scripts/parity.sh` builds the Go baseline provider and the Rust
template, runs the same chainsaw scenarios against each on one kind cluster,
and compares the API writes, Events, calls to a fake external service and
chainsaw steps of the two runs. It exits 0 when they match apart from the
accepted differences. See [parity/README.md](parity/README.md) for how to
reproduce it and what it compares.

## Documents

- [docs/design/core.md](docs/design/core.md): the design, unit by unit.
- [DIVERGENCES.md](DIVERGENCES.md): every known difference from the Go stack and why.
- [docs/plan.md](docs/plan.md): the plan and its definition of done.
- [docs/phase0/](docs/phase0/): the scope, the parity spec of the Go reconcilers and the version pins.

//! The vocabulary every other crate speaks. No async runtime and no HTTP client: a provider's
//! `apis` module and its CRD generator need nothing else, and `crossplane-test` implements
//! [`client::Client`] without depending on `crossplane-runtime`.
//!
//! The module map, with the Go package and the plan unit for each module, is in
//! `docs/design/core.md`.

extern crate self as crossplane_core;

pub mod client;
pub mod duration;
pub mod errors;
pub mod meta;
pub mod resource;
pub mod v2;

/// The derives live at the crate root, apart from the traits of the same name in [`resource`]:
/// every provider has a kind named `ProviderConfig`, and a `use` of the trait would collide with
/// it. Derives have their own namespace.
pub use crossplane_derive::{Inline, Managed, ProviderConfig, ProviderConfigUsage};

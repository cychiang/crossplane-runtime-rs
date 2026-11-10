//! The provider-facing part of crossplane-runtime v2.4.3: reconcilers, default collaborators, the
//! controller driver and the manager. Types, traits and the client seam are in `crossplane-core`;
//! there are no re-exports, so every name has one path.
//!
//! The kube-backed client, the informer cache and the work queue are private. Provider code
//! reaches Kubernetes only through the `Arc<dyn Client>` from [`manager::Manager::client`], and
//! registers controllers only through [`controller::new_managed_by`]:
//!
//! ```compile_fail,E0603
//! use crossplane_runtime::kube_client::KubeClient;
//! ```
//!
//! ```compile_fail,E0603
//! use crossplane_runtime::cache::Cache;
//! ```
//!
//! ```compile_fail,E0603
//! use crossplane_runtime::controller::workqueue::WorkQueue;
//! ```
//!
//! The module map, with the Go package and the plan unit for each module, is in
//! `docs/design/core.md`.

// Most bodies are still `todo!()`, so the private items they will use look dead. Each unit
// removes this once its module is filled in; the last one deletes the line.

mod cache;
mod kube_client;
#[cfg(test)]
mod live_test;

pub mod clock;
pub mod controller;
pub mod event;
pub mod feature;
pub mod gate;
pub mod logging;
pub mod manager;
pub mod ratelimiter;
pub mod reconciler;
pub mod resource;
pub mod statemetrics;

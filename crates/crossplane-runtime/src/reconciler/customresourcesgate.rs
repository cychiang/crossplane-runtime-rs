//! `pkg/reconciler/customresourcesgate`: flip the safe-start gate per GVK as CRDs become
//! Established.

use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::{Result, ResultExt};
use futures::FutureExt;
use futures::future::BoxFuture;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::core::GroupVersionKind;

use crate::controller;
use crate::gate::Gate;
use crate::manager::Manager;
use crate::reconciler::{Outcome, Reconcile, Request};

pub const CONTROLLER_NAME: &str = "crd-gate";

pub struct Reconciler {
    client: Arc<dyn Client>,
    gate: Arc<Gate<GroupVersionKind>>,
}

impl Reconciler {
    pub fn new(client: Arc<dyn Client>, gate: Arc<Gate<GroupVersionKind>>) -> Self {
        Self { client, gate }
    }
}

impl Reconcile for Reconciler {
    /// G1, G2. No writes.
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        async move {
            let crd = match self.client.get_as::<CustomResourceDefinition>(req).await {
                Ok(c) => c,
                Err(e) if e.is_not_found() => return Ok(Outcome::Done),
                Err(e) => return Err(e),
            };
            let _ = (&self.gate, crd);
            // TODO: established && !deleting -> set(gvk, true) per served version;
            // otherwise set(gvk, false) for every version.
            todo!()
        }
        .boxed()
    }
}

/// Go `customresourcesgate.Setup`: a `crd-gate` controller over CRDs.
pub fn setup(mgr: &Manager, o: &controller::Options) -> Result<()> {
    let r = Reconciler::new(mgr.client(), o.gate.clone());
    controller::new_managed_by(mgr)
        .named(CONTROLLER_NAME)
        .with_options(o.for_controller())
        .for_kind::<CustomResourceDefinition>()
        .complete(r)
        .wrap_err("cannot setup CRD gate controller")
}

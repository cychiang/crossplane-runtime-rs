//! `pkg/reconciler/customresourcesgate`: flip the safe-start gate per GVK as CRDs become
//! Established.

use std::collections::HashMap;
use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::{Result, ResultExt};
use crossplane_core::resource::Object;
use futures::FutureExt;
use futures::future::BoxFuture;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::core::GroupVersionKind;

use crate::controller;
use crate::gate::SetGate;
use crate::manager::Manager;
use crate::reconciler::{Outcome, Reconcile, Request};

pub const CONTROLLER_NAME: &str = "crd-gate";

const LAST_APPLIED_ANNOTATION: &str = "kubectl.kubernetes.io/last-applied-configuration";

pub struct Reconciler {
    client: Arc<dyn Client>,
    gate: Arc<dyn SetGate<GroupVersionKind>>,
}

impl Reconciler {
    pub fn new(client: Arc<dyn Client>, gate: Arc<dyn SetGate<GroupVersionKind>>) -> Self {
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
            let gvks = to_gvks(&crd);
            if !is_established(&crd) || crd.metadata.deletion_timestamp.is_some() {
                for gvk in gvks.into_keys() {
                    self.gate.set(gvk, false);
                }
            } else {
                for (gvk, _) in gvks.into_iter().filter(|(_, served)| *served) {
                    self.gate.set(gvk, true);
                }
            }
            Ok(Outcome::Done)
        }
        .boxed()
    }
}

/// Go `toGVKs`: every version of the CRD, with whether it is served. A repeated version name keeps
/// the last entry, as Go's map assignment does.
fn to_gvks(crd: &CustomResourceDefinition) -> HashMap<GroupVersionKind, bool> {
    crd.spec
        .versions
        .iter()
        .map(|v| {
            let gvk = GroupVersionKind {
                group: crd.spec.group.clone(),
                version: v.name.clone(),
                kind: crd.spec.names.kind.clone(),
            };
            (gvk, v.served)
        })
        .collect()
}

/// Go `isEstablished`: the first `Established` condition decides.
fn is_established(crd: &CustomResourceDefinition) -> bool {
    crd.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|cs| cs.iter().find(|c| c.type_ == "Established"))
        .is_some_and(|c| c.status == "True")
}

/// Go `TransformStripCRDSchema`, the informer cache's transform for CRDs: drops what the gate does
/// not read (version schemas, managed fields, the last-applied annotation) before an object is
/// stored. Any other kind is left alone.
pub fn transform_strip_crd_schema(obj: &mut dyn Object) {
    let Some(crd) = obj.downcast_mut::<CustomResourceDefinition>() else {
        return;
    };
    for version in &mut crd.spec.versions {
        version.schema = None;
    }
    crd.metadata.managed_fields = None;
    if let Some(annotations) = crd.metadata.annotations.as_mut() {
        annotations.remove(LAST_APPLIED_ANNOTATION);
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

#[cfg(test)]
#[path = "customresourcesgate_test.rs"]
mod tests;

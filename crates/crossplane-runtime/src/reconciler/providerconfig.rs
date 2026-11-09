//! `pkg/reconciler/providerconfig`: count usages, block deletion while in use.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::Client;
use crossplane_core::errors::Result;
use crossplane_core::resource::{Kind, ObjectType, ProviderConfig, ProviderConfigUsage};
use futures::FutureExt;
use futures::future::BoxFuture;

use crate::event::{NopRecorder, Recorder};
use crate::reconciler::{Outcome, Reconcile, Request};

pub const FINALIZER_NAME: &str = "in-use.crossplane.io";
pub const SHORT_WAIT: Duration = Duration::from_secs(30);
pub const TIMEOUT: Duration = Duration::from_secs(120);

/// Go `ControllerName(kind)`: `"providerconfig/" + lower(<Kind>.<group>)`.
pub fn controller_name<K: Kind>() -> String {
    format!("providerconfig/{}", ObjectType::of::<K>().group_kind()).to_lowercase()
}

/// Go `providerconfig.Reconciler`, typed by config and usage kind instead of `ProviderConfigKinds`
/// GVKs.
pub struct Reconciler<PC, U> {
    client: Arc<dyn Client>,
    recorder: Arc<dyn Recorder>,
    _kinds: PhantomData<fn() -> (PC, U)>,
}

pub struct ReconcilerBuilder<PC, U> {
    client: Arc<dyn Client>,
    recorder: Option<Arc<dyn Recorder>>,
    _kinds: PhantomData<fn() -> (PC, U)>,
}

impl<PC: ProviderConfig, U: ProviderConfigUsage> Reconciler<PC, U> {
    pub fn builder(client: Arc<dyn Client>) -> ReconcilerBuilder<PC, U> {
        ReconcilerBuilder {
            client,
            recorder: None,
            _kinds: PhantomData,
        }
    }
}

impl<PC: ProviderConfig, U: ProviderConfigUsage> ReconcilerBuilder<PC, U> {
    pub fn recorder(mut self, r: Arc<dyn Recorder>) -> Self {
        self.recorder = Some(r);
        self
    }

    pub fn build(self) -> Reconciler<PC, U> {
        Reconciler {
            client: self.client,
            recorder: self.recorder.unwrap_or_else(|| Arc::new(NopRecorder)),
            _kinds: PhantomData,
        }
    }
}

impl<PC: ProviderConfig, U: ProviderConfigUsage> Reconcile for Reconciler<PC, U> {
    /// PC1-PC8 (parity spec 4.1). Unconditional PUT and status PUT on every pass, as Go does; no
    /// conflict handling.
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        async move {
            let _ = (&self.client, &self.recorder, req, TIMEOUT, SHORT_WAIT, FINALIZER_NAME);
            // TODO: timeout(TIMEOUT); get_as::<PC> (ignore NotFound); list_as::<U> by labels (plus
            // namespace when PC is namespaced); delete uncontrolled usages; users = n;
            // deleted and n > 0 -> Terminating/InUse + status PUT; deleted and 0 -> remove
            // finalizer PUT; else add finalizer PUT, status.users = n, status PUT.
            todo!()
        }
        .boxed()
    }
}

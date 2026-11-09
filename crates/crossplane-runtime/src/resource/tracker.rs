use std::marker::PhantomData;
use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::Result;
use crossplane_core::resource::{Managed, ProviderConfigUsage};

pub const ERR_MISSING_PC_REF: &str = "managed resource does not reference a ProviderConfig";
pub const ERR_MISSING_PC_KIND: &str = "managed resource ProviderConfig reference has no Kind";
pub const ERR_APPLY_PCU: &str = "cannot apply ProviderConfigUsage";

/// Go `resource.ProviderConfigUsageTracker` (the typed, modern one).
pub struct ProviderConfigUsageTracker<U> {
    client: Arc<dyn Client>,
    _usage: PhantomData<fn() -> U>,
}

impl<U: ProviderConfigUsage> ProviderConfigUsageTracker<U> {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self {
            client,
            _usage: PhantomData,
        }
    }

    /// Ensures a usage named after the MR's UID, in the MR's namespace, labelled with the config
    /// name and kind, controller-owned by the MR and pointing back at it. Steady state is one Get
    /// and no write.
    pub async fn track<M: Managed>(&self, mg: &M) -> Result<()> {
        let _ = (&self.client, mg, U::from_usage);
        // TODO (pkg/resource/providerconfig.go:174): ref None -> ERR_MISSING_PC_REF;
        // kind empty -> ERR_MISSING_PC_KIND; U::from_usage(meta, usage); ApiUpdatingApplicator
        // with ControllersMustMatch + AllowUpdateIf(ref changed); swallow "not allowed";
        // wrap others in ERR_APPLY_PCU.
        todo!()
    }
}

use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::Result;
use crossplane_core::resource::Object;

/// Go `resource.ApplyOption`. A closed set: in-scope code uses these three.
pub enum ApplyOption {
    /// `MustBeControllableBy(uid)` / `ControllersMustMatch`.
    ControllersMustMatch,
    /// `ConnectionSecretMustBeControllableBy(uid)`.
    ConnectionSecretMustBeControllableBy(String),
    /// `AllowUpdateIf(fn)`, compared on the JSON forms of current and desired.
    AllowUpdateIf(Box<dyn Fn(&serde_json::Value, &serde_json::Value) -> bool + Send + Sync>),
}

/// Go `APIPatchingApplicator`.
pub struct ApiPatchingApplicator {
    client: Arc<dyn Client>,
}

impl ApiPatchingApplicator {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }

    pub async fn apply(&self, desired: &mut dyn Object, opts: &[ApplyOption]) -> Result<()> {
        let _ = (&self.client, desired, opts);
        // TODO (pkg/resource/api.go:53): Get; NotFound -> Create; else run the options, then
        // a merge Patch whose body is the whole desired object.
        todo!()
    }
}

/// Go `APIUpdatingApplicator`.
pub struct ApiUpdatingApplicator {
    client: Arc<dyn Client>,
}

impl ApiUpdatingApplicator {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }

    pub async fn apply(&self, desired: &mut dyn Object, opts: &[ApplyOption]) -> Result<()> {
        let _ = (&self.client, desired, opts);
        // TODO (pkg/resource/api.go:104): Get; NotFound -> Create; else run the options,
        // then Update.
        todo!()
    }
}

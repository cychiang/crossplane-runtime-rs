use crossplane_core::resource::ProviderConfigUsage;

use crate::controller::{EventHandler, WatchEvent};
use crate::reconciler::Request;

/// Go `resource.EnqueueRequestForProviderConfig` for typed usages: map a usage event to a request
/// for the config it references.
#[derive(Clone, Debug)]
pub struct EnqueueRequestForProviderConfig {
    kind: String,
}

impl EnqueueRequestForProviderConfig {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.to_owned() }
    }
}

impl<U: ProviderConfigUsage> EventHandler<U> for EnqueueRequestForProviderConfig {
    fn requests(&self, event: &WatchEvent<'_, U>) -> Vec<Request> {
        let _ = (&self.kind, event);
        // TODO (pkg/resource/enqueue_handlers.go:70): empty ref kind -> "ProviderConfig";
        // kind mismatch -> []; "Cluster*" -> name only; else name + the usage's namespace.
        todo!()
    }
}

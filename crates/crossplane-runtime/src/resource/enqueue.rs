use crossplane_core::resource::ProviderConfigUsage;

use crate::controller::{EventHandler, WatchEvent};
use crate::reconciler::Request;

/// Go `resource.EnqueueRequestForProviderConfig` for typed usages: map a usage event to a request
/// for the config it references. An empty kind filters nothing.
#[derive(Clone, Debug)]
pub struct EnqueueRequestForProviderConfig {
    kind: String,
}

impl EnqueueRequestForProviderConfig {
    pub fn new(kind: &str) -> Self {
        Self { kind: kind.to_owned() }
    }

    fn request_for<U: ProviderConfigUsage>(&self, usage: &U) -> Option<Request> {
        let r = usage.provider_config_reference();
        let kind = if r.kind.is_empty() { "ProviderConfig" } else { &r.kind };
        if !self.kind.is_empty() && kind != self.kind {
            return None;
        }
        Some(if kind.starts_with("Cluster") {
            Request::cluster(&r.name)
        } else {
            Request {
                namespace: usage.meta().namespace.clone(),
                name: r.name.clone(),
            }
        })
    }
}

impl<U: ProviderConfigUsage> EventHandler<U> for EnqueueRequestForProviderConfig {
    fn requests(&self, event: &WatchEvent<'_, U>) -> Vec<Request> {
        let usages = match event {
            WatchEvent::Create(u) | WatchEvent::Delete(u) => vec![*u],
            WatchEvent::Update { old, new } => vec![*old, *new],
        };
        usages.into_iter().filter_map(|u| self.request_for(u)).collect()
    }
}

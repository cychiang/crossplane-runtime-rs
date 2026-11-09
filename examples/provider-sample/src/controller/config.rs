//! Go `internal/controller/config/config.go`.

use crossplane_core::errors::Result;
use crossplane_runtime::controller::{self, Options};
use crossplane_runtime::manager::Manager;
use crossplane_runtime::reconciler::providerconfig;
use crossplane_runtime::{ratelimiter, resource};

use crate::apis::v1alpha1::{ProviderConfig, ProviderConfigUsage};

pub fn setup(mgr: &Manager, o: &Options) -> Result<()> {
    let name = providerconfig::controller_name::<ProviderConfig>();
    let r = providerconfig::Reconciler::<ProviderConfig, ProviderConfigUsage>::builder(mgr.client())
        .recorder(mgr.event_recorder_for(&name))
        .build();
    controller::new_managed_by(mgr)
        .named(&name)
        .with_options(o.for_controller())
        .for_kind::<ProviderConfig>()
        .watches::<ProviderConfigUsage>(resource::EnqueueRequestForProviderConfig::new("ProviderConfig"))
        .complete(ratelimiter::Reconciler::new(&name, r, o.global_rate_limiter.clone()))
}

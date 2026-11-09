//! Go `internal/controller/mytype/wiring.go`.

use crossplane_core::errors::{Result, ResultExt};
use crossplane_core::resource::ObjectType;
use crossplane_runtime::controller::{self, Options};
use crossplane_runtime::manager::Manager;
use crossplane_runtime::reconciler::managed;
use crossplane_runtime::statemetrics::MrStateRecorder;
use crossplane_runtime::{feature, ratelimiter, resource};

use super::{External, reconciler_options};
use crate::apis::sample_v1alpha1::MyType;
use crate::provider::Connector;

/// Registers `setup` behind the MyType CRD in the safe-start gate.
pub fn setup_gated(mgr: &Manager, o: &Options) -> Result<()> {
    let (mgr2, o2) = (mgr.clone(), o.clone());
    o.gate.register(
        move || {
            if let Err(e) = setup(&mgr2, &o2) {
                panic!("{}", e.wrap("cannot setup MyType controller"));
            }
        },
        [ObjectType::of::<MyType>().gvk()],
    );
    Ok(())
}

pub fn setup(mgr: &Manager, o: &Options) -> Result<()> {
    let name = managed::controller_name::<MyType>();
    let mut b = managed::Reconciler::<MyType>::builder(mgr.client())
        .external_connector(Connector::new(mgr, External::new))
        .poll_interval(o.poll_interval)
        .recorder(mgr.event_recorder_for(&name));
    if o.features.enabled(feature::ENABLE_BETA_MANAGEMENT_POLICIES) {
        b = b.management_policies();
    }
    if o.features.enabled(feature::ENABLE_ALPHA_CHANGE_LOGS)
        && let Some(cl) = &o.change_log_options
    {
        b = b.change_logger(cl.change_logger.clone());
    }
    if let Some(m) = &o.metric_options {
        b = b.metric_recorder(m.mr_metrics.clone());
        if let Some(sm) = &m.mr_state_metrics {
            mgr.add(MrStateRecorder::<MyType>::new(
                mgr.client(),
                sm.clone(),
                m.poll_state_metric_interval,
            ))
            .wrap_err("cannot register MR state metrics recorder for kind v1alpha1.MyTypeList")?;
        }
    }
    let r = reconciler_options(b, mgr, o)
        .wrap_err("cannot build reconciler options for MyType")?
        .build();

    controller::new_managed_by(mgr)
        .named(&name)
        .with_options(o.for_controller())
        .with_event_filter(resource::desired_state_changed())
        .for_kind::<MyType>()
        .complete(ratelimiter::Reconciler::new(&name, r, o.global_rate_limiter.clone()))
}

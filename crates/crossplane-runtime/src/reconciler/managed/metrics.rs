//! `pkg/reconciler/managed/metrics.go`.

use std::sync::Arc;

use crossplane_core::v2::ConditionedStatus;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use prometheus::core::{Collector, Desc};
use prometheus::proto::MetricFamily;

/// What a metric recorder sees of a managed resource. Not generic, so one recorder serves every
/// kind.
pub struct MetricSubject<'a> {
    pub gvk: &'a str,
    pub meta: &'a ObjectMeta,
    pub conditions: Option<&'a ConditionedStatus>,
}

/// Go `MetricRecorder` minus the Prometheus methods, which are the concrete recorder's
/// `Collector` impl.
pub trait MetricRecorder: Send + Sync + 'static {
    fn record_unchanged(&self, name: &str);
    fn record_first_time_reconciled(&self, mr: &MetricSubject<'_>);
    fn record_first_time_ready(&self, mr: &MetricSubject<'_>);
    fn record_drift(&self, mr: &MetricSubject<'_>);
    fn record_deleted(&self, mr: &MetricSubject<'_>);
}

/// Go `NopMetricRecorder`, the default.
pub struct NopMetricRecorder;

impl MetricRecorder for NopMetricRecorder {
    fn record_unchanged(&self, _: &str) {}
    fn record_first_time_reconciled(&self, _: &MetricSubject<'_>) {}
    fn record_first_time_ready(&self, _: &MetricSubject<'_>) {}
    fn record_drift(&self, _: &MetricSubject<'_>) {}
    fn record_deleted(&self, _: &MetricSubject<'_>) {}
}

/// Go `MRMetricRecorder`. Clone shares the histograms and the two maps, which stay keyed by
/// `metadata.name` only, as Go's are.
#[derive(Clone)]
pub struct MrMetricRecorder {
    inner: Arc<()>,
}

impl MrMetricRecorder {
    pub fn new() -> Self {
        todo!("HistogramVec with ExponentialBuckets(10e-9, 10, 10), subsystem crossplane")
    }
}

impl Default for MrMetricRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for MrMetricRecorder {
    fn desc(&self) -> Vec<&Desc> {
        let _ = &self.inner;
        todo!()
    }
    fn collect(&self) -> Vec<MetricFamily> {
        todo!()
    }
}

impl MetricRecorder for MrMetricRecorder {
    fn record_unchanged(&self, name: &str) {
        let _ = name;
        todo!()
    }
    fn record_first_time_reconciled(&self, mr: &MetricSubject<'_>) {
        let _ = mr;
        todo!("observe only when Synced is Unknown")
    }
    fn record_first_time_ready(&self, mr: &MetricSubject<'_>) {
        let _ = mr;
        todo!()
    }
    fn record_drift(&self, mr: &MetricSubject<'_>) {
        let _ = mr;
        todo!()
    }
    fn record_deleted(&self, mr: &MetricSubject<'_>) {
        let _ = mr;
        todo!()
    }
}

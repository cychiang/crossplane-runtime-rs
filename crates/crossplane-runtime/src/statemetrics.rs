//! `pkg/statemetrics`: the `crossplane_managed_resource_{exists,ready,synced}` gauges, refreshed
//! by listing each kind on an interval.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::Client;
use crossplane_core::errors::Result;
use crossplane_core::resource::Managed;
use futures::future::BoxFuture;
use prometheus::core::{Collector, Desc};
use prometheus::proto::MetricFamily;
use tokio_util::sync::CancellationToken;

use crate::manager::Runnable;

/// Go `MRStateMetrics`. Clone shares the gauges.
#[derive(Clone)]
pub struct MrStateMetrics {
    inner: Arc<()>,
}

impl MrStateMetrics {
    pub fn new() -> Self {
        todo!("GaugeVec exists/ready/synced, label gvk")
    }
}

impl Default for MrStateMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for MrStateMetrics {
    fn desc(&self) -> Vec<&Desc> {
        let _ = &self.inner;
        todo!()
    }
    fn collect(&self) -> Vec<MetricFamily> {
        todo!()
    }
}

/// Go `MRStateRecorder`: a runnable that lists `M` every `interval` and sets the gauges for its
/// GVK.
pub struct MrStateRecorder<M> {
    client: Arc<dyn Client>,
    metrics: MrStateMetrics,
    interval: Duration,
    _kind: PhantomData<fn() -> M>,
}

impl<M: Managed> MrStateRecorder<M> {
    pub fn new(client: Arc<dyn Client>, metrics: MrStateMetrics, interval: Duration) -> Self {
        Self {
            client,
            metrics,
            interval,
            _kind: PhantomData,
        }
    }
}

impl<M: Managed> Runnable for MrStateRecorder<M> {
    fn start(self: Box<Self>, shutdown: CancellationToken) -> BoxFuture<'static, Result<()>> {
        let _ = (&self.client, &self.metrics, self.interval, shutdown);
        Box::pin(async { todo!("interval loop: list_as::<M>, count exists/ready/synced") })
    }
}

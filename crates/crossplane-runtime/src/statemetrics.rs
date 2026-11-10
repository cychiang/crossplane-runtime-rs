//! `pkg/statemetrics`: the `crossplane_managed_resource_{exists,ready,synced}` gauges, refreshed
//! by listing each kind on an interval.

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::{Client, ListOptions};
use crossplane_core::errors::{Result, ResultExt};
use crossplane_core::resource::{Managed, ObjectType};
use crossplane_core::v2::{ConditionStatus, ConditionType};
use futures::future::BoxFuture;
use prometheus::core::{Collector, Desc};
use prometheus::proto::MetricFamily;
use prometheus::{GaugeVec, Opts};
use tokio_util::sync::CancellationToken;

use crate::manager::Runnable;

/// Go `subSystem` (`pkg/statemetrics/state_recorder.go:26`).
const SUBSYSTEM: &str = "crossplane";

/// Go `MRStateMetrics`. Clone shares the gauges.
#[derive(Clone)]
pub struct MrStateMetrics {
    inner: Arc<Gauges>,
}

struct Gauges {
    exists: GaugeVec,
    ready: GaugeVec,
    synced: GaugeVec,
}

fn gauge(name: &str, help: &str) -> GaugeVec {
    GaugeVec::new(Opts::new(name, help).subsystem(SUBSYSTEM), &["gvk"]).expect("a valid gauge")
}

impl MrStateMetrics {
    /// Go `NewMRStateMetrics` (`pkg/statemetrics/mr_state_metrics.go:46-59`).
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Gauges {
                exists: gauge("managed_resource_exists", "The number of managed resources that exist"),
                ready: gauge(
                    "managed_resource_ready",
                    "The number of managed resources in Ready=True state",
                ),
                synced: gauge(
                    "managed_resource_synced",
                    "The number of managed resources in Synced=True state",
                ),
            }),
        }
    }
}

impl Default for MrStateMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for MrStateMetrics {
    fn desc(&self) -> Vec<&Desc> {
        let g = &self.inner;
        [g.exists.desc(), g.ready.desc(), g.synced.desc()].concat()
    }

    fn collect(&self) -> Vec<MetricFamily> {
        let g = &self.inner;
        [g.exists.collect(), g.ready.collect(), g.synced.collect()].concat()
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

    /// Go `MRStateRecorder.Record`.
    pub(crate) async fn record(&self) -> Result<()> {
        let mrs: Vec<M> = self
            .client
            .list_as(&ListOptions::default())
            .await
            .wrap_err("failed to list managed resources")?;
        let label = gvk_label(&ObjectType::of::<M>());
        let count = |ct: &ConditionType| {
            mrs.iter()
                .filter(|m| m.get_condition(ct).status == ConditionStatus::True)
                .count()
        };
        let g = &self.metrics.inner;
        g.exists.with_label_values(&[&label]).set(mrs.len() as f64);
        g.ready
            .with_label_values(&[&label])
            .set(count(&ConditionType::READY) as f64);
        g.synced
            .with_label_values(&[&label])
            .set(count(&ConditionType::SYNCED) as f64);
        Ok(())
    }
}

/// Go `getLabels`: the list kind's GVK string with its first `List` removed.
fn gvk_label(object_type: &ObjectType) -> String {
    let r = object_type.api_resource();
    format!("{}/{}, Kind={}List", r.group, r.version, r.kind).replacen("List", "", 1)
}

impl<M: Managed> Runnable for MrStateRecorder<M> {
    /// Go `MRStateRecorder.Start`: record on every tick, the first one interval after start; a
    /// failed record ends the runnable with its error, which stops the manager.
    fn start(self: Box<Self>, shutdown: CancellationToken) -> BoxFuture<'static, Result<()>> {
        Box::pin(async move {
            let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + self.interval, self.interval);
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => return Ok(()),
                    _ = ticker.tick() => self.record().await?,
                }
            }
        })
    }
}

#[cfg(test)]
#[path = "statemetrics_test.rs"]
mod tests;

//! `pkg/reconciler/managed/metrics.go`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crossplane_core::v2::{ConditionStatus, ConditionType, ConditionedStatus};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::jiff::Timestamp;
use prometheus::core::{Collector, Desc};
use prometheus::proto::MetricFamily;
use prometheus::{HistogramOpts, HistogramVec};

use crate::clock::{Clock, SystemClock};

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

const SUBSYSTEM: &str = "crossplane";
const LABEL_GVK: &str = "gvk";

/// The buckets of the two lifecycle histograms measured in whole seconds.
const LIFECYCLE_BUCKETS: [f64; 11] = [1., 5., 10., 15., 30., 60., 120., 300., 600., 1800., 3600.];

/// Go `kmetrics.ExponentialBuckets(10e-9, 10, 10)`, multiplied up the same way so the bounds are
/// the same floats.
fn exponential_buckets() -> Vec<f64> {
    let mut bound = 10e-9;
    (0..10)
        .map(|_| {
            let b = bound;
            bound *= 10.;
            b
        })
        .collect()
}

fn histogram(name: &str, help: &str, buckets: Vec<f64>) -> HistogramVec {
    let opts = HistogramOpts::new(name, help).subsystem(SUBSYSTEM).buckets(buckets);
    HistogramVec::new(opts, &[LABEL_GVK]).expect("static histogram options are valid")
}

/// Go `MRMetricRecorder`. Clone shares the histograms and the two maps, which stay keyed by
/// `metadata.name` only, as Go's are.
#[derive(Clone)]
pub struct MrMetricRecorder {
    inner: Arc<Inner>,
}

struct Inner {
    clock: Arc<dyn Clock>,
    /// Names reconciled for the first time and not yet seen ready.
    first_observation: Mutex<HashSet<String>>,
    /// When each name was last found up to date.
    last_observation: Mutex<HashMap<String, Timestamp>>,
    detected: HistogramVec,
    first_time_ready: HistogramVec,
    deletion: HistogramVec,
    drift: HistogramVec,
}

impl MrMetricRecorder {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        let inner = Inner {
            clock,
            first_observation: Mutex::default(),
            last_observation: Mutex::default(),
            detected: histogram(
                "managed_resource_first_time_to_reconcile_seconds",
                "The time it took for a managed resource to be detected by the controller",
                exponential_buckets(),
            ),
            first_time_ready: histogram(
                "managed_resource_first_time_to_readiness_seconds",
                "The time it took for a managed resource to become ready first time after creation",
                LIFECYCLE_BUCKETS.to_vec(),
            ),
            deletion: histogram(
                "managed_resource_deletion_seconds",
                "The time it took for a managed resource to be deleted",
                LIFECYCLE_BUCKETS.to_vec(),
            ),
            drift: histogram(
                "managed_resource_drift_seconds",
                "ALPHA: How long since the previous successful reconcile when a resource was found to be out of sync; excludes restart of the provider",
                exponential_buckets(),
            ),
        };
        Self { inner: Arc::new(inner) }
    }
}

impl Inner {
    fn histograms(&self) -> [&HistogramVec; 4] {
        [&self.detected, &self.first_time_ready, &self.deletion, &self.drift]
    }

    /// Go `time.Since(t).Seconds()`.
    fn seconds_since(&self, t: Timestamp) -> f64 {
        self.clock.now().duration_since(t).as_secs_f64()
    }

    fn status(mr: &MetricSubject<'_>, ct: &ConditionType) -> ConditionStatus {
        mr.conditions
            .map_or(ConditionStatus::Unknown, |c| c.get_condition(ct).status)
    }
}

fn name<'a>(mr: &MetricSubject<'a>) -> &'a str {
    mr.meta.name.as_deref().unwrap_or_default()
}

impl Default for MrMetricRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector for MrMetricRecorder {
    fn desc(&self) -> Vec<&Desc> {
        self.inner.histograms().iter().flat_map(|h| h.desc()).collect()
    }
    fn collect(&self) -> Vec<MetricFamily> {
        self.inner.histograms().iter().flat_map(|h| h.collect()).collect()
    }
}

/// A timestamp the API server always sets is read as Go reads it; an object without one (only a
/// test double) observes nothing, where Go would observe the seconds since year one.
impl MetricRecorder for MrMetricRecorder {
    fn record_unchanged(&self, name: &str) {
        let now = self.inner.clock.now();
        self.inner.last_observation.lock().unwrap().insert(name.to_owned(), now);
    }

    fn record_first_time_reconciled(&self, mr: &MetricSubject<'_>) {
        if Inner::status(mr, &ConditionType::SYNCED) != ConditionStatus::Unknown {
            return;
        }
        if let Some(created) = &mr.meta.creation_timestamp {
            let seconds = self.inner.seconds_since(created.0);
            self.inner.detected.with_label_values(&[mr.gvk]).observe(seconds);
        }
        self.inner.first_observation.lock().unwrap().insert(name(mr).to_owned());
    }

    fn record_first_time_ready(&self, mr: &MetricSubject<'_>) {
        if Inner::status(mr, &ConditionType::READY) != ConditionStatus::True {
            return;
        }
        if !self.inner.first_observation.lock().unwrap().remove(name(mr)) {
            return;
        }
        if let Some(created) = &mr.meta.creation_timestamp {
            let seconds = self.inner.seconds_since(created.0);
            self.inner
                .first_time_ready
                .with_label_values(&[mr.gvk])
                .observe(seconds);
        }
    }

    fn record_drift(&self, mr: &MetricSubject<'_>) {
        let now = self.inner.clock.now();
        let mut last = self.inner.last_observation.lock().unwrap();
        let Some(previous) = last.get_mut(name(mr)) else {
            return;
        };
        let seconds = now.duration_since(*previous).as_secs_f64();
        self.inner.drift.with_label_values(&[mr.gvk]).observe(seconds);
        *previous = now;
    }

    fn record_deleted(&self, mr: &MetricSubject<'_>) {
        if let Some(deleted) = &mr.meta.deletion_timestamp {
            let seconds = self.inner.seconds_since(deleted.0);
            self.inner.deletion.with_label_values(&[mr.gvk]).observe(seconds);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossplane_core::v2::Condition;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

    use super::*;

    const GVK: &str = "example.org/v1, Kind=Thing";

    struct SetClock(Mutex<Timestamp>);

    impl Clock for SetClock {
        fn now(&self) -> Timestamp {
            *self.0.lock().unwrap()
        }
    }

    fn at(seconds: i64) -> Timestamp {
        Timestamp::from_second(1_700_000_000 + seconds).unwrap()
    }

    fn recorder() -> (MrMetricRecorder, Arc<SetClock>) {
        let clock = Arc::new(SetClock(Mutex::new(at(0))));
        (MrMetricRecorder::with_clock(clock.clone()), clock)
    }

    fn meta(created: Option<i64>, deleted: Option<i64>) -> ObjectMeta {
        ObjectMeta {
            name: Some("thing".to_owned()),
            creation_timestamp: created.map(|s| Time(at(s))),
            deletion_timestamp: deleted.map(|s| Time(at(s))),
            ..Default::default()
        }
    }

    fn subject<'a>(meta: &'a ObjectMeta, conditions: &'a ConditionedStatus) -> MetricSubject<'a> {
        MetricSubject {
            gvk: GVK,
            meta,
            conditions: Some(conditions),
        }
    }

    /// The `(sample count, sample sum)` of one histogram for [`GVK`], zeros when it has no sample.
    fn sample(r: &MrMetricRecorder, family: &str) -> (u64, f64) {
        let families = r.collect();
        let f = families
            .iter()
            .find(|f| f.name() == family)
            .unwrap_or_else(|| panic!("no metric family {family}"));
        f.get_metric()
            .iter()
            .find(|m| m.get_label().iter().any(|l| l.name() == "gvk" && l.value() == GVK))
            .map_or((0, 0.), |m| {
                let h = m.get_histogram();
                (h.get_sample_count(), h.get_sample_sum())
            })
    }

    const DETECTED: &str = "crossplane_managed_resource_first_time_to_reconcile_seconds";
    const READY: &str = "crossplane_managed_resource_first_time_to_readiness_seconds";
    const DELETION: &str = "crossplane_managed_resource_deletion_seconds";
    const DRIFT: &str = "crossplane_managed_resource_drift_seconds";

    #[test]
    fn describes_the_four_go_histograms() {
        let (r, _) = recorder();
        let mut names: Vec<&str> = r.desc().iter().map(|d| d.fq_name.as_str()).collect();
        names.sort_unstable();
        let mut want = [DETECTED, READY, DELETION, DRIFT];
        want.sort_unstable();
        assert_eq!(want.to_vec(), names);
        assert!(r.desc().iter().all(|d| d.variable_labels == ["gvk"]));
    }

    #[test]
    fn buckets_match_go() {
        let (r, _) = recorder();
        let none = ConditionedStatus::default();
        let m = meta(Some(-1), Some(-1));
        r.record_first_time_reconciled(&subject(&m, &none));
        r.record_deleted(&subject(&m, &none));

        let bounds = |family: &str| -> Vec<f64> {
            let families = r.collect();
            let f = families.iter().find(|f| f.name() == family).unwrap();
            f.get_metric()[0]
                .get_histogram()
                .get_bucket()
                .iter()
                .map(|b| b.upper_bound())
                .collect()
        };
        let exponential = bounds(DETECTED);
        assert_eq!(10, exponential.len());
        assert_eq!(10e-9, exponential[0]);
        // Ten multiplications by ten accumulate float error, as they do in Go.
        assert!((exponential[9] - 10.).abs() < 1e-9, "{}", exponential[9]);
        assert_eq!(LIFECYCLE_BUCKETS.to_vec(), bounds(DELETION));
    }

    #[test]
    fn first_reconcile_is_observed_only_while_synced_is_unknown() {
        let (r, _) = recorder();
        let m = meta(Some(-3), None);

        let synced = ConditionedStatus::new([Condition::reconcile_success()]);
        r.record_first_time_reconciled(&subject(&m, &synced));
        assert_eq!((0, 0.), sample(&r, DETECTED));

        r.record_first_time_reconciled(&subject(&m, &ConditionedStatus::default()));
        assert_eq!((1, 3.), sample(&r, DETECTED));
    }

    #[test]
    fn a_kind_without_conditions_counts_as_unknown() {
        let (r, _) = recorder();
        let m = meta(Some(-3), None);
        r.record_first_time_reconciled(&MetricSubject {
            gvk: GVK,
            meta: &m,
            conditions: None,
        });
        assert_eq!((1, 3.), sample(&r, DETECTED));
    }

    #[test]
    fn readiness_is_observed_once_after_a_first_reconcile() {
        let (r, clock) = recorder();
        let m = meta(Some(-3), None);
        let ready = ConditionedStatus::new([Condition::available()]);
        let unready = ConditionedStatus::new([Condition::unavailable()]);

        // Ready, but this recorder never saw the first reconcile (a provider restart).
        r.record_first_time_ready(&subject(&m, &ready));
        assert_eq!((0, 0.), sample(&r, READY));

        r.record_first_time_reconciled(&subject(&m, &ConditionedStatus::default()));
        r.record_first_time_ready(&subject(&m, &unready));
        assert_eq!((0, 0.), sample(&r, READY));

        *clock.0.lock().unwrap() = at(7);
        r.record_first_time_ready(&subject(&m, &ready));
        assert_eq!((1, 10.), sample(&r, READY));

        r.record_first_time_ready(&subject(&m, &ready));
        assert_eq!((1, 10.), sample(&r, READY));
    }

    #[test]
    fn drift_is_the_time_since_the_resource_was_last_up_to_date() {
        let (r, clock) = recorder();
        let m = meta(Some(-3), None);
        let none = ConditionedStatus::default();

        // Never seen up to date: nothing to measure from.
        r.record_drift(&subject(&m, &none));
        assert_eq!((0, 0.), sample(&r, DRIFT));

        r.record_unchanged("thing");
        *clock.0.lock().unwrap() = at(60);
        r.record_drift(&subject(&m, &none));
        assert_eq!((1, 60.), sample(&r, DRIFT));

        // A drift resets the reference point.
        *clock.0.lock().unwrap() = at(75);
        r.record_drift(&subject(&m, &none));
        assert_eq!((2, 75.), sample(&r, DRIFT));
    }

    #[test]
    fn deletion_is_the_time_since_the_deletion_timestamp() {
        let (r, clock) = recorder();
        *clock.0.lock().unwrap() = at(0) + Duration::from_millis(2500);
        r.record_deleted(&subject(&meta(Some(-9), Some(0)), &ConditionedStatus::default()));
        assert_eq!((1, 2.5), sample(&r, DELETION));
    }

    #[test]
    fn clones_share_state() {
        let (r, _) = recorder();
        let clone = r.clone();
        let m = meta(Some(-3), None);
        clone.record_first_time_reconciled(&subject(&m, &ConditionedStatus::default()));
        assert_eq!((1, 3.), sample(&r, DETECTED));
    }
}

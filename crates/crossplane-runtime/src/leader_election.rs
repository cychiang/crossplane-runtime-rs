//! client-go's Lease elector (`tools/leaderelection/leaderelection.go` with
//! `resourcelock/leaselock.go`), as controller-runtime runs it: get, create or update (PUT) of a
//! `coordination.k8s.io/v1` Lease with the resourceVersion as the guard, expiry judged from the
//! local time the record was last seen to change, never from the server's `renewTime`.
//!
//! Written here rather than taken from `kube-lease-manager` 0.13.1, which judges expiry from the
//! Lease's `renewTime` against the local wall clock, renews with a patch that carries no
//! resourceVersion, and bumps `leaseTransitions` in a second write (`state.rs:114-224` of that
//! crate).

use std::hash::BuildHasher;
use std::time::Duration;

use crossplane_core::errors::{Error, Result};
use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, ObjectMeta};
use k8s_openapi::jiff::Timestamp;
use kube::api::{Api, PostParams};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::event::{ApiRecorder, EventType, object_reference};

/// client-go `JitterFactor`.
const JITTER_FACTOR: f64 = 1.2;

#[derive(Clone, Debug, Default, PartialEq)]
struct Record {
    holder: String,
    lease_duration_seconds: i32,
    acquire_time: Option<MicroTime>,
    renew_time: Option<MicroTime>,
    transitions: i32,
}

impl Record {
    fn of(spec: &LeaseSpec) -> Self {
        Self {
            holder: spec.holder_identity.clone().unwrap_or_default(),
            lease_duration_seconds: spec.lease_duration_seconds.unwrap_or_default(),
            acquire_time: spec.acquire_time.clone(),
            renew_time: spec.renew_time.clone(),
            transitions: spec.lease_transitions.unwrap_or_default(),
        }
    }

    fn spec(&self) -> LeaseSpec {
        LeaseSpec {
            holder_identity: Some(self.holder.clone()),
            lease_duration_seconds: Some(self.lease_duration_seconds),
            acquire_time: self.acquire_time.clone(),
            renew_time: self.renew_time.clone(),
            lease_transitions: Some(self.transitions),
            ..Default::default()
        }
    }
}

pub(crate) struct Elector {
    api: Api<Lease>,
    name: String,
    namespace: String,
    identity: String,
    lease_duration: Duration,
    renew_deadline: Duration,
    retry_period: Duration,
    recorder: Option<ApiRecorder>,
    observed: Record,
    observed_raw: Option<String>,
    observed_time: Option<Instant>,
    lease: Option<Lease>,
}

/// controller-runtime's identity: the hostname, `_`, and a random UUID
/// (`pkg/leaderelection/leader_election.go`).
pub(crate) fn identity() -> String {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    let random = || std::collections::hash_map::RandomState::new().hash_one(0u8);
    let (hi, lo) = (random(), random());
    let version = (hi & 0x0fff) | 0x4000;
    let variant = ((lo >> 48) & 0x3fff) | 0x8000;
    format!(
        "{host}_{:08x}-{:04x}-{version:04x}-{variant:04x}-{:012x}",
        hi >> 32,
        (hi >> 16) & 0xffff,
        lo & 0xffff_ffff_ffff
    )
}

/// controller-runtime `getInClusterNamespace`.
pub(crate) fn in_cluster_namespace() -> Result<String> {
    const PATH: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";
    if !std::path::Path::new(PATH).exists() {
        return Err(Error::new(
            "unable to find leader election namespace: not running in-cluster, please specify LeaderElectionNamespace",
        ));
    }
    std::fs::read_to_string(PATH).map_err(|e| {
        Error::new(format!(
            "unable to find leader election namespace: error reading namespace file: {e}"
        ))
    })
}

fn jittered(period: Duration) -> Duration {
    period + period.mul_f64(crate::event::random_fraction() * JITTER_FACTOR)
}

impl Elector {
    /// client-go `NewLeaderElector`'s checks.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        kube: kube::Client,
        namespace: &str,
        name: &str,
        identity: String,
        lease_duration: Duration,
        renew_deadline: Duration,
        retry_period: Duration,
        recorder: Option<ApiRecorder>,
    ) -> Result<Self> {
        if lease_duration <= renew_deadline {
            return Err(Error::new("leaseDuration must be greater than renewDeadline"));
        }
        if renew_deadline <= retry_period.mul_f64(JITTER_FACTOR) {
            return Err(Error::new(
                "renewDeadline must be greater than retryPeriod*JitterFactor",
            ));
        }
        Ok(Self {
            api: Api::namespaced(kube, namespace),
            name: name.to_owned(),
            namespace: namespace.to_owned(),
            identity,
            lease_duration,
            renew_deadline,
            retry_period,
            recorder,
            observed: Record::default(),
            observed_raw: None,
            observed_time: None,
            lease: None,
        })
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    /// client-go `acquire`: try every retry period, with jitter, until the lease is ours. False
    /// when cancelled first.
    pub(crate) async fn acquire(&mut self, cancel: &CancellationToken) -> bool {
        tracing::info!(lock = %self.describe(), "Attempting to acquire leader lease...");
        loop {
            if self.try_acquire_or_renew().await {
                tracing::info!(lock = %self.describe(), "Successfully acquired lease");
                self.record_event("became leader");
                return true;
            }
            tokio::select! {
                () = cancel.cancelled() => return false,
                () = tokio::time::sleep(jittered(self.retry_period)) => {}
            }
        }
    }

    /// client-go `renew`: renew every retry period; return when a renewal has kept failing for
    /// the renew deadline (the lease is lost) or when cancelled. True means lost. The lease is
    /// not released on cancel, as controller-runtime leaves `ReleaseOnCancel` off.
    pub(crate) async fn renew(&mut self, cancel: &CancellationToken) -> bool {
        loop {
            let deadline = Instant::now() + self.renew_deadline;
            loop {
                if cancel.is_cancelled() {
                    return false;
                }
                if self.try_acquire_or_renew().await {
                    break;
                }
                let next = Instant::now() + self.retry_period;
                if next > deadline {
                    tracing::info!(lock = %self.describe(), "Failed to renew lease");
                    self.record_event("stopped leading");
                    return true;
                }
                tokio::select! {
                    () = cancel.cancelled() => return false,
                    () = tokio::time::sleep_until(next) => {}
                }
            }
            tokio::select! {
                () = cancel.cancelled() => return false,
                () = tokio::time::sleep(self.retry_period) => {}
            }
        }
    }

    fn describe(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    fn is_leader(&self) -> bool {
        self.observed.holder == self.identity
    }

    fn lease_valid(&self, now: Instant) -> bool {
        self.observed_time
            .is_some_and(|seen| seen + Duration::from_secs(self.observed.lease_duration_seconds.max(0) as u64) > now)
    }

    fn set_observed(&mut self, record: Record) {
        self.observed = record;
        self.observed_time = Some(Instant::now());
    }

    /// client-go `tryAcquireOrRenew` (`leaderelection.go:432-503`).
    async fn try_acquire_or_renew(&mut self) -> bool {
        let now = Instant::now();
        let stamp = Some(MicroTime(Timestamp::now()));
        let mut record = Record {
            holder: self.identity.clone(),
            lease_duration_seconds: self.lease_duration.as_secs() as i32,
            acquire_time: stamp.clone(),
            renew_time: stamp,
            transitions: 0,
        };

        if self.is_leader() && self.lease_valid(now) {
            record.acquire_time = self.observed.acquire_time.clone();
            record.transitions = self.observed.transitions;
            if self.update(&record).await.is_ok() {
                self.set_observed(record);
                return true;
            }
        }

        let old = match self.api.get(&self.name).await {
            Ok(lease) => lease,
            Err(kube::Error::Api(status)) if status.code == 404 => {
                let lease = Lease {
                    metadata: ObjectMeta {
                        name: Some(self.name.clone()),
                        namespace: Some(self.namespace.clone()),
                        ..Default::default()
                    },
                    spec: Some(record.spec()),
                };
                return match self.api.create(&PostParams::default(), &lease).await {
                    Ok(created) => {
                        self.lease = Some(created);
                        self.set_observed(record);
                        true
                    }
                    Err(err) => {
                        tracing::error!(error = %err, "Error initially creating lease lock");
                        false
                    }
                };
            }
            Err(err) => {
                tracing::error!(error = %err, "Error retrieving lease lock");
                return false;
            }
        };
        let old_record = Record::of(&old.spec.clone().unwrap_or_default());
        self.lease = Some(old);
        let raw = serde_json::to_string(&old_record.spec()).ok();
        if raw != self.observed_raw {
            self.set_observed(old_record.clone());
            self.observed_raw = raw;
        }
        if !old_record.holder.is_empty() && self.lease_valid(now) && !self.is_leader() {
            return false;
        }

        if self.is_leader() {
            record.acquire_time = old_record.acquire_time;
            record.transitions = old_record.transitions;
        } else {
            record.transitions = old_record.transitions + 1;
        }
        match self.update(&record).await {
            Ok(()) => {
                self.set_observed(record);
                true
            }
            Err(err) => {
                tracing::error!(error = %err, "Failed to update lease");
                false
            }
        }
    }

    /// client-go `LeaseLock.Update`: PUT the last seen Lease with the new spec.
    async fn update(&mut self, record: &Record) -> Result<()> {
        let Some(mut lease) = self.lease.clone() else {
            return Err(Error::new("lease not initialized, call get or create first"));
        };
        lease.spec = Some(record.spec());
        let updated = self
            .api
            .replace(&self.name, &PostParams::default(), &lease)
            .await
            .map_err(Error::foreign)?;
        self.lease = Some(updated);
        Ok(())
    }

    /// client-go `LeaseLock.RecordEvent`: a Normal `LeaderElection` event about the Lease.
    fn record_event(&self, what: &str) {
        let (Some(recorder), Some(lease)) = (&self.recorder, &self.lease) else {
            return;
        };
        let subject = Lease {
            metadata: lease.metadata.clone(),
            spec: None,
        };
        recorder.record(
            object_reference(&subject),
            EventType::Normal,
            "LeaderElection",
            &format!("{} {what}", self.identity),
        );
    }
}

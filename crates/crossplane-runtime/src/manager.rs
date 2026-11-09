//! What controller-runtime's manager does: one owner for the API client, the informer cache, the
//! event broadcaster, the metrics registry, leader election and shutdown. It accepts runnables
//! before and after start, because the safe-start gate adds controllers late.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::client::Client;
use crossplane_core::errors::Result;
use futures::future::BoxFuture;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cache::Cache;
use crate::event::{ApiRecorder, PendingEvent, Recorder};
use crate::kube_client::KubeClient;

/// How to reach the API server, and how fast. [`RestConfig::infer`] is the only constructor and
/// [`Manager::new`] the only consumer, so every request in the process carries the client-side
/// throttle and the Go-style User-Agent.
pub struct RestConfig {
    kube: kube::Config,
    qps: f32,
    burst: u32,
    user_agent: String,
}

impl RestConfig {
    /// Go `ctrl.GetConfig`: in-cluster config, or the local kubeconfig. Starts at client-go's
    /// defaults of 5 QPS and burst 10; `ratelimiter::limit_rest_config` raises them.
    ///
    /// The User-Agent has client-go's format, `<binary>/<version> (<os>/<arch>) kubernetes/<commit>`.
    pub async fn infer(binary: &str, version: &str) -> Result<Self> {
        todo!("kube::Config::infer; user agent for {binary}/{version}")
    }

    pub(crate) fn throttled(mut self, qps: f32, burst: u32) -> Self {
        self.qps = qps;
        self.burst = burst;
        self
    }

    /// Builds the one `kube::Client`, with the throttle as a tower layer and the User-Agent as a
    /// default header.
    fn into_kube_client(self) -> Result<kube::Client> {
        let _ = (self.kube, self.qps, self.burst, self.user_agent);
        todo!()
    }
}

/// Go `ctrl.Options` as the baseline sets them.
#[derive(Clone, Debug)]
pub struct ManagerOptions {
    /// Informer resync, the `--sync` flag. controller-runtime's default is 10h.
    pub sync_period: Duration,
    pub leader_election: Option<LeaderElection>,
    /// How long runnables get to stop after shutdown begins.
    pub graceful_shutdown_timeout: Duration,
    pub metrics_bind_address: String,
}

impl Default for ManagerOptions {
    fn default() -> Self {
        Self {
            sync_period: Duration::from_secs(10 * 3600),
            leader_election: None,
            graceful_shutdown_timeout: Duration::from_secs(30),
            metrics_bind_address: ":8080".to_owned(),
        }
    }
}

/// Lease-based leader election. Runnables that need it start only once the lease is held.
/// Losing the lease fails the manager without a graceful shutdown, as in Go.
#[derive(Clone, Debug)]
pub struct LeaderElection {
    pub id: String,
    pub namespace: Option<String>,
    pub lease_duration: Duration,
    pub renew_deadline: Duration,
    pub retry_period: Duration,
}

impl LeaderElection {
    /// controller-runtime's defaults: 15s lease, 10s renew deadline, 2s retry.
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            namespace: None,
            lease_duration: Duration::from_secs(15),
            renew_deadline: Duration::from_secs(10),
            retry_period: Duration::from_secs(2),
        }
    }

    pub fn lease_duration(mut self, d: Duration) -> Self {
        self.lease_duration = d;
        self
    }

    pub fn renew_deadline(mut self, d: Duration) -> Self {
        self.renew_deadline = d;
        self
    }
}

/// Go `manager.Runnable`: something the manager runs until shutdown. Controllers are registered
/// through `controller::new_managed_by`, never as a hand-written runnable.
pub trait Runnable: Send + 'static {
    fn start(self: Box<Self>, shutdown: CancellationToken) -> BoxFuture<'static, Result<()>>;

    /// Controllers wait for leadership; metrics serving does not.
    fn needs_leader_election(&self) -> bool {
        true
    }
}

/// A cheap handle. Clones share everything, so a gated setup closure can capture one and register
/// its controller later.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

struct Inner {
    client: Arc<dyn Client>,
    cache: Cache,
    kube: kube::Client,
    options: ManagerOptions,
    events: mpsc::UnboundedSender<PendingEvent>,
    metrics: prometheus::Registry,
    runnables: mpsc::UnboundedSender<Box<dyn Runnable>>,
    /// Taken by `start`.
    pending: Mutex<Option<Pending>>,
}

struct Pending {
    runnables: mpsc::UnboundedReceiver<Box<dyn Runnable>>,
    events: mpsc::UnboundedReceiver<PendingEvent>,
}

impl Manager {
    /// Go `ctrl.NewManager(cfg, opts)`.
    pub fn new(config: RestConfig, options: ManagerOptions) -> Result<Self> {
        let kube = config.into_kube_client()?;
        let cache = Cache::new(kube.clone(), options.sync_period);
        let (events, events_rx) = mpsc::unbounded_channel();
        let (runnables, runnables_rx) = mpsc::unbounded_channel();
        Ok(Self {
            inner: Arc::new(Inner {
                client: Arc::new(KubeClient::new(kube.clone(), cache.clone())),
                cache,
                kube,
                options,
                events,
                metrics: prometheus::Registry::new(),
                runnables,
                pending: Mutex::new(Some(Pending {
                    runnables: runnables_rx,
                    events: events_rx,
                })),
            }),
        })
    }

    /// Go `mgr.GetClient()`: cached reads, direct writes.
    pub fn client(&self) -> Arc<dyn Client> {
        self.inner.client.clone()
    }

    pub(crate) fn cache(&self) -> Cache {
        self.inner.cache.clone()
    }

    /// Go `event.NewAPIRecorder(mgr.GetEventRecorderFor(name))`.
    pub fn event_recorder_for(&self, name: &str) -> Arc<dyn Recorder> {
        Arc::new(ApiRecorder::new(name, self.inner.events.clone()))
    }

    /// Go `metrics.Registry.MustRegister`.
    pub fn register_metrics(&self, collector: Box<dyn prometheus::core::Collector>) -> Result<()> {
        let _ = (&self.inner.metrics, collector);
        todo!()
    }

    /// Go `mgr.Add(r)`. Before `start` the runnable waits for it; after, it starts at once.
    pub fn add(&self, runnable: impl Runnable) -> Result<()> {
        let _ = (&self.inner.runnables, Box::new(runnable) as Box<dyn Runnable>);
        todo!("send; an error when the manager has stopped")
    }

    /// Go `mgr.Start(ctx)`: run every runnable (leader-gated ones after acquiring the lease),
    /// keep accepting late adds, and on `shutdown` cancel and wait up to
    /// `graceful_shutdown_timeout`. An error when called twice.
    pub async fn start(&self, shutdown: impl Future<Output = ()> + Send) -> Result<()> {
        let _ = (
            &self.inner.pending,
            &self.inner.options,
            &self.inner.kube,
            crate::event::run_broadcaster,
        );
        shutdown.await;
        todo!()
    }
}

/// Go `ctrl.SetupSignalHandler()`: resolves on the first SIGINT or SIGTERM. A second one exits the
/// process with status 1.
pub async fn setup_signal_handler() {
    todo!("tokio::signal::unix; spawn second-signal exit(1)")
}

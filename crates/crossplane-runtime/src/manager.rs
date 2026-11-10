//! What controller-runtime's manager does: one owner for the API client, the informer cache, the
//! event broadcaster, the metrics registry, leader election and shutdown. It accepts runnables
//! before and after start, because the safe-start gate adds controllers late.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crossplane_core::client::Client;
use crossplane_core::errors::{Error, Result};
use futures::future::BoxFuture;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::cache::Cache;
use crate::clock::SystemClock;
use crate::event::{ApiRecorder, KubeEventSink, MAX_QUEUED_EVENTS, Recorder};
use crate::kube_client::{self, KubeClient};
use crate::ratelimiter::Throttle;
use k8s_openapi::api::core::v1::Event as CoreEvent;
use leader_election::Elector;

#[path = "leader_election.rs"]
pub(crate) mod leader_election;

/// How to reach the API server, and how fast. [`RestConfig::infer`] and
/// [`RestConfig::from_kubeconfig`] are the public constructors and [`Manager::new`] the only consumer, so every request in the process carries
/// the client-side throttle and the Go-style User-Agent.
#[derive(Clone)]
pub struct RestConfig {
    kube: kube::Config,
    qps: f32,
    burst: u32,
    user_agent: String,
}

impl RestConfig {
    /// Go `ctrl.GetConfig`: in-cluster config, or the local kubeconfig. Starts at client-go's
    /// defaults of 5 QPS and burst 10 (`rest/config.go` `DefaultQPS`, `DefaultBurst`);
    /// `ratelimiter::limit_rest_config` raises them.
    ///
    /// The User-Agent has client-go's format, `<command>/<version> (<os>/<arch>) kubernetes/<commit>`
    /// (`rest/config.go:515-536`). `binary` stands in when the process has no `argv[0]`.
    pub async fn infer(binary: &str, version: &str) -> Result<Self> {
        let kube = kube::Config::infer().await.map_err(Error::foreign)?;
        Ok(Self::from_config(kube, binary, version))
    }

    /// Like [`RestConfig::infer`], but reads the named kubeconfig file's current context and
    /// ignores the environment, as a `--kubeconfig` flag would.
    pub async fn from_kubeconfig(path: impl AsRef<std::path::Path>, binary: &str, version: &str) -> Result<Self> {
        let kubeconfig = kube::config::Kubeconfig::read_from(path).map_err(Error::foreign)?;
        let kube = kube::Config::from_custom_kubeconfig(kubeconfig, &kube::config::KubeConfigOptions::default())
            .await
            .map_err(Error::foreign)?;
        Ok(Self::from_config(kube, binary, version))
    }

    pub(crate) fn from_config(kube: kube::Config, binary: &str, version: &str) -> Self {
        let argv0 = std::env::args_os().next();
        let command = argv0
            .as_deref()
            .and_then(|p| std::path::Path::new(p).file_name())
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| binary.to_owned());
        Self {
            kube,
            qps: 5.0,
            burst: 10,
            user_agent: user_agent(&command, version, std::env::consts::OS, std::env::consts::ARCH, ""),
        }
    }

    #[cfg(test)]
    pub(crate) fn user_agent(&self) -> &str {
        &self.user_agent
    }

    pub(crate) fn throttled(mut self, qps: f32, burst: u32) -> Self {
        self.qps = qps;
        self.burst = burst;
        self
    }

    /// Go `rest.AddUserAgent`: appends `/<suffix>`, as the leader elector's client does.
    fn with_user_agent_suffix(mut self, suffix: &str) -> Self {
        self.user_agent = format!("{}/{suffix}", self.user_agent);
        self
    }

    /// Builds a `kube::Client` with its own throttle, as each client-go client gets its own rate
    /// limiter from `QPS` and `Burst`.
    pub(crate) fn into_kube_client(self) -> Result<kube::Client> {
        kube_client::connect(self.kube, &self.user_agent, Throttle::new(self.qps, self.burst))
    }
}

/// client-go `buildUserAgent` with `adjustVersion`, `adjustCommit` and Go's names for the OS and
/// architecture, so the string has the shape the Go baseline sends.
fn user_agent(command: &str, version: &str, os: &str, arch: &str, commit: &str) -> String {
    let or_unknown = |s: &str| {
        if s.is_empty() {
            "unknown".to_owned()
        } else {
            s.to_owned()
        }
    };
    let version = or_unknown(version.split('-').next().unwrap_or_default());
    let commit = or_unknown(&commit.chars().take(7).collect::<String>());
    let os = match os {
        "macos" => "darwin",
        other => other,
    };
    let arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        "powerpc64" => "ppc64le",
        other => other,
    };
    format!("{}/{version} ({os}/{arch}) kubernetes/{commit}", or_unknown(command))
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
    rest: RestConfig,
    options: ManagerOptions,
    events: mpsc::Sender<CoreEvent>,
    metrics: prometheus::Registry,
    metrics_address: OnceLock<SocketAddr>,
    runnables: mpsc::UnboundedSender<Box<dyn Runnable>>,
    /// Taken by `start`.
    pending: Mutex<Option<Pending>>,
}

struct Pending {
    runnables: mpsc::UnboundedReceiver<Box<dyn Runnable>>,
    events: mpsc::Receiver<CoreEvent>,
}

/// controller-runtime's default `CacheSyncTimeout` (`pkg/controller/controller.go:248-250`).
pub(crate) const CACHE_SYNC_TIMEOUT: Duration = Duration::from_secs(120);

impl Manager {
    /// Go `ctrl.NewManager(cfg, opts)`.
    pub fn new(config: RestConfig, options: ManagerOptions) -> Result<Self> {
        let kube = config.clone().into_kube_client()?;
        let cache = Cache::new(kube.clone(), options.sync_period);
        let (events, events_rx) = mpsc::channel(MAX_QUEUED_EVENTS);
        let (runnables, runnables_rx) = mpsc::unbounded_channel();
        Ok(Self {
            inner: Arc::new(Inner {
                client: Arc::new(KubeClient::new(kube.clone(), cache.clone())),
                cache,
                kube,
                rest: config,
                options,
                events,
                metrics: prometheus::Registry::new(),
                metrics_address: OnceLock::new(),
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

    /// Go `metrics.Registry.MustRegister`, returning the error instead of panicking.
    pub fn register_metrics(&self, collector: Box<dyn prometheus::core::Collector>) -> Result<()> {
        self.inner.metrics.register(collector).map_err(Error::foreign)
    }

    /// Where the metrics endpoint listens, once `start` has bound it. Tests bind port 0 and read
    /// the port back here.
    #[cfg(test)]
    pub(crate) fn metrics_address(&self) -> Option<SocketAddr> {
        self.inner.metrics_address.get().copied()
    }

    /// Go `mgr.Add(r)`. Before `start` the runnable waits for it; after, it starts at once.
    pub fn add(&self, runnable: impl Runnable) -> Result<()> {
        self.inner
            .runnables
            .send(Box::new(runnable))
            .map_err(|_| Error::new("can't accept new runnables as stop procedure is already engaged"))
    }

    /// Go `mgr.Start(ctx)`: serve metrics, run every runnable (leader-gated ones after acquiring
    /// the lease), keep accepting late adds, and on `shutdown` cancel and wait up to
    /// `graceful_shutdown_timeout`. A runnable's error stops the manager the same way and is
    /// returned. Losing the lease returns "leader election lost" without waiting. An error when
    /// called twice.
    pub async fn start(&self, shutdown: impl Future<Output = ()> + Send) -> Result<()> {
        let pending = self.inner.pending.lock().unwrap().take();
        let Some(Pending {
            runnables: mut added,
            events,
        }) = pending
        else {
            return Err(Error::new("manager already started"));
        };
        let stop = CancellationToken::new();
        let stop_events = CancellationToken::new();
        let broadcaster = tokio::spawn(crate::event::run_broadcaster(
            Arc::new(KubeEventSink::new(self.inner.kube.clone())),
            events,
            Arc::new(SystemClock),
            crate::event::DEFAULT_SLEEP_DURATION,
            stop_events.clone(),
        ));
        let result = self.run(&mut added, shutdown, &stop).await;
        added.close();
        self.inner.cache.stop();
        stop_events.cancel();
        let _ = broadcaster.await;
        result
    }

    async fn run(
        &self,
        added: &mut mpsc::UnboundedReceiver<Box<dyn Runnable>>,
        shutdown: impl Future<Output = ()> + Send,
        stop: &CancellationToken,
    ) -> Result<()> {
        let mut running = JoinSet::new();
        if let Some(server) = self.bind_metrics().await? {
            running.spawn(server.serve(self.inner.metrics.clone(), stop.clone()));
        }
        let (leading_tx, leading) = watch::channel(self.inner.options.leader_election.is_none());
        let lost = CancellationToken::new();
        if let Some(le) = &self.inner.options.leader_election {
            let mut elector = self.elector(le)?;
            let (stop, lost) = (stop.clone(), lost.clone());
            tokio::spawn(async move {
                if elector.acquire(&stop).await {
                    leading_tx.send_replace(true);
                    if elector.renew(&stop).await {
                        lost.cancel();
                    }
                }
            });
        }

        let mut shutdown = std::pin::pin!(shutdown);
        let outcome = loop {
            tokio::select! {
                () = &mut shutdown => break Ok(()),
                () = lost.cancelled() => return Err(Error::new("leader election lost")),
                Some(runnable) = added.recv() => {
                    running.spawn(start_runnable(runnable, leading.clone(), stop.clone()));
                }
                Some(joined) = running.join_next() => match joined {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => break Err(err),
                    Err(join) => break Err(Error::new(format!("runnable panicked: {join}"))),
                },
            }
        };

        stop.cancel();
        let timeout = self.inner.options.graceful_shutdown_timeout;
        let drained = tokio::time::timeout(timeout, async { while running.join_next().await.is_some() {} }).await;
        match (outcome, drained) {
            (Err(err), _) => Err(err),
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(_)) => Err(Error::new(format!(
                "failed waiting for all runnables to end within grace period of {}: context deadline exceeded",
                go_duration(timeout)
            ))),
        }
    }

    /// controller-runtime `NewResourceLock`: its own client, with `/leader-election` appended to
    /// the User-Agent and a request timeout of half the renew deadline, at least a second.
    fn elector(&self, le: &LeaderElection) -> Result<Elector> {
        let namespace = match &le.namespace {
            Some(ns) => ns.clone(),
            None => leader_election::in_cluster_namespace()?,
        };
        let mut rest = self.inner.rest.clone().with_user_agent_suffix("leader-election");
        rest.kube.read_timeout = Some((le.renew_deadline / 2).max(Duration::from_secs(1)));
        let identity = leader_election::identity();
        let recorder = ApiRecorder::new(&identity, self.inner.events.clone());
        Elector::new(
            rest.into_kube_client()?,
            &namespace,
            &le.id,
            identity,
            le.lease_duration,
            le.renew_deadline,
            le.retry_period,
            Some(recorder),
        )
    }

    async fn bind_metrics(&self) -> Result<Option<MetricsServer>> {
        let address = &self.inner.options.metrics_bind_address;
        if address == "0" || address.is_empty() {
            return Ok(None);
        }
        let address = if address.starts_with(':') {
            format!("0.0.0.0{address}")
        } else {
            address.clone()
        };
        let listener = tokio::net::TcpListener::bind(&address)
            .await
            .map_err(|e| Error::foreign(e).wrap("failed to start metrics server: failed to create listener"))?;
        let _ = self
            .inner
            .metrics_address
            .set(listener.local_addr().map_err(Error::foreign)?);
        Ok(Some(MetricsServer { listener }))
    }
}

async fn start_runnable(
    runnable: Box<dyn Runnable>,
    mut leading: watch::Receiver<bool>,
    stop: CancellationToken,
) -> Result<()> {
    if runnable.needs_leader_election() {
        tokio::select! {
            () = stop.cancelled() => return Ok(()),
            led = leading.wait_for(|l| *l) => if led.is_err() {
                // The elector ended without the lease.
                return Ok(());
            },
        }
    }
    runnable.start(stop).await
}

/// Go's `time.Duration.String()` for the whole-second values the manager prints.
fn go_duration(d: Duration) -> String {
    let secs = d.as_secs();
    match (secs / 3600, secs % 3600 / 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m{s}s"),
        (h, m, s) => format!("{h}h{m}m{s}s"),
    }
}

/// controller-runtime's metrics server: Prometheus text on `/metrics`, 404 elsewhere
/// (`pkg/metrics/server/server.go`).
struct MetricsServer {
    listener: tokio::net::TcpListener,
}

impl MetricsServer {
    async fn serve(self, registry: prometheus::Registry, stop: CancellationToken) -> Result<()> {
        loop {
            let (stream, _) = tokio::select! {
                () = stop.cancelled() => return Ok(()),
                accepted = self.listener.accept() => match accepted {
                    Ok(accepted) => accepted,
                    Err(err) => {
                        tracing::warn!(error = %err, "metrics server accept failed");
                        continue;
                    }
                },
            };
            let registry = registry.clone();
            let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                let response = metrics_response(&registry, req.method(), req.uri().path());
                async move { Ok::<_, std::convert::Infallible>(response) }
            });
            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                if let Err(err) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    tracing::debug!(error = %err, "metrics connection ended");
                }
            });
        }
    }
}

fn metrics_response(
    registry: &prometheus::Registry,
    method: &http::Method,
    path: &str,
) -> hyper::Response<http_body_util::Full<hyper::body::Bytes>> {
    use prometheus::Encoder;
    let text = |status: u16, body: Vec<u8>, content_type: &str| {
        hyper::Response::builder()
            .status(status)
            .header(http::header::CONTENT_TYPE, content_type)
            .body(http_body_util::Full::new(hyper::body::Bytes::from(body)))
            .expect("a valid response")
    };
    if path != "/metrics" || (method != http::Method::GET && method != http::Method::HEAD) {
        return text(404, b"404 page not found\n".to_vec(), "text/plain; charset=utf-8");
    }
    let encoder = prometheus::TextEncoder::new();
    let mut body = Vec::new();
    match encoder.encode(&registry.gather(), &mut body) {
        Ok(()) => text(200, body, encoder.format_type()),
        Err(err) => text(500, err.to_string().into_bytes(), "text/plain; charset=utf-8"),
    }
}

/// Go `ctrl.SetupSignalHandler()`: resolves on the first SIGINT or SIGTERM. A second one exits the
/// process with status 1 (`pkg/manager/signals/signal.go:27-45`).
pub async fn setup_signal_handler() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
        tracing::error!("cannot install signal handlers");
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
    tokio::spawn(async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        std::process::exit(1);
    });
}

#[cfg(test)]
#[path = "manager_test.rs"]
mod tests;

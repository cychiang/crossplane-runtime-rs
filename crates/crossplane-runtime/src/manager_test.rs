use std::convert::Infallible;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tower::{Layer, Service};

use super::*;

#[test]
fn user_agent_has_client_go_shape() {
    let cases = [
        (
            "the baseline's shape",
            (
                "crossplane-provider-baseline-provider",
                "v0.0.0",
                "linux",
                "aarch64",
                "",
            ),
            "crossplane-provider-baseline-provider/v0.0.0 (linux/arm64) kubernetes/unknown",
        ),
        (
            "adjustVersion drops the pre-release",
            ("p", "v1.2.3-rc.1+dirty", "linux", "x86_64", ""),
            "p/v1.2.3 (linux/amd64) kubernetes/unknown",
        ),
        (
            "adjustCommit keeps seven characters",
            ("p", "v1", "macos", "aarch64", "0123456789abcdef"),
            "p/v1 (darwin/arm64) kubernetes/0123456",
        ),
        (
            "empty parts are unknown",
            ("", "", "linux", "x86_64", ""),
            "unknown/unknown (linux/amd64) kubernetes/unknown",
        ),
    ];
    for (name, (command, version, os, arch, commit), want) in cases {
        assert_eq!(user_agent(command, version, os, arch, commit), want, "{name}");
    }
}

#[derive(Clone)]
struct Echo;

impl Service<http::Request<()>> for Echo {
    type Response = ();
    type Error = Infallible;
    type Future = std::future::Ready<std::result::Result<(), Infallible>>;
    fn poll_ready(&mut self, _: &mut std::task::Context<'_>) -> std::task::Poll<std::result::Result<(), Infallible>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn call(&mut self, _: http::Request<()>) -> Self::Future {
        std::future::ready(Ok(()))
    }
}

fn req(uri: &str) -> http::Request<()> {
    http::Request::get(uri).body(()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn throttle_waits_after_the_burst_and_never_for_watches() {
    let mut svc = Throttle::new(2.0, 3).layer(Echo);
    let start = tokio::time::Instant::now();
    for _ in 0..3 {
        svc.call(req("/api/v1/pods")).await.unwrap();
    }
    assert_eq!(start.elapsed(), Duration::ZERO, "the burst passes at once");
    for _ in 0..5 {
        svc.call(req("/api/v1/pods?watch=true&resourceVersion=1"))
            .await
            .unwrap();
    }
    assert_eq!(start.elapsed(), Duration::ZERO, "watches take no token");
    svc.call(req("/api/v1/pods")).await.unwrap();
    svc.call(req("/api/v1/pods")).await.unwrap();
    assert_eq!(
        start.elapsed(),
        Duration::from_secs(1),
        "two more at 2 QPS take one second"
    );
}

fn offline(options: ManagerOptions) -> Manager {
    let config = kube::Config::new("http://127.0.0.1:9".parse().unwrap());
    Manager::new(RestConfig::from_config(config, "xprs-test", "v0.0.0"), options).unwrap()
}

fn quiet_options() -> ManagerOptions {
    ManagerOptions {
        metrics_bind_address: "0".into(),
        graceful_shutdown_timeout: Duration::from_millis(200),
        ..Default::default()
    }
}

/// Reports when it starts, then runs until shutdown, or longer when `linger` is set.
struct Probe {
    started: mpsc::UnboundedSender<&'static str>,
    name: &'static str,
    linger: Option<Duration>,
    fail: bool,
}

impl Runnable for Probe {
    fn start(self: Box<Self>, shutdown: CancellationToken) -> BoxFuture<'static, Result<()>> {
        Box::pin(async move {
            self.started.send(self.name).unwrap();
            if self.fail {
                return Err(Error::new("probe failed"));
            }
            shutdown.cancelled().await;
            if let Some(d) = self.linger {
                tokio::time::sleep(d).await;
            }
            Ok(())
        })
    }
}

fn probe(started: &mpsc::UnboundedSender<&'static str>, name: &'static str) -> Probe {
    Probe {
        started: started.clone(),
        name,
        linger: None,
        fail: false,
    }
}

#[tokio::test]
async fn runnables_added_before_and_after_start_both_run() {
    let mgr = offline(quiet_options());
    let (tx, mut started) = mpsc::unbounded_channel();
    mgr.add(probe(&tx, "before")).unwrap();
    let stop = CancellationToken::new();
    let running = tokio::spawn({
        let (mgr, stop) = (mgr.clone(), stop.clone());
        async move { mgr.start(stop.cancelled_owned()).await }
    });
    assert_eq!(started.recv().await, Some("before"));
    mgr.add(probe(&tx, "after")).unwrap();
    assert_eq!(started.recv().await, Some("after"), "a late add starts at once");
    stop.cancel();
    running.await.unwrap().expect("clean stop");
    assert!(
        mgr.add(probe(&tx, "late")).is_err(),
        "the manager refuses runnables once it has stopped"
    );
    let again = mgr.start(std::future::ready(())).await.unwrap_err();
    assert_eq!(again.to_string(), "manager already started");
}

#[tokio::test]
async fn a_failing_runnable_stops_the_manager_with_its_error() {
    let mgr = offline(quiet_options());
    let (tx, mut started) = mpsc::unbounded_channel();
    mgr.add(probe(&tx, "steady")).unwrap();
    mgr.add(Probe {
        fail: true,
        ..probe(&tx, "failing")
    })
    .unwrap();
    let err = mgr.start(std::future::pending()).await.unwrap_err();
    assert_eq!(err.to_string(), "probe failed");
    let mut names = vec![started.recv().await.unwrap(), started.recv().await.unwrap()];
    names.sort();
    assert_eq!(names, ["failing", "steady"]);
}

#[tokio::test(start_paused = true)]
async fn shutdown_waits_for_the_grace_period_and_no_longer() {
    let mgr = offline(ManagerOptions {
        graceful_shutdown_timeout: Duration::from_secs(30),
        ..quiet_options()
    });
    let (tx, mut started) = mpsc::unbounded_channel();
    mgr.add(Probe {
        linger: Some(Duration::from_secs(3600)),
        ..probe(&tx, "stuck")
    })
    .unwrap();
    let stop = CancellationToken::new();
    let begun = tokio::time::Instant::now();
    let running = tokio::spawn({
        let (mgr, stop) = (mgr.clone(), stop.clone());
        async move { mgr.start(stop.cancelled_owned()).await }
    });
    started.recv().await.unwrap();
    stop.cancel();
    let err = running.await.unwrap().unwrap_err();
    assert_eq!(
        err.to_string(),
        "failed waiting for all runnables to end within grace period of 30s: context deadline exceeded"
    );
    assert_eq!(begun.elapsed(), Duration::from_secs(30));
}

/// Leader-gated runnables wait for the lease; others start at once. With no elector configured
/// every runnable starts.
struct Gated(mpsc::UnboundedSender<&'static str>);

impl Runnable for Gated {
    fn start(self: Box<Self>, _: CancellationToken) -> BoxFuture<'static, Result<()>> {
        self.0.send("gated").unwrap();
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn leader_gated_runnables_wait_for_the_lease() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (_lead, leading) = watch::channel(false);
    let stop = CancellationToken::new();
    let gated = tokio::spawn(start_runnable(Box::new(Gated(tx)), leading, stop.clone()));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(rx.try_recv().is_err(), "not started without the lease");
    stop.cancel();
    gated.await.unwrap().unwrap();
    assert!(rx.try_recv().is_err(), "shut down before ever leading");

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (lead, leading) = watch::channel(false);
    drop(lead);
    start_runnable(Box::new(Gated(tx)), leading, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        rx.try_recv().is_err(),
        "an elector that ends without the lease opens nothing"
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (lead, leading) = watch::channel(false);
    let gated = tokio::spawn(start_runnable(Box::new(Gated(tx)), leading, CancellationToken::new()));
    lead.send_replace(true);
    gated.await.unwrap().unwrap();
    assert_eq!(rx.try_recv().unwrap(), "gated");
}

#[test]
fn leader_election_defaults_are_controller_runtimes() {
    let le = LeaderElection::new("id");
    assert_eq!(
        (le.lease_duration, le.renew_deadline, le.retry_period),
        (Duration::from_secs(15), Duration::from_secs(10), Duration::from_secs(2))
    );
}

#[test]
fn metrics_are_served_on_slash_metrics_only() {
    let registry = prometheus::Registry::new();
    let gauge = prometheus::Gauge::new("xprs_test_gauge", "A test gauge").unwrap();
    gauge.set(3.0);
    registry.register(Box::new(gauge)).unwrap();
    let ok = metrics_response(&registry, &http::Method::GET, "/metrics");
    assert_eq!(ok.status(), 200);
    let other = metrics_response(&registry, &http::Method::GET, "/");
    assert_eq!(other.status(), 404);
    let post = metrics_response(&registry, &http::Method::POST, "/metrics");
    assert_eq!(post.status(), 404);
}

#[test]
fn the_leader_election_client_appends_to_the_user_agent() {
    let config = kube::Config::new("http://127.0.0.1:9".parse().unwrap());
    let rest = RestConfig::from_config(config, "xprs-test", "v1.2.3");
    let base = rest.user_agent().to_owned();
    assert_eq!(
        rest.with_user_agent_suffix("leader-election").user_agent(),
        format!("{base}/leader-election"),
        "client-go rest.AddUserAgent"
    );
}

const KUBECONFIG: &str = "
apiVersion: v1
kind: Config
current-context: other
clusters:
- name: one
  cluster: {server: 'https://one.invalid:6443'}
- name: two
  cluster: {server: 'https://two.invalid:6443'}
users:
- name: u
  user: {token: t}
contexts:
- name: one
  context: {cluster: one, user: u}
- name: other
  context: {cluster: two, user: u}
";

fn write_kubeconfig(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("xprs-{name}-{}.yaml", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[tokio::test]
async fn from_kubeconfig_reads_the_current_context_of_the_named_file() {
    let path = write_kubeconfig("from-kubeconfig", KUBECONFIG);
    let rest = RestConfig::from_kubeconfig(&path, "xprs-test", "v1.2.3").await;
    std::fs::remove_file(&path).unwrap();
    let rest = rest.unwrap();
    assert_eq!(rest.kube.cluster_url.to_string(), "https://two.invalid:6443/");
    assert!(
        rest.user_agent().contains("v1.2.3"),
        "the User-Agent carries the version: {}",
        rest.user_agent()
    );
}

#[tokio::test]
async fn from_kubeconfig_reports_a_missing_or_unusable_file() {
    let missing = std::env::temp_dir().join(format!("xprs-no-such-kubeconfig-{}.yaml", std::process::id()));
    assert!(RestConfig::from_kubeconfig(&missing, "xprs-test", "v0").await.is_err());

    let unusable = write_kubeconfig("unusable-kubeconfig", "apiVersion: v1\nkind: Config\n");
    let rest = RestConfig::from_kubeconfig(&unusable, "xprs-test", "v0").await;
    std::fs::remove_file(&unusable).unwrap();
    assert!(rest.is_err(), "a kubeconfig with no context names no cluster");
}

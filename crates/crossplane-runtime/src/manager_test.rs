use std::convert::Infallible;
use std::time::Duration;

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

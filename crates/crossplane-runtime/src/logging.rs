//! Go threads a `logging.Logger` through options. Here the global `tracing` subscriber does the
//! job and each controller attaches its name as a span field, so no option carries a logger.

use std::fmt;

use k8s_openapi::jiff::Timestamp;
use tracing_subscriber::EnvFilter;

use crate::reconciler::Request;

/// Go logs `request` as a JSON object, `{"name":"...","namespace":"..."}`.
pub(crate) struct RequestFields<'a>(pub &'a Request);

impl fmt::Display for RequestFields<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = serde_json::json!({
            "name": self.0.name,
            "namespace": self.0.namespace.as_deref().unwrap_or_default(),
        });
        write!(f, "{json}")
    }
}

/// Go logs a `time.Time` or `metav1.Time` field as RFC 3339 to the second, in UTC.
pub(crate) fn rfc3339(ts: Timestamp) -> String {
    Timestamp::from_second(ts.as_second()).unwrap_or(ts).to_string()
}

/// `--debug` selects the pretty format at `debug`; otherwise JSON at `info`. `RUST_LOG` overrides
/// the level. Panics when a global subscriber is already set.
pub fn init(debug: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter(debug)));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if debug {
        builder.pretty().init();
    } else {
        builder.json().init();
    }
}

/// Go's `--debug` turns the provider's zap logger to debug, but client-go and the HTTP transport
/// log through klog at its default verbosity, so no request is traced. The transport crates stay
/// at `info` to match: at `debug` they log every API request.
fn default_filter(debug: bool) -> &'static str {
    if debug {
        "debug,hyper=info,hyper_util=info,h2=info,tower=info,tower_http=info,kube_client=info,rustls=info"
    } else {
        "info"
    }
}

/// Runs `fut` under a debug-level subscriber and returns what it logged. The subscriber is
/// thread-local, so the test must run on a current-thread runtime, which `#[tokio::test]` gives.
#[cfg(test)]
pub(crate) async fn capture<F: std::future::Future>(fut: F) -> (F::Output, String) {
    use std::io;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let sink = Sink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let out = fut.await;
    let logged = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    (out, logged)
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use tracing::Level;

    use super::*;

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn logged(debug: bool) -> String {
        let buf = Buf::default();
        let out = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new(default_filter(debug)))
            .with_writer(move || out.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::event!(target: "crossplane_runtime::reconciler", Level::DEBUG, "reconciling");
            tracing::event!(target: "hyper_util::client::legacy::pool", Level::DEBUG, "pooling idle connection");
            tracing::event!(target: "kube_client::client::builder", Level::DEBUG, "requesting");
            tracing::event!(target: "hyper_util::client::legacy::client", Level::INFO, "transport notice");
        });
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn debug_logs_the_provider_at_debug_but_not_each_http_request() {
        let out = logged(true);
        assert!(out.contains("reconciling"), "{out}");
        assert!(out.contains("transport notice"), "{out}");
        assert!(!out.contains("pooling idle connection"), "{out}");
        assert!(!out.contains("requesting"), "{out}");
    }

    #[test]
    fn without_debug_nothing_below_info_is_logged() {
        let out = logged(false);
        assert!(!out.contains("reconciling"), "{out}");
        assert!(out.contains("transport notice"), "{out}");
    }
}

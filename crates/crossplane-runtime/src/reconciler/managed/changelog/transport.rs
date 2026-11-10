//! The change log sidecar's transport: gRPC over a Unix socket.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::errors::{Error, Result};
use futures::future::BoxFuture;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};

use super::ChangeLogServiceClient;
use super::proto::{SEND_CHANGE_LOG_PATH, SendChangeLogRequest, SendChangeLogResponse};

/// How long to wait before dialing the socket again.
const REDIAL_INTERVAL: Duration = Duration::from_millis(100);

/// Go's `v1alpha1.NewChangeLogServiceClient` over `grpc.NewClient("unix://" + path)`. The socket
/// is dialed on the first send, not here.
#[derive(Clone)]
pub struct UnixChangeLogClient {
    channel: Channel,
}

impl UnixChangeLogClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        let path: Arc<PathBuf> = Arc::new(socket_path.into());
        // The connector ignores the URI; tonic only needs one that parses.
        let channel = Endpoint::from_static("http://changelogs.sock").connect_with_connector_lazy(tower::service_fn(
            move |_: Uri| {
                let path = path.clone();
                async move { UnixStream::connect(&*path).await.map(TokioIo::new) }
            },
        ));
        Self { channel }
    }
}

impl ChangeLogServiceClient for UnixChangeLogClient {
    /// Go sends with `grpc.WaitForReady(true)`: while the socket cannot be dialed the call waits
    /// instead of failing, until the caller's send timeout cancels it.
    fn send_change_log(&self, request: SendChangeLogRequest) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            loop {
                let mut grpc = tonic::client::Grpc::new(self.channel.clone());
                let sent = match grpc.ready().await {
                    Ok(()) => {
                        let path = http::uri::PathAndQuery::from_static(SEND_CHANGE_LOG_PATH);
                        let codec = tonic_prost::ProstCodec::<SendChangeLogRequest, SendChangeLogResponse>::default();
                        grpc.unary(tonic::Request::new(request.clone()), path, codec).await
                    }
                    Err(e) => Err(tonic::Status::from_error(e.into())),
                };
                match sent {
                    Ok(_) => return Ok(()),
                    Err(status) if not_ready(&status) => tokio::time::sleep(REDIAL_INTERVAL).await,
                    Err(status) => return Err(rpc_error(&status)),
                }
            }
        })
    }
}

/// A status tonic made from a failure to connect, as opposed to one the server sent.
fn not_ready(status: &tonic::Status) -> bool {
    std::error::Error::source(status).is_some_and(|e| e.is::<tonic::transport::Error>())
}

/// Go's `status.Error` text.
fn rpc_error(status: &tonic::Status) -> Error {
    Error::new(format!(
        "rpc error: code = {:?} desc = {}",
        status.code(),
        status.message()
    ))
}

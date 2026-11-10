//! Change logs (`changelogger.go`). The reconciler builds a domain [`ChangeLogEntry`]; the gRPC
//! logger converts it to the proto at its own boundary, so the reconciler never sees protobuf
//! types.

use std::collections::BTreeMap;
use std::time::Duration;

use crossplane_core::errors::{Error, Result, ResultExt};
use crossplane_core::meta;
use crossplane_core::resource::{Managed, Object};
use futures::future::BoxFuture;
use k8s_openapi::jiff::Timestamp;

pub mod proto;
mod transport;

pub use transport::UnixChangeLogClient;

/// `defaultSendTimeout`, `changelogger.go:33`.
const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(10);

const ERR_SNAPSHOT: &str = "cannot snapshot managed resource";
const ERR_SEND: &str = "cannot send change log entry";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationType {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeLogEntry {
    pub timestamp: Timestamp,
    pub api_version: String,
    pub kind: String,
    pub name: String,
    /// The external name of the pre-operation copy.
    pub external_name: String,
    pub operation: OperationType,
    /// The pre-operation object, as JSON (Go: a `structpb.Struct`).
    pub snapshot: serde_json::Map<String, serde_json::Value>,
    pub error_message: Option<String>,
    pub additional_details: BTreeMap<String, String>,
}

impl ChangeLogEntry {
    /// The entry Go's `GRPCChangeLogger.Log` builds from its arguments
    /// (`changelogger.go:85-109`). `managed` is the pre-operation copy.
    pub fn new<M: Managed>(
        managed: &M,
        operation: OperationType,
        change_error: Option<&Error>,
        additional_details: BTreeMap<String, String>,
        timestamp: Timestamp,
    ) -> Result<Self> {
        let serde_json::Value::Object(snapshot) = managed.to_json().wrap_err(ERR_SNAPSHOT)? else {
            return Err(Error::new("managed resource is not a JSON object").wrap(ERR_SNAPSHOT));
        };
        Ok(Self {
            timestamp,
            api_version: M::api_version(&()).into_owned(),
            kind: M::kind(&()).into_owned(),
            name: meta::name(managed).to_owned(),
            external_name: meta::external_name(managed).to_owned(),
            operation,
            snapshot,
            error_message: change_error.map(Error::to_string),
            additional_details,
        })
    }
}

/// Go `ChangeLogger`. A failure is logged at info and never changes the reconcile outcome.
pub trait ChangeLogger: Send + Sync + 'static {
    fn log(&self, entry: ChangeLogEntry) -> BoxFuture<'_, Result<()>>;
}

/// The default when change logs are off.
pub struct NopChangeLogger;

impl ChangeLogger for NopChangeLogger {
    fn log(&self, _: ChangeLogEntry) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// Go `v1alpha1.ChangeLogServiceClient`: the one RPC, so tests can stand in for the sidecar.
pub trait ChangeLogServiceClient: Send + Sync + 'static {
    fn send_change_log(&self, request: proto::SendChangeLogRequest) -> BoxFuture<'_, Result<()>>;
}

/// Go `GRPCChangeLogger`.
pub struct GrpcChangeLogger {
    client: Box<dyn ChangeLogServiceClient>,
    provider_version: String,
    send_timeout: Duration,
}

impl GrpcChangeLogger {
    pub fn new(client: impl ChangeLogServiceClient) -> Self {
        Self {
            client: Box::new(client),
            provider_version: String::new(),
            send_timeout: DEFAULT_SEND_TIMEOUT,
        }
    }

    /// What a provider's `main` does in Go: a client on the sidecar's Unix socket, dialed on the
    /// first send, and the provider version.
    pub fn connect_unix(socket_path: &str, provider_version: String) -> Self {
        Self::new(UnixChangeLogClient::new(socket_path)).provider_version(provider_version)
    }

    /// Go `WithProviderVersion`.
    pub fn provider_version(mut self, version: impl Into<String>) -> Self {
        self.provider_version = version.into();
        self
    }

    /// Go `WithSendTimeout`: bounds one send, including the wait for the sidecar to be reachable.
    pub fn send_timeout(mut self, timeout: Duration) -> Self {
        self.send_timeout = timeout;
        self
    }
}

impl ChangeLogger for GrpcChangeLogger {
    fn log(&self, entry: ChangeLogEntry) -> BoxFuture<'_, Result<()>> {
        let request = proto::SendChangeLogRequest {
            entry: Some(proto::ChangeLogEntry::from_entry(entry, self.provider_version.clone())),
        };
        Box::pin(async move {
            tokio::time::timeout(self.send_timeout, self.client.send_change_log(request))
                .await
                .unwrap_or_else(|_| Err(Error::deadline_exceeded()))
                .wrap_err(ERR_SEND)
        })
    }
}

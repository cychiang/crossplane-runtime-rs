//! Change logs (`changelogger.go`). The reconciler builds a domain [`ChangeLogEntry`]; the gRPC
//! logger converts it to the proto at its own boundary, so tables assert entries without
//! protobuf types.

use std::collections::BTreeMap;

use crossplane_core::errors::Result;
use futures::future::BoxFuture;
use k8s_openapi::jiff::Timestamp;

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
    pub snapshot: serde_json::Value,
    pub error_message: Option<String>,
    pub additional_details: BTreeMap<String, String>,
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

/// Go `GRPCChangeLogger` over a Unix socket. The tonic client is generated in unit 1.6, behind a
/// cargo feature, so a provider that never enables change logs does not need protoc.
pub struct GrpcChangeLogger {
    socket_path: String,
    provider_version: String,
}

impl GrpcChangeLogger {
    pub fn connect_unix(socket_path: &str, provider_version: String) -> Self {
        Self {
            socket_path: socket_path.to_owned(),
            provider_version,
        }
    }
}

impl ChangeLogger for GrpcChangeLogger {
    fn log(&self, entry: ChangeLogEntry) -> BoxFuture<'_, Result<()>> {
        let _ = (&self.socket_path, &self.provider_version, entry);
        Box::pin(async { todo!("SendChangeLog with a 10s timeout") })
    }
}

//! The wire messages of `apis/changelogs/proto/v1alpha1/changelog.proto`, written by hand against
//! prost so that building needs no protoc. `changelogger_test.rs` pins the encoding to bytes that
//! protoc produced from the Go repository's `.proto`.

use std::collections::BTreeMap;

use k8s_openapi::jiff::Timestamp;
use prost_types::value::Kind;
use serde_json::Value;

/// The gRPC path of `ChangeLogService.SendChangeLog`.
pub const SEND_CHANGE_LOG_PATH: &str = "/changelogs.proto.v1alpha1.ChangeLogService/SendChangeLog";

#[derive(Clone, PartialEq, prost::Message)]
pub struct SendChangeLogRequest {
    #[prost(message, optional, tag = "1")]
    pub entry: Option<ChangeLogEntry>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ChangeLogEntry {
    #[prost(message, optional, tag = "1")]
    pub timestamp: Option<prost_types::Timestamp>,
    #[prost(string, tag = "2")]
    pub provider: String,
    #[prost(string, tag = "3")]
    pub api_version: String,
    #[prost(string, tag = "4")]
    pub kind: String,
    #[prost(string, tag = "5")]
    pub name: String,
    #[prost(string, tag = "6")]
    pub external_name: String,
    #[prost(enumeration = "OperationType", tag = "7")]
    pub operation: i32,
    #[prost(message, optional, tag = "8")]
    pub snapshot: Option<prost_types::Struct>,
    #[prost(string, optional, tag = "9")]
    pub error_message: Option<String>,
    #[prost(btree_map = "string, string", tag = "10")]
    pub additional_details: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum OperationType {
    Unspecified = 0,
    Create = 1,
    Update = 2,
    Delete = 3,
}

#[derive(Clone, Copy, PartialEq, prost::Message)]
pub struct SendChangeLogResponse {}

impl From<super::OperationType> for OperationType {
    fn from(op: super::OperationType) -> Self {
        match op {
            super::OperationType::Create => Self::Create,
            super::OperationType::Update => Self::Update,
            super::OperationType::Delete => Self::Delete,
        }
    }
}

impl ChangeLogEntry {
    /// `provider` is the one field the logger adds; the rest is the reconciler's entry.
    pub fn from_entry(entry: super::ChangeLogEntry, provider: String) -> Self {
        Self {
            timestamp: Some(timestamp(entry.timestamp)),
            provider,
            api_version: entry.api_version,
            kind: entry.kind,
            name: entry.name,
            external_name: entry.external_name,
            operation: OperationType::from(entry.operation).into(),
            snapshot: Some(to_struct(entry.snapshot)),
            error_message: entry.error_message,
            additional_details: entry.additional_details,
        }
    }
}

fn timestamp(t: Timestamp) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: t.as_second(),
        nanos: t.subsec_nanosecond(),
    }
}

/// Go `structpb.NewStruct` over the object's JSON: every number becomes a double.
pub fn to_struct(object: serde_json::Map<String, Value>) -> prost_types::Struct {
    prost_types::Struct {
        fields: object.into_iter().map(|(k, v)| (k, to_value(v))).collect(),
    }
}

fn to_value(v: Value) -> prost_types::Value {
    let kind = match v {
        Value::Null => Kind::NullValue(0),
        Value::Bool(b) => Kind::BoolValue(b),
        Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Kind::StringValue(s),
        Value::Array(items) => Kind::ListValue(prost_types::ListValue {
            values: items.into_iter().map(to_value).collect(),
        }),
        Value::Object(fields) => Kind::StructValue(to_struct(fields)),
    };
    prost_types::Value { kind: Some(kind) }
}

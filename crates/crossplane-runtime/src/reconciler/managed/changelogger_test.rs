//! Port of `changelogger_test.go` and of `TestReconcilerChangeLogs` in
//! `reconciler_modern_test.go`, then tests of what Go leaves to generated code and grpc-go: the
//! wire encoding and the Unix socket transport.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::errors::{Error, Result};
use crossplane_core::meta;
use crossplane_test::{self as test, MockClient};
use futures::future::BoxFuture;
use k8s_openapi::jiff::Timestamp;
use kube::Resource;
use prost::Message;
use serde_json::json;

use super::changelog::proto::{self, SendChangeLogRequest, SendChangeLogResponse};
use super::changelog::{ChangeLogServiceClient, UnixChangeLogClient};
use super::reconciler_modern_test::{Mg, boom, connector, get_managed, get_with, observes, set_deleted};
use super::*;
use crate::reconciler::{Reconcile, Request};

const PROVIDER: &str = "provider-cool:v9.99.999";

type Requests = Arc<Mutex<Vec<SendChangeLogRequest>>>;

/// Go's `changeLogServiceClient`: records every request, then answers with `err`.
struct RecordingClient {
    requests: Requests,
    err: Option<Error>,
}

impl ChangeLogServiceClient for RecordingClient {
    fn send_change_log(&self, request: SendChangeLogRequest) -> BoxFuture<'_, Result<()>> {
        self.requests.lock().unwrap().push(request);
        let result = self.err.clone().map_or(Ok(()), Err);
        Box::pin(async move { result })
    }
}

fn object(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().expect("a JSON object").clone()
}

fn seconds(t: Timestamp) -> Option<prost_types::Timestamp> {
    Some(prost_types::Timestamp {
        seconds: t.as_second(),
        nanos: t.subsec_nanosecond(),
    })
}

#[tokio::test]
async fn change_logger() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        mr: Mg,
        ad: BTreeMap<String, String>,
        err: Option<Error>,
        send_err: Option<Error>,
        want_requests: Vec<SendChangeLogRequest>,
        want: Result<()>,
    }

    let now = test::now();
    let api_version = Mg::api_version(&()).into_owned();
    let kind = Mg::kind(&()).into_owned();
    let cool = || {
        let mut mg = Mg::default();
        mg.metadata.name = Some("cool-managed".to_owned());
        meta::set_external_name(&mut mg, "cool-managed");
        mg
    };
    let details: BTreeMap<String, String> = [("key", "value"), ("key2", "value2")]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into();

    let cases = [
        // go: pkg/reconciler/managed TestChangeLogger/ChangeLogsSuccess
        Case {
            name: "ChangeLogsSuccess",
            reason: "Change log entry should be recorded successfully.",
            mr: cool(),
            ad: details.clone(),
            err: Some(boom()),
            send_err: None,
            want_requests: vec![SendChangeLogRequest {
                entry: Some(proto::ChangeLogEntry {
                    timestamp: seconds(now),
                    provider: PROVIDER.to_owned(),
                    api_version: api_version.clone(),
                    kind: kind.clone(),
                    name: "cool-managed".to_owned(),
                    external_name: "cool-managed".to_owned(),
                    operation: proto::OperationType::Create.into(),
                    snapshot: Some(proto::to_struct(object(json!({
                        "apiVersion": api_version,
                        "kind": kind,
                        "metadata": {
                            "name": "cool-managed",
                            "annotations": {"crossplane.io/external-name": "cool-managed"},
                        },
                        "spec": {},
                        "status": {},
                    })))),
                    error_message: Some("boom".to_owned()),
                    additional_details: details.clone(),
                }),
            }],
            want: Ok(()),
        },
        // go: pkg/reconciler/managed TestChangeLogger/SendChangeLogsFailure
        Case {
            name: "SendChangeLogsFailure",
            reason: "Error from sending change log entry should be handled and recorded.",
            mr: Mg::default(),
            ad: BTreeMap::new(),
            err: None,
            send_err: Some(boom()),
            // The entry is still built and handed to the client.
            want_requests: vec![SendChangeLogRequest {
                entry: Some(proto::ChangeLogEntry {
                    timestamp: seconds(now),
                    provider: PROVIDER.to_owned(),
                    api_version: api_version.clone(),
                    kind: kind.clone(),
                    operation: proto::OperationType::Create.into(),
                    snapshot: Some(proto::to_struct(object(json!({
                        "apiVersion": api_version,
                        "kind": kind,
                        "metadata": {},
                        "spec": {},
                        "status": {},
                    })))),
                    ..Default::default()
                }),
            }],
            want: Err(boom().wrap("cannot send change log entry")),
        },
    ];

    for tc in cases {
        let requests = Requests::default();
        let change = GrpcChangeLogger::new(RecordingClient {
            requests: requests.clone(),
            err: tc.send_err,
        })
        .provider_version(PROVIDER);
        let entry = ChangeLogEntry::new(&tc.mr, OperationType::Create, tc.err.as_ref(), tc.ad, now).unwrap();
        let got = change.log(entry).await;

        assert_eq!(
            tc.want_requests,
            *requests.lock().unwrap(),
            "\n{}\nReason: {}\n-want requests, +got requests",
            tc.name,
            tc.reason
        );
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

#[tokio::test]
async fn reconciler_change_logs() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        deleted: bool,
        external: fn() -> ExternalClientFns<Mg>,
        want_op: proto::OperationType,
        want_err_message: &'static str,
    }

    fn absent() -> ExternalClientFns<Mg> {
        observes(ExternalObservation::default())
    }
    fn outdated() -> ExternalClientFns<Mg> {
        observes(ExternalObservation {
            resource_exists: true,
            ..Default::default()
        })
    }

    let cases = [
        // go: pkg/reconciler/managed TestReconcilerChangeLogs/CreateSuccessfulWithChangeLogs
        Case {
            name: "CreateSuccessfulWithChangeLogs",
            reason: "Successful managed resource creation should send a create change log entry when change logs are enabled.",
            deleted: false,
            external: || ExternalClientFns {
                create: Some(Box::new(|_| Ok(ExternalCreation::default()))),
                ..absent()
            },
            want_op: proto::OperationType::Create,
            want_err_message: "",
        },
        // go: pkg/reconciler/managed TestReconcilerChangeLogs/CreateFailureWithChangeLogs
        Case {
            name: "CreateFailureWithChangeLogs",
            reason: "Failed managed resource creation should send a create change log entry with the error when change logs are enabled.",
            deleted: false,
            external: || ExternalClientFns {
                create: Some(Box::new(|_| Err(boom()))),
                ..absent()
            },
            want_op: proto::OperationType::Create,
            want_err_message: "boom",
        },
        // go: pkg/reconciler/managed TestReconcilerChangeLogs/UpdateSuccessfulWithChangeLogs
        Case {
            name: "UpdateSuccessfulWithChangeLogs",
            reason: "Successful managed resource update should send an update change log entry when change logs are enabled.",
            deleted: false,
            external: || ExternalClientFns {
                update: Some(Box::new(|_| Ok(ExternalUpdate::default()))),
                ..outdated()
            },
            want_op: proto::OperationType::Update,
            want_err_message: "",
        },
        // go: pkg/reconciler/managed TestReconcilerChangeLogs/UpdateFailureWithChangeLogs
        Case {
            name: "UpdateFailureWithChangeLogs",
            reason: "Failed managed resource update should send an update change log entry with the error when change logs are enabled.",
            deleted: false,
            external: || ExternalClientFns {
                update: Some(Box::new(|_| Err(boom()))),
                ..outdated()
            },
            want_op: proto::OperationType::Update,
            want_err_message: "boom",
        },
        // go: pkg/reconciler/managed TestReconcilerChangeLogs/DeleteSuccessfulWithChangeLogs
        Case {
            name: "DeleteSuccessfulWithChangeLogs",
            reason: "Successful managed resource delete should send a delete change log entry when change logs are enabled.",
            deleted: true,
            external: || ExternalClientFns {
                delete: Some(Box::new(|_| Ok(ExternalDelete::default()))),
                ..outdated()
            },
            want_op: proto::OperationType::Delete,
            want_err_message: "",
        },
        // go: pkg/reconciler/managed TestReconcilerChangeLogs/DeleteFailureWithChangeLogs
        Case {
            name: "DeleteFailureWithChangeLogs",
            reason: "Failed managed resource delete should send a delete change log entry with the error when change logs are enabled.",
            deleted: true,
            external: || ExternalClientFns {
                delete: Some(Box::new(|_| Err(boom()))),
                ..outdated()
            },
            want_op: proto::OperationType::Delete,
            want_err_message: "boom",
        },
    ];

    let now = test::now();
    for tc in cases {
        let client = MockClient {
            get: Some(if tc.deleted {
                get_with(move |mg| set_deleted(mg, now))
            } else {
                get_managed()
            }),
            update: Some(test::new_mock_update_fn(None)),
            update_status: Some(test::new_mock_sub_resource_update_fn(None)),
            ..Default::default()
        };
        let requests = Requests::default();
        let logger = GrpcChangeLogger::new(RecordingClient {
            requests: requests.clone(),
            err: None,
        })
        .provider_version(PROVIDER);
        // Every other collaborator is the default, as in Go.
        let r = Reconciler::<Mg>::builder(Arc::new(client))
            .external_connector(connector(tc.external))
            .change_logger(Arc::new(logger))
            .build();
        let _ = r.reconcile(&Request::default()).await;

        let requests = requests.lock().unwrap();
        assert_eq!(1, requests.len(), "\n{}\nReason: {}\ncallCount", tc.name, tc.reason);
        let entry = requests[0].entry.clone().unwrap_or_default();
        assert_eq!(
            tc.want_op,
            entry.operation(),
            "\n{}\nReason: {}\nopType",
            tc.name,
            tc.reason
        );
        assert_eq!(
            tc.want_err_message,
            entry.error_message.unwrap_or_default(),
            "\n{}\nReason: {}\nerrMessage",
            tc.name,
            tc.reason
        );
    }
}

/// The bytes are `protoc --deterministic_output --encode=changelogs.proto.v1alpha1.SendChangeLogRequest`
/// over crossplane-runtime v2.4.3's `apis/changelogs/proto/v1alpha1/changelog.proto`, fed the
/// same entry in text format. They prove the hand-written messages match the Go wire format.
#[test]
fn request_encodes_as_protoc_does() {
    const PROTOC: &str = "0a82020a0808c8ff93ad061005121770726f76696465722d636f6f6c3a76392e39392e3939391a0e6578616d706c652e6f72672f763122055468696e672a0c636f6f6c2d6d616e61676564320d636f6f6c2d65787465726e616c38024282010a260a086d65746164617461121a2a180a160a046e616d65120e1a0c636f6f6c2d6d616e616765640a580a047370656312502a4e0a120a05636f756e7412091100000000000008400a0d0a07656e61626c6564120220010a0d0a076e6f7468696e67120208000a1a0a0474616773121232100a031a01610a0911000000000000f83f4a04626f6f6d520c0a036b6579120576616c7565520e0a046b657932120676616c756532";

    let entry = ChangeLogEntry {
        timestamp: Timestamp::new(1_705_312_200, 5).unwrap(),
        api_version: "example.org/v1".to_owned(),
        kind: "Thing".to_owned(),
        name: "cool-managed".to_owned(),
        external_name: "cool-external".to_owned(),
        operation: OperationType::Update,
        snapshot: object(json!({
            "metadata": {"name": "cool-managed"},
            "spec": {"count": 3, "enabled": true, "nothing": null, "tags": ["a", 1.5]},
        })),
        error_message: Some("boom".to_owned()),
        additional_details: [("key", "value"), ("key2", "value2")]
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .into(),
    };
    let request = SendChangeLogRequest {
        entry: Some(proto::ChangeLogEntry::from_entry(entry, PROVIDER.to_owned())),
    };
    let got: String = request.encode_to_vec().iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(PROTOC, got);
}

mod sidecar {
    //! A change log sidecar on a Unix socket, served by tonic, standing in for the real one.

    use std::convert::Infallible;
    use std::task::{Context, Poll};

    use tokio::net::UnixListener;
    use tokio_stream::wrappers::UnixListenerStream;
    use tonic::codegen::{Body, BoxFuture, Service, StdError, http};

    use super::*;

    #[derive(Clone, Default)]
    pub struct Sidecar {
        pub requests: Requests,
        /// The status every call is answered with, when set.
        pub fail: Option<(tonic::Code, &'static str)>,
    }

    impl Sidecar {
        pub fn serve(&self, socket: &std::path::Path) {
            let incoming = UnixListenerStream::new(UnixListener::bind(socket).expect("bind the socket"));
            let server = tonic::transport::Server::builder().add_service(self.clone());
            tokio::spawn(server.serve_with_incoming(incoming));
        }
    }

    impl tonic::server::NamedService for Sidecar {
        const NAME: &'static str = "changelogs.proto.v1alpha1.ChangeLogService";
    }

    impl tonic::server::UnaryService<SendChangeLogRequest> for Sidecar {
        type Response = SendChangeLogResponse;
        type Future = BoxFuture<tonic::Response<SendChangeLogResponse>, tonic::Status>;

        fn call(&mut self, request: tonic::Request<SendChangeLogRequest>) -> Self::Future {
            self.requests.lock().unwrap().push(request.into_inner());
            let result = match self.fail {
                Some((code, message)) => Err(tonic::Status::new(code, message)),
                None => Ok(tonic::Response::new(SendChangeLogResponse {})),
            };
            Box::pin(async move { result })
        }
    }

    impl<B> Service<http::Request<B>> for Sidecar
    where
        B: Body + Send + 'static,
        B::Error: Into<StdError> + Send + 'static,
    {
        type Response = http::Response<tonic::body::Body>;
        type Error = Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<std::result::Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, request: http::Request<B>) -> Self::Future {
            let sidecar = self.clone();
            Box::pin(async move {
                let codec = tonic_prost::ProstCodec::<SendChangeLogResponse, SendChangeLogRequest>::default();
                Ok(tonic::server::Grpc::new(codec).unary(sidecar, request).await)
            })
        }
    }
}

/// A socket path no other test uses. Unix socket paths are short, so this stays out of the
/// target directory.
fn socket_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("xpr-changelogs-{}-{n}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn sample_entry() -> ChangeLogEntry {
    let mut mg = Mg::default();
    mg.metadata.name = Some("cool-managed".to_owned());
    ChangeLogEntry::new(&mg, OperationType::Delete, None, BTreeMap::new(), test::now()).unwrap()
}

#[tokio::test]
async fn unix_client_sends_the_entry_to_the_sidecar() {
    let socket = socket_path();
    let sidecar = sidecar::Sidecar::default();
    sidecar.serve(&socket);

    let logger = GrpcChangeLogger::new(UnixChangeLogClient::new(&socket)).provider_version(PROVIDER);
    let entry = sample_entry();
    logger.log(entry.clone()).await.expect("the send succeeds");

    let want = SendChangeLogRequest {
        entry: Some(proto::ChangeLogEntry::from_entry(entry, PROVIDER.to_owned())),
    };
    assert_eq!(vec![want], *sidecar.requests.lock().unwrap());
    let _ = std::fs::remove_file(socket);
}

/// Go's `grpc.WaitForReady(true)`: a send made before the sidecar listens is delivered once it
/// does.
#[tokio::test]
async fn unix_client_waits_for_the_sidecar_to_listen() {
    let socket = socket_path();
    let logger = Arc::new(GrpcChangeLogger::new(UnixChangeLogClient::new(&socket)));
    let send = tokio::spawn({
        let logger = logger.clone();
        async move { logger.log(sample_entry()).await }
    });

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!send.is_finished(), "the send must wait while nothing listens");
    let sidecar = sidecar::Sidecar::default();
    sidecar.serve(&socket);

    send.await.unwrap().expect("the send succeeds once the sidecar listens");
    assert_eq!(1, sidecar.requests.lock().unwrap().len());
    let _ = std::fs::remove_file(socket);
}

#[tokio::test]
async fn send_gives_up_after_the_send_timeout() {
    let logger =
        GrpcChangeLogger::new(UnixChangeLogClient::new(socket_path())).send_timeout(Duration::from_millis(250));
    let got = logger.log(sample_entry()).await;
    let want = Err(Error::deadline_exceeded().wrap("cannot send change log entry"));
    test::assert_result("send_timeout", "Nothing listens on the socket.", &want, &got);
}

/// Waiting for ready covers only an unreachable sidecar. An error the sidecar sends is final.
#[tokio::test]
async fn an_error_from_the_sidecar_is_not_retried() {
    let socket = socket_path();
    let sidecar = sidecar::Sidecar {
        fail: Some((tonic::Code::Internal, "boom")),
        ..Default::default()
    };
    sidecar.serve(&socket);

    let got = GrpcChangeLogger::new(UnixChangeLogClient::new(&socket))
        .log(sample_entry())
        .await;
    let want = Err(Error::new("rpc error: code = Internal desc = boom").wrap("cannot send change log entry"));
    test::assert_result("sidecar_error", "The sidecar answers Internal.", &want, &got);
    assert_eq!(1, sidecar.requests.lock().unwrap().len());
    let _ = std::fs::remove_file(socket);
}

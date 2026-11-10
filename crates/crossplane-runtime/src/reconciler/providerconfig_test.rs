use std::sync::Mutex;

use crossplane_core::resource::{Object, ObjectKey, ProviderConfigUsage as _};
use crossplane_core::v2::TypedProviderConfigUsage;
use crossplane_test::fake::{ProviderConfig, ProviderConfigUsage};
use crossplane_test::{
    MockClient, assert_result, new_mock_delete_fn, new_mock_get_empty_fn, new_mock_get_err_fn, new_mock_get_fn,
    new_mock_list_err_fn, new_mock_list_fn, new_mock_sub_resource_update_fn, new_mock_update_fn,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use serde_json::{Value, json};

use super::*;
use crate::event::EventType;

fn boom() -> Error {
    Error::new("boom")
}

fn not_found() -> Error {
    Error::status(crossplane_core::errors::StatusReason::NotFound, "")
}

fn deleted(mut pc: ProviderConfig) -> ProviderConfig {
    pc.metadata.deletion_timestamp = Some(Time(Timestamp::now()));
    pc
}

fn controlled_usage() -> ProviderConfigUsage {
    ProviderConfigUsage::from_usage(
        ObjectMeta {
            owner_references: Some(vec![OwnerReference {
                uid: "so-unique".into(),
                controller: Some(true),
                ..Default::default()
            }]),
            ..Default::default()
        },
        TypedProviderConfigUsage::default(),
    )
}

fn pc_named(namespace: Option<&str>) -> ProviderConfig {
    let mut pc = ProviderConfig::default();
    pc.metadata.name = Some("my-pc".into());
    pc.metadata.namespace = namespace.map(str::to_owned);
    pc
}

fn reconciler(client: MockClient) -> Reconciler<ProviderConfig, ProviderConfigUsage> {
    Reconciler::builder(Arc::new(client)).build()
}

struct Case {
    name: &'static str,
    reason: &'static str,
    client: MockClient,
    want: Result<Outcome>,
}

#[tokio::test]
async fn reconcile_cases() {
    let cases = vec![
        // go: pkg/reconciler/providerconfig TestReconciler/GetProviderConfigError
        Case {
            name: "GetProviderConfigError",
            reason: "Errors getting a provider config should be returned",
            client: MockClient {
                get: Some(new_mock_get_err_fn(boom())),
                ..Default::default()
            },
            want: Err(boom().wrap(ERR_GET_PC)),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/ProviderConfigNotFound
        Case {
            name: "ProviderConfigNotFound",
            reason: "We should return without requeueing if the provider config no longer exists",
            client: MockClient {
                get: Some(new_mock_get_err_fn(not_found())),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/ListProviderConfigUsageError
        Case {
            name: "ListProviderConfigUsageError",
            reason: "We should requeue after a short wait if we encounter an error listing provider config usages",
            client: MockClient {
                get: Some(new_mock_get_empty_fn()),
                list: Some(new_mock_list_err_fn(boom())),
                ..Default::default()
            },
            want: Ok(Outcome::RequeueAfter(SHORT_WAIT)),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/DeleteProviderConfigUsageError
        Case {
            name: "DeleteProviderConfigUsageError",
            reason: "We should requeue after a short wait if we encounter an error deleting a provider config usage",
            client: MockClient {
                get: Some(new_mock_get_empty_fn()),
                list: Some(new_mock_list_fn(|| vec![ProviderConfigUsage::default()])),
                delete: Some(new_mock_delete_fn(Some(boom()))),
                ..Default::default()
            },
            want: Ok(Outcome::RequeueAfter(SHORT_WAIT)),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/BlockDeleteWhileInUse
        Case {
            name: "BlockDeleteWhileInUse",
            reason: "We should return without requeueing if the provider config is still in use",
            client: MockClient {
                get: Some(new_mock_get_fn(|| {
                    let mut pc = deleted(ProviderConfig::default());
                    pc.metadata.uid = Some("so-unique".into());
                    pc
                })),
                list: Some(new_mock_list_fn(|| vec![controlled_usage()])),
                update_status: Some(new_mock_sub_resource_update_fn(None)),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/RemoveFinalizerError
        Case {
            name: "RemoveFinalizerError",
            reason: "We should requeue after a short wait if we encounter an error while removing our finalizer",
            client: MockClient {
                get: Some(new_mock_get_fn(|| deleted(ProviderConfig::default()))),
                list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
                update: Some(new_mock_update_fn(Some(boom()))),
                ..Default::default()
            },
            want: Ok(Outcome::RequeueAfter(SHORT_WAIT)),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/SuccessfulDelete
        Case {
            name: "SuccessfulDelete",
            reason: "We should return without requeueing when we successfully remove our finalizer",
            client: MockClient {
                get: Some(new_mock_get_fn(|| deleted(ProviderConfig::default()))),
                list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
                update: Some(new_mock_update_fn(None)),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/AddFinalizerError
        Case {
            name: "AddFinalizerError",
            reason: "We should requeue after a short wait if we encounter an error while adding our finalizer",
            client: MockClient {
                get: Some(new_mock_get_empty_fn()),
                list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
                update: Some(new_mock_update_fn(Some(boom()))),
                ..Default::default()
            },
            want: Ok(Outcome::RequeueAfter(SHORT_WAIT)),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/UpdateStatusError
        Case {
            name: "UpdateStatusError",
            reason: "We return errors encountered while updating our status",
            client: MockClient {
                get: Some(new_mock_get_empty_fn()),
                list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
                update: Some(new_mock_update_fn(None)),
                update_status: Some(new_mock_sub_resource_update_fn(Some(boom()))),
                ..Default::default()
            },
            want: Err(boom().wrap(ERR_UPDATE_STATUS)),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/SuccessfulSetUsers
        Case {
            name: "SuccessfulSetUsers",
            reason: "We should return without requeuing if we successfully update our user count",
            client: MockClient {
                get: Some(new_mock_get_empty_fn()),
                list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
                update: Some(new_mock_update_fn(None)),
                update_status: Some(new_mock_sub_resource_update_fn(None)),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/ListUsagesScopedToNamespaceWhenNamespaced
        Case {
            name: "ListUsagesScopedToNamespaceWhenNamespaced",
            reason: "When ProviderConfig is namespaced, List should be called with InNamespace option",
            client: MockClient {
                get: Some(new_mock_get_fn(|| {
                    let mut pc = pc_named(Some("test-ns"));
                    pc.metadata.name = Some("my-pc".into());
                    pc
                })),
                list: Some(Box::new(|_, options| {
                    assert_eq!(
                        options.namespace.as_deref(),
                        Some("test-ns"),
                        "List called without InNamespace(\"test-ns\")"
                    );
                    Ok(vec![])
                })),
                update: Some(new_mock_update_fn(None)),
                update_status: Some(new_mock_sub_resource_update_fn(None)),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
        },
        // go: pkg/reconciler/providerconfig TestReconciler/ListUsagesNotScopedToNamespaceWhenClusterScoped
        Case {
            name: "ListUsagesNotScopedToNamespaceWhenClusterScoped",
            reason: "When ProviderConfig is cluster-scoped (empty namespace), List should not include InNamespace",
            client: MockClient {
                get: Some(new_mock_get_fn(|| pc_named(Some("")))),
                list: Some(Box::new(|_, options| {
                    assert_eq!(
                        options.namespace, None,
                        "List called with a namespace for a cluster-scoped ProviderConfig"
                    );
                    Ok(vec![])
                })),
                update: Some(new_mock_update_fn(None)),
                update_status: Some(new_mock_sub_resource_update_fn(None)),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
        },
    ];
    for tc in cases {
        let got = reconciler(tc.client).reconcile(&ObjectKey::cluster("my-pc")).await;
        assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

type Calls = Arc<Mutex<Vec<(&'static str, Value)>>>;

fn recording_client(calls: &Calls, pc: ProviderConfig, usages: Vec<ProviderConfigUsage>) -> MockClient {
    let record = |verb: &'static str| {
        let calls = calls.clone();
        move |obj: &mut dyn Object| {
            calls.lock().unwrap().push((verb, obj.to_json()?));
            Ok(())
        }
    };
    let (get_calls, list_calls, delete_calls) = (calls.clone(), calls.clone(), calls.clone());
    MockClient {
        get: Some(Box::new(move |_, _| {
            get_calls.lock().unwrap().push(("get", Value::Null));
            Ok(Box::new(pc.clone()))
        })),
        list: Some(Box::new(move |_, options| {
            let labels = serde_json::to_value(&options.labels).unwrap();
            list_calls
                .lock()
                .unwrap()
                .push(("list", json!({"namespace": options.namespace, "labels": labels})));
            Ok(usages.iter().cloned().map(|u| Box::new(u) as Box<dyn Object>).collect())
        })),
        update: Some(Box::new(record("update"))),
        update_status: Some(Box::new(record("update_status"))),
        delete: Some(Box::new(move |obj| {
            delete_calls.lock().unwrap().push(("delete", obj.to_json()?));
            Ok(())
        })),
        ..Default::default()
    }
}

fn verbs(calls: &Calls) -> Vec<&'static str> {
    calls.lock().unwrap().iter().map(|(v, _)| *v).collect()
}

fn body(calls: &Calls, verb: &str) -> Value {
    calls
        .lock()
        .unwrap()
        .iter()
        .find(|(v, _)| *v == verb)
        .unwrap_or_else(|| panic!("no {verb} call"))
        .1
        .clone()
}

#[derive(Clone, Default)]
struct RecordingRecorder {
    events: Arc<Mutex<Vec<(String, Event)>>>,
}

impl Recorder for RecordingRecorder {
    fn event(&self, obj: &dyn Object, event: Event) {
        let subject = format!(
            "{}/{}",
            obj.object_type().group_kind(),
            obj.meta().name.as_deref().unwrap_or("")
        );
        self.events.lock().unwrap().push((subject, event));
    }
    fn with_annotations(&self, _: &[(&str, &str)]) -> Arc<dyn Recorder> {
        Arc::new(self.clone())
    }
}

fn with_recorder(client: MockClient, recorder: &RecordingRecorder) -> Reconciler<ProviderConfig, ProviderConfigUsage> {
    Reconciler::builder(Arc::new(client))
        .recorder(Arc::new(recorder.clone()))
        .build()
}

// Extra tests below have no Go case: the Go table asserts neither the order of verbs, nor the
// bodies written, nor the events emitted. They pin the behaviour the audit-log diff compares.

#[tokio::test]
async fn a_live_config_is_put_then_its_status_is_put_every_pass() {
    let calls = Calls::default();
    let pc = pc_named(None);
    let r = reconciler(recording_client(
        &calls,
        pc,
        vec![controlled_usage(), controlled_usage()],
    ));
    for _ in 0..2 {
        assert_eq!(r.reconcile(&ObjectKey::cluster("my-pc")).await.unwrap(), Outcome::Done);
    }
    assert_eq!(
        verbs(&calls),
        [
            "get",
            "list",
            "update",
            "update_status",
            "get",
            "list",
            "update",
            "update_status"
        ],
        "a PUT and a status PUT on every pass, even when nothing changed"
    );
    let put = body(&calls, "update");
    assert_eq!(put["metadata"]["finalizers"], json!(["in-use.crossplane.io"]));
    assert_eq!(body(&calls, "update_status")["status"]["users"], 2);
}

#[tokio::test]
async fn usages_are_listed_by_config_name_and_kind_label() {
    let calls = Calls::default();
    reconciler(recording_client(&calls, pc_named(None), vec![]))
        .reconcile(&ObjectKey::cluster("my-pc"))
        .await
        .unwrap();
    assert_eq!(
        body(&calls, "list"),
        json!({
            "namespace": null,
            "labels": {
                "crossplane.io/provider-config": "my-pc",
                "crossplane.io/provider-config-kind": "ProviderConfig",
            },
        })
    );
}

#[tokio::test]
async fn a_usage_without_a_controller_is_deleted_and_not_counted() {
    let calls = Calls::default();
    let r = reconciler(recording_client(
        &calls,
        pc_named(Some("ns")),
        vec![ProviderConfigUsage::default(), controlled_usage()],
    ));
    assert_eq!(
        r.reconcile(&ObjectKey::namespaced("ns", "my-pc")).await.unwrap(),
        Outcome::Done
    );
    assert_eq!(verbs(&calls), ["get", "list", "delete", "update", "update_status"]);
    assert_eq!(body(&calls, "list")["namespace"], "ns");
    assert_eq!(body(&calls, "update_status")["status"]["users"], 1);
}

#[tokio::test]
async fn a_usage_that_is_already_gone_is_still_not_counted() {
    let c = MockClient {
        get: Some(new_mock_get_empty_fn()),
        list: Some(new_mock_list_fn(|| vec![ProviderConfigUsage::default()])),
        delete: Some(new_mock_delete_fn(Some(not_found()))),
        update: Some(new_mock_update_fn(None)),
        update_status: Some(Box::new(|obj| {
            assert_eq!(
                obj.to_json()?["status"]["users"],
                Value::Null,
                "zero users is omitted from the wire"
            );
            Ok(())
        })),
        ..Default::default()
    };
    let got = reconciler(c).reconcile(&ObjectKey::cluster("x")).await;
    assert_result("not found on delete", "", &Ok(Outcome::Done), &got);
}

#[tokio::test]
async fn a_blocked_deletion_writes_only_the_status_with_a_terminating_condition() {
    let calls = Calls::default();
    let recorder = RecordingRecorder::default();
    let r = with_recorder(
        recording_client(&calls, deleted(pc_named(None)), vec![controlled_usage()]),
        &recorder,
    );
    assert_eq!(r.reconcile(&ObjectKey::cluster("my-pc")).await.unwrap(), Outcome::Done);
    assert_eq!(
        verbs(&calls),
        ["get", "list", "update_status"],
        "no PUT of the config while blocked"
    );
    let status = body(&calls, "update_status")["status"].clone();
    assert_eq!(status["users"], 1);
    let c = &status["conditions"][0];
    assert_eq!(
        (&c["type"], &c["status"], &c["reason"], &c["message"]),
        (
            &json!("Terminating"),
            &json!("True"),
            &json!("InUse"),
            &json!("Blocking deletion while usages still exist")
        )
    );
    assert!(c.get("observedGeneration").is_none(), "set without a generation");

    assert_eq!(
        *recorder.events.lock().unwrap(),
        [(
            "ProviderConfig.fake.crossplane.io/my-pc".to_owned(),
            Event {
                type_: EventType::Warning,
                reason: Reason(Cow::Borrowed("UsageAccounting")),
                message: "Blocking deletion while usages still exist".into(),
                annotations: Default::default(),
            }
        )]
    );
}

#[tokio::test]
async fn an_unused_deleted_config_loses_its_finalizer_with_one_put() {
    let calls = Calls::default();
    let mut pc = deleted(pc_named(None));
    pc.metadata.finalizers = Some(vec!["in-use.crossplane.io".into(), "other".into()]);
    let r = reconciler(recording_client(&calls, pc, vec![]));
    assert_eq!(r.reconcile(&ObjectKey::cluster("my-pc")).await.unwrap(), Outcome::Done);
    assert_eq!(verbs(&calls), ["get", "list", "update"]);
    assert_eq!(body(&calls, "update")["metadata"]["finalizers"], json!(["other"]));
}

#[tokio::test]
async fn a_failed_status_write_after_the_put_returns_the_wrapped_error() {
    let c = MockClient {
        get: Some(new_mock_get_empty_fn()),
        list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
        update: Some(new_mock_update_fn(None)),
        update_status: Some(new_mock_sub_resource_update_fn(Some(Error::conflict()))),
        ..Default::default()
    };
    let err = reconciler(c).reconcile(&ObjectKey::cluster("x")).await.unwrap_err();
    assert!(err.is_conflict(), "no conflict handling: the error is returned as is");
    assert!(
        err.to_string().starts_with("cannot update ProviderConfig status: "),
        "{err}"
    );
}

#[tokio::test]
async fn warning_events_carry_the_wrapped_error_text() {
    let list_failed = RecordingRecorder::default();
    let c = MockClient {
        get: Some(new_mock_get_fn(|| pc_named(None))),
        list: Some(new_mock_list_err_fn(boom())),
        ..Default::default()
    };
    with_recorder(c, &list_failed)
        .reconcile(&ObjectKey::cluster("my-pc"))
        .await
        .unwrap();
    let delete_failed = RecordingRecorder::default();
    let c = MockClient {
        get: Some(new_mock_get_fn(|| pc_named(None))),
        list: Some(new_mock_list_fn(|| vec![ProviderConfigUsage::default()])),
        delete: Some(new_mock_delete_fn(Some(boom()))),
        ..Default::default()
    };
    with_recorder(c, &delete_failed)
        .reconcile(&ObjectKey::cluster("my-pc"))
        .await
        .unwrap();

    for (recorder, want) in [
        (list_failed, "cannot list ProviderConfigUsages: boom"),
        (delete_failed, "cannot delete ProviderConfigUsage: boom"),
    ] {
        let events = recorder.events.lock().unwrap();
        assert_eq!(events.len(), 1, "{want}");
        let (subject, event) = &events[0];
        assert_eq!(subject, "ProviderConfig.fake.crossplane.io/my-pc");
        assert_eq!(event.type_, EventType::Warning);
        assert_eq!(event.reason, Reason(Cow::Borrowed("UsageAccounting")));
        assert_eq!(event.message, want);
    }
}

#[tokio::test]
async fn a_failed_put_emits_no_event() {
    let recorder = RecordingRecorder::default();
    let c = MockClient {
        get: Some(new_mock_get_empty_fn()),
        list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
        update: Some(new_mock_update_fn(Some(boom()))),
        ..Default::default()
    };
    with_recorder(c, &recorder)
        .reconcile(&ObjectKey::cluster("x"))
        .await
        .unwrap();
    assert!(recorder.events.lock().unwrap().is_empty());
}

/// A `MockClient` whose `get` and `list` first wait, so a test on tokio's paused clock can put a
/// call past the reconcile's deadline.
struct Slow {
    inner: MockClient,
    get: Duration,
    list: Duration,
}

impl Client for Slow {
    fn get<'a>(&'a self, t: &'a ObjectType, key: &'a ObjectKey) -> BoxFuture<'a, Result<Box<dyn Object>>> {
        async move {
            tokio::time::sleep(self.get).await;
            self.inner.get(t, key).await
        }
        .boxed()
    }
    fn list<'a>(&'a self, t: &'a ObjectType, o: &'a ListOptions) -> BoxFuture<'a, Result<Vec<Box<dyn Object>>>> {
        async move {
            tokio::time::sleep(self.list).await;
            self.inner.list(t, o).await
        }
        .boxed()
    }
    fn create<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        self.inner.create(obj)
    }
    fn update<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        self.inner.update(obj)
    }
    fn update_status<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        self.inner.update_status(obj)
    }
    fn patch<'a>(&'a self, obj: &'a mut dyn Object, p: crossplane_core::client::Patch) -> BoxFuture<'a, Result<()>> {
        self.inner.patch(obj, p)
    }
    fn delete<'a>(&'a self, obj: &'a dyn Object) -> BoxFuture<'a, Result<()>> {
        self.inner.delete(obj)
    }
}

fn slow(inner: MockClient, get: Duration, list: Duration) -> Arc<Slow> {
    Arc::new(Slow { inner, get, list })
}

#[tokio::test(start_paused = true)]
async fn a_get_past_the_deadline_fails_wrapped_as_go_returns_it() {
    let c = MockClient {
        get: Some(new_mock_get_fn(|| pc_named(None))),
        ..Default::default()
    };
    let r = Reconciler::<ProviderConfig, ProviderConfigUsage>::builder(slow(c, TIMEOUT * 2, Duration::ZERO)).build();
    let got = r.reconcile(&ObjectKey::cluster("my-pc")).await;
    assert_result(
        "GetPastDeadline",
        "Go's Get fails on the expired context and is wrapped with errGetPC",
        &Err(Error::deadline_exceeded().wrap(ERR_GET_PC)),
        &got,
    );
}

/// Go bounds the run with one context. A List that outlives it fails on that context and the
/// reconcile carries on into the List branch: a warning event and a short wait, not an error.
#[tokio::test(start_paused = true)]
async fn a_hung_list_takes_the_list_failure_branch_at_the_deadline() {
    let recorder = RecordingRecorder::default();
    let c = MockClient {
        get: Some(new_mock_get_fn(|| pc_named(None))),
        list: Some(new_mock_list_fn(Vec::<ProviderConfigUsage>::new)),
        ..Default::default()
    };
    let r = Reconciler::<ProviderConfig, ProviderConfigUsage>::builder(slow(c, Duration::ZERO, Duration::MAX))
        .recorder(Arc::new(recorder.clone()))
        .build();
    let got = r.reconcile(&ObjectKey::cluster("my-pc")).await;
    assert_result(
        "ListPastDeadline",
        "the List branch's result",
        &Ok(Outcome::RequeueAfter(SHORT_WAIT)),
        &got,
    );
    let events = recorder.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].1.message,
        "cannot list ProviderConfigUsages: context deadline exceeded"
    );
}

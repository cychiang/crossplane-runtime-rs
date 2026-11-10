use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use crossplane_core::errors::Error;
use crossplane_core::resource::ObjectKey;
use crossplane_test::{MockClient, assert_object_eq, assert_result, fake, new_mock_get_err_fn, new_mock_get_fn};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::{
    CustomResourceDefinition, CustomResourceDefinitionCondition, CustomResourceDefinitionNames,
    CustomResourceDefinitionSpec, CustomResourceDefinitionStatus, CustomResourceDefinitionVersion,
    CustomResourceValidation, JSONSchemaProps,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ManagedFieldsEntry, ObjectMeta, Time};
use kube::core::GroupVersionKind;

use super::*;
use crate::gate::{Gate, SetGate};

fn gvk(version: &str) -> GroupVersionKind {
    GroupVersionKind {
        group: "example.com".into(),
        version: version.into(),
        kind: "TestResource".into(),
    }
}

fn version(name: &str, served: bool) -> CustomResourceDefinitionVersion {
    CustomResourceDefinitionVersion {
        name: name.into(),
        served,
        ..Default::default()
    }
}

fn condition(type_: &str, status: &str) -> CustomResourceDefinitionCondition {
    CustomResourceDefinitionCondition {
        type_: type_.into(),
        status: status.into(),
        ..Default::default()
    }
}

fn crd(versions: Vec<CustomResourceDefinitionVersion>) -> CustomResourceDefinition {
    CustomResourceDefinition {
        metadata: ObjectMeta {
            name: Some("testresources.example.com".into()),
            ..Default::default()
        },
        spec: CustomResourceDefinitionSpec {
            group: "example.com".into(),
            names: CustomResourceDefinitionNames {
                kind: "TestResource".into(),
                ..Default::default()
            },
            versions,
            ..Default::default()
        },
        status: None,
    }
}

fn with_conditions(
    mut crd: CustomResourceDefinition,
    conditions: Vec<CustomResourceDefinitionCondition>,
) -> CustomResourceDefinition {
    crd.status = Some(CustomResourceDefinitionStatus {
        conditions: Some(conditions),
        ..Default::default()
    });
    crd
}

struct ToGvks {
    name: &'static str,
    crd: CustomResourceDefinition,
    want: HashMap<GroupVersionKind, bool>,
}

#[test]
fn to_gvks_cases() {
    type Case = ToGvks;
    let cases = vec![
        // go: pkg/reconciler/customresourcesgate TestToGVKs/SingleVersionServed
        Case {
            name: "SingleVersionServed",
            crd: crd(vec![version("v1", true)]),
            want: HashMap::from([(gvk("v1"), true)]),
        },
        // go: pkg/reconciler/customresourcesgate TestToGVKs/MultipleVersionsWithServedStatus
        Case {
            name: "MultipleVersionsWithServedStatus",
            crd: crd(vec![
                version("v1alpha1", false),
                version("v1beta1", true),
                version("v1", true),
            ]),
            want: HashMap::from([(gvk("v1alpha1"), false), (gvk("v1beta1"), true), (gvk("v1"), true)]),
        },
        // go: pkg/reconciler/customresourcesgate TestToGVKs/NoVersions
        Case {
            name: "NoVersions",
            crd: crd(vec![]),
            want: HashMap::new(),
        },
    ];
    for tc in cases {
        assert_eq!(to_gvks(&tc.crd), tc.want, "{}", tc.name);
    }
}

struct Established {
    name: &'static str,
    crd: CustomResourceDefinition,
    want: bool,
}

#[test]
fn is_established_cases() {
    type Case = Established;
    let cases = vec![
        // go: pkg/reconciler/customresourcesgate TestIsEstablished/EstablishedTrue
        Case {
            name: "EstablishedTrue",
            crd: with_conditions(crd(vec![]), vec![condition("Established", "True")]),
            want: true,
        },
        // go: pkg/reconciler/customresourcesgate TestIsEstablished/EstablishedFalse
        Case {
            name: "EstablishedFalse",
            crd: with_conditions(crd(vec![]), vec![condition("Established", "False")]),
            want: false,
        },
        // go: pkg/reconciler/customresourcesgate TestIsEstablished/EstablishedUnknown
        Case {
            name: "EstablishedUnknown",
            crd: with_conditions(crd(vec![]), vec![condition("Established", "Unknown")]),
            want: false,
        },
        // go: pkg/reconciler/customresourcesgate TestIsEstablished/NoEstablishedCondition
        Case {
            name: "NoEstablishedCondition",
            crd: with_conditions(crd(vec![]), vec![condition("NamesAccepted", "True")]),
            want: false,
        },
        // go: pkg/reconciler/customresourcesgate TestIsEstablished/NoConditions
        Case {
            name: "NoConditions",
            crd: with_conditions(crd(vec![]), vec![]),
            want: false,
        },
        // go: pkg/reconciler/customresourcesgate TestIsEstablished/MultipleConditions
        Case {
            name: "MultipleConditions",
            crd: with_conditions(
                crd(vec![]),
                vec![
                    condition("NamesAccepted", "True"),
                    condition("Established", "True"),
                    condition("Terminating", "False"),
                ],
            ),
            want: true,
        },
    ];
    for tc in cases {
        assert_eq!(is_established(&tc.crd), tc.want, "{}", tc.name);
    }
}

#[test]
fn a_crd_with_no_status_is_not_established() {
    assert!(!is_established(&crd(vec![])));
}

#[test]
fn the_first_established_condition_decides() {
    let c = with_conditions(
        crd(vec![]),
        vec![condition("Established", "False"), condition("Established", "True")],
    );
    assert!(!is_established(&c));
}

#[derive(Default)]
struct RecordingGate {
    true_calls: Mutex<Vec<GroupVersionKind>>,
    false_calls: Mutex<Vec<GroupVersionKind>>,
}

impl SetGate<GroupVersionKind> for RecordingGate {
    fn set(&self, key: GroupVersionKind, value: bool) -> bool {
        let calls = if value { &self.true_calls } else { &self.false_calls };
        calls.lock().unwrap().push(key);
        true
    }
}

fn sorted(mut calls: Vec<GroupVersionKind>) -> Vec<GroupVersionKind> {
    calls.sort_by(|a, b| (&a.group, &a.version, &a.kind).cmp(&(&b.group, &b.version, &b.kind)));
    calls
}

fn client_returning(crd: CustomResourceDefinition) -> Arc<dyn Client> {
    Arc::new(MockClient {
        get: Some(new_mock_get_fn(move || crd.clone())),
        ..Default::default()
    })
}

struct ReconcileCase {
    name: &'static str,
    crd: CustomResourceDefinition,
    want_true: Vec<GroupVersionKind>,
    want_false: Vec<GroupVersionKind>,
}

#[tokio::test]
async fn reconcile_cases() {
    type Case = ReconcileCase;
    let established = vec![condition("Established", "True")];
    let cases = vec![
        // go: pkg/reconciler/customresourcesgate TestReconcile/EstablishedCRDCallsGateTrue
        Case {
            name: "EstablishedCRDCallsGateTrue",
            crd: with_conditions(
                crd(vec![version("v1alpha1", true), version("v1", true)]),
                established.clone(),
            ),
            want_true: vec![gvk("v1alpha1"), gvk("v1")],
            want_false: vec![],
        },
        // go: pkg/reconciler/customresourcesgate TestReconcile/NotEstablishedCRDCallsGateFalse
        Case {
            name: "NotEstablishedCRDCallsGateFalse",
            crd: with_conditions(crd(vec![version("v1", true)]), vec![condition("Established", "False")]),
            want_true: vec![],
            want_false: vec![gvk("v1")],
        },
        // go: pkg/reconciler/customresourcesgate TestReconcile/DeletingCRDCallsGateFalse
        Case {
            name: "DeletingCRDCallsGateFalse",
            crd: {
                let mut c = with_conditions(crd(vec![version("v1", true)]), established.clone());
                c.metadata.deletion_timestamp = Some(Time(k8s_openapi::jiff::Timestamp::now()));
                c
            },
            want_true: vec![],
            want_false: vec![gvk("v1")],
        },
        // go: pkg/reconciler/customresourcesgate TestReconcile/MixedServedVersions
        Case {
            name: "MixedServedVersions",
            crd: with_conditions(
                crd(vec![version("v1alpha1", false), version("v1", true)]),
                established.clone(),
            ),
            want_true: vec![gvk("v1")],
            want_false: vec![],
        },
        // go: pkg/reconciler/customresourcesgate TestReconcile/NoVersionsCRD
        Case {
            name: "NoVersionsCRD",
            crd: with_conditions(crd(vec![]), established.clone()),
            want_true: vec![],
            want_false: vec![],
        },
    ];
    for tc in cases {
        let gate = Arc::new(RecordingGate::default());
        let r = Reconciler::new(client_returning(tc.crd), gate.clone());
        let got = r.reconcile(&ObjectKey::cluster("testresources.example.com")).await;
        assert_result(tc.name, "", &Ok(Outcome::Done), &got);
        assert_eq!(
            sorted(gate.true_calls.lock().unwrap().clone()),
            sorted(tc.want_true),
            "{}: gate.set(.., true) calls",
            tc.name
        );
        assert_eq!(
            sorted(gate.false_calls.lock().unwrap().clone()),
            sorted(tc.want_false),
            "{}: gate.set(.., false) calls",
            tc.name
        );
    }
}

#[tokio::test]
async fn a_missing_crd_is_not_an_error_and_touches_nothing() {
    let gate = Arc::new(RecordingGate::default());
    let client = Arc::new(MockClient {
        get: Some(new_mock_get_err_fn(Error::status(
            crossplane_core::errors::StatusReason::NotFound,
            "gone",
        ))),
        ..Default::default()
    });
    let got = Reconciler::new(client, gate.clone())
        .reconcile(&ObjectKey::cluster("x"))
        .await;
    assert_result("not found", "", &Ok(Outcome::Done), &got);
    assert!(gate.true_calls.lock().unwrap().is_empty() && gate.false_calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_get_error_is_returned_unchanged() {
    let gate = Arc::new(RecordingGate::default());
    let client = Arc::new(MockClient {
        get: Some(new_mock_get_err_fn(Error::new("boom"))),
        ..Default::default()
    });
    let got = Reconciler::new(client, gate).reconcile(&ObjectKey::cluster("x")).await;
    assert_result("get error", "", &Err(Error::new("boom")), &got);
}

/// The CRD controller feeds a real gate: a callback registered behind the kind fires when the CRD
/// becomes Established, and a later Unestablished CRD does not run it again.
#[tokio::test]
async fn an_established_crd_releases_a_registration_on_the_real_gate() {
    let gate = Arc::new(Gate::<GroupVersionKind>::default());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    gate.register(move || tx.send(()).unwrap(), [gvk("v1")]);

    let unready = crd(vec![version("v1", true)]);
    Reconciler::new(client_returning(unready.clone()), gate.clone())
        .reconcile(&ObjectKey::cluster("x"))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(
        rx.try_recv().is_err(),
        "an unestablished CRD must not release the callback"
    );

    let ready = with_conditions(unready, vec![condition("Established", "True")]);
    Reconciler::new(client_returning(ready), gate.clone())
        .reconcile(&ObjectKey::cluster("x"))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("released")
        .expect("sent");
}

struct Strip {
    name: &'static str,
    obj: Box<dyn Object>,
    want: Box<dyn Object>,
}

fn schema() -> CustomResourceValidation {
    CustomResourceValidation {
        open_api_v3_schema: Some(JSONSchemaProps {
            type_: Some("object".into()),
            ..Default::default()
        }),
    }
}

fn versions_with_schema(served: &[(&str, bool)]) -> Vec<CustomResourceDefinitionVersion> {
    served
        .iter()
        .map(|(n, s)| CustomResourceDefinitionVersion {
            schema: Some(schema()),
            ..version(n, *s)
        })
        .collect()
}

fn annotated(mut c: CustomResourceDefinition, pairs: &[(&str, &str)]) -> CustomResourceDefinition {
    c.metadata.annotations = Some(pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect());
    c
}

#[test]
fn transform_strip_crd_schema_cases() {
    type Case = Strip;
    let last_applied = "kubectl.kubernetes.io/last-applied-configuration";
    let cases: Vec<Case> = vec![
        // go: pkg/reconciler/customresourcesgate TestTransformStripCRDSchema/StripsSchemaManagedFieldsAndAnnotation
        Case {
            name: "StripsSchemaManagedFieldsAndAnnotation",
            obj: Box::new({
                let mut c = annotated(
                    with_conditions(
                        crd(versions_with_schema(&[("v1", true)])),
                        vec![condition("Established", "True")],
                    ),
                    &[
                        (last_applied, r#"{"very":"large","json":"blob"}"#),
                        ("other-annotation", "keep-me"),
                    ],
                );
                c.metadata.managed_fields = Some(vec![ManagedFieldsEntry {
                    manager: Some("kubectl".into()),
                    operation: Some("Apply".into()),
                    ..Default::default()
                }]);
                c
            }),
            want: Box::new(annotated(
                with_conditions(crd(vec![version("v1", true)]), vec![condition("Established", "True")]),
                &[("other-annotation", "keep-me")],
            )),
        },
        // go: pkg/reconciler/customresourcesgate TestTransformStripCRDSchema/MultipleVersions
        Case {
            name: "MultipleVersions",
            obj: Box::new(crd(versions_with_schema(&[("v1", true), ("v1beta1", false)]))),
            want: Box::new(crd(vec![version("v1", true), version("v1beta1", false)])),
        },
        // go: pkg/reconciler/customresourcesgate TestTransformStripCRDSchema/NoVersions
        Case {
            name: "NoVersions",
            obj: Box::new(crd(vec![])),
            want: Box::new(crd(vec![])),
        },
        // go: pkg/reconciler/customresourcesgate TestTransformStripCRDSchema/NilAnnotations
        Case {
            name: "NilAnnotations",
            obj: Box::new(crd(vec![])),
            want: Box::new(crd(vec![])),
        },
        // go: pkg/reconciler/customresourcesgate TestTransformStripCRDSchema/NoLastAppliedAnnotation
        Case {
            name: "NoLastAppliedAnnotation",
            obj: Box::new(annotated(crd(vec![]), &[("custom-annotation", "value")])),
            want: Box::new(annotated(crd(vec![]), &[("custom-annotation", "value")])),
        },
        // go: pkg/reconciler/customresourcesgate TestTransformStripCRDSchema/NonCRDObject
        Case {
            name: "NonCRDObject",
            obj: Box::new(fake::Object::default()),
            want: Box::new(fake::Object::default()),
        },
    ];
    for mut tc in cases {
        transform_strip_crd_schema(&mut *tc.obj);
        assert_object_eq(&*tc.want, &*tc.obj, tc.name);
    }
}

#[test]
fn the_annotation_is_removed_alone_when_it_is_the_only_one() {
    let mut c = annotated(
        crd(vec![]),
        &[("kubectl.kubernetes.io/last-applied-configuration", "{}")],
    );
    transform_strip_crd_schema(&mut c);
    assert_eq!(c.metadata.annotations, Some(BTreeMap::new()));
}

#[tokio::test]
async fn gvks_are_logged_as_go_prints_them() {
    let established = vec![condition("Established", "True")];
    let served = with_conditions(crd(vec![version("v1", true)]), established);
    let key = ObjectKey::cluster("testresources.example.com");
    let r = Reconciler::new(client_returning(served), Arc::new(RecordingGate::default()));
    let (_, logged) = crate::logging::capture(r.reconcile(&key)).await;
    assert!(logged.contains("DEBUG"), "{logged}");
    assert!(logged.contains(r#"gvk=example.com/v1, Kind=TestResource"#), "{logged}");
    assert!(logged.contains("gvk is ready"), "{logged}");

    let unready = with_conditions(crd(vec![version("v1", true)]), vec![condition("Established", "False")]);
    let r = Reconciler::new(client_returning(unready), Arc::new(RecordingGate::default()));
    let (_, logged) = crate::logging::capture(r.reconcile(&key)).await;
    assert!(logged.contains("gvk is not ready"), "{logged}");
}

//! Port of `pkg/meta/meta_test.go`, case for case. Go's `corev1.Pod` is `k8s_openapi`'s `Pod`.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{ObjectReference, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference, Time};
use k8s_openapi::jiff::{Timestamp, ToSpan};

use super::*;

const GROUP: &str = "coolstuff";
const VERSION: &str = "v1";
const GROUP_VERSION: &str = "coolstuff/v1";
const KIND: &str = "coolresource";
const NAMESPACE: &str = "coolns";
const NAME: &str = "cool";
const UID: &str = "definitely-a-uuid";

fn pod_with(metadata: ObjectMeta) -> Pod {
    Pod {
        metadata,
        ..Default::default()
    }
}

fn with_annotations(annotations: &[(&str, &str)]) -> Pod {
    pod_with(ObjectMeta {
        annotations: Some(strings(annotations)),
        ..Default::default()
    })
}

fn with_owners(owners: Vec<OwnerReference>) -> Pod {
    pod_with(ObjectMeta {
        owner_references: Some(owners),
        ..Default::default()
    })
}

fn with_finalizers(finalizers: &[&str]) -> Pod {
    pod_with(ObjectMeta {
        finalizers: Some(finalizers.iter().map(|f| (*f).to_owned()).collect()),
        ..Default::default()
    })
}

fn with_labels(labels: &[(&str, &str)]) -> Pod {
    pod_with(ObjectMeta {
        labels: Some(strings(labels)),
        ..Default::default()
    })
}

fn strings(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn owner(uid: &str) -> OwnerReference {
    OwnerReference {
        uid: uid.to_owned(),
        ..Default::default()
    }
}

fn controller(uid: &str) -> OwnerReference {
    OwnerReference {
        uid: uid.to_owned(),
        controller: Some(true),
        ..Default::default()
    }
}

fn gvk() -> GroupVersionKind {
    GroupVersionKind::gvk(GROUP, VERSION, KIND)
}

fn pod_named() -> Pod {
    pod_with(ObjectMeta {
        namespace: Some(NAMESPACE.to_owned()),
        name: Some(NAME.to_owned()),
        uid: Some(UID.to_owned()),
        ..Default::default()
    })
}

fn whole_second(t: Timestamp) -> Timestamp {
    Timestamp::from_second(t.as_second()).unwrap()
}

#[test]
fn reference_to_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        of: GroupVersionKind,
        want: ObjectReference,
    }
    let cases = vec![
        // go: pkg/meta TestReferenceTo/WithTypeMeta
        Case {
            name: "WithTypeMeta",
            o: pod_named(),
            of: gvk(),
            want: ObjectReference {
                api_version: Some(GROUP_VERSION.to_owned()),
                kind: Some(KIND.to_owned()),
                namespace: Some(NAMESPACE.to_owned()),
                name: Some(NAME.to_owned()),
                uid: Some(UID.to_owned()),
                ..Default::default()
            },
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, reference_to(&tc.o, &tc.of), "{}: reference_to", tc.name);
    }
}

#[test]
fn typed_reference_to_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        of: GroupVersionKind,
        want: TypedReference,
    }
    let cases = vec![
        // go: pkg/meta TestTypedReferenceTo/WithTypeMeta
        Case {
            name: "WithTypeMeta",
            o: pod_named(),
            of: gvk(),
            want: TypedReference {
                api_version: GROUP_VERSION.to_owned(),
                kind: KIND.to_owned(),
                name: NAME.to_owned(),
                uid: UID.to_owned(),
            },
        },
    ];
    for tc in cases {
        assert_eq!(
            tc.want,
            typed_reference_to(&tc.o, &tc.of),
            "{}: typed_reference_to",
            tc.name
        );
    }
}

fn typed_reference() -> TypedReference {
    TypedReference {
        api_version: GROUP_VERSION.to_owned(),
        kind: KIND.to_owned(),
        name: NAME.to_owned(),
        uid: UID.to_owned(),
    }
}

#[test]
fn as_owner_cases() {
    struct Case {
        name: &'static str,
        r: TypedReference,
        want: OwnerReference,
    }
    let cases = vec![
        // go: pkg/meta TestAsOwner/Successful
        Case {
            name: "Successful",
            r: typed_reference(),
            want: OwnerReference {
                api_version: GROUP_VERSION.to_owned(),
                kind: KIND.to_owned(),
                name: NAME.to_owned(),
                uid: UID.to_owned(),
                ..Default::default()
            },
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, as_owner(&tc.r), "{}: as_owner", tc.name);
    }
}

#[test]
fn as_controller_cases() {
    struct Case {
        name: &'static str,
        r: TypedReference,
        want: OwnerReference,
    }
    let cases = vec![
        // go: pkg/meta TestAsController/Successful
        Case {
            name: "Successful",
            r: typed_reference(),
            want: OwnerReference {
                api_version: GROUP_VERSION.to_owned(),
                kind: KIND.to_owned(),
                name: NAME.to_owned(),
                uid: UID.to_owned(),
                controller: Some(true),
                block_owner_deletion: Some(true),
            },
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, as_controller(&tc.r), "{}: as_controller", tc.name);
    }
}

#[test]
fn have_same_controller_cases() {
    struct Case {
        name: &'static str,
        a: Pod,
        b: Pod,
        want: bool,
    }
    let cases = vec![
        // go: pkg/meta TestHaveSameController/SameController
        Case {
            name: "SameController",
            a: with_owners(vec![controller(UID)]),
            b: with_owners(vec![controller(UID)]),
            want: true,
        },
        // go: pkg/meta TestHaveSameController/AHasNoController
        Case {
            name: "AHasNoController",
            a: Pod::default(),
            b: with_owners(vec![controller("a-different-uuid")]),
            want: false,
        },
        // go: pkg/meta TestHaveSameController/BHasNoController
        Case {
            name: "BHasNoController",
            a: with_owners(vec![controller(UID)]),
            b: Pod::default(),
            want: false,
        },
        // go: pkg/meta TestHaveSameController/ControllersDiffer
        Case {
            name: "ControllersDiffer",
            a: with_owners(vec![controller(UID)]),
            b: with_owners(vec![controller("a-different-uuid")]),
            want: false,
        },
    ];
    for tc in cases {
        assert_eq!(
            tc.want,
            have_same_controller(&tc.a, &tc.b),
            "{}: have_same_controller",
            tc.name
        );
    }
}

#[test]
fn namespaced_name_of_cases() {
    struct Case {
        name: &'static str,
        r: ObjectReference,
        want: ObjectKey,
    }
    let cases = vec![
        // go: pkg/meta TestNamespacedNameOf/Success
        Case {
            name: "Success",
            r: ObjectReference {
                namespace: Some(NAMESPACE.to_owned()),
                name: Some(NAME.to_owned()),
                ..Default::default()
            },
            want: ObjectKey::namespaced(NAMESPACE, NAME),
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, namespaced_name_of(&tc.r), "{}: namespaced_name_of", tc.name);
    }
}

#[test]
fn add_owner_reference_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        r: OwnerReference,
        want: Vec<OwnerReference>,
    }
    let cases = vec![
        // go: pkg/meta TestAddOwnerReference/NoExistingOwners
        Case {
            name: "NoExistingOwners",
            o: Pod::default(),
            r: owner(UID),
            want: vec![owner(UID)],
        },
        // go: pkg/meta TestAddOwnerReference/UpdateExistingOwner
        Case {
            name: "UpdateExistingOwner",
            o: with_owners(vec![controller(UID)]),
            r: owner(UID),
            want: vec![owner(UID)],
        },
        // go: pkg/meta TestAddOwnerReference/OwnedByAnotherObject
        Case {
            name: "OwnedByAnotherObject",
            o: with_owners(vec![owner("a-different-uuid")]),
            r: owner(UID),
            want: vec![owner("a-different-uuid"), owner(UID)],
        },
    ];
    for mut tc in cases {
        add_owner_reference(&mut tc.o, tc.r);
        let got = tc.o.metadata.owner_references.unwrap_or_default();
        assert_eq!(tc.want, got, "{}: owner references", tc.name);
    }
}

#[test]
fn add_controller_reference_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        r: OwnerReference,
        want_owners: Vec<OwnerReference>,
        want_err: Option<Error>,
    }
    let otrlr = OwnerReference {
        kind: "lame".to_owned(),
        name: "othercontroller".to_owned(),
        uid: "a-different-uuid".to_owned(),
        controller: Some(true),
        ..Default::default()
    };
    let cases = vec![
        // go: pkg/meta TestAddControllerReference/NoExistingOwners
        Case {
            name: "NoExistingOwners",
            o: Pod::default(),
            r: owner(UID),
            want_owners: vec![owner(UID)],
            want_err: None,
        },
        // go: pkg/meta TestAddControllerReference/UpdateExistingOwner
        Case {
            name: "UpdateExistingOwner",
            o: with_owners(vec![controller(UID)]),
            r: owner(UID),
            want_owners: vec![owner(UID)],
            want_err: None,
        },
        // go: pkg/meta TestAddControllerReference/OwnedByAnotherObject
        Case {
            name: "OwnedByAnotherObject",
            o: with_owners(vec![owner("a-different-uuid")]),
            r: owner(UID),
            want_owners: vec![owner("a-different-uuid"), owner(UID)],
            want_err: None,
        },
        // go: pkg/meta TestAddControllerReference/ControlledByAnotherObject
        Case {
            name: "ControlledByAnotherObject",
            o: pod_with(ObjectMeta {
                name: Some(NAME.to_owned()),
                owner_references: Some(vec![otrlr.clone()]),
                ..Default::default()
            }),
            r: owner(UID),
            want_owners: vec![otrlr.clone()],
            want_err: Some(Error::new(format!(
                "{} is already controlled by {} {} (UID {})",
                NAME, otrlr.kind, otrlr.name, otrlr.uid
            ))),
        },
    ];
    for mut tc in cases {
        let got_err = add_controller_reference(&mut tc.o, tc.r).err();
        assert_eq!(tc.want_err, got_err, "{}: error", tc.name);
        let got = tc.o.metadata.owner_references.unwrap_or_default();
        assert_eq!(tc.want_owners, got, "{}: owner references", tc.name);
    }
}

#[test]
fn add_finalizer_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        finalizer: &'static str,
        want: Vec<&'static str>,
    }
    let cases = vec![
        // go: pkg/meta TestAddFinalizer/NoExistingFinalizers
        Case {
            name: "NoExistingFinalizers",
            o: Pod::default(),
            finalizer: "fin",
            want: vec!["fin"],
        },
        // go: pkg/meta TestAddFinalizer/FinalizerAlreadyExists
        Case {
            name: "FinalizerAlreadyExists",
            o: with_finalizers(&["fin"]),
            finalizer: "fin",
            want: vec!["fin"],
        },
        // go: pkg/meta TestAddFinalizer/AnotherFinalizerExists
        Case {
            name: "AnotherFinalizerExists",
            o: with_finalizers(&["fun"]),
            finalizer: "fin",
            want: vec!["fun", "fin"],
        },
    ];
    for mut tc in cases {
        add_finalizer(&mut tc.o, tc.finalizer);
        let got = tc.o.metadata.finalizers.unwrap_or_default();
        assert_eq!(tc.want, got, "{}: finalizers", tc.name);
    }
}

#[test]
fn remove_finalizer_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        finalizer: &'static str,
        want: Option<Vec<&'static str>>,
    }
    let cases = vec![
        // go: pkg/meta TestRemoveFinalizer/NoExistingFinalizers
        Case {
            name: "NoExistingFinalizers",
            o: Pod::default(),
            finalizer: "fin",
            want: None,
        },
        // go: pkg/meta TestRemoveFinalizer/FinalizerExists
        Case {
            name: "FinalizerExists",
            o: with_finalizers(&["fin"]),
            finalizer: "fin",
            // Go leaves an empty slice, which `omitempty` drops from the JSON; here that is None.
            want: None,
        },
        // go: pkg/meta TestRemoveFinalizer/AnotherFinalizerExists
        Case {
            name: "AnotherFinalizerExists",
            o: with_finalizers(&["fin", "fun"]),
            finalizer: "fin",
            want: Some(vec!["fun"]),
        },
    ];
    for mut tc in cases {
        remove_finalizer(&mut tc.o, tc.finalizer);
        let want = tc.want.map(|w| w.into_iter().map(str::to_owned).collect::<Vec<_>>());
        assert_eq!(want, tc.o.metadata.finalizers, "{}: finalizers", tc.name);
    }
}

#[test]
fn finalizer_exists_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        finalizer: &'static str,
        want: bool,
    }
    let cases = vec![
        // go: pkg/meta TestFinalizerExists/NoExistingFinalizers
        Case {
            name: "NoExistingFinalizers",
            o: Pod::default(),
            finalizer: "fin",
            want: false,
        },
        // go: pkg/meta TestFinalizerExists/FinalizerExists
        Case {
            name: "FinalizerExists",
            o: with_finalizers(&["fin"]),
            finalizer: "fin",
            want: true,
        },
        // go: pkg/meta TestFinalizerExists/AnotherFinalizerExists
        Case {
            name: "AnotherFinalizerExists",
            o: with_finalizers(&["fun"]),
            finalizer: "fin",
            want: false,
        },
    ];
    for tc in cases {
        assert_eq!(
            tc.want,
            finalizer_exists(&tc.o, tc.finalizer),
            "{}: finalizer_exists",
            tc.name
        );
    }
}

#[test]
fn add_labels_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        labels: Vec<(&'static str, &'static str)>,
        want: BTreeMap<String, String>,
    }
    let cases = vec![
        // go: pkg/meta TestAddLabels/ExistingLabels
        Case {
            name: "ExistingLabels",
            o: with_labels(&[("ekey", "evalue")]),
            labels: vec![("key", "value")],
            want: strings(&[("ekey", "evalue"), ("key", "value")]),
        },
        // go: pkg/meta TestAddLabels/NoExistingLabels
        Case {
            name: "NoExistingLabels",
            o: Pod::default(),
            labels: vec![("key", "value")],
            want: strings(&[("key", "value")]),
        },
    ];
    for mut tc in cases {
        add_labels(
            &mut tc.o,
            tc.labels.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        );
        assert_eq!(Some(tc.want), tc.o.metadata.labels, "{}: labels", tc.name);
    }
}

#[test]
fn remove_labels_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        labels: Vec<&'static str>,
        want: Option<BTreeMap<String, String>>,
    }
    let cases = vec![
        // go: pkg/meta TestRemoveLabels/ExistingLabels
        Case {
            name: "ExistingLabels",
            o: with_labels(&[("keyA", "valueA"), ("keyB", "valueB")]),
            labels: vec!["keyA"],
            want: Some(strings(&[("keyB", "valueB")])),
        },
        // go: pkg/meta TestRemoveLabels/NoExistingLabels
        Case {
            name: "NoExistingLabels",
            o: Pod::default(),
            labels: vec!["keyA"],
            want: None,
        },
    ];
    for mut tc in cases {
        remove_labels(&mut tc.o, &tc.labels);
        assert_eq!(tc.want, tc.o.metadata.labels, "{}: labels", tc.name);
    }
}

#[test]
fn add_annotations_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        annotations: Vec<(&'static str, &'static str)>,
        want: BTreeMap<String, String>,
    }
    let cases = vec![
        // go: pkg/meta TestAddAnnotations/ExistingAnnotations
        Case {
            name: "ExistingAnnotations",
            o: with_annotations(&[("ekey", "evalue")]),
            annotations: vec![("key", "value")],
            want: strings(&[("ekey", "evalue"), ("key", "value")]),
        },
        // go: pkg/meta TestAddAnnotations/NoExistingAnnotations
        Case {
            name: "NoExistingAnnotations",
            o: Pod::default(),
            annotations: vec![("key", "value")],
            want: strings(&[("key", "value")]),
        },
    ];
    for mut tc in cases {
        add_annotations(
            &mut tc.o,
            tc.annotations.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        );
        assert_eq!(Some(tc.want), tc.o.metadata.annotations, "{}: annotations", tc.name);
    }
}

#[test]
fn remove_annotations_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        annotations: Vec<&'static str>,
        want: Option<BTreeMap<String, String>>,
    }
    let cases = vec![
        // go: pkg/meta TestRemoveAnnotations/ExistingAnnotations
        Case {
            name: "ExistingAnnotations",
            o: with_annotations(&[("keyA", "valueA"), ("keyB", "valueB")]),
            annotations: vec!["keyA"],
            want: Some(strings(&[("keyB", "valueB")])),
        },
        // go: pkg/meta TestRemoveAnnotations/NoExistingAnnotations
        Case {
            name: "NoExistingAnnotations",
            o: Pod::default(),
            annotations: vec!["keyA"],
            want: None,
        },
    ];
    for mut tc in cases {
        remove_annotations(&mut tc.o, &tc.annotations);
        assert_eq!(tc.want, tc.o.metadata.annotations, "{}: annotations", tc.name);
    }
}

#[test]
fn was_deleted_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: bool,
    }
    let cases = vec![
        // go: pkg/meta TestWasDeleted/ObjectWasDeleted
        Case {
            name: "ObjectWasDeleted",
            o: pod_with(ObjectMeta {
                deletion_timestamp: Some(Time(Timestamp::now())),
                ..Default::default()
            }),
            want: true,
        },
        // go: pkg/meta TestWasDeleted/ObjectWasNotDeleted
        Case {
            name: "ObjectWasNotDeleted",
            o: Pod::default(),
            want: false,
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, was_deleted(&tc.o), "{}: was_deleted", tc.name);
    }
}

#[test]
fn was_created_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: bool,
    }
    let cases = vec![
        // go: pkg/meta TestWasCreated/ObjectWasCreated
        Case {
            name: "ObjectWasCreated",
            o: pod_with(ObjectMeta {
                creation_timestamp: Some(Time(Timestamp::now())),
                ..Default::default()
            }),
            want: true,
        },
        // go: pkg/meta TestWasCreated/ObjectWasNotCreated
        Case {
            name: "ObjectWasNotCreated",
            o: Pod::default(),
            want: false,
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, was_created(&tc.o), "{}: was_created", tc.name);
    }
}

#[test]
fn get_external_name_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: &'static str,
    }
    let cases = vec![
        // go: pkg/meta TestGetExternalName/ExternalNameExists
        Case {
            name: "ExternalNameExists",
            o: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_NAME, NAME)]),
            want: NAME,
        },
        // go: pkg/meta TestGetExternalName/NoExternalName
        Case {
            name: "NoExternalName",
            o: Pod::default(),
            want: "",
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, external_name(&tc.o), "{}: external_name", tc.name);
    }
}

#[test]
fn set_external_name_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        external_name: &'static str,
        want: Pod,
    }
    let cases = vec![
        // go: pkg/meta TestSetExternalName/SetsTheCorrectKey
        Case {
            name: "SetsTheCorrectKey",
            o: Pod::default(),
            external_name: NAME,
            want: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_NAME, NAME)]),
        },
    ];
    for mut tc in cases {
        set_external_name(&mut tc.o, tc.external_name);
        assert_eq!(tc.want, tc.o, "{}: set_external_name", tc.name);
    }
}

#[test]
fn get_external_create_pending_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: Option<Timestamp>,
    }
    let now = whole_second(Timestamp::now());
    let cases = vec![
        // go: pkg/meta TestGetExternalCreatePending/ExternalCreatePendingExists
        Case {
            name: "ExternalCreatePendingExists",
            o: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, &rfc3339(now))]),
            want: Some(now),
        },
        // go: pkg/meta TestGetExternalCreatePending/NoExternalCreatePending
        Case {
            name: "NoExternalCreatePending",
            o: Pod::default(),
            want: None,
        },
    ];
    for tc in cases {
        assert_eq!(
            tc.want,
            external_create_pending(&tc.o),
            "{}: external_create_pending",
            tc.name
        );
    }
}

#[test]
fn set_external_create_pending_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        t: Timestamp,
        want: Pod,
    }
    let now = Timestamp::now();
    let cases = vec![
        // go: pkg/meta TestSetExternalCreatePending/SetsTheCorrectKey
        Case {
            name: "SetsTheCorrectKey",
            o: Pod::default(),
            t: now,
            want: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, &rfc3339(now))]),
        },
    ];
    for mut tc in cases {
        set_external_create_pending(&mut tc.o, tc.t);
        assert_eq!(tc.want, tc.o, "{}: set_external_create_pending", tc.name);
    }
}

#[test]
fn get_external_create_succeeded_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: Option<Timestamp>,
    }
    let now = whole_second(Timestamp::now());
    let cases = vec![
        // go: pkg/meta TestGetExternalCreateSucceeded/ExternalCreateTimeExists
        Case {
            name: "ExternalCreateTimeExists",
            o: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED, &rfc3339(now))]),
            want: Some(now),
        },
        // go: pkg/meta TestGetExternalCreateSucceeded/NoExternalCreateTime
        Case {
            name: "NoExternalCreateTime",
            o: Pod::default(),
            want: None,
        },
    ];
    for tc in cases {
        assert_eq!(
            tc.want,
            external_create_succeeded(&tc.o),
            "{}: external_create_succeeded",
            tc.name
        );
    }
}

#[test]
fn set_external_create_succeeded_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        t: Timestamp,
        want: Pod,
    }
    let now = Timestamp::now();
    let cases = vec![
        // go: pkg/meta TestSetExternalCreateSucceeded/SetsTheCorrectKey
        Case {
            name: "SetsTheCorrectKey",
            o: Pod::default(),
            t: now,
            want: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED, &rfc3339(now))]),
        },
    ];
    for mut tc in cases {
        set_external_create_succeeded(&mut tc.o, tc.t);
        assert_eq!(tc.want, tc.o, "{}: set_external_create_succeeded", tc.name);
    }
}

#[test]
fn get_external_create_failed_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: Option<Timestamp>,
    }
    let now = whole_second(Timestamp::now());
    let cases = vec![
        // go: pkg/meta TestGetExternalCreateFailed/ExternalCreateFailedExists
        Case {
            name: "ExternalCreateFailedExists",
            o: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_CREATE_FAILED, &rfc3339(now))]),
            want: Some(now),
        },
        // go: pkg/meta TestGetExternalCreateFailed/NoExternalCreateFailed
        Case {
            name: "NoExternalCreateFailed",
            o: Pod::default(),
            want: None,
        },
    ];
    for tc in cases {
        assert_eq!(
            tc.want,
            external_create_failed(&tc.o),
            "{}: external_create_failed",
            tc.name
        );
    }
}

#[test]
fn set_external_create_failed_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        t: Timestamp,
        want: Pod,
    }
    let now = Timestamp::now();
    let cases = vec![
        // go: pkg/meta TestSetExternalCreateFailed/SetsTheCorrectKey
        Case {
            name: "SetsTheCorrectKey",
            o: Pod::default(),
            t: now,
            want: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_CREATE_FAILED, &rfc3339(now))]),
        },
    ];
    for mut tc in cases {
        set_external_create_failed(&mut tc.o, tc.t);
        assert_eq!(tc.want, tc.o, "{}: set_external_create_failed", tc.name);
    }
}

#[test]
fn external_create_succeeded_during_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        d: Duration,
        want: bool,
    }
    let now = Timestamp::now();
    let succeeded_at = |ago: k8s_openapi::jiff::Span| {
        let mut o = Pod::default();
        set_external_create_succeeded(&mut o, now.checked_sub(ago).unwrap());
        o
    };
    let cases = vec![
        // go: pkg/meta TestExternalCreateSucceededDuring/NotYetSuccessfullyCreated
        Case {
            name: "NotYetSuccessfullyCreated",
            o: Pod::default(),
            d: Duration::from_secs(60),
            want: false,
        },
        // go: pkg/meta TestExternalCreateSucceededDuring/SuccessfullyCreatedTooLongAgo
        Case {
            name: "SuccessfullyCreatedTooLongAgo",
            o: succeeded_at(2.minutes()),
            d: Duration::from_secs(60),
            want: false,
        },
        // go: pkg/meta TestExternalCreateSucceededDuring/SuccessfullyCreatedWithinDuration
        Case {
            name: "SuccessfullyCreatedWithinDuration",
            o: succeeded_at(30.seconds()),
            d: Duration::from_secs(60),
            want: true,
        },
    ];
    for tc in cases {
        assert_eq!(
            tc.want,
            external_create_succeeded_during(&tc.o, tc.d, now),
            "{}: external_create_succeeded_during",
            tc.name
        );
    }
}

#[test]
fn external_create_incomplete_cases() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        o: Pod,
        want: bool,
    }
    let now = whole_second(Timestamp::now());
    let earlier = rfc3339(now.checked_sub(1.second()).unwrap());
    let even_earlier = rfc3339(now.checked_sub(1.minute()).unwrap());
    let now = rfc3339(now);
    let cases = vec![
        // go: pkg/meta TestExternalCreateIncomplete/CreateNeverPending
        Case {
            name: "CreateNeverPending",
            reason: "If we've never called Create it can't be incomplete.",
            o: Pod::default(),
            want: false,
        },
        // go: pkg/meta TestExternalCreateIncomplete/CreateSucceeded
        Case {
            name: "CreateSucceeded",
            reason: "If Create succeeded since it was pending, it's complete.",
            o: with_annotations(&[
                (ANNOTATION_KEY_EXTERNAL_CREATE_FAILED, &even_earlier),
                (ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, &earlier),
                (ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED, &now),
            ]),
            want: false,
        },
        // go: pkg/meta TestExternalCreateIncomplete/CreateFailed
        Case {
            name: "CreateFailed",
            reason: "If Create failed since it was pending, it's complete.",
            o: with_annotations(&[
                (ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED, &even_earlier),
                (ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, &earlier),
                (ANNOTATION_KEY_EXTERNAL_CREATE_FAILED, &now),
            ]),
            want: false,
        },
        // go: pkg/meta TestExternalCreateIncomplete/CreateNeverCompleted
        Case {
            name: "CreateNeverCompleted",
            reason: "If Create was pending but never succeeded or failed, it's incomplete.",
            o: with_annotations(&[(ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, &earlier)]),
            want: true,
        },
        // go: pkg/meta TestExternalCreateIncomplete/RecreateNeverCompleted
        Case {
            name: "RecreateNeverCompleted",
            reason: "If Create is pending and there's an older success we're probably trying to recreate a deleted external resource, and it's incomplete.",
            o: with_annotations(&[
                (ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED, &earlier),
                (ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, &now),
            ]),
            want: true,
        },
        // go: pkg/meta TestExternalCreateIncomplete/RetryNeverCompleted
        Case {
            name: "RetryNeverCompleted",
            reason: "If Create is pending and there's an older failure we're probably trying to recreate a deleted external resource, and it's incomplete.",
            o: with_annotations(&[
                (ANNOTATION_KEY_EXTERNAL_CREATE_FAILED, &earlier),
                (ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, &now),
            ]),
            want: true,
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, external_create_incomplete(&tc.o), "{}: {}", tc.name, tc.reason);
    }
}

#[test]
fn is_paused_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: bool,
    }
    let cases = vec![
        // go: pkg/meta TestIsPaused/HasPauseAnnotationSetTrue
        Case {
            name: "HasPauseAnnotationSetTrue",
            o: with_annotations(&[(ANNOTATION_KEY_RECONCILIATION_PAUSED, "true")]),
            want: true,
        },
        // go: pkg/meta TestIsPaused/NoPauseAnnotation
        Case {
            name: "NoPauseAnnotation",
            o: Pod::default(),
            want: false,
        },
        // go: pkg/meta TestIsPaused/HasEmptyPauseAnnotation
        Case {
            name: "HasEmptyPauseAnnotation",
            o: with_annotations(&[(ANNOTATION_KEY_RECONCILIATION_PAUSED, "")]),
            want: false,
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, is_paused(&tc.o), "{}: is_paused", tc.name);
    }
}

#[test]
fn get_poll_interval_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: Option<Duration>,
    }
    let poll = |v: &str| with_annotations(&[(ANNOTATION_KEY_POLL_INTERVAL, v)]);
    let cases = vec![
        // go: pkg/meta TestGetPollInterval/NoAnnotation
        Case {
            name: "NoAnnotation",
            o: Pod::default(),
            want: None,
        },
        // go: pkg/meta TestGetPollInterval/EmptyAnnotation
        Case {
            name: "EmptyAnnotation",
            o: poll(""),
            want: None,
        },
        // go: pkg/meta TestGetPollInterval/InvalidDuration
        Case {
            name: "InvalidDuration",
            o: poll("not-a-duration"),
            want: None,
        },
        // go: pkg/meta TestGetPollInterval/NegativeDuration
        Case {
            name: "NegativeDuration",
            o: poll("-5m"),
            want: None,
        },
        // go: pkg/meta TestGetPollInterval/ZeroDuration
        Case {
            name: "ZeroDuration",
            o: poll("0s"),
            want: None,
        },
        // go: pkg/meta TestGetPollInterval/ValidDuration
        Case {
            name: "ValidDuration",
            o: poll("24h"),
            want: Some(Duration::from_secs(24 * 3600)),
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, poll_interval(&tc.o), "{}: poll_interval", tc.name);
    }
}

#[test]
fn get_reconcile_request_cases() {
    struct Case {
        name: &'static str,
        o: Pod,
        want: Option<&'static str>,
    }
    let cases = vec![
        // go: pkg/meta TestGetReconcileRequest/NoAnnotation
        Case {
            name: "NoAnnotation",
            o: Pod::default(),
            want: None,
        },
        // go: pkg/meta TestGetReconcileRequest/EmptyAnnotation
        Case {
            name: "EmptyAnnotation",
            o: with_annotations(&[(ANNOTATION_KEY_RECONCILE_REQUESTED_AT, "")]),
            want: None,
        },
        // go: pkg/meta TestGetReconcileRequest/HasToken
        Case {
            name: "HasToken",
            o: with_annotations(&[(ANNOTATION_KEY_RECONCILE_REQUESTED_AT, "2024-01-15T10:30:00Z")]),
            want: Some("2024-01-15T10:30:00Z"),
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, reconcile_request(&tc.o), "{}: reconcile_request", tc.name);
    }
}

// go: pkg/meta TestSetReconcileRequest
#[test]
fn set_reconcile_request_sets_the_annotation() {
    let mut o = Pod::default();
    set_reconcile_request(&mut o, "my-token");
    assert_eq!(
        Some("my-token"),
        annotation(&o, ANNOTATION_KEY_RECONCILE_REQUESTED_AT),
        "set_reconcile_request"
    );
}

#[test]
fn create_stamps_are_second_precision_utc() {
    let mut o = Pod::default();
    let t: Timestamp = "2024-01-15T10:30:00.987Z".parse().unwrap();
    set_external_create_pending(&mut o, t);
    assert_eq!(
        Some("2024-01-15T10:30:00Z"),
        annotation(&o, ANNOTATION_KEY_EXTERNAL_CREATE_PENDING),
        "the stamp Go's time.RFC3339 writes in UTC"
    );
}

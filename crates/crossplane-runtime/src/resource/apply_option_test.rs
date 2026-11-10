use crossplane_core::errors::Error;
use crossplane_test::fake;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};

use super::*;

fn controlled_by<K>(mut o: K, uid: &str) -> K
where
    K: kube::Resource<DynamicType = ()>,
{
    o.meta_mut().owner_references = Some(vec![OwnerReference {
        uid: uid.to_owned(),
        controller: Some(true),
        ..Default::default()
    }]);
    o
}

fn secret(type_: &str, uid: Option<&str>) -> Secret {
    let s = Secret {
        metadata: ObjectMeta::default(),
        type_: Some(type_.to_owned()),
        ..Default::default()
    };
    match uid {
        Some(uid) => controlled_by(s, uid),
        None => s,
    }
}

/// Table rows compare the error and the predicate that Go's tests assert through `EquateErrors`'s
/// type check, which `Error`'s equality includes.
fn assert_option(name: &str, reason: &str, want: Option<Error>, got: Result<(), Error>) {
    crossplane_test::assert_result(name, reason, &want.map_or(Ok(()), Err), &got);
}

#[test]
fn is_not_controllable_by_error_kind() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        err: Error,
        want: bool,
    }
    let cases = vec![
        // go: pkg/resource TestIsNotControllable/UnknownError
        Case {
            name: "UnknownError",
            reason: "An error without the NotControllable marker does not indicate something is not controllable.",
            err: Error::new("boom"),
            want: false,
        },
        // go: pkg/resource TestIsNotControllable/NotControllableError
        Case {
            name: "NotControllableError",
            reason: "An error with the NotControllable marker indicates something is not controllable.",
            err: Error::with_kind(ErrorKind::NotControllable, "boom"),
            want: true,
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, is_not_controllable(&tc.err), "{}: {}", tc.name, tc.reason);
    }
}

// go: pkg/resource Test_notControllableError_NotControllable
#[test]
fn not_controllable_error_is_not_controllable() {
    assert!(is_not_controllable(&Error::with_kind(
        ErrorKind::NotControllable,
        "test-error"
    )));
}

// go: pkg/resource Test_notAllowedError_NotAllowed
#[test]
fn not_allowed_error_is_not_allowed() {
    assert!(is_not_allowed(&new_not_allowed("test-error")));
    assert!(!is_not_allowed(&Error::with_kind(
        ErrorKind::NotControllable,
        "test-error"
    )));
}

#[test]
fn markers_are_not_seen_through_a_wrap() {
    let wrapped = new_not_allowed("x").wrap("context");
    assert!(!is_not_allowed(&wrapped), "Go asserts on the error itself");
}

#[test]
fn must_be_controllable_by() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        current: fake::Object,
        want: Option<Error>,
    }
    let uid = "very-unique-string";
    let cases = vec![
        // go: pkg/resource TestMustBeControllableBy/Adoptable
        Case {
            name: "Adoptable",
            reason: "A current object with no controller reference may be adopted and controlled",
            current: fake::Object::default(),
            want: None,
        },
        // go: pkg/resource TestMustBeControllableBy/ControlledBySuppliedUID
        Case {
            name: "ControlledBySuppliedUID",
            reason: "A current object that is already controlled by the supplied UID is controllable",
            current: controlled_by(fake::Object::default(), uid),
            want: None,
        },
        // go: pkg/resource TestMustBeControllableBy/ControlledBySomeoneElse
        Case {
            name: "ControlledBySomeoneElse",
            reason: "A current object that is already controlled by a different UID is not controllable",
            current: controlled_by(fake::Object::default(), "some-other-uid"),
            want: Some(Error::with_kind(
                ErrorKind::NotControllable,
                format!("existing object is not controlled by UID {uid:?}"),
            )),
        },
    ];
    for mut tc in cases {
        let got = ApplyOption::MustBeControllableBy(uid.to_owned()).run(&mut tc.current, &fake::Object::default());
        assert_option(tc.name, tc.reason, tc.want, got);
    }
}

#[test]
fn connection_secret_must_be_controllable_by() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        current: Secret,
        want: Option<Error>,
    }
    let uid = "very-unique-string";
    let cases = vec![
        // go: pkg/resource TestConnectionSecretMustBeControllableBy/Adoptable
        Case {
            name: "Adoptable",
            reason: "A Secret of SecretTypeConnection with no controller reference may be adopted and controlled",
            current: secret(CONNECTION_SECRET_TYPE, None),
            want: None,
        },
        // go: pkg/resource TestConnectionSecretMustBeControllableBy/ControlledBySuppliedUID
        Case {
            name: "ControlledBySuppliedUID",
            reason: "A Secret of any type that is already controlled by the supplied UID is controllable",
            current: secret("Opaque", Some(uid)),
            want: None,
        },
        // go: pkg/resource TestConnectionSecretMustBeControllableBy/ControlledBySomeoneElse
        Case {
            name: "ControlledBySomeoneElse",
            reason: "A Secret of any type that is already controlled by the another UID is not controllable",
            current: secret(CONNECTION_SECRET_TYPE, Some("some-other-uid")),
            want: Some(Error::with_kind(
                ErrorKind::NotControllable,
                format!("existing secret is not controlled by UID {uid:?}"),
            )),
        },
        // go: pkg/resource TestConnectionSecretMustBeControllableBy/UncontrolledOpaqueSecret
        Case {
            name: "UncontrolledOpaqueSecret",
            reason: "A Secret of Opaque type with no controller is not controllable",
            current: secret("Opaque", None),
            want: Some(Error::with_kind(
                ErrorKind::NotControllable,
                "refusing to modify uncontrolled secret of type \"Opaque\"",
            )),
        },
    ];
    for mut tc in cases {
        let got =
            ApplyOption::ConnectionSecretMustBeControllableBy(uid.to_owned()).run(&mut tc.current, &Secret::default());
        assert_option(tc.name, tc.reason, tc.want, got);
    }
}

#[test]
fn connection_secret_option_refuses_a_non_secret() {
    let got = ApplyOption::ConnectionSecretMustBeControllableBy("u".into())
        .run(&mut fake::Object::default(), &fake::Object::default());
    assert_eq!(got, Err(Error::new("current resource is not a Secret")));
}

#[test]
fn allow_update_if() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        allowed: bool,
        want: Option<Error>,
    }
    let cases = vec![
        // go: pkg/resource TestAllowUpdateIf/Allowed
        Case {
            name: "Allowed",
            reason: "No error should be returned when the supplied function returns true",
            allowed: true,
            want: None,
        },
        // go: pkg/resource TestAllowUpdateIf/NotAllowed
        Case {
            name: "NotAllowed",
            reason: "An error that satisfies IsNotAllowed should be returned when the supplied function returns false",
            allowed: false,
            want: Some(new_not_allowed("update not allowed")),
        },
    ];
    for mut tc in cases {
        let allowed = tc.allowed;
        let got = ApplyOption::AllowUpdateIf(Box::new(move |_, _| allowed))
            .run(&mut fake::Object::default(), &fake::Object::default());
        assert_option(tc.name, tc.reason, tc.want, got);
    }
}

// go: pkg/resource TestUpdate/Update
#[test]
fn update_fn_changes_the_current_object() {
    let mut current = Secret {
        metadata: ObjectMeta {
            name: Some("current".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let desired = Secret {
        metadata: ObjectMeta {
            name: Some("desired".into()),
            ..Default::default()
        },
        string_data: Some([("key".to_owned(), "value".to_owned())].into()),
        ..Default::default()
    };
    let opt = ApplyOption::update_fn(|c, d| {
        let (c, d) = (c.downcast_mut::<Secret>().unwrap(), d.downcast_ref::<Secret>().unwrap());
        c.string_data = d.string_data.clone();
    });
    opt.run(&mut current, &desired).unwrap();
    let want = Secret {
        metadata: ObjectMeta {
            name: Some("current".into()),
            ..Default::default()
        },
        string_data: Some([("key".to_owned(), "value".to_owned())].into()),
        ..Default::default()
    };
    crossplane_test::assert_object_eq(&want, &current, "UpdateFn updated object mismatch");
}

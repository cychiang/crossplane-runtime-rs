use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crossplane_core::client::Patch;
use crossplane_core::errors::Error;
use crossplane_test::{self as test, MockClient, fake};

use super::*;

fn named(name: &str) -> fake::Object {
    let mut o = fake::Object::default();
    o.metadata.name = Some(name.to_owned());
    o
}

fn boom() -> Error {
    Error::new("boom")
}

struct Case {
    name: &'static str,
    reason: &'static str,
    client: MockClient,
    o: fake::Object,
    opts: Vec<ApplyOption>,
    want_err: Option<Error>,
    want_o: fake::Object,
}

fn option_error() -> Vec<ApplyOption> {
    vec![ApplyOption::Custom(Box::new(|_, _| Err(boom())))]
}

async fn run_cases(cases: Vec<Case>, patching: bool) {
    for mut tc in cases {
        let client = Arc::new(tc.client);
        let got = if patching {
            ApiPatchingApplicator::new(client).apply(&mut tc.o, &tc.opts).await
        } else {
            ApiUpdatingApplicator::new(client).apply(&mut tc.o, &tc.opts).await
        };
        test::assert_result(tc.name, tc.reason, &tc.want_err.map_or(Ok(()), Err), &got);
        test::assert_object_eq(&tc.want_o, &tc.o, tc.reason);
    }
}

#[tokio::test]
async fn api_patching_applicator() {
    let cases = vec![
        // go: pkg/resource TestAPIPatchingApplicator/GetError
        Case {
            name: "GetError",
            reason: "An error should be returned if we can't get the object",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(boom())),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: vec![],
            want_err: Some(boom().wrap("cannot get object")),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIPatchingApplicator/CreateError
        Case {
            name: "CreateError",
            reason: "An error should be returned if we can't create the object",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(Error::not_found())),
                create: Some(test::new_mock_create_fn(Some(boom()))),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: vec![],
            want_err: Some(boom().wrap("cannot create object")),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIPatchingApplicator/ApplyOptionError
        Case {
            name: "ApplyOptionError",
            reason: "Any errors from an apply option should be returned",
            client: MockClient {
                get: Some(test::new_mock_get_fn(fake::Object::default)),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: option_error(),
            want_err: Some(boom()),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIPatchingApplicator/PatchError
        Case {
            name: "PatchError",
            reason: "An error should be returned if we can't patch the object",
            client: MockClient {
                get: Some(test::new_mock_get_fn(fake::Object::default)),
                patch: Some(test::new_mock_patch_fn(Some(boom()))),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: vec![],
            want_err: Some(boom().wrap("cannot patch object")),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIPatchingApplicator/Created
        Case {
            name: "Created",
            reason: "No error should be returned if we successfully create a new object",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(Error::not_found())),
                create: Some(test::new_mock_create_fn(None)),
                ..Default::default()
            },
            o: named("desired"),
            opts: vec![],
            want_err: None,
            want_o: named("desired"),
        },
        // go: pkg/resource TestAPIPatchingApplicator/Patched
        Case {
            name: "Patched",
            reason: "No error should be returned if we successfully patch an existing object",
            client: MockClient {
                get: Some(test::new_mock_get_fn(|| named("desired"))),
                patch: Some(test::new_mock_patch_fn(None)),
                ..Default::default()
            },
            o: named("desired"),
            opts: vec![],
            want_err: None,
            want_o: named("desired"),
        },
    ];
    run_cases(cases, true).await;
}

#[tokio::test]
async fn api_updating_applicator() {
    let cases = vec![
        // go: pkg/resource TestAPIUpdatingApplicator/GetError
        Case {
            name: "GetError",
            reason: "An error should be returned if we can't get the object",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(boom())),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: vec![],
            want_err: Some(boom().wrap("cannot get object")),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIUpdatingApplicator/CreateError
        Case {
            name: "CreateError",
            reason: "An error should be returned if we can't create the object",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(Error::not_found())),
                create: Some(test::new_mock_create_fn(Some(boom()))),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: vec![],
            want_err: Some(boom().wrap("cannot create object")),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIUpdatingApplicator/ApplyOptionError
        Case {
            name: "ApplyOptionError",
            reason: "Any errors from an apply option should be returned",
            client: MockClient {
                get: Some(test::new_mock_get_fn(fake::Object::default)),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: option_error(),
            want_err: Some(boom()),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIUpdatingApplicator/UpdateError
        Case {
            name: "UpdateError",
            reason: "An error should be returned if we can't update the object",
            client: MockClient {
                get: Some(test::new_mock_get_fn(fake::Object::default)),
                update: Some(test::new_mock_update_fn(Some(boom()))),
                ..Default::default()
            },
            o: fake::Object::default(),
            opts: vec![],
            want_err: Some(boom().wrap("cannot update object")),
            want_o: fake::Object::default(),
        },
        // go: pkg/resource TestAPIUpdatingApplicator/Created
        Case {
            name: "Created",
            reason: "No error should be returned if we successfully create a new object",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(Error::not_found())),
                create: Some(test::new_mock_create_fn(None)),
                ..Default::default()
            },
            o: named("desired"),
            opts: vec![],
            want_err: None,
            want_o: named("desired"),
        },
        // go: pkg/resource TestAPIUpdatingApplicator/Updated
        Case {
            name: "Updated",
            reason: "No error should be returned if we successfully update an existing object. If no ApplyOption is passed the existing should not be modified",
            client: MockClient {
                get: Some(test::new_mock_get_fn(|| named("current"))),
                update: Some(test::new_mock_object_fn(None, |o| {
                    test::assert_object_eq(&named("desired"), o, "the update carries the desired object");
                    Ok(())
                })),
                ..Default::default()
            },
            o: named("desired"),
            opts: vec![],
            want_err: None,
            want_o: named("desired"),
        },
    ];
    run_cases(cases, false).await;
}

#[tokio::test]
async fn a_generate_name_object_is_created_without_a_get() {
    let mut o = fake::Object::default();
    o.metadata.generate_name = Some("gen-".into());
    let client = MockClient {
        create: Some(test::new_mock_create_fn(None)),
        ..Default::default()
    };
    ApiPatchingApplicator::new(Arc::new(client))
        .apply(&mut o, &[])
        .await
        .unwrap();
    let client = MockClient {
        create: Some(test::new_mock_create_fn(None)),
        ..Default::default()
    };
    ApiUpdatingApplicator::new(Arc::new(client))
        .apply(&mut o, &[])
        .await
        .unwrap();
}

#[tokio::test]
async fn the_patch_is_the_whole_desired_object_and_options_see_the_server_copy() {
    let mut server = named("desired");
    server.metadata.resource_version = Some("7".into());
    let client = MockClient {
        get: Some(test::new_mock_get_fn(move || server.clone())),
        patch: Some(Box::new(|o, patch| {
            assert_eq!(o.meta().resource_version.as_deref(), Some("7"));
            let Patch::Merge(body) = patch else {
                panic!("merge patch expected")
            };
            assert_eq!(body["metadata"]["name"], "desired");
            assert!(body["metadata"].get("resourceVersion").is_none(), "{body}");
            Ok(())
        })),
        ..Default::default()
    };
    let seen = ApplyOption::Custom(Box::new(|current, desired| {
        assert_eq!(current.meta().resource_version.as_deref(), Some("7"));
        assert_eq!(desired.meta().resource_version, None);
        Ok(())
    }));
    ApiPatchingApplicator::new(Arc::new(client))
        .apply(&mut named("desired"), &[seen])
        .await
        .unwrap();
}

#[tokio::test]
async fn the_update_carries_the_current_resource_version() {
    let mut server = named("desired");
    server.metadata.resource_version = Some("9".into());
    let client = MockClient {
        get: Some(test::new_mock_get_fn(move || server.clone())),
        update: Some(Box::new(|o| {
            assert_eq!(o.meta().resource_version.as_deref(), Some("9"));
            Ok(())
        })),
        ..Default::default()
    };
    ApiUpdatingApplicator::new(Arc::new(client))
        .apply(&mut named("desired"), &[])
        .await
        .unwrap();
}

#[test]
fn new_applicator_with_retry() {
    struct Case {
        name: &'static str,
        backoff: Option<Backoff>,
        want: Backoff,
    }
    let test_backoff = Backoff::default();
    let cases = vec![
        // go: pkg/resource TestNewApplicatorWithRetry/DefaultBackoff
        Case {
            name: "DefaultBackoff",
            backoff: None,
            want: DEFAULT_RETRY,
        },
        // go: pkg/resource TestNewApplicatorWithRetry/CustomBackoff
        Case {
            name: "CustomBackoff",
            backoff: Some(test_backoff),
            want: test_backoff,
        },
    ];
    for tc in cases {
        let awr = ApplicatorWithRetry::new(Arc::new(ApplyFn(Box::new(|_, _| Ok(())))), |_| false, tc.backoff);
        assert_eq!(&tc.want, awr.backoff(), "{}", tc.name);
    }
}

#[tokio::test]
async fn applicator_with_retry_apply() {
    struct Case {
        name: &'static str,
        returns_error: bool,
        retry: bool,
        want_err: Option<Error>,
        want_count: u32,
    }
    let cases = vec![
        // go: pkg/resource TestApplicatorWithRetry_Apply/NoRetry
        Case {
            name: "NoRetry",
            returns_error: true,
            retry: false,
            want_err: Some(Error::new("test-error")),
            want_count: 1,
        },
        // go: pkg/resource TestApplicatorWithRetry_Apply/ShouldRetry
        Case {
            name: "ShouldRetry",
            returns_error: true,
            retry: true,
            want_err: Some(Error::new("test-error")),
            want_count: 3,
        },
        // go: pkg/resource TestApplicatorWithRetry_Apply/NoError
        Case {
            name: "NoError",
            returns_error: false,
            retry: true,
            want_err: None,
            want_count: 1,
        },
    ];
    for tc in cases {
        let count = Arc::new(AtomicU32::new(0));
        let counter = count.clone();
        let returns_error = tc.returns_error;
        let inner = ApplyFn(Box::new(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            if returns_error {
                Err(Error::new("test-error"))
            } else {
                Ok(())
            }
        }));
        let retry = tc.retry;
        let awr = ApplicatorWithRetry::new(
            Arc::new(inner),
            move |_| retry,
            Some(Backoff {
                steps: 3,
                ..Default::default()
            }),
        );
        let got = awr.apply(&mut fake::Object::default(), &[]).await;
        test::assert_result(tc.name, "", &tc.want_err.map_or(Ok(()), Err), &got);
        assert_eq!(tc.want_count, count.load(Ordering::SeqCst), "{}: attempts", tc.name);
    }
}

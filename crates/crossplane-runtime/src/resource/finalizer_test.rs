use std::sync::Arc;

use crossplane_core::errors::Error;
use crossplane_test::{self as test, MockClient, fake};

use super::*;

fn with_finalizers(f: &[&str]) -> fake::Object {
    let mut o = fake::Object::default();
    o.metadata.finalizers = (!f.is_empty()).then(|| f.iter().map(|s| (*s).to_owned()).collect());
    o
}

struct Case {
    name: &'static str,
    update: Option<Error>,
    obj: fake::Object,
    want_err: Option<Error>,
    want_obj: fake::Object,
}

#[tokio::test]
async fn managed_remove_finalizer() {
    let finalizer = "veryfinal";
    let boom = || Error::new("boom");
    let cases = vec![
        // go: pkg/resource TestManagedRemoveFinalizer/UpdateError
        Case {
            name: "UpdateError",
            update: Some(boom()),
            obj: with_finalizers(&[finalizer]),
            want_err: Some(boom().wrap("cannot update object")),
            want_obj: with_finalizers(&[]),
        },
        // go: pkg/resource TestManagedRemoveFinalizer/Successful
        Case {
            name: "Successful",
            update: None,
            obj: with_finalizers(&[finalizer]),
            want_err: None,
            want_obj: with_finalizers(&[]),
        },
    ];
    for mut tc in cases {
        let client = MockClient {
            update: Some(test::new_mock_update_fn(tc.update)),
            ..Default::default()
        };
        let api = ApiFinalizer::new(Arc::new(client), finalizer);
        let got = api.remove_finalizer(&mut tc.obj).await;
        test::assert_result(tc.name, "", &tc.want_err.map_or(Ok(()), Err), &got);
        test::assert_object_eq(&tc.want_obj, &tc.obj, tc.name);
    }
}

#[tokio::test]
async fn api_finalizer_adder() {
    let finalizer = "veryfinal";
    let boom = || Error::new("boom");
    let cases = vec![
        // go: pkg/resource TestAPIFinalizerAdder/UpdateError
        Case {
            name: "UpdateError",
            update: Some(boom()),
            obj: with_finalizers(&[]),
            want_err: Some(boom().wrap("cannot update object")),
            want_obj: with_finalizers(&[finalizer]),
        },
        // go: pkg/resource TestAPIFinalizerAdder/Successful
        Case {
            name: "Successful",
            update: None,
            obj: with_finalizers(&[]),
            want_err: None,
            want_obj: with_finalizers(&[finalizer]),
        },
    ];
    for mut tc in cases {
        let client = MockClient {
            update: Some(test::new_mock_update_fn(tc.update)),
            ..Default::default()
        };
        let api = ApiFinalizer::new(Arc::new(client), finalizer);
        let got = api.add_finalizer(&mut tc.obj).await;
        test::assert_result(tc.name, "", &tc.want_err.map_or(Ok(()), Err), &got);
        test::assert_object_eq(&tc.want_obj, &tc.obj, tc.name);
    }
}

#[tokio::test]
async fn finalizer_no_ops_make_no_write_and_not_found_is_ignored() {
    let api = ApiFinalizer::new(Arc::new(MockClient::default()), "f");
    api.add_finalizer(&mut with_finalizers(&["f"])).await.unwrap();
    api.remove_finalizer(&mut with_finalizers(&[])).await.unwrap();

    let client = MockClient {
        update: Some(test::new_mock_update_fn(Some(Error::not_found()))),
        ..Default::default()
    };
    let api = ApiFinalizer::new(Arc::new(client), "f");
    api.remove_finalizer(&mut with_finalizers(&["f"])).await.unwrap();
}

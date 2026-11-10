//! Port of `pkg/reconciler/managed/api_test.go`. Go's tables use `fake.LegacyManaged`; the port
//! has modern kinds only, so they run on `fake.ModernManaged`.

use std::borrow::Cow;
use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::{Error, Result};
use crossplane_core::meta;
use crossplane_core::resource::{Managed, Object};
use crossplane_core::v2::{ConnectionDetails, LocalSecretReference, ManagedResourceSpec, ManagedResourceStatus};
use crossplane_test::{self as test, MockClient};
use futures::FutureExt;
use futures::future::BoxFuture;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Resource;
use kube::core::NamespaceResourceScope;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::collaborators::prepare_json_merge;
use super::reconciler_modern_test::{Mg, boom};
use super::*;
use crate::resource::{ApplyFn, local_connection_secret_for};

fn named(name: &str) -> Mg {
    let mut mg = Mg::default();
    mg.metadata.name = Some(name.to_owned());
    mg
}

fn with_external_name(mut mg: Mg, external_name: &str) -> Mg {
    meta::set_external_name(&mut mg, external_name);
    mg
}

#[tokio::test]
async fn name_as_external_name() {
    struct Case {
        name: &'static str,
        client: MockClient,
        mg: Mg,
        want: Result<()>,
        want_mg: Mg,
    }

    const TEST_EXTERNAL_NAME: &str = "my-external-name";

    let cases = [
        // go: pkg/reconciler/managed TestNameAsExternalName/UpdateManagedError
        Case {
            name: "UpdateManagedError",
            client: MockClient {
                update: Some(test::new_mock_update_fn(Some(boom()))),
                ..Default::default()
            },
            mg: named(TEST_EXTERNAL_NAME),
            want: Err(boom().wrap(ERR_UPDATE_MANAGED)),
            want_mg: with_external_name(named(TEST_EXTERNAL_NAME), TEST_EXTERNAL_NAME),
        },
        // go: pkg/reconciler/managed TestNameAsExternalName/UpdateSuccessful
        Case {
            name: "UpdateSuccessful",
            client: MockClient {
                update: Some(test::new_mock_update_fn(None)),
                ..Default::default()
            },
            mg: named(TEST_EXTERNAL_NAME),
            want: Ok(()),
            want_mg: with_external_name(named(TEST_EXTERNAL_NAME), TEST_EXTERNAL_NAME),
        },
        // go: pkg/reconciler/managed TestNameAsExternalName/UpdateNotNeeded
        Case {
            name: "UpdateNotNeeded",
            client: MockClient::default(),
            mg: with_external_name(named(TEST_EXTERNAL_NAME), "some-name"),
            want: Ok(()),
            want_mg: with_external_name(named(TEST_EXTERNAL_NAME), "some-name"),
        },
    ];

    for mut tc in cases {
        let api = NameAsExternalName::new(Arc::new(tc.client));
        let got = api.initialize(&mut tc.mg).await;
        test::assert_result(tc.name, "api.Initialize(...)", &tc.want, &got);
        test::assert_object_eq(&tc.want_mg, &tc.mg, tc.name);
    }
}

#[tokio::test]
async fn api_local_secret_publisher() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        secret: ApplyFn,
        mg: Mg,
        c: ConnectionDetails,
        want: Result<bool>,
    }

    fn publishing() -> Mg {
        let mut mg = Mg::default();
        mg.set_write_connection_secret_to_reference(Some(LocalSecretReference {
            name: "coolsecret".to_owned(),
        }));
        mg
    }
    fn cd() -> ConnectionDetails {
        [("cool".to_owned(), vec![42])].into()
    }

    let cases = [
        // go: pkg/reconciler/managed TestAPILocalSecretPublisher/ResourceDoesNotPublishSecret
        Case {
            name: "ResourceDoesNotPublishSecret",
            reason: "A managed resource with a nil GetWriteConnectionSecretToReference should not publish a secret",
            secret: ApplyFn(Box::new(|_, _| panic!("unexpected call to Applicator::apply"))),
            mg: Mg::default(),
            c: ConnectionDetails::new(),
            want: Ok(false),
        },
        // go: pkg/reconciler/managed TestAPILocalSecretPublisher/ApplyError
        Case {
            name: "ApplyError",
            reason: "An error applying the connection secret should be returned",
            secret: ApplyFn(Box::new(|_, _| Err(boom()))),
            mg: publishing(),
            c: ConnectionDetails::new(),
            want: Err(boom().wrap(ERR_CREATE_OR_UPDATE_SECRET)),
        },
        // go: pkg/reconciler/managed TestAPILocalSecretPublisher/AlreadyPublished
        Case {
            name: "AlreadyPublished",
            reason: "An up to date connection secret should result in no error and not being published",
            // The applicator finds the wanted Secret already there and runs the options on it.
            secret: ApplyFn(Box::new(|o, ao| {
                let mut current = local_connection_secret_for(&publishing(), &cd()).unwrap();
                ao.iter().try_for_each(|option| option.run(&mut current, &*o))
            })),
            mg: publishing(),
            c: cd(),
            want: Ok(false),
        },
        // go: pkg/reconciler/managed TestAPILocalSecretPublisher/Success
        Case {
            name: "Success",
            reason: "A successful application of the connection secret should result in no error",
            secret: ApplyFn(Box::new(|o, _| {
                let want = local_connection_secret_for(&publishing(), &cd()).unwrap();
                test::assert_object_eq(&want, &*o, "the applied Secret");
                Ok(())
            })),
            mg: publishing(),
            c: cd(),
            want: Ok(true),
        },
    ];

    for tc in cases {
        let a = ApiLocalSecretPublisher::with_applicator(tc.secret);
        let got = a.publish_connection(&tc.mg, &tc.c).await;
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

/// Go's `mockSimpleReferencer`: a managed kind that has references, resolved by `resolve`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Referencer {
    metadata: ObjectMeta,
    #[serde(default)]
    spec: ManagedResourceSpec,
    #[serde(default)]
    status: ManagedResourceStatus,
    #[serde(skip, default = "resolves_nothing")]
    resolve: fn(&mut Referencer) -> Result<()>,
}

fn resolves_nothing() -> fn(&mut Referencer) -> Result<()> {
    |_| Ok(())
}

fn referencer(resolve: fn(&mut Referencer) -> Result<()>) -> Referencer {
    Referencer {
        metadata: ObjectMeta::default(),
        spec: ManagedResourceSpec::default(),
        status: ManagedResourceStatus::default(),
        resolve,
    }
}

impl Resource for Referencer {
    type DynamicType = ();
    type Scope = NamespaceResourceScope;

    fn kind(_: &()) -> Cow<'_, str> {
        "Referencer".into()
    }
    fn group(_: &()) -> Cow<'_, str> {
        "fake.crossplane.io".into()
    }
    fn version(_: &()) -> Cow<'_, str> {
        "v1".into()
    }
    fn plural(_: &()) -> Cow<'_, str> {
        "referencers".into()
    }
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

impl Managed for Referencer {
    fn managed_spec(&self) -> &ManagedResourceSpec {
        &self.spec
    }
    fn managed_spec_mut(&mut self) -> &mut ManagedResourceSpec {
        &mut self.spec
    }
    fn managed_status(&self) -> Option<&ManagedResourceStatus> {
        Some(&self.status)
    }
    fn managed_status_mut(&mut self) -> &mut ManagedResourceStatus {
        &mut self.status
    }
    fn resolve_references<'a>(&'a mut self, _: &'a dyn Client) -> Option<BoxFuture<'a, Result<()>>> {
        let resolved = (self.resolve)(self);
        Some(futures::future::ready(resolved).boxed())
    }
}

// go: pkg/reconciler/managed TestResolveReferences/NoReferencersFound
#[tokio::test]
async fn resolve_references_no_referencers_found() {
    let r = ApiSimpleReferenceResolver::new(Arc::new(MockClient::default()));
    let got = r.resolve_references(&mut Mg::default()).await;
    test::assert_result(
        "NoReferencersFound",
        "Should return early without error when the managed resource has no references.",
        &Ok(()),
        &got,
    );
}

#[tokio::test]
async fn resolve_references() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        c: MockClient,
        mg: Referencer,
        want: Result<()>,
    }

    let cases = [
        // go: pkg/reconciler/managed TestResolveReferences/ResolveReferencesError
        Case {
            name: "ResolveReferencesError",
            reason: "Should return errors encountered while resolving references.",
            c: MockClient {
                update: Some(test::new_mock_update_fn(None)),
                ..Default::default()
            },
            mg: referencer(|_| Err(boom())),
            want: Err(boom().wrap(ERR_RESOLVE_REFERENCES)),
        },
        // go: pkg/reconciler/managed TestResolveReferences/SuccessfulNoop
        Case {
            name: "SuccessfulNoop",
            reason: "Should return without error when resolution does not change the managed resource.",
            c: MockClient {
                update: Some(test::new_mock_update_fn(None)),
                ..Default::default()
            },
            mg: referencer(|_| Ok(())),
            want: Ok(()),
        },
        // go: pkg/reconciler/managed TestResolveReferences/SuccessfulUpdate
        Case {
            name: "SuccessfulUpdate",
            reason: "Should return without error when a value is successfully resolved.",
            c: MockClient {
                patch: Some(test::new_mock_patch_fn(None)),
                ..Default::default()
            },
            mg: referencer(|mg| {
                mg.metadata.name = Some("I'm different!".to_owned());
                Ok(())
            }),
            want: Ok(()),
        },
        // go: pkg/reconciler/managed TestResolveReferences/PatchError
        Case {
            name: "PatchError",
            reason: "Should return an error when the managed resource cannot be updated.",
            c: MockClient {
                patch: Some(test::new_mock_patch_fn(Some(boom()))),
                ..Default::default()
            },
            mg: referencer(|mg| {
                mg.metadata.name = Some("I'm different-er!".to_owned());
                Ok(())
            }),
            want: Err(boom().wrap(ERR_PATCH_MANAGED)),
        },
    ];

    for mut tc in cases {
        let r = ApiSimpleReferenceResolver::new(Arc::new(tc.c));
        let got = r.resolve_references(&mut tc.mg).await;
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

/// Go's fake has neither TypeMeta nor a `metadata` wrapper, so its patch is `{"name":"resolved"}`.
/// A real kind's patch nests the name and carries `apiVersion` and `kind`, which is what
/// `prepareJSONMerge` clears the existing object's type for.
// go: pkg/reconciler/managed TestPrepareJSONMerge/SuccessfulPatch
#[test]
fn prepare_json_merge_successful_patch() {
    let existing = Mg::default().to_json().unwrap();
    let resolved = named("resolved").to_json().unwrap();
    let want = json!({
        "apiVersion": "fake.crossplane.io/v1",
        "kind": "ModernManaged",
        "metadata": {"name": "resolved"},
    });
    assert_eq!(
        want,
        prepare_json_merge(existing, &resolved),
        "Should successfully compute the JSON merge patch document."
    );
}

#[tokio::test(start_paused = true)]
async fn retrying_critical_annotation_updater() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        c: MockClient,
        want: Result<()>,
        want_o: Mg,
    }

    fn object_returned_by_get() -> Mg {
        let mut mg = Mg::default();
        mg.metadata.labels = Some([("getcalled".to_owned(), "true".to_owned())].into());
        mg
    }

    let cases = [
        // The Go mock sets labels on the object and then fails the Get, so Go wants the labels.
        // `Client::get` returns an object or an error, never both, so the object is untouched.
        // go: pkg/reconciler/managed TestRetryingCriticalAnnotationUpdater/UpdateConflictGetError
        Case {
            name: "UpdateConflictGetError",
            reason: "We should return any error we encounter getting the supplied object",
            c: MockClient {
                get: Some(test::new_mock_get_err_fn(boom())),
                update: Some(test::new_mock_update_fn(Some(Error::conflict()))),
                ..Default::default()
            },
            want: Err(boom().wrap(ERR_UPDATE_CRITICAL_ANNOTATIONS)),
            want_o: Mg::default(),
        },
        // go: pkg/reconciler/managed TestRetryingCriticalAnnotationUpdater/UpdateError
        Case {
            name: "UpdateError",
            reason: "We should return any error we encounter updating the supplied object",
            c: MockClient {
                get: Some(test::new_mock_get_fn(object_returned_by_get)),
                update: Some(test::new_mock_update_fn(Some(boom()))),
                ..Default::default()
            },
            want: Err(boom().wrap(ERR_UPDATE_CRITICAL_ANNOTATIONS)),
            want_o: Mg::default(),
        },
        // go: pkg/reconciler/managed TestRetryingCriticalAnnotationUpdater/SuccessfulGetAfterAConflict
        Case {
            name: "SuccessfulGetAfterAConflict",
            reason: "A successful get after a conflict should not hide the conflict error and prevent retries",
            c: MockClient {
                get: Some(test::new_mock_get_fn(object_returned_by_get)),
                update: Some(test::new_mock_update_fn(Some(Error::conflict()))),
                ..Default::default()
            },
            want: Err(Error::conflict().wrap(ERR_UPDATE_CRITICAL_ANNOTATIONS)),
            want_o: object_returned_by_get(),
        },
        // The Go case is named Success but its update fails, exactly as in UpdateError.
        // go: pkg/reconciler/managed TestRetryingCriticalAnnotationUpdater/Success
        Case {
            name: "Success",
            reason: "We should return without error if we successfully update our annotations",
            c: MockClient {
                get: Some(test::new_mock_get_fn(object_returned_by_get)),
                update: Some(test::new_mock_update_fn(Some(boom()))),
                ..Default::default()
            },
            want: Err(boom().wrap(ERR_UPDATE_CRITICAL_ANNOTATIONS)),
            want_o: Mg::default(),
        },
    ];

    for tc in cases {
        let u = RetryingCriticalAnnotationUpdater::new(Arc::new(tc.c));
        let mut o = Mg::default();
        let got = u.update_critical_annotations(&mut o).await;
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
        test::assert_object_eq(&tc.want_o, &o, tc.reason);
    }
}

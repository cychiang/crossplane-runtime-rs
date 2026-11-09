//! The test-support crate as a table port uses it: the mock client, the helper constructors, the
//! comparison helpers and the fakes.

use std::panic::{AssertUnwindSafe, catch_unwind};

use crossplane_core::client::{Client, ListOptions, Patch};
use crossplane_core::errors::{Error, Result, StatusReason};
use crossplane_core::resource::{
    Managed, Object, ObjectKey, ObjectType, ProviderConfig as _, ProviderConfigUsage as _,
};
use crossplane_core::v2::{
    Condition, LocalSecretReference, ManagementAction, ManagementPolicies, ProviderConfigReference,
    TypedProviderConfigUsage, TypedReference,
};
use crossplane_test::fake::{self, ModernManaged, ProviderConfig, ProviderConfigUsage, new_modern_managed};
use crossplane_test::{self as test, MockClient};
use futures::executor::block_on;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde_json::json;

fn panic_message(f: impl FnOnce()) -> String {
    let payload = catch_unwind(AssertUnwindSafe(f)).expect_err("the call should panic");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

#[test]
fn an_unset_operation_fails_the_test_naming_the_verb() {
    let c = MockClient::default();
    let t = ObjectType::of::<ModernManaged>();
    let key = ObjectKey::default();
    let mg = new_modern_managed(1);
    let calls: Vec<(&str, Box<dyn FnOnce()>)> = vec![
        ("get", Box::new(|| drop(block_on(c.get(&t, &key))))),
        ("list", Box::new(|| drop(block_on(c.list(&t, &ListOptions::default()))))),
        ("create", Box::new(|| drop(block_on(c.create(&mut mg.clone()))))),
        ("update", Box::new(|| drop(block_on(c.update(&mut mg.clone()))))),
        (
            "update_status",
            Box::new(|| drop(block_on(c.update_status(&mut mg.clone())))),
        ),
        (
            "patch",
            Box::new(|| drop(block_on(c.patch(&mut mg.clone(), Patch::Merge(json!({})))))),
        ),
        ("delete", Box::new(|| drop(block_on(c.delete(&mg))))),
    ];
    for (verb, call) in calls {
        let msg = panic_message(call);
        assert!(
            msg.contains(&format!("MockClient::{verb} was called")),
            "{verb}: panic message {msg:?} does not name the verb"
        );
    }
}

#[tokio::test]
async fn get_as_downcasts_what_the_mock_returns() {
    let c: Box<dyn Client> = Box::new(MockClient {
        get: Some(test::new_mock_get_fn(|| new_modern_managed(42))),
        ..Default::default()
    });
    let got: ModernManaged = c.get_as(&ObjectKey::namespaced("ns", "a")).await.unwrap();
    test::assert_object_eq(&new_modern_managed(42), &got, "get_as returns the mock's object");
}

#[tokio::test]
async fn get_as_of_the_wrong_kind_panics_with_both_types() {
    let c: Box<dyn Client> = Box::new(MockClient {
        get: Some(test::new_mock_get_fn(fake::Object::default)),
        ..Default::default()
    });
    let got = tokio::spawn(async move { c.get_as::<ModernManaged>(&ObjectKey::default()).await.map(|_| ()) }).await;
    let msg = *got.expect_err("panicked").into_panic().downcast::<String>().unwrap();
    assert!(
        msg.contains("for a get of fake.crossplane.io/v1, Kind=ModernManaged"),
        "{msg}"
    );
}

#[tokio::test]
async fn list_as_downcasts_every_item_and_sees_the_options() {
    let c: Box<dyn Client> = Box::new(MockClient {
        list: Some(Box::new(|object_type, options| {
            assert_eq!(object_type, &ObjectType::of::<ProviderConfigUsage>());
            assert_eq!(options.namespace.as_deref(), Some("test-ns"), "Go InNamespace");
            assert_eq!(
                options.labels.get("crossplane.io/provider-config").map(String::as_str),
                Some("pc")
            );
            (test::new_mock_list_fn(|| vec![ProviderConfigUsage::default(), ProviderConfigUsage::default()]))(
                object_type,
                options,
            )
        })),
        ..Default::default()
    });
    let options = ListOptions {
        namespace: Some("test-ns".into()),
        labels: [("crossplane.io/provider-config".to_owned(), "pc".to_owned())].into(),
    };
    let got: Vec<ProviderConfigUsage> = c.list_as(&options).await.unwrap();
    assert_eq!(got, vec![ProviderConfigUsage::default(); 2]);
}

#[tokio::test]
async fn error_helpers_return_their_error() {
    let boom = || Error::new("boom");
    let c = MockClient {
        get: Some(test::new_mock_get_err_fn(boom())),
        list: Some(test::new_mock_list_err_fn(boom())),
        create: Some(test::new_mock_create_fn(Some(boom()))),
        update: Some(test::new_mock_update_fn(Some(boom()))),
        update_status: Some(test::new_mock_sub_resource_update_fn(Some(boom()))),
        patch: Some(test::new_mock_patch_fn(Some(boom()))),
        delete: Some(test::new_mock_delete_fn(Some(boom()))),
    };
    let t = ObjectType::of::<ModernManaged>();
    let mut mg = new_modern_managed(1);
    let want: Result<()> = Err(boom());
    test::assert_result("get", "", &want, &c.get(&t, &ObjectKey::default()).await.map(drop));
    test::assert_result("list", "", &want, &c.list(&t, &ListOptions::default()).await.map(drop));
    test::assert_result("create", "", &want, &c.create(&mut mg).await);
    test::assert_result("update", "", &want, &c.update(&mut mg).await);
    test::assert_result("update_status", "", &want, &c.update_status(&mut mg).await);
    test::assert_result("patch", "", &want, &c.patch(&mut mg, Patch::Merge(json!({}))).await);
    test::assert_result("delete", "", &want, &c.delete(&mg).await);
}

#[tokio::test]
async fn nil_error_helpers_succeed_and_leave_the_object_alone() {
    let c = MockClient {
        get: Some(test::new_mock_get_empty_fn()),
        list: Some(test::new_mock_list_fn(Vec::<ModernManaged>::new)),
        create: Some(test::new_mock_create_fn(None)),
        update: Some(test::new_mock_update_fn(None)),
        update_status: Some(test::new_mock_sub_resource_update_fn(None)),
        patch: Some(test::new_mock_patch_fn(None)),
        delete: Some(test::new_mock_delete_fn(None)),
    };
    let t = ObjectType::of::<ModernManaged>();
    let got = c.get(&t, &ObjectKey::namespaced("ns", "a")).await.unwrap();
    test::assert_object_eq(
        &ModernManaged::default(),
        &*got,
        "Go NewMockGetFn(nil) leaves the object empty",
    );
    assert!(c.list(&t, &ListOptions::default()).await.unwrap().is_empty());

    let mut mg = new_modern_managed(7);
    mg.set_conditions([Condition::available()]);
    let before = mg.clone();
    c.create(&mut mg).await.unwrap();
    c.update(&mut mg).await.unwrap();
    c.update_status(&mut mg).await.unwrap();
    c.patch(&mut mg, Patch::Merge(json!({"a": 1}))).await.unwrap();
    c.delete(&mg).await.unwrap();
    test::assert_object_eq(
        &before,
        &mg,
        "a write with no response leaves the object as the caller left it",
    );
}

#[test]
fn get_empty_panics_for_a_kind_that_cannot_be_empty() {
    let get = test::new_mock_get_empty_fn();
    let msg = panic_message(|| drop(get(&ObjectType::of::<ProviderConfigUsage>(), &ObjectKey::default())));
    assert!(msg.contains("Kind=ProviderConfigUsage cannot be built empty"), "{msg}");
}

#[tokio::test]
async fn object_fn_runs_before_the_error_is_returned() {
    let c = MockClient {
        update: Some(test::new_mock_object_fn(Some(Error::conflict()), |obj| {
            obj.meta_mut().resource_version = Some("seen".into());
            Ok(())
        })),
        ..Default::default()
    };
    let mut mg = new_modern_managed(1);
    let got = c.update(&mut mg).await;
    assert!(got.unwrap_err().is_conflict());
    assert_eq!(mg.metadata.resource_version.as_deref(), Some("seen"));
}

/// Go's `Update` decodes the response into the object it was given. The reconciler's late-init
/// branch (parity spec S5) relies on the response resetting status the caller had set in memory.
#[tokio::test]
async fn a_write_response_overwrites_the_callers_object() {
    let server = || {
        let mut mg = new_modern_managed(2);
        mg.metadata.resource_version = Some("43".into());
        mg.spec.management_policies = ManagementPolicies(vec![ManagementAction::Observe]);
        mg
    };
    let client: Box<dyn Client> = Box::new(MockClient {
        update: Some(test::new_mock_response_fn(server)),
        ..Default::default()
    });

    let mut local = new_modern_managed(1);
    local.metadata.resource_version = Some("42".into());
    local.set_conditions([Condition::reconcile_success().with_observed_generation(1)]);
    client.update(&mut local).await.unwrap();

    test::assert_object_eq(
        &server(),
        &local,
        "the response replaces status, spec and resourceVersion",
    );
    assert!(
        local.status.conditioned.conditions.is_empty(),
        "the unsaved condition is gone"
    );
}

fn with_conditions(cs: impl IntoIterator<Item = Condition>) -> ModernManaged {
    let mut mg = new_modern_managed(1);
    mg.set_conditions(cs);
    mg
}

#[test]
fn assert_object_eq_follows_go_equate_conditions() {
    let mut early = Condition::available();
    early.last_transition_time.0 = early
        .last_transition_time
        .0
        .checked_sub(k8s_openapi::jiff::SignedDuration::from_hours(1))
        .unwrap();
    test::assert_object_eq(
        &with_conditions([Condition::creating(), Condition::reconcile_success()]),
        &with_conditions([Condition::reconcile_success(), Condition::creating()]),
        "condition order does not matter",
    );
    test::assert_object_eq(
        &with_conditions([early]),
        &with_conditions([Condition::available()]),
        "lastTransitionTime does not matter",
    );
    test::assert_object_eq(
        &with_conditions([Condition::available()]),
        &with_conditions([Condition::available().with_observed_generation(3)]),
        "Go's Condition.Equal skips observedGeneration when either side is zero",
    );
}

#[test]
fn assert_object_eq_catches_what_go_would() {
    let cases: Vec<(&str, Box<dyn Fn()>)> = vec![
        (
            "reason",
            Box::new(|| {
                test::assert_object_eq(
                    &with_conditions([Condition::available()]),
                    &with_conditions([Condition::unavailable()]),
                    "r",
                )
            }),
        ),
        (
            "message",
            Box::new(|| {
                test::assert_object_eq(
                    &with_conditions([Condition::available()]),
                    &with_conditions([Condition::available().with_message("m")]),
                    "r",
                )
            }),
        ),
        (
            "both generations set and different",
            Box::new(|| {
                test::assert_object_eq(
                    &with_conditions([Condition::available().with_observed_generation(1)]),
                    &with_conditions([Condition::available().with_observed_generation(2)]),
                    "r",
                )
            }),
        ),
        (
            "an extra condition",
            Box::new(|| {
                test::assert_object_eq(
                    &with_conditions([Condition::available()]),
                    &with_conditions([Condition::available(), Condition::creating()]),
                    "r",
                )
            }),
        ),
        (
            "another kind with the same metadata",
            Box::new(|| test::assert_object_eq(&fake::Object::default(), &ProviderConfig::default(), "r")),
        ),
        (
            "metadata",
            Box::new(|| test::assert_object_eq(&new_modern_managed(1), &new_modern_managed(2), "r")),
        ),
    ];
    for (name, case) in cases {
        let msg = panic_message(case);
        assert!(msg.contains("-want:"), "{name}: expected a diff, got {msg:?}");
    }
}

#[test]
fn assert_conditions_eq_compares_slices_like_go() {
    test::assert_conditions_eq(
        &[
            Condition::reconcile_success(),
            Condition::available().with_observed_generation(2),
        ],
        &[Condition::available(), Condition::reconcile_success()],
        "sorted by type, generation skipped when one side is zero",
    );
    let msg = panic_message(|| {
        test::assert_conditions_eq(
            &[Condition::available().with_observed_generation(1)],
            &[Condition::available().with_observed_generation(2)],
            "r",
        )
    });
    assert!(msg.contains("-want:"), "{msg}");
}

#[test]
fn assert_result_equates_errors_by_text_and_reason() {
    let want: Result<()> = Err(Error::new("boom").wrap("cannot get"));
    test::assert_result("same text", "", &want, &Err(Error::new("boom").wrap("cannot get")));
    let msg =
        panic_message(|| test::assert_result("other text", "r", &want, &Err(Error::new("bang").wrap("cannot get"))));
    assert!(msg.contains("other text"), "{msg}");
    let not_found: Result<()> = Err(Error::status(StatusReason::NotFound, "boom"));
    let msg = panic_message(|| test::assert_result("other reason", "r", &not_found, &Err(Error::new("boom"))));
    assert!(msg.contains("other reason"), "{msg}");
    let msg = panic_message(|| test::assert_result("ok vs err", "r", &Ok(()), &want));
    assert!(msg.contains("ok vs err"), "{msg}");
}

#[test]
fn modern_managed_has_the_go_fake_accessors() {
    let mut mg = ModernManaged::default();
    let pc = ProviderConfigReference {
        kind: "ClusterProviderConfig".into(),
        name: "default".into(),
    };
    let secret = LocalSecretReference { name: "conn".into() };
    let policies = ManagementPolicies(vec![ManagementAction::Observe, ManagementAction::Create]);
    mg.set_provider_config_reference(Some(pc.clone()));
    mg.set_write_connection_secret_to_reference(Some(secret.clone()));
    mg.set_management_policies(policies.clone());
    mg.set_conditions([Condition::available()]);

    assert_eq!(mg.provider_config_reference(), Some(&pc));
    assert_eq!(mg.write_connection_secret_to_reference(), Some(&secret));
    assert_eq!(mg.management_policies(), &policies);
    test::assert_conditions_eq(
        &[Condition::available()],
        &mg.status.conditioned.conditions,
        "set_conditions writes status",
    );

    let wire = mg.to_json().unwrap();
    assert_eq!(wire["apiVersion"], "fake.crossplane.io/v1");
    assert_eq!(wire["kind"], "ModernManaged");
    assert_eq!(
        wire["spec"]["providerConfigRef"],
        json!({"kind": "ClusterProviderConfig", "name": "default"})
    );
    assert_eq!(wire["spec"]["writeConnectionSecretToRef"], json!({"name": "conn"}));
    assert_eq!(wire["spec"]["managementPolicies"], json!(["Observe", "Create"]));
    assert_eq!(wire["status"]["conditions"][0]["type"], "Ready");

    mg.set_provider_config_reference(None);
    assert_eq!(mg.provider_config_reference(), None);
    assert_eq!(fake::as_modern_managed(&mut mg, 9).metadata.generation, Some(9));
}

#[test]
fn provider_config_has_the_go_fake_accessors() {
    let mut pc = ProviderConfig::default();
    assert_eq!(pc.users(), 0);
    pc.set_users(3);
    pc.set_conditions([Condition::reconcile_success()]);
    assert_eq!(pc.users(), 3);
    test::assert_conditions_eq(
        &[Condition::reconcile_success()],
        &pc.status.conditioned.conditions,
        "set_conditions writes status",
    );
    let wire = pc.to_json().unwrap();
    assert_eq!(wire["status"]["users"], 3);
    assert_eq!(wire["status"]["conditions"][0]["reason"], "ReconcileSuccess");
}

#[test]
fn provider_config_usage_has_the_go_fake_accessors() {
    let pc = ProviderConfigReference {
        kind: "ProviderConfig".into(),
        name: "pc".into(),
    };
    let mr = TypedReference {
        api_version: "fake.crossplane.io/v1".into(),
        kind: "ModernManaged".into(),
        name: "mr".into(),
        uid: "u".into(),
    };
    let meta = ObjectMeta {
        name: Some("u".into()),
        ..Default::default()
    };
    let mut pcu = ProviderConfigUsage::from_usage(meta, TypedProviderConfigUsage::default());
    pcu.set_provider_config_reference(pc.clone());
    pcu.set_resource_reference(mr.clone());
    assert_eq!(pcu.provider_config_reference(), &pc);
    assert_eq!(pcu.resource_reference(), &mr);

    let wire = pcu.to_json().unwrap();
    assert_eq!(
        wire["providerConfigRef"],
        json!({"kind": "ProviderConfig", "name": "pc"})
    );
    assert_eq!(
        wire["resourceRef"]["kind"], "ModernManaged",
        "the usage is inlined at the root"
    );
    let back = ObjectType::of::<ProviderConfigUsage>().decode(wire).unwrap();
    assert_eq!(back.downcast_ref::<ProviderConfigUsage>(), Some(&pcu));
}

//! `KubeClient` against a real API server, plus the error conversion.
//!
//! The cluster test runs only when `XPRS_TEST_KUBECONFIG` names a kubeconfig. It works in a
//! namespace of its own, which it creates and deletes, and touches nothing else. Its objects are
//! `MyType`s carrying the pause annotation, so an installed baseline provider only marks them
//! paused. With `XPRS_TEST_AUDIT_LOG` naming the kube-apiserver audit log it also checks the verbs
//! and the User-Agent the server recorded.

use std::borrow::Cow;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use crossplane_core::client::{Client, ListOptions, Patch};
use crossplane_core::errors::StatusReason;
use crossplane_core::resource::ObjectKey;
use futures::FutureExt;
use k8s_openapi::api::core::v1::Namespace;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Resource;
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::core::NamespaceResourceScope;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{KubeClient, connect, convert_error};

#[test]
fn kube_errors_keep_their_status_reason() {
    let api =
        |reason: &str, code: u16| kube::Error::Api(kube::core::Status::failure("msg", reason).with_code(code).boxed());
    let cases = [
        ("NotFound by reason", api("NotFound", 404), Some(StatusReason::NotFound)),
        ("Conflict by reason", api("Conflict", 409), Some(StatusReason::Conflict)),
        (
            "AlreadyExists by reason",
            api("AlreadyExists", 409),
            Some(StatusReason::AlreadyExists),
        ),
        (
            "Forbidden by reason",
            api("Forbidden", 403),
            Some(StatusReason::Forbidden),
        ),
        ("Invalid by reason", api("Invalid", 422), Some(StatusReason::Invalid)),
        ("NotFound by code", api("", 404), Some(StatusReason::NotFound)),
        (
            "Conflict by code",
            api("SomethingNew", 409),
            Some(StatusReason::Conflict),
        ),
        (
            "a known reason wins over the code",
            api("Gone", 404),
            Some(StatusReason::Other("Gone".into())),
        ),
        (
            "other",
            api("TooManyRequests", 429),
            Some(StatusReason::Other("TooManyRequests".into())),
        ),
        ("not an API error", kube::Error::LinesCodecMaxLineLengthExceeded, None),
    ];
    for (name, err, want) in cases {
        let got = convert_error(err).wrap("cannot get");
        assert_eq!(got.status_reason(), want.as_ref(), "{name}");
    }
    assert_eq!(
        convert_error(api("NotFound", 404)).to_string(),
        "msg",
        "the server's message is kept"
    );
}

/// The baseline provider's kind, with an open spec and status so the test can write anything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct MyType {
    metadata: ObjectMeta,
    spec: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<Value>,
}

impl Resource for MyType {
    type DynamicType = ();
    type Scope = NamespaceResourceScope;

    fn kind(_: &()) -> Cow<'_, str> {
        "MyType".into()
    }
    fn group(_: &()) -> Cow<'_, str> {
        "sample.example.com".into()
    }
    fn version(_: &()) -> Cow<'_, str> {
        "v1alpha1".into()
    }
    fn plural(_: &()) -> Cow<'_, str> {
        "mytypes".into()
    }
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

const USER_AGENT: &str = "xprs-kube-client-test/v0.0.0 (test/test) kubernetes/test";

fn my_type(namespace: &str, name: &str, labels: &[(&str, &str)]) -> MyType {
    MyType {
        metadata: ObjectMeta {
            name: Some(name.into()),
            namespace: Some(namespace.into()),
            annotations: Some([("crossplane.io/paused".to_owned(), "true".to_owned())].into()),
            labels: (!labels.is_empty()).then(|| labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
            ..Default::default()
        },
        spec: json!({"forProvider": {"configurableField": "a"}}),
        status: None,
    }
}

fn label(obj: &MyType, key: &str) -> Option<String> {
    obj.metadata.labels.as_ref()?.get(key).cloned()
}

fn field(obj: &MyType, path: &str) -> Value {
    serde_json::to_value(obj)
        .unwrap()
        .pointer(path)
        .cloned()
        .unwrap_or(Value::Null)
}

/// An installed provider writes a paused condition once per generation. Waiting for it keeps that
/// write from moving the resourceVersion under a later step. Without a provider this returns after
/// the deadline with whatever the server holds.
async fn settled(client: &dyn Client, key: &ObjectKey) -> MyType {
    for _ in 0..30 {
        let obj: MyType = client.get_as(key).await.unwrap();
        let synced_generation = obj
            .status
            .as_ref()
            .and_then(|s| s["conditions"].as_array())
            .and_then(|cs| cs.iter().find(|c| c["type"] == "Synced"))
            .and_then(|c| c["observedGeneration"].as_i64());
        if synced_generation == obj.metadata.generation {
            return obj;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    client.get_as(key).await.unwrap()
}

#[tokio::test]
async fn kube_client_against_a_real_api_server() {
    let Some(path) = std::env::var_os("XPRS_TEST_KUBECONFIG") else {
        eprintln!("skipping kube_client_against_a_real_api_server: set XPRS_TEST_KUBECONFIG to a kubeconfig");
        return;
    };
    let kubeconfig = Kubeconfig::read_from(&path).expect("readable kubeconfig");
    let config = kube::Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
        .await
        .expect("usable kubeconfig");
    let client = KubeClient::uncached(connect(config, USER_AGENT).expect("client"));
    let ns = format!("xprs-kube-client-{}", std::process::id());

    let mut namespace = Namespace {
        metadata: ObjectMeta {
            name: Some(ns.clone()),
            ..Default::default()
        },
        ..Default::default()
    };
    client.create(&mut namespace).await.expect("create the test namespace");
    let outcome = AssertUnwindSafe(scenarios(&client, &ns)).catch_unwind().await;
    let cleanup = client.delete(&namespace).await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    cleanup.expect("delete the test namespace");

    if let Some(audit) = std::env::var_os("XPRS_TEST_AUDIT_LOG") {
        check_audit_log(std::path::Path::new(&audit), &ns).await;
    }
}

async fn scenarios(client: &KubeClient, ns: &str) {
    let client: &dyn Client = client;
    let key = ObjectKey::namespaced(ns, "a");

    let mut created = my_type(ns, "a", &[]);
    client.create(&mut created).await.expect("create");
    assert!(
        created.metadata.uid.is_some(),
        "create decodes the response into the object"
    );
    assert!(created.metadata.resource_version.is_some());
    assert_eq!(
        field(&created, "/spec/managementPolicies"),
        json!(["*"]),
        "schema defaults come back"
    );
    let got: MyType = client.get_as(&key).await.expect("get");
    assert_eq!(got.metadata.uid, created.metadata.uid);
    assert_eq!(got.spec, created.spec);

    let already = client.create(&mut my_type(ns, "a", &[])).await.unwrap_err();
    assert!(already.is_already_exists(), "second create: {already:?}");

    let mut obj = settled(client, &key).await;
    let stale = obj.clone();
    let server_status = obj.status.clone();
    obj.spec["forProvider"]["configurableField"] = json!("b");
    obj.status = Some(json!({"atProvider": {"configurableField": "never-persisted"}}));
    client.update(&mut obj).await.expect("update");
    assert_eq!(
        obj.status, server_status,
        "the response replaced the status set in memory"
    );
    assert_ne!(obj.metadata.resource_version, stale.metadata.resource_version);
    assert_eq!(obj.metadata.generation, Some(2));
    let fresh: MyType = client.get_as(&key).await.unwrap();
    assert_eq!(field(&fresh, "/spec/forProvider/configurableField"), "b");
    assert_eq!(
        (&obj.spec, &obj.metadata.generation),
        (&fresh.spec, &fresh.metadata.generation),
        "after update the object holds the server's spec and generation"
    );

    let conflict = client.update(&mut stale.clone()).await.unwrap_err();
    assert!(conflict.is_conflict(), "stale update: {conflict:?}");
    assert!(conflict.wrap("cannot update managed resource").is_conflict());

    let mut obj = settled(client, &key).await;
    let generation = obj.metadata.generation;
    obj.spec["forProvider"]["configurableField"] = json!("never-persisted");
    let mut status = obj.status.clone().unwrap_or_else(|| json!({}));
    status["atProvider"] = json!({"configurableField": "observed"});
    obj.status = Some(status);
    client.update_status(&mut obj).await.expect("update status");
    assert_eq!(
        field(&obj, "/spec/forProvider/configurableField"),
        "b",
        "the spec edit was reverted"
    );
    assert_eq!(field(&obj, "/status/atProvider/configurableField"), "observed");
    let fresh: MyType = client.get_as(&key).await.unwrap();
    assert_eq!(
        fresh.metadata.generation, generation,
        "a status write does not bump the generation"
    );
    assert_eq!(field(&fresh, "/spec/forProvider/configurableField"), "b");
    assert_eq!(field(&fresh, "/status/atProvider/configurableField"), "observed");

    let missing = client
        .get_as::<MyType>(&ObjectKey::namespaced(ns, "missing"))
        .await
        .unwrap_err()
        .wrap("cannot get managed resource");
    assert!(missing.is_not_found(), "{missing:?}");
    assert_eq!(
        missing.to_string(),
        r#"cannot get managed resource: mytypes.sample.example.com "missing" not found"#
    );

    let mut obj: MyType = client.get_as(&key).await.unwrap();
    let before = obj.metadata.resource_version.clone();
    client
        .patch(
            &mut obj,
            Patch::Merge(json!({"metadata": {"labels": {"patched": "merge"}}})),
        )
        .await
        .expect("merge patch");
    assert_eq!(
        label(&obj, "patched").as_deref(),
        Some("merge"),
        "the response is decoded"
    );
    assert_ne!(obj.metadata.resource_version, before);
    let fresh: MyType = client.get_as(&key).await.unwrap();
    assert_eq!(label(&fresh, "patched").as_deref(), Some("merge"));

    let apply = |owner: &str, force: bool| Patch::Apply {
        body: json!({
            "apiVersion": "sample.example.com/v1alpha1",
            "kind": "MyType",
            "metadata": {"name": "a", "namespace": ns, "labels": {"owner": owner}},
        }),
        field_manager: format!("xprs-test-{owner}"),
        force,
    };
    let mut obj: MyType = client.get_as(&key).await.unwrap();
    client.patch(&mut obj, apply("one", false)).await.expect("apply");
    assert_eq!(label(&obj, "owner").as_deref(), Some("one"));
    let managers = |o: &MyType| -> Vec<(String, String)> {
        o.metadata
            .managed_fields
            .iter()
            .flatten()
            .map(|m| {
                (
                    m.manager.clone().unwrap_or_default(),
                    m.operation.clone().unwrap_or_default(),
                )
            })
            .collect()
    };
    assert!(
        managers(&obj).contains(&("xprs-test-one".into(), "Apply".into())),
        "{:?}",
        managers(&obj)
    );
    let refused = client.patch(&mut obj.clone(), apply("two", false)).await.unwrap_err();
    assert!(
        refused.is_conflict(),
        "apply by a second owner without force: {refused:?}"
    );
    client.patch(&mut obj, apply("two", true)).await.expect("forced apply");
    assert_eq!(label(&obj, "owner").as_deref(), Some("two"));
    assert!(managers(&obj).contains(&("xprs-test-two".into(), "Apply".into())));

    let team = ns.to_owned();
    for (name, value) in [("b", team.as_str()), ("c", "other")] {
        client
            .create(&mut my_type(ns, name, &[("xprs-team", value)]))
            .await
            .expect("create for list");
    }
    let in_ns = ListOptions {
        namespace: Some(ns.into()),
        labels: [("xprs-team".to_owned(), team.clone())].into(),
    };
    let listed: Vec<MyType> = client.list_as(&in_ns).await.expect("list");
    let names: Vec<_> = listed.iter().map(|o| o.metadata.name.clone().unwrap()).collect();
    assert_eq!(names, ["b"]);
    let everywhere = ListOptions {
        namespace: None,
        ..in_ns.clone()
    };
    let listed: Vec<MyType> = client.list_as(&everywhere).await.expect("list all namespaces");
    assert_eq!(listed.len(), 1);
    let all_in_ns: Vec<MyType> = client
        .list_as(&ListOptions {
            namespace: Some(ns.into()),
            labels: Default::default(),
        })
        .await
        .unwrap();
    assert_eq!(all_in_ns.len(), 3, "no selector lists every object in the namespace");

    let obj: MyType = client.get_as(&key).await.unwrap();
    client.delete(&obj).await.expect("delete");
    let gone = client.get_as::<MyType>(&key).await.unwrap_err();
    assert!(gone.is_not_found(), "after delete: {gone:?}");
    let again = client.delete(&obj).await.unwrap_err();
    assert!(again.is_not_found(), "second delete: {again:?}");
}

/// The audit log records each request's verb, subresource and User-Agent. Writes from this test
/// are logged at Metadata level, so the request body is not there to compare.
async fn check_audit_log(path: &std::path::Path, ns: &str) {
    let mut ours = Vec::new();
    for _ in 0..20 {
        let log = std::fs::read_to_string(path).expect("readable audit log");
        ours = log
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|e| e["objectRef"]["namespace"] == ns && e["userAgent"] == USER_AGENT)
            .collect();
        if ours
            .iter()
            .any(|e| e["verb"] == "delete" && e["objectRef"]["resource"] == "mytypes")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    for e in &ours {
        eprintln!(
            "audit: {} {}{} {} -> {} ua={}",
            e["verb"].as_str().unwrap_or_default(),
            e["objectRef"]["resource"].as_str().unwrap_or_default(),
            e["objectRef"]["subresource"]
                .as_str()
                .map(|s| format!("/{s}"))
                .unwrap_or_default(),
            e["objectRef"]["name"].as_str().unwrap_or_default(),
            e["responseStatus"]["code"],
            e["userAgent"].as_str().unwrap_or_default(),
        );
    }
    let has = |verb: &str, subresource: Option<&str>, code: u16| {
        ours.iter().any(|e| {
            e["verb"] == verb
                && e["objectRef"]["resource"] == "mytypes"
                && e["objectRef"]["subresource"].as_str() == subresource
                && e["responseStatus"]["code"] == code
        })
    };
    assert!(has("update", None, 200), "update is a PUT on the object");
    assert!(has("update", Some("status"), 200), "update_status is a PUT on /status");
    assert!(has("update", None, 409), "the stale update was refused");
    assert!(has("patch", None, 200), "patches");
    assert!(has("create", None, 201), "create");
    assert!(has("delete", None, 200), "delete");
}

//! CRD post-processing that brings kube-derive output to controller-gen's shape. Types stay
//! idiomatic (`Option` for Go pointers); differences that are only about the schema generator are
//! fixed here, once, at CRD emission.

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use serde_json::{Map, Value, json};

/// Drops `nullable` and `title` everywhere, adds `names.listKind`, the root `apiVersion`, `kind`
/// and `metadata` properties, and an empty `subresources` on versions that have none.
/// Descriptions are left alone; `hack/crd-diff` excludes them.
pub fn controller_gen_compat(crd: CustomResourceDefinition) -> CustomResourceDefinition {
    let mut v = serde_json::to_value(crd).expect("a CRD serializes");
    if let Some(kind) = v.pointer("/spec/names/kind").and_then(Value::as_str).map(str::to_owned) {
        v["spec"]["names"]["listKind"] = json!(format!("{kind}List"));
    }
    if let Some(versions) = v.pointer_mut("/spec/versions").and_then(Value::as_array_mut) {
        for ver in versions {
            if let Some(root) = ver.pointer_mut("/schema/openAPIV3Schema") {
                strip(root);
                let props = root
                    .as_object_mut()
                    .expect("schema object")
                    .entry("properties")
                    .or_insert_with(|| Value::Object(Map::new()));
                props["apiVersion"] = json!({"type": "string"});
                props["kind"] = json!({"type": "string"});
                props["metadata"] = json!({"type": "object"});
            }
            ver.as_object_mut()
                .expect("version object")
                .entry("subresources")
                .or_insert_with(|| json!({}));
        }
    }
    serde_json::from_value(v).expect("still a CRD")
}

fn strip(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("nullable");
            m.remove("title");
            m.values_mut().for_each(strip);
        }
        Value::Array(a) => a.iter_mut().for_each(strip),
        _ => {}
    }
}

/// `#[schemars(transform = no_default)]` on a `#[serde(default)]` field keeps the serde default
/// without a schema `default`, as Go's `omitempty` on a struct field has none.
pub fn no_default(s: &mut schemars::Schema) {
    s.remove("default");
}

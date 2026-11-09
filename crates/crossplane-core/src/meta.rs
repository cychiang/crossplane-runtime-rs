//! `pkg/meta`: annotation keys and the helpers the reconcilers and providers use.
//!
//! Go keeps empty `labels`, `annotations` and `finalizers` out of the JSON (`omitempty`). The
//! helpers here leave `None` rather than an empty collection for the same reason, so a mutation
//! that empties one cannot add `"finalizers": []` to a write.

use std::collections::BTreeMap;
use std::time::Duration;

use k8s_openapi::api::core::v1::ObjectReference;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use k8s_openapi::jiff::Timestamp;
use kube::Resource;
use kube::core::GroupVersionKind;

use crate::errors::{Error, Result};
use crate::resource::ObjectKey;
use crate::v2::TypedReference;

pub const ANNOTATION_KEY_EXTERNAL_NAME: &str = "crossplane.io/external-name";
pub const ANNOTATION_KEY_EXTERNAL_CREATE_PENDING: &str = "crossplane.io/external-create-pending";
pub const ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED: &str = "crossplane.io/external-create-succeeded";
pub const ANNOTATION_KEY_EXTERNAL_CREATE_FAILED: &str = "crossplane.io/external-create-failed";
pub const ANNOTATION_KEY_RECONCILIATION_PAUSED: &str = "crossplane.io/paused";
pub const ANNOTATION_KEY_POLL_INTERVAL: &str = "crossplane.io/poll-interval";
pub const ANNOTATION_KEY_RECONCILE_REQUESTED_AT: &str = "crossplane.io/reconcile-requested-at";

fn annotation<'a, K: Resource>(o: &'a K, key: &str) -> Option<&'a str> {
    o.meta().annotations.as_ref()?.get(key).map(String::as_str)
}

fn set_annotation<K: Resource>(o: &mut K, key: &str, value: String) {
    add_annotations(o, [(key.to_owned(), value)]);
}

/// Second precision, as Go's `time.RFC3339` formats.
fn rfc3339(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn timestamp_annotation<K: Resource>(o: &K, key: &str) -> Option<Timestamp> {
    annotation(o, key)?.parse().ok()
}

/// Go `api_version` of a `GroupVersionKind`: `group/version`, or `version` for the core group.
fn api_version_of(gvk: &GroupVersionKind) -> String {
    if gvk.group.is_empty() {
        gvk.version.clone()
    } else {
        format!("{}/{}", gvk.group, gvk.version)
    }
}

/// Go `ReferenceTo`.
pub fn reference_to<K: Resource>(o: &K, of: &GroupVersionKind) -> ObjectReference {
    ObjectReference {
        api_version: Some(api_version_of(of)),
        kind: Some(of.kind.clone()),
        namespace: o.meta().namespace.clone(),
        name: o.meta().name.clone(),
        uid: o.meta().uid.clone(),
        ..Default::default()
    }
}

/// Go `TypedReferenceTo`.
pub fn typed_reference_to<K: Resource>(o: &K, of: &GroupVersionKind) -> TypedReference {
    TypedReference {
        api_version: api_version_of(of),
        kind: of.kind.clone(),
        name: name(o).to_owned(),
        uid: o.meta().uid.clone().unwrap_or_default(),
    }
}

/// Go `AsOwner`.
pub fn as_owner(r: &TypedReference) -> OwnerReference {
    OwnerReference {
        api_version: r.api_version.clone(),
        kind: r.kind.clone(),
        name: r.name.clone(),
        uid: r.uid.clone(),
        controller: None,
        block_owner_deletion: None,
    }
}

/// Go `AsController`.
pub fn as_controller(r: &TypedReference) -> OwnerReference {
    OwnerReference {
        controller: Some(true),
        block_owner_deletion: Some(true),
        ..as_owner(r)
    }
}

/// Go `metav1.GetControllerOf`: the owner reference with `controller: true`.
fn controller_of<K: Resource>(o: &K) -> Option<&OwnerReference> {
    o.meta()
        .owner_references
        .as_ref()?
        .iter()
        .find(|r| r.controller == Some(true))
}

/// Two objects without a controller do not have the same controller.
pub fn have_same_controller<A: Resource, B: Resource>(a: &A, b: &B) -> bool {
    match (controller_of(a), controller_of(b)) {
        (Some(ac), Some(bc)) => ac.uid == bc.uid,
        _ => false,
    }
}

/// Go `NamespacedNameOf`.
pub fn namespaced_name_of(r: &ObjectReference) -> ObjectKey {
    ObjectKey {
        namespace: r.namespace.clone().filter(|ns| !ns.is_empty()),
        name: r.name.clone().unwrap_or_default(),
    }
}

/// Any existing owner with the same UID is replaced.
pub fn add_owner_reference<K: Resource>(o: &mut K, r: OwnerReference) {
    let refs = o.meta_mut().owner_references.get_or_insert_default();
    match refs.iter_mut().find(|existing| existing.uid == r.uid) {
        Some(existing) => *existing = r,
        None => refs.push(r),
    }
}

/// Fails, leaving the owners alone, when another object already controls `o`.
pub fn add_controller_reference<K: Resource>(o: &mut K, r: OwnerReference) -> Result<()> {
    if let Some(c) = controller_of(o).filter(|c| c.uid != r.uid) {
        return Err(Error::new(format!(
            "{} is already controlled by {} {} (UID {})",
            name(o),
            c.kind,
            c.name,
            c.uid
        )));
    }
    add_owner_reference(o, r);
    Ok(())
}

pub fn namespace<K: Resource>(o: &K) -> &str {
    o.meta().namespace.as_deref().unwrap_or_default()
}

pub fn name<K: Resource>(o: &K) -> &str {
    o.meta().name.as_deref().unwrap_or_default()
}

pub fn add_finalizer<K: Resource>(o: &mut K, finalizer: &str) {
    let f = o.meta_mut().finalizers.get_or_insert_default();
    if !f.iter().any(|e| e == finalizer) {
        f.push(finalizer.to_owned());
    }
}

pub fn remove_finalizer<K: Resource>(o: &mut K, finalizer: &str) {
    let meta = o.meta_mut();
    if let Some(f) = meta.finalizers.as_mut() {
        f.retain(|e| e != finalizer);
        if f.is_empty() {
            meta.finalizers = None;
        }
    }
}

pub fn finalizer_exists<K: Resource>(o: &K, finalizer: &str) -> bool {
    o.meta()
        .finalizers
        .as_ref()
        .is_some_and(|f| f.iter().any(|e| e == finalizer))
}

pub fn add_labels<K: Resource>(o: &mut K, labels: impl IntoIterator<Item = (String, String)>) {
    let labels: BTreeMap<_, _> = labels.into_iter().collect();
    if labels.is_empty() {
        return;
    }
    o.meta_mut().labels.get_or_insert_default().extend(labels);
}

pub fn remove_labels<K: Resource>(o: &mut K, labels: &[&str]) {
    let meta = o.meta_mut();
    if let Some(l) = meta.labels.as_mut() {
        l.retain(|k, _| !labels.contains(&k.as_str()));
        if l.is_empty() {
            meta.labels = None;
        }
    }
}

pub fn add_annotations<K: Resource>(o: &mut K, annotations: impl IntoIterator<Item = (String, String)>) {
    let annotations: BTreeMap<_, _> = annotations.into_iter().collect();
    if annotations.is_empty() {
        return;
    }
    o.meta_mut().annotations.get_or_insert_default().extend(annotations);
}

pub fn remove_annotations<K: Resource>(o: &mut K, annotations: &[&str]) {
    let meta = o.meta_mut();
    if let Some(a) = meta.annotations.as_mut() {
        a.retain(|k, _| !annotations.contains(&k.as_str()));
        if a.is_empty() {
            meta.annotations = None;
        }
    }
}

pub fn was_deleted<K: Resource>(o: &K) -> bool {
    o.meta().deletion_timestamp.is_some()
}

pub fn was_created<K: Resource>(o: &K) -> bool {
    o.meta().creation_timestamp.is_some()
}

pub fn external_name<K: Resource>(o: &K) -> &str {
    annotation(o, ANNOTATION_KEY_EXTERNAL_NAME).unwrap_or_default()
}

pub fn set_external_name<K: Resource>(o: &mut K, name: &str) {
    set_annotation(o, ANNOTATION_KEY_EXTERNAL_NAME, name.to_owned());
}

/// Go's zero time, an absent or unparsable stamp, is `None`.
pub fn external_create_pending<K: Resource>(o: &K) -> Option<Timestamp> {
    timestamp_annotation(o, ANNOTATION_KEY_EXTERNAL_CREATE_PENDING)
}

pub fn set_external_create_pending<K: Resource>(o: &mut K, t: Timestamp) {
    set_annotation(o, ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, rfc3339(t));
}

pub fn external_create_succeeded<K: Resource>(o: &K) -> Option<Timestamp> {
    timestamp_annotation(o, ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED)
}

pub fn set_external_create_succeeded<K: Resource>(o: &mut K, t: Timestamp) {
    set_annotation(o, ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED, rfc3339(t));
}

pub fn external_create_failed<K: Resource>(o: &K) -> Option<Timestamp> {
    timestamp_annotation(o, ANNOTATION_KEY_EXTERNAL_CREATE_FAILED)
}

pub fn set_external_create_failed<K: Resource>(o: &mut K, t: Timestamp) {
    set_annotation(o, ANNOTATION_KEY_EXTERNAL_CREATE_FAILED, rfc3339(t));
}

/// `pending` parses and is strictly after the later of `succeeded` and `failed`. An unparsable
/// stamp reads as zero time.
pub fn external_create_incomplete<K: Resource>(o: &K) -> bool {
    let Some(pending) = external_create_pending(o) else {
        return false;
    };
    let latest = external_create_succeeded(o).max(external_create_failed(o));
    latest.is_none_or(|l| pending > l)
}

/// Go `ExternalCreateSucceededDuring`, with the clock passed in as `now` where Go calls
/// `time.Since`.
pub fn external_create_succeeded_during<K: Resource>(o: &K, d: Duration, now: Timestamp) -> bool {
    let Some(t) = external_create_succeeded(o) else {
        return false;
    };
    now.as_nanosecond() - t.as_nanosecond() < d.as_nanos() as i128
}

pub fn is_paused<K: Resource>(o: &K) -> bool {
    annotation(o, ANNOTATION_KEY_RECONCILIATION_PAUSED) == Some("true")
}

/// `None` when the annotation is missing, empty, unparsable, zero or negative.
pub fn poll_interval<K: Resource>(o: &K) -> Option<Duration> {
    let v = annotation(o, ANNOTATION_KEY_POLL_INTERVAL)?;
    crate::duration::parse(v).ok().filter(|d| !d.is_zero())
}

/// The reconcile-request token, when present and non-empty.
pub fn reconcile_request<K: Resource>(o: &K) -> Option<&str> {
    annotation(o, ANNOTATION_KEY_RECONCILE_REQUESTED_AT).filter(|s| !s.is_empty())
}

pub fn set_reconcile_request<K: Resource>(o: &mut K, token: &str) {
    set_annotation(o, ANNOTATION_KEY_RECONCILE_REQUESTED_AT, token.to_owned());
}

#[cfg(test)]
mod meta_test;

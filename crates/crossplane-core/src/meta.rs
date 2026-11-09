//! `pkg/meta`: annotation keys and the helpers the reconcilers and providers use.

use std::time::Duration;

use k8s_openapi::jiff::Timestamp;
use kube::Resource;

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
    o.meta_mut()
        .annotations
        .get_or_insert_default()
        .insert(key.to_owned(), value);
}

/// Second precision, as Go's `time.RFC3339` formats.
fn rfc3339(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn namespace<K: Resource>(o: &K) -> &str {
    o.meta().namespace.as_deref().unwrap_or_default()
}

pub fn name<K: Resource>(o: &K) -> &str {
    o.meta().name.as_deref().unwrap_or_default()
}

pub fn was_deleted<K: Resource>(o: &K) -> bool {
    o.meta().deletion_timestamp.is_some()
}

pub fn external_name<K: Resource>(o: &K) -> &str {
    annotation(o, ANNOTATION_KEY_EXTERNAL_NAME).unwrap_or_default()
}

pub fn set_external_name<K: Resource>(o: &mut K, name: &str) {
    todo!("set {ANNOTATION_KEY_EXTERNAL_NAME} to {name:?} on {:?}", o.meta().name)
}

pub fn set_external_create_pending<K: Resource>(o: &mut K, t: Timestamp) {
    set_annotation(o, ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, rfc3339(t));
}

pub fn set_external_create_succeeded<K: Resource>(o: &mut K, t: Timestamp) {
    todo!(
        "set {ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED} to {t} on {:?}",
        o.meta().name
    )
}

pub fn set_external_create_failed<K: Resource>(o: &mut K, t: Timestamp) {
    todo!(
        "set {ANNOTATION_KEY_EXTERNAL_CREATE_FAILED} to {t} on {:?}",
        o.meta().name
    )
}

/// Go `ExternalCreateSucceededDuring(o, d)` is this compared against the injected clock.
pub fn external_create_succeeded<K: Resource>(o: &K) -> Option<Timestamp> {
    todo!(
        "parse {ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED} on {:?}",
        o.meta().name
    )
}

/// `pending` parses and is strictly after the later of `succeeded` and `failed`. An unparsable
/// stamp reads as zero time.
pub fn external_create_incomplete<K: Resource>(o: &K) -> bool {
    let stamp = |key| annotation(o, key).and_then(|s| s.parse::<Timestamp>().ok());
    let Some(pending) = stamp(ANNOTATION_KEY_EXTERNAL_CREATE_PENDING) else {
        return false;
    };
    let latest = stamp(ANNOTATION_KEY_EXTERNAL_CREATE_SUCCEEDED).max(stamp(ANNOTATION_KEY_EXTERNAL_CREATE_FAILED));
    latest.is_none_or(|l| pending > l)
}

pub fn is_paused<K: Resource>(o: &K) -> bool {
    annotation(o, ANNOTATION_KEY_RECONCILIATION_PAUSED) == Some("true")
}

/// `None` when the annotation is missing, empty, unparsable, zero or negative.
pub fn poll_interval<K: Resource>(o: &K) -> Option<Duration> {
    todo!(
        "crate::duration::parse of {ANNOTATION_KEY_POLL_INTERVAL} on {:?}",
        o.meta().name
    )
}

/// The reconcile-request token, when present and non-empty.
pub fn reconcile_request<K: Resource>(o: &K) -> Option<&str> {
    annotation(o, ANNOTATION_KEY_RECONCILE_REQUESTED_AT).filter(|s| !s.is_empty())
}

pub fn finalizer_exists<K: Resource>(o: &K, finalizer: &str) -> bool {
    todo!("whether {:?} has finalizer {finalizer:?}", o.meta().name)
}

pub fn add_finalizer<K: Resource>(o: &mut K, finalizer: &str) {
    todo!("add {finalizer:?} to {:?} unless present", o.meta().name)
}

pub fn remove_finalizer<K: Resource>(o: &mut K, finalizer: &str) {
    todo!("remove {finalizer:?} from {:?}", o.meta().name)
}

//! Test support for the runtime and for providers: Go `pkg/test` at the crate root and
//! `pkg/resource/fake` in [`fake`].
//!
//! Depends only on `crossplane-core`, where the `Client` trait lives, so `crossplane-runtime`
//! uses this crate from its own unit tests without a second copy of any type in the build.
//! Closure adapters for the runtime's collaborators (`InitializerFn`, `ExternalClientFns` and so
//! on) live in the runtime next to their traits, as in Go.

pub mod fake;

use std::fmt::Debug;

use crossplane_core::client::{Client, ListOptions, Patch};
use crossplane_core::errors::{Error, Result};
use crossplane_core::resource::{Object, ObjectKey, ObjectType};
use futures::FutureExt;
use futures::future::BoxFuture;
use k8s_openapi::jiff::{Timestamp, ToSpan};

pub type MockGetFn = Box<dyn Fn(&ObjectType, &ObjectKey) -> Result<Box<dyn Object>> + Send + Sync>;
pub type MockListFn = Box<dyn Fn(&ObjectType, &ListOptions) -> Result<Vec<Box<dyn Object>>> + Send + Sync>;
pub type MockObjectFn = Box<dyn Fn(&mut dyn Object) -> Result<()> + Send + Sync>;
pub type MockPatchFn = Box<dyn Fn(&mut dyn Object, &Patch) -> Result<()> + Send + Sync>;
pub type MockDeleteFn = Box<dyn Fn(&dyn Object) -> Result<()> + Send + Sync>;

/// Go `test.MockClient`. A case sets only the operations it expects. Calling an operation whose
/// field is `None` panics, which is how a Go table pins "this call must not happen" with a nil
/// function field.
#[derive(Default)]
pub struct MockClient {
    pub get: Option<MockGetFn>,
    pub list: Option<MockListFn>,
    pub create: Option<MockObjectFn>,
    pub update: Option<MockObjectFn>,
    pub update_status: Option<MockObjectFn>,
    pub patch: Option<MockPatchFn>,
    pub delete: Option<MockDeleteFn>,
}

fn must<'a, T: ?Sized>(f: &'a Option<Box<T>>, op: &str) -> &'a T {
    f.as_deref()
        .unwrap_or_else(|| panic!("MockClient::{op} was called but the test set no `{op}` closure"))
}

impl Client for MockClient {
    fn get<'a>(&'a self, object_type: &'a ObjectType, key: &'a ObjectKey) -> BoxFuture<'a, Result<Box<dyn Object>>> {
        futures::future::ready(must(&self.get, "get")(object_type, key)).boxed()
    }
    fn list<'a>(
        &'a self,
        object_type: &'a ObjectType,
        options: &'a ListOptions,
    ) -> BoxFuture<'a, Result<Vec<Box<dyn Object>>>> {
        futures::future::ready(must(&self.list, "list")(object_type, options)).boxed()
    }
    fn create<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        futures::future::ready(must(&self.create, "create")(obj)).boxed()
    }
    fn update<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        futures::future::ready(must(&self.update, "update")(obj)).boxed()
    }
    fn update_status<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        futures::future::ready(must(&self.update_status, "update_status")(obj)).boxed()
    }
    fn patch<'a>(&'a self, obj: &'a mut dyn Object, patch: Patch) -> BoxFuture<'a, Result<()>> {
        futures::future::ready(must(&self.patch, "patch")(obj, &patch)).boxed()
    }
    fn delete<'a>(&'a self, obj: &'a dyn Object) -> BoxFuture<'a, Result<()>> {
        futures::future::ready(must(&self.delete, "delete")(obj)).boxed()
    }
}

/// Go `test.NewMockGetFn(nil, fns...)`. `object` builds what the server holds. It is a
/// constructor, not a mutator of an empty value, so a kind under test needs no `Default`.
pub fn new_mock_get_fn<K: Object>(object: impl Fn() -> K + Send + Sync + 'static) -> MockGetFn {
    Box::new(move |_, _| Ok(Box::new(object())))
}

/// Go `test.NewMockGetFn(err)`.
pub fn new_mock_get_err_fn(err: Error) -> MockGetFn {
    Box::new(move |_, _| Err(err.clone()))
}

/// Go `test.NewMockUpdateFn(err, fns...)` and the create and status variants. With a no-op
/// `mutate` the object stays as the caller left it. A `mutate` that overwrites fields models the
/// server's response replacing the in-memory object (parity spec S5).
pub fn new_mock_object_fn(
    err: Option<Error>,
    mutate: impl Fn(&mut dyn Object) -> Result<()> + Send + Sync + 'static,
) -> MockObjectFn {
    Box::new(move |obj| {
        mutate(obj)?;
        err.clone().map_or(Ok(()), Err)
    })
}

/// Go `test.MockSubResourceUpdateFn`: the raw closure where tables assert the written status.
pub fn mock_status_update_fn(f: impl Fn(&mut dyn Object) -> Result<()> + Send + Sync + 'static) -> MockObjectFn {
    Box::new(f)
}

/// Now, truncated to seconds, so a value set through the annotation helpers round-trips exactly.
pub fn now() -> Timestamp {
    let t = Timestamp::now();
    t.checked_sub(t.subsec_nanosecond().nanoseconds()).expect("in range")
}

/// Go `cmp.Diff(want, got, test.EquateConditions())`: compares the JSON forms with conditions
/// sorted by type and `lastTransitionTime` removed.
pub fn assert_object_eq(want: &dyn Object, got: &dyn Object, reason: &str) {
    let (w, g) = (normalize(want), normalize(got));
    assert!(w == g, "\nReason: {reason}\n-want: {w:#}\n+got:  {g:#}");
}

fn normalize(o: &dyn Object) -> serde_json::Value {
    let mut v = o.to_json().expect("serializable object");
    if let Some(conds) = v.pointer_mut("/status/conditions").and_then(|c| c.as_array_mut()) {
        for c in conds.iter_mut() {
            if let Some(m) = c.as_object_mut() {
                m.remove("lastTransitionTime");
            }
        }
        conds.sort_by(|a, b| a["type"].as_str().cmp(&b["type"].as_str()));
    }
    v
}

/// The comparison every Go table makes on a returned `(value, error)` pair. Errors compare by
/// `test.EquateErrors` through `Error`'s `PartialEq`.
pub fn assert_result<T: PartialEq + Debug>(name: &str, reason: &str, want: &Result<T>, got: &Result<T>) {
    assert!(want == got, "\n{name}\nReason: {reason}\n-want {want:?}, +got {got:?}");
}

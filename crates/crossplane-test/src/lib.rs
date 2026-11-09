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
use crossplane_core::v2::Condition;
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

/// Go `test.NewMockGetFn(nil)`: the server holds an object of the requested type with nothing set
/// but its type. Panics when the kind cannot decode from empty metadata, as a kind with required
/// spec fields cannot; such a case needs [`new_mock_get_fn`].
pub fn new_mock_get_empty_fn() -> MockGetFn {
    Box::new(|object_type, _| {
        let empty = serde_json::json!({"metadata": {}});
        Ok(object_type
            .decode(empty)
            .unwrap_or_else(|err| panic!("{object_type:?} cannot be built empty: {err}")))
    })
}

/// Go `test.NewMockListFn(nil, fns...)`, where the functions fill the list's items.
pub fn new_mock_list_fn<K: Object>(items: impl Fn() -> Vec<K> + Send + Sync + 'static) -> MockListFn {
    Box::new(move |_, _| Ok(items().into_iter().map(|o| Box::new(o) as Box<dyn Object>).collect()))
}

/// Go `test.NewMockListFn(err)`.
pub fn new_mock_list_err_fn(err: Error) -> MockListFn {
    Box::new(move |_, _| Err(err.clone()))
}

/// Go `test.NewMockCreateFn(err)`. `None` is Go's `nil`.
pub fn new_mock_create_fn(err: Option<Error>) -> MockObjectFn {
    new_mock_object_fn(err, |_| Ok(()))
}

/// Go `test.NewMockUpdateFn(err)`. The object stays as the caller left it.
pub fn new_mock_update_fn(err: Option<Error>) -> MockObjectFn {
    new_mock_object_fn(err, |_| Ok(()))
}

/// Go `test.NewMockSubResourceUpdateFn(err)`, for `MockClient::update_status`.
pub fn new_mock_sub_resource_update_fn(err: Option<Error>) -> MockObjectFn {
    new_mock_object_fn(err, |_| Ok(()))
}

/// Go `test.NewMockPatchFn(err)`.
pub fn new_mock_patch_fn(err: Option<Error>) -> MockPatchFn {
    Box::new(move |_, _| err.clone().map_or(Ok(()), Err))
}

/// Go `test.NewMockDeleteFn(err)`.
pub fn new_mock_delete_fn(err: Option<Error>) -> MockDeleteFn {
    Box::new(move |_| err.clone().map_or(Ok(()), Err))
}

/// A successful write whose server response is `response()`. The response replaces the caller's
/// object, status and resourceVersion included, as Go's client decodes the response into the
/// object passed to `Update` (parity spec S5).
pub fn new_mock_response_fn<K: Object>(response: impl Fn() -> K + Send + Sync + 'static) -> MockObjectFn {
    Box::new(move |obj| obj.set_from_json(response().to_json()?))
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

/// Go `cmp.Diff(want, got, test.EquateConditions())`: compares the JSON forms, so two kinds
/// never compare equal, with `status.conditions` compared as [`assert_conditions_eq`] does.
pub fn assert_object_eq(want: &dyn Object, got: &dyn Object, reason: &str) {
    let mut w = want.to_json().expect("serializable object");
    let mut g = got.to_json().expect("serializable object");
    if let (Some(wc), Some(gc)) = (conditions_mut(&mut w), conditions_mut(&mut g)) {
        equate_conditions(wc, gc);
    }
    assert!(w == g, "\nReason: {reason}\n-want: {w:#}\n+got:  {g:#}");
}

/// Go `cmp.Diff(want, got, test.EquateConditions())` on condition slices: sorted by type, then
/// compared pairwise by Go's `Condition.Equal`, which ignores `lastTransitionTime`, and ignores
/// `observedGeneration` when either side's is zero.
pub fn assert_conditions_eq(want: &[Condition], got: &[Condition], reason: &str) {
    let to_json = |c: &[Condition]| serde_json::to_value(c).expect("serializable conditions");
    let (mut w, mut g) = (to_json(want), to_json(got));
    equate_conditions(w.as_array_mut().expect("array"), g.as_array_mut().expect("array"));
    assert!(w == g, "\nReason: {reason}\n-want: {w:#}\n+got:  {g:#}");
}

fn conditions_mut(v: &mut serde_json::Value) -> Option<&mut Vec<serde_json::Value>> {
    v.pointer_mut("/status/conditions").and_then(|c| c.as_array_mut())
}

/// Rewrites both sides so that `==` on the JSON is Go's comparison.
fn equate_conditions(want: &mut [serde_json::Value], got: &mut [serde_json::Value]) {
    let by_type = |a: &serde_json::Value, b: &serde_json::Value| a["type"].as_str().cmp(&b["type"].as_str());
    want.sort_by(by_type);
    got.sort_by(by_type);
    for c in want.iter_mut().chain(got.iter_mut()) {
        if let Some(m) = c.as_object_mut() {
            m.remove("lastTransitionTime");
        }
    }
    let generation = |c: &serde_json::Value| c.get("observedGeneration").and_then(|g| g.as_i64()).unwrap_or(0);
    for (w, g) in want.iter_mut().zip(got.iter_mut()) {
        if generation(w) == 0 || generation(g) == 0 {
            for c in [w, g] {
                if let Some(m) = c.as_object_mut() {
                    m.remove("observedGeneration");
                }
            }
        }
    }
}

/// The comparison every Go table makes on a returned `(value, error)` pair. Errors compare by
/// `test.EquateErrors` through `Error`'s `PartialEq`.
pub fn assert_result<T: PartialEq + Debug>(name: &str, reason: &str, want: &Result<T>, got: &Result<T>) {
    assert!(want == got, "\n{name}\nReason: {reason}\n-want {want:?}, +got {got:?}");
}

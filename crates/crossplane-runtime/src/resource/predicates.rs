use std::collections::BTreeMap;

use crossplane_core::meta::{ANNOTATION_KEY_EXTERNAL_CREATE_FAILED, ANNOTATION_KEY_EXTERNAL_CREATE_PENDING};
use crossplane_core::resource::Object;

use crate::controller::Predicate;

/// Go `resource.DesiredStateChanged`. Status-only writes and resyncs pass none of its three
/// checks, which is what keeps the managed reconciler from looping on its own status writes.
#[derive(Clone, Copy, Debug, Default)]
pub struct DesiredStateChanged;

pub fn desired_state_changed() -> DesiredStateChanged {
    DesiredStateChanged
}

type Map = Option<BTreeMap<String, String>>;

/// Go `maps.Equal` after dropping the ignored keys: a missing map equals an empty one.
fn same(a: &Map, b: &Map, ignored: &[&str]) -> bool {
    let keep = |m: &Map| -> BTreeMap<String, String> {
        m.iter()
            .flatten()
            .filter(|(k, _)| !ignored.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    keep(a) == keep(b)
}

impl Predicate for DesiredStateChanged {
    fn update(&self, old: &dyn Object, new: &dyn Object) -> bool {
        new.meta().generation != old.meta().generation
            || !same(&old.meta().labels, &new.meta().labels, &[])
            || !same(
                &old.meta().annotations,
                &new.meta().annotations,
                &[
                    ANNOTATION_KEY_EXTERNAL_CREATE_FAILED,
                    ANNOTATION_KEY_EXTERNAL_CREATE_PENDING,
                ],
            )
    }
}

#[cfg(test)]
#[path = "predicates_test.rs"]
mod tests;

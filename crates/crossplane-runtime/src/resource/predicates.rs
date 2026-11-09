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

impl Predicate for DesiredStateChanged {
    fn update(&self, old: &dyn Object, new: &dyn Object) -> bool {
        let _ = (
            old.meta(),
            new.meta(),
            ANNOTATION_KEY_EXTERNAL_CREATE_PENDING,
            ANNOTATION_KEY_EXTERNAL_CREATE_FAILED,
        );
        // TODO (pkg/resource/predicates.go:55): generation changed, or labels changed, or
        // annotations changed ignoring the two create annotations. A nil map equals an empty one.
        todo!()
    }
}

//! `pkg/gate`: run a callback once, when every dependency is set true.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;

type Callback = Box<dyn FnOnce() + Send + 'static>;

/// Go `gate.Gate[T]`. Shared through `Arc` in `controller::Options`.
pub struct Gate<T> {
    state: Mutex<GateState<T>>,
}

struct GateState<T> {
    values: HashMap<T, bool>,
    pending: Vec<(Callback, Vec<T>)>,
}

impl<T> Default for Gate<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(GateState {
                values: HashMap::new(),
                pending: Vec::new(),
            }),
        }
    }
}

impl<T: Hash + Eq + Clone + Send + 'static> Gate<T> {
    /// `f` runs once, on a spawned task (Go runs it on a new goroutine), after the lock is
    /// released.
    pub fn register(&self, f: impl FnOnce() + Send + 'static, deps: impl IntoIterator<Item = T>) {
        let _ = (
            &self.state,
            Box::new(f) as Callback,
            deps.into_iter().collect::<Vec<_>>(),
        );
        todo!("push, then release every registration whose deps are all true")
    }

    /// Returns true when the stored value changed or was absent. Registrations are evaluated only
    /// on a change.
    pub fn set(&self, key: T, value: bool) -> bool {
        let _ = (&self.state, key, value);
        todo!("update value; on change, release satisfied registrations")
    }
}

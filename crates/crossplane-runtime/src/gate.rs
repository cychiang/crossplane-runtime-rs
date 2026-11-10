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
        let released = {
            let mut state = self.state.lock().unwrap();
            state.pending.push((Box::new(f), deps.into_iter().collect()));
            state.take_released()
        };
        spawn_all(released);
    }

    /// Returns true when the stored value changed or was absent. Registrations are evaluated only
    /// on a change.
    pub fn set(&self, key: T, value: bool) -> bool {
        let released = {
            let mut state = self.state.lock().unwrap();
            if state.values.insert(key, value) == Some(value) {
                return false;
            }
            state.take_released()
        };
        spawn_all(released);
        true
    }
}

/// Go `controller.Gate`'s `Set`, the half the CRD reconciler needs. The reconciler holds this so
/// a test can record the calls (Go's `MockGate`).
pub trait SetGate<T>: Send + Sync {
    fn set(&self, key: T, value: bool) -> bool;
}

impl<T: Hash + Eq + Clone + Send + 'static> SetGate<T> for Gate<T> {
    fn set(&self, key: T, value: bool) -> bool {
        Gate::set(self, key, value)
    }
}

impl<T: Hash + Eq> GateState<T> {
    fn take_released(&mut self) -> Vec<Callback> {
        let values = &self.values;
        let (released, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|(_, deps)| deps.iter().all(|d| values.get(d).copied().unwrap_or(false)));
        self.pending = waiting;
        released.into_iter().map(|(f, _)| f).collect()
    }
}

/// A callback may register controllers, which needs a runtime; outside one it gets a thread.
fn spawn_all(callbacks: Vec<Callback>) {
    for f in callbacks {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => drop(handle.spawn_blocking(f)),
            Err(_) => drop(std::thread::spawn(f)),
        }
    }
}

#[cfg(test)]
#[path = "gate_test.rs"]
mod tests;

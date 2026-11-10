/// Go `resource.LateInitializer`. One generic method replaces the four typed Go helpers
/// (`LateInitializeStringPtr`, `Int64Ptr`, `BoolPtr`, `TimePtr`).
#[derive(Debug, Default)]
pub struct LateInitializer {
    changed: bool,
}

impl LateInitializer {
    /// Keeps `org` when set or when `from` is `None`. Otherwise takes `from` and remembers that
    /// something changed.
    pub fn late_initialize<T>(&mut self, org: &mut Option<T>, from: Option<T>) {
        if org.is_some() || from.is_none() {
            return;
        }
        self.changed = true;
        *org = from;
    }

    /// Go `SetChanged`.
    pub fn set_changed(&mut self) {
        self.changed = true;
    }

    pub fn is_changed(&self) -> bool {
        self.changed
    }
}

#[cfg(test)]
#[path = "late_init_test.rs"]
mod tests;

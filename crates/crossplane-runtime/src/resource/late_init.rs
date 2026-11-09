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
        let _ = (org, from);
        todo!()
    }

    pub fn is_changed(&self) -> bool {
        self.changed
    }
}

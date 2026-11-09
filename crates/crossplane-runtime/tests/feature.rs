//! Port of `pkg/feature/feature_test.go` `TestEnable`.

use crossplane_runtime::feature::{Features, Flag};

// go: pkg/feature TestEnable
#[test]
fn enable() {
    const COOL: Flag = Flag("cool");

    let mut f = Features::default();
    f.enable(COOL);
    assert!(f.enabled(COOL), "EnableMutatesZeroValue: f.enabled(...)");

    let f = Features::default();
    assert!(!f.enabled(COOL), "EnabledOnEmptyFlagsReturnsFalse: f.enabled(...)");

    // Go's third subtest calls Enabled on a nil *Flags. A Rust receiver cannot be null.
}

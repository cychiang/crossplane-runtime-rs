use std::collections::BTreeMap;

use crossplane_core::meta::ANNOTATION_KEY_EXTERNAL_CREATE_PENDING;
use crossplane_core::resource::Managed;
use crossplane_core::v2::Condition;
use crossplane_test::fake::ModernManaged;

use super::*;

struct Case {
    name: &'static str,
    old: ModernManaged,
    new: ModernManaged,
    want: bool,
}

fn map(k: &str, v: &str) -> Option<BTreeMap<String, String>> {
    Some([(k.to_owned(), v.to_owned())].into())
}

#[test]
fn desired_state_changed_table() {
    let with = |f: fn(&mut ModernManaged)| {
        let mut mg = ModernManaged::default();
        f(&mut mg);
        mg
    };
    let cases = vec![
        // go: pkg/resource TestDesiredStateChanged/NothingChanged
        Case {
            name: "NothingChanged",
            old: ModernManaged::default(),
            new: ModernManaged::default(),
            want: false,
        },
        // go: pkg/resource TestDesiredStateChanged/StatusChanged
        Case {
            name: "StatusChanged",
            old: ModernManaged::default(),
            new: with(|mg| mg.set_conditions([Condition::reconcile_success()])),
            want: false,
        },
        // go: pkg/resource TestDesiredStateChanged/IgnoredAnnotationsChanged
        Case {
            name: "IgnoredAnnotationsChanged",
            old: ModernManaged::default(),
            new: with(|mg| mg.metadata.annotations = map(ANNOTATION_KEY_EXTERNAL_CREATE_PENDING, "now")),
            want: false,
        },
        // go: pkg/resource TestDesiredStateChanged/AnnotationsChanged
        Case {
            name: "AnnotationsChanged",
            old: ModernManaged::default(),
            new: with(|mg| mg.metadata.annotations = map("foo", "bar")),
            want: true,
        },
        // go: pkg/resource TestDesiredStateChanged/LabelsChanged
        Case {
            name: "LabelsChanged",
            old: ModernManaged::default(),
            new: with(|mg| mg.metadata.labels = map("foo", "bar")),
            want: true,
        },
        // go: pkg/resource TestDesiredStateChanged/GenerationChanged
        Case {
            name: "GenerationChanged",
            old: with(|mg| mg.metadata.generation = Some(1)),
            new: with(|mg| mg.metadata.generation = Some(2)),
            want: true,
        },
    ];
    for tc in cases {
        let got = desired_state_changed().update(&tc.old, &tc.new);
        assert_eq!(tc.want, got, "{}: DesiredStateChanged().update", tc.name);
    }
}

#[test]
fn an_empty_map_equals_a_missing_one() {
    let mut new = ModernManaged::default();
    new.metadata.labels = Some(BTreeMap::new());
    new.metadata.annotations = Some(BTreeMap::new());
    assert!(!desired_state_changed().update(&ModernManaged::default(), &new));
}

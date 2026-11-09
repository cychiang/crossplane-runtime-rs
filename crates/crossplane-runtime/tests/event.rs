//! Port of `pkg/event/event_test.go` `TestSliceMap`. Go's `sliceMap` folds a variadic
//! `key, value, key, value` list into the annotation map of an event. The Rust counterpart is
//! `Event::with_annotation`, called once per pair, so the two cases that feed Go an unpaired
//! trailing key (`NoValue`, `ExtraneousKey`) cannot be written and are in `docs/port-skips.tsv`.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crossplane_runtime::event::{Event, Reason};

fn annotations(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

#[test]
fn slice_map() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        from: Vec<(&'static str, &'static str)>,
        want: BTreeMap<String, String>,
    }
    let cases = vec![
        // go: pkg/event TestSliceMap/OnePair
        Case {
            name: "OnePair",
            reason: "One key value pair should be added.",
            from: vec![("key", "val")],
            want: annotations(&[("key", "val")]),
        },
        // go: pkg/event TestSliceMap/TwoPairs
        Case {
            name: "TwoPairs",
            reason: "Two key value pairs should be added.",
            from: vec![("key", "val"), ("another", "value")],
            want: annotations(&[("key", "val"), ("another", "value")]),
        },
    ];
    for tc in cases {
        let event = tc.from.iter().fold(
            Event::normal(Reason(Cow::Borrowed("Reason")), "message"),
            |e, (k, v)| e.with_annotation(k, v),
        );
        assert_eq!(tc.want, event.annotations, "{}\n{}: annotations", tc.reason, tc.name);
    }
}

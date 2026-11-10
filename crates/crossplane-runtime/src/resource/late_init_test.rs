use k8s_openapi::jiff::{Timestamp, ToSpan};

use super::*;

struct Case<T> {
    name: &'static str,
    org: Option<T>,
    from: Option<T>,
    want: Option<T>,
    changed: bool,
}

fn check<T: PartialEq + std::fmt::Debug>(cases: Vec<Case<T>>) {
    for tc in cases {
        let mut li = LateInitializer::default();
        let mut org = tc.org;
        li.late_initialize(&mut org, tc.from);
        assert_eq!(tc.want, org, "{}: result", tc.name);
        assert_eq!(tc.changed, li.is_changed(), "{}: IsChanged", tc.name);
    }
}

#[test]
fn late_initialize_string_ptr() {
    let (s1, s2) = ("desired".to_owned(), "observed".to_owned());
    check(vec![
        // go: pkg/resource TestLateInitializeStringPtr/Original
        Case {
            name: "Original",
            org: Some(s1.clone()),
            from: Some(s2.clone()),
            want: Some(s1),
            changed: false,
        },
        // go: pkg/resource TestLateInitializeStringPtr/LateInitialized
        Case {
            name: "LateInitialized",
            org: None,
            from: Some(s2.clone()),
            want: Some(s2),
            changed: true,
        },
        // go: pkg/resource TestLateInitializeStringPtr/Neither
        Case {
            name: "Neither",
            org: None,
            from: None,
            want: None,
            changed: false,
        },
    ]);
}

#[test]
fn late_initialize_int64_ptr() {
    check(vec![
        // go: pkg/resource TestLateInitializeInt64Ptr/Original
        Case {
            name: "Original",
            org: Some(10i64),
            from: Some(20),
            want: Some(10),
            changed: false,
        },
        // go: pkg/resource TestLateInitializeInt64Ptr/LateInitialized
        Case {
            name: "LateInitialized",
            org: None,
            from: Some(20),
            want: Some(20),
            changed: true,
        },
        // go: pkg/resource TestLateInitializeInt64Ptr/Neither
        Case {
            name: "Neither",
            org: None,
            from: None,
            want: None,
            changed: false,
        },
    ]);
}

#[test]
fn late_initialize_bool_ptr() {
    check(vec![
        // go: pkg/resource TestLateInitializeBoolPtr/Original
        Case {
            name: "Original",
            org: Some(true),
            from: Some(false),
            want: Some(true),
            changed: false,
        },
        // go: pkg/resource TestLateInitializeBoolPtr/LateInitialized
        Case {
            name: "LateInitialized",
            org: None,
            from: Some(false),
            want: Some(false),
            changed: true,
        },
        // go: pkg/resource TestLateInitializeBoolPtr/Neither
        Case {
            name: "Neither",
            org: None,
            from: None,
            want: None,
            changed: false,
        },
    ]);
}

#[test]
fn late_initialize_time_ptr() {
    let t1 = Timestamp::now();
    let t2 = t1.checked_add(1.minute()).unwrap();
    check(vec![
        // go: pkg/resource TestLateInitializeTimePtr/Original
        Case {
            name: "Original",
            org: Some(t1),
            from: Some(t2),
            want: Some(t1),
            changed: false,
        },
        // go: pkg/resource TestLateInitializeTimePtr/LateInitialized
        Case {
            name: "LateInitialized",
            org: None,
            from: Some(t2),
            want: Some(t2),
            changed: true,
        },
        // go: pkg/resource TestLateInitializeTimePtr/Neither
        Case {
            name: "Neither",
            org: None,
            from: None,
            want: None,
            changed: false,
        },
    ]);
}

#[test]
fn changed_is_sticky_and_settable() {
    let mut li = LateInitializer::default();
    let mut v = None;
    li.late_initialize(&mut v, Some(1));
    li.late_initialize(&mut v, None);
    assert!(li.is_changed());
    let mut li = LateInitializer::default();
    li.set_changed();
    assert!(li.is_changed());
}

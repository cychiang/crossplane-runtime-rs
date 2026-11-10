use crossplane_core::errors::Error;
use crossplane_core::v2::{Condition, ConditionStatus, ProviderConfigReference};
use crossplane_test::fake::ModernManaged;

use super::*;

fn is(answer: bool) -> impl Fn(&Error) -> bool {
    move |_| answer
}

#[test]
fn ignore_table() {
    struct Case {
        name: &'static str,
        is: bool,
        want: Result<(), Error>,
    }
    let boom = || Error::new("boom");
    let cases = vec![
        // go: pkg/resource TestIgnore/IgnoreError
        Case {
            name: "IgnoreError",
            is: true,
            want: Ok(()),
        },
        // go: pkg/resource TestIgnore/PropagateError
        Case {
            name: "PropagateError",
            is: false,
            want: Err(boom()),
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, ignore(is(tc.is), Err(boom())), "{}", tc.name);
    }
}

#[test]
fn ignore_any_table() {
    struct Case {
        name: &'static str,
        is: Vec<bool>,
        want: Result<(), Error>,
    }
    let boom = || Error::new("boom");
    let cases = vec![
        // go: pkg/resource TestIgnoreAny/IgnoreError
        Case {
            name: "IgnoreError",
            is: vec![true],
            want: Ok(()),
        },
        // go: pkg/resource TestIgnoreAny/IgnoreErrorArr
        Case {
            name: "IgnoreErrorArr",
            is: vec![true, false],
            want: Ok(()),
        },
        // go: pkg/resource TestIgnoreAny/PropagateError
        Case {
            name: "PropagateError",
            is: vec![false],
            want: Err(boom()),
        },
    ];
    for tc in cases {
        let fns: Vec<_> = tc.is.iter().map(|a| is(*a)).collect();
        let refs: Vec<&dyn Fn(&Error) -> bool> = fns.iter().map(|f| f as &dyn Fn(&Error) -> bool).collect();
        assert_eq!(tc.want, ignore_any(Err(boom()), &refs), "{}", tc.name);
    }
}

#[test]
fn is_condition_true_table() {
    struct Case {
        name: &'static str,
        status: ConditionStatus,
        want: bool,
    }
    let cases = vec![
        // go: pkg/resource TestIsConditionTrue/IsTrue
        Case {
            name: "IsTrue",
            status: ConditionStatus::True,
            want: true,
        },
        // go: pkg/resource TestIsConditionTrue/IsFalse
        Case {
            name: "IsFalse",
            status: ConditionStatus::False,
            want: false,
        },
        // go: pkg/resource TestIsConditionTrue/IsUnknown
        Case {
            name: "IsUnknown",
            status: ConditionStatus::Unknown,
            want: false,
        },
        // go: pkg/resource TestIsConditionTrue/IsUnset
        Case {
            name: "IsUnset",
            status: ConditionStatus::Unknown,
            want: false,
        },
    ];
    for tc in cases {
        let c = Condition {
            status: tc.status,
            ..Condition::available()
        };
        assert_eq!(tc.want, is_condition_true(&c), "{}", tc.name);
    }
}

#[test]
fn is_api_error_wrapped_table() {
    struct Case {
        name: &'static str,
        err: Error,
        want: bool,
    }
    let cases = vec![
        // go: pkg/resource TestIsAPIErrorWrapped/NotAPIError
        Case {
            name: "NotAPIError",
            err: Error::new("test-error"),
            want: false,
        },
        // go: pkg/resource TestIsAPIErrorWrapped/APIError
        Case {
            name: "APIError",
            err: Error::not_found(),
            want: true,
        },
        // go: pkg/resource TestIsAPIErrorWrapped/WrappedAPIError
        Case {
            name: "WrappedAPIError",
            err: Error::not_found().wrap("test-wrapper"),
            want: true,
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, is_api_error_wrapped(&tc.err), "{}", tc.name);
    }
}

#[test]
fn get_external_tags_table() {
    struct Case {
        name: &'static str,
        namespace: Option<&'static str>,
        want: BTreeMap<String, String>,
    }
    let kind = "modernmanaged.fake.crossplane.io";
    let tags = |extra: &[(&str, &str)]| -> BTreeMap<String, String> {
        [
            (EXTERNAL_RESOURCE_TAG_KEY_KIND, kind),
            (EXTERNAL_RESOURCE_TAG_KEY_NAME, "cool"),
            (EXTERNAL_RESOURCE_TAG_KEY_PROVIDER, "prov"),
            (EXTERNAL_RESOURCE_TAG_KEY_PROVIDER_CONFIG_KIND, "ProviderConfig"),
        ]
        .iter()
        .chain(extra)
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
    };
    let cases = vec![
        // go: pkg/resource TestGetExternalTags/SuccessfulWithTypedProviderConfig
        Case {
            name: "SuccessfulWithTypedProviderConfig",
            namespace: None,
            want: tags(&[]),
        },
        // go: pkg/resource TestGetExternalTags/SuccessfulWithNamespacedObject
        Case {
            name: "SuccessfulWithNamespacedObject",
            namespace: Some("coolns"),
            want: tags(&[(EXTERNAL_RESOURCE_TAG_KEY_NAMESPACE, "coolns")]),
        },
    ];
    for tc in cases {
        let mut mg = ModernManaged::default();
        mg.metadata.name = Some("cool".into());
        mg.metadata.namespace = tc.namespace.map(str::to_owned);
        mg.set_provider_config_reference(Some(ProviderConfigReference {
            name: "prov".into(),
            kind: "ProviderConfig".into(),
        }));
        assert_eq!(tc.want, get_external_tags(&mg), "{}", tc.name);
    }
}

#[test]
fn tags_omit_an_absent_provider_config() {
    let got = get_external_tags(&ModernManaged::default());
    assert_eq!(got.len(), 2, "{got:?}");
}

// go: pkg/resource TestFirstNAndSomeMore
#[test]
fn first_n_and_some_more_cases() {
    let v = |s: &str| -> Vec<String> { s.split_whitespace().map(str::to_owned).collect() };
    let cases: Vec<(i32, Vec<String>, &str)> = vec![
        (3, v("a b c d e"), "a, b, c, and 2 more"),
        (3, v("a b c"), "a, b, and c"),
        (3, v("a b"), "a, b"),
        (3, v("a"), "a"),
        (3, v(""), ""),
        (3, v("a c e b d"), "a, c, e, and 2 more"),
        (3, v("a b b b d"), "a, b, b, and 2 more"),
        (2, v("a c e b d"), "a, c, and 3 more"),
        (0, v("a c e b d"), "5"),
        (-7, v("a c e b d"), "5"),
        (1, v("a c e b d"), "a, and 4 more"),
    ];
    for (n, names, want) in cases {
        assert_eq!(want, first_n_and_some_more(n, &names), "n={n} names={names:?}");
    }
}

#[test]
fn stable_n_sorts_a_copy() {
    let names: Vec<String> = ["c", "a", "b", "d"].iter().map(|s| (*s).to_owned()).collect();
    assert_eq!("a, b, c, and 1 more", stable_n_and_some_more(3, &names));
    assert_eq!(names[0], "c");
}

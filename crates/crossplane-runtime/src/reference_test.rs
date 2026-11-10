use crossplane_core::errors::Error;
use crossplane_core::v2::{Policy, ResolutionPolicy, ResolvePolicy, TypedReference};
use crossplane_test::fake::ModernManaged;
use crossplane_test::{
    MockClient, assert_result, new_mock_get_empty_fn, new_mock_get_err_fn, new_mock_get_fn, new_mock_list_err_fn,
    new_mock_list_fn,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::jiff::Timestamp;

use super::*;

const TEST_RESOURCE_NAME_PREFIX: &str = "cool-resource-";
const TEST_VALUE_PREFIX: &str = "cool-value-";
const TEST_CONTROLLER_UID: &str = "very-unique";

type Req = NamespacedResolutionRequest<ModernManaged>;
type MultiReq = MultiNamespacedResolutionRequest<ModernManaged>;

fn controlled_by(mut mg: ModernManaged, uid: &str) -> ModernManaged {
    let owner = meta::as_controller(&TypedReference {
        uid: uid.into(),
        ..Default::default()
    });
    meta::add_controller_reference(&mut mg, owner).unwrap();
    mg
}

fn named(name: &str, external_name: &str) -> ModernManaged {
    let mut mg = ModernManaged::default();
    mg.metadata.name = Some(name.into());
    meta::set_external_name(&mut mg, external_name);
    mg
}

fn controlled(name: &str, external_name: &str) -> ModernManaged {
    controlled_by(named(name, external_name), TEST_CONTROLLER_UID)
}

fn deleted() -> ModernManaged {
    let mut mg = ModernManaged::default();
    mg.metadata.deletion_timestamp = Some(Time(Timestamp::now()));
    mg
}

fn in_namespace(name: &str, namespace: &str) -> ModernManaged {
    let mut mg = ModernManaged::default();
    mg.metadata.name = Some(name.into());
    mg.metadata.namespace = Some(namespace.into());
    mg
}

fn err_boom() -> Error {
    Error::new("boom")
}

fn reference(name: &str, namespace: &str) -> NamespacedReference {
    NamespacedReference {
        name: name.into(),
        namespace: namespace.into(),
        policy: None,
    }
}

fn with_policy(mut r: NamespacedReference, policy: Policy) -> NamespacedReference {
    r.policy = Some(policy);
    r
}

fn optional() -> Policy {
    Policy {
        resolution: Some(ResolutionPolicy::Optional),
        ..Default::default()
    }
}

fn always() -> Policy {
    Policy {
        resolve: Some(ResolvePolicy::Always),
        ..Default::default()
    }
}

fn must_match_controller(policy: Option<Policy>) -> NamespacedSelector {
    NamespacedSelector {
        match_controller_ref: Some(true),
        policy,
        ..Default::default()
    }
}

fn get_with_value(value: &'static str) -> MockClient {
    MockClient {
        get: Some(new_mock_get_fn(move || named("", value))),
        ..Default::default()
    }
}

fn get_empty() -> MockClient {
    MockClient {
        get: Some(new_mock_get_empty_fn()),
        ..Default::default()
    }
}

fn get_err() -> MockClient {
    MockClient {
        get: Some(new_mock_get_err_fn(err_boom())),
        ..Default::default()
    }
}

fn list_of(items: Vec<ModernManaged>) -> MockClient {
    MockClient {
        list: Some(new_mock_list_fn(move || items.clone())),
        ..Default::default()
    }
}

fn list_err() -> MockClient {
    MockClient {
        list: Some(new_mock_list_err_fn(err_boom())),
        ..Default::default()
    }
}

fn get_in_namespace(want: &'static str, value: &'static str) -> MockClient {
    MockClient {
        get: Some(Box::new(move |_, key| {
            assert_eq!(
                key.namespace.as_deref(),
                Some(want),
                "Resolve did not use the namespace {want}: {key:?}"
            );
            Ok(Box::new(named("", value)))
        })),
        ..Default::default()
    }
}

fn resp(value: &str, reference: Option<NamespacedReference>) -> NamespacedResolutionResponse {
    NamespacedResolutionResponse {
        resolved_value: value.into(),
        resolved_reference: reference,
    }
}

fn multi_resp(values: &[&str], references: Vec<NamespacedReference>) -> MultiNamespacedResolutionResponse {
    MultiNamespacedResolutionResponse {
        resolved_values: values.iter().map(|v| (*v).to_owned()).collect(),
        resolved_references: references,
    }
}

fn no_value() -> ExtractValueFn<ModernManaged> {
    Box::new(|_| String::new())
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

#[tokio::test]
async fn namespaced_resolve() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        client: MockClient,
        from: ModernManaged,
        req: Req,
        want: Result<NamespacedResolutionResponse>,
    }

    let value = "coolv";
    let reference = reference("cool", "cool-ns");
    let ns_omitted_ref = self::reference("cool", "");
    let optional_ref = with_policy(reference.clone(), optional());
    let always_ref = with_policy(reference.clone(), always());
    let controlled = controlled(value, value);

    let cases = vec![
        // go: pkg/reference TestNamespacedResolve/FromDeleted
        Case {
            name: "FromDeleted",
            reason: "Should return early if the referencing managed resource was deleted",
            client: MockClient::default(),
            from: deleted(),
            req: Req::default(),
            want: Ok(resp("", None)),
        },
        // go: pkg/reference TestNamespacedResolve/AlreadyResolved
        Case {
            name: "AlreadyResolved",
            reason: "Should return early if the current value is non-zero",
            client: MockClient::default(),
            from: ModernManaged::default(),
            req: Req {
                current_value: value.into(),
                ..Default::default()
            },
            want: Ok(resp(value, None)),
        },
        // go: pkg/reference TestNamespacedResolve/AlwaysResolveReference
        Case {
            name: "AlwaysResolveReference",
            reason: "Should not return early if the current value is non-zero, when the resolve policy is set to Always",
            client: get_with_value("coolv"),
            from: ModernManaged::default(),
            req: Req {
                reference: Some(always_ref.clone()),
                current_value: "oldValue".into(),
                ..Default::default()
            },
            want: Ok(resp(value, Some(always_ref))),
        },
        // go: pkg/reference TestNamespacedResolve/Unresolvable
        Case {
            name: "Unresolvable",
            reason: "Should return early if neither a reference or selector were provided",
            client: MockClient::default(),
            from: ModernManaged::default(),
            req: Req::default(),
            want: Ok(resp("", None)),
        },
        // go: pkg/reference TestNamespacedResolve/GetError
        Case {
            name: "GetError",
            reason: "Should return errors encountered while getting the referenced resource",
            client: get_err(),
            from: ModernManaged::default(),
            req: Req {
                reference: Some(reference.clone()),
                ..Default::default()
            },
            want: Err(err_boom().wrap(ERR_GET_MANAGED)),
        },
        // go: pkg/reference TestNamespacedResolve/ResolvedNoValue
        Case {
            name: "ResolvedNoValue",
            reason: "Should return an error if the extract function returns the empty string",
            client: get_empty(),
            from: ModernManaged::default(),
            req: Req {
                reference: Some(reference.clone()),
                extract: no_value(),
                ..Default::default()
            },
            want: Err(Error::new(ERR_NO_VALUE)),
        },
        // go: pkg/reference TestNamespacedResolve/SuccessfulResolve
        Case {
            name: "SuccessfulResolve",
            reason: "No error should be returned when the value is successfully extracted",
            client: get_with_value("coolv"),
            from: ModernManaged::default(),
            req: Req {
                reference: Some(reference.clone()),
                ..Default::default()
            },
            want: Ok(resp(value, Some(reference.clone()))),
        },
        // go: pkg/reference TestNamespacedResolve/SuccessfulResolveNamespaced
        Case {
            name: "SuccessfulResolveNamespaced",
            reason: "Resolve should be successful when a namespace is given",
            client: get_with_value("coolv"),
            from: ModernManaged::default(),
            req: Req {
                reference: Some(reference.clone()),
                ..Default::default()
            },
            want: Ok(resp(value, Some(reference.clone()))),
        },
        // go: pkg/reference TestNamespacedResolve/SuccessfulResolveInferredNamespace
        Case {
            name: "SuccessfulResolveInferredNamespace",
            reason: "Resolve should be successful with namespace inferred from MR, when reference omits namespace",
            client: get_in_namespace("from-ns", "coolv"),
            from: in_namespace("some-mr", "from-ns"),
            req: Req {
                reference: Some(ns_omitted_ref.clone()),
                ..Default::default()
            },
            want: Ok(resp(value, Some(ns_omitted_ref))),
        },
        // go: pkg/reference TestNamespacedResolve/SuccessfulResolveCrossNamespace
        Case {
            name: "SuccessfulResolveCrossNamespace",
            reason: "Resolve should be successful when a namespace is given",
            client: get_in_namespace("cool-ns", "coolv"),
            from: in_namespace("some-mr", "from-ns"),
            req: Req {
                reference: Some(reference.clone()),
                ..Default::default()
            },
            want: Ok(resp(value, Some(reference.clone()))),
        },
        // go: pkg/reference TestNamespacedResolve/OptionalReference
        Case {
            name: "OptionalReference",
            reason: "No error should be returned when the resolution policy is Optional",
            client: get_empty(),
            from: ModernManaged::default(),
            req: Req {
                reference: Some(optional_ref.clone()),
                extract: no_value(),
                ..Default::default()
            },
            want: Ok(resp("", Some(optional_ref))),
        },
        // go: pkg/reference TestNamespacedResolve/ListError
        Case {
            name: "ListError",
            reason: "Should return errors encountered while listing potential referenced resources",
            client: list_err(),
            from: ModernManaged::default(),
            req: Req {
                selector: Some(NamespacedSelector::default()),
                ..Default::default()
            },
            want: Err(err_boom().wrap(ERR_LIST_MANAGED)),
        },
        // go: pkg/reference TestNamespacedResolve/NoMatches
        Case {
            name: "NoMatches",
            reason: "Should return an error when no managed resources match the selector",
            client: list_of(vec![]),
            from: ModernManaged::default(),
            req: Req {
                selector: Some(NamespacedSelector::default()),
                ..Default::default()
            },
            want: Err(Error::new(ERR_NO_MATCHES)),
        },
        // go: pkg/reference TestNamespacedResolve/OptionalSelector
        Case {
            name: "OptionalSelector",
            reason: "No error should be returned when the resolution policy is Optional",
            client: list_of(vec![]),
            from: ModernManaged::default(),
            req: Req {
                selector: Some(NamespacedSelector {
                    policy: Some(optional()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            want: Ok(resp("", None)),
        },
        // go: pkg/reference TestNamespacedResolve/SuccessfulSelect
        Case {
            name: "SuccessfulSelect",
            reason: "A managed resource with a matching controller reference should be selected and returned",
            client: list_of(vec![ModernManaged::default(), controlled.clone()]),
            from: controlled.clone(),
            req: Req {
                selector: Some(must_match_controller(None)),
                ..Default::default()
            },
            want: Ok(resp(value, Some(self::reference(value, "")))),
        },
        // go: pkg/reference TestNamespacedResolve/SuccessfulSelectNamespaced
        Case {
            name: "SuccessfulSelectNamespaced",
            reason: "Resolve should be successful when a namespace is given",
            client: list_of(vec![ModernManaged::default(), controlled.clone()]),
            from: controlled.clone(),
            req: Req {
                selector: Some(must_match_controller(None)),
                ..Default::default()
            },
            want: Ok(resp(value, Some(self::reference(value, "")))),
        },
        // go: pkg/reference TestNamespacedResolve/AlwaysResolveSelector
        Case {
            name: "AlwaysResolveSelector",
            reason: "Should not return early if the current value is non-zero, when the resolve policy is set to Always",
            client: list_of(vec![ModernManaged::default(), controlled.clone()]),
            from: controlled.clone(),
            req: Req {
                selector: Some(must_match_controller(Some(always()))),
                current_value: "oldValue".into(),
                ..Default::default()
            },
            want: Ok(resp(value, Some(self::reference(value, "")))),
        },
        // go: pkg/reference TestNamespacedResolve/BothReferenceSelector
        Case {
            name: "BothReferenceSelector",
            reason: "When both Reference and Selector fields set and Policy is not set, the Reference must be resolved",
            client: get_with_value("coolv"),
            from: ModernManaged::default(),
            req: Req {
                reference: Some(reference.clone()),
                selector: Some(must_match_controller(None)),
                ..Default::default()
            },
            want: Ok(resp(value, Some(reference))),
        },
    ];

    for tc in cases {
        let r = ApiNamespacedResolver::new(&tc.client, &tc.from);
        let got = r.resolve(tc.req).await;
        assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

fn test_examples(n: usize) -> (Vec<String>, Vec<NamespacedReference>, Vec<ModernManaged>) {
    let values: Vec<String> = (0..n).map(|i| format!("{TEST_VALUE_PREFIX}{i}")).collect();
    let refs: Vec<NamespacedReference> = (0..n)
        .map(|i| reference(&format!("{TEST_RESOURCE_NAME_PREFIX}{i}"), ""))
        .collect();
    let objects = refs.iter().zip(&values).map(|(r, v)| controlled(&r.name, v)).collect();
    (values, refs, objects)
}

#[tokio::test]
async fn namespaced_resolve_multiple() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        client: MockClient,
        from: ModernManaged,
        req: MultiReq,
        want: Result<MultiNamespacedResolutionResponse>,
    }

    let value = "coolv";
    let value2 = "cooler";
    let reference = reference("cool", "cool-ns");
    let ns_omitted_ref = self::reference("cool", "");
    let optional_ref = with_policy(reference.clone(), optional());
    let always_ref = with_policy(reference.clone(), always());
    let controlled1 = controlled(value, value);
    let controlled2 = controlled(value2, value2);
    let (test_values, test_refs, test_controlled) = test_examples(10);

    let cases = vec![
        // go: pkg/reference TestNamespacedResolveMultiple/FromDeleted
        Case {
            name: "FromDeleted",
            reason: "Should return early if the referencing managed resource was deleted",
            client: MockClient::default(),
            from: deleted(),
            req: MultiReq::default(),
            want: Ok(multi_resp(&[], vec![])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/AlreadyResolved
        Case {
            name: "AlreadyResolved",
            reason: "Should return early if the current value is non-zero",
            client: MockClient::default(),
            from: ModernManaged::default(),
            req: MultiReq {
                current_values: strings(&[value]),
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/AlwaysResolveReference
        Case {
            name: "AlwaysResolveReference",
            reason: "Should not return early if the current value is non-zero, when the resolve policy is set to Always",
            client: get_with_value("coolv"),
            from: ModernManaged::default(),
            req: MultiReq {
                references: vec![always_ref.clone()],
                current_values: strings(&["oldValue"]),
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![always_ref])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/Unresolvable
        Case {
            name: "Unresolvable",
            reason: "Should return early if neither a reference or selector were provided",
            client: MockClient::default(),
            from: ModernManaged::default(),
            req: MultiReq::default(),
            want: Ok(multi_resp(&[], vec![])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/GetError
        Case {
            name: "GetError",
            reason: "Should return errors encountered while getting the referenced resource",
            client: get_err(),
            from: ModernManaged::default(),
            req: MultiReq {
                references: vec![reference.clone()],
                ..Default::default()
            },
            want: Err(err_boom().wrap(ERR_GET_MANAGED)),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/ResolvedNoValue
        Case {
            name: "ResolvedNoValue",
            reason: "Should return an error if the extract function returns the empty string",
            client: get_empty(),
            from: ModernManaged::default(),
            req: MultiReq {
                references: vec![reference.clone()],
                extract: no_value(),
                ..Default::default()
            },
            want: Err(Error::new(ERR_NO_VALUE)),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/SuccessfulResolve
        Case {
            name: "SuccessfulResolve",
            reason: "No error should be returned when the value is successfully extracted",
            client: get_with_value("coolv"),
            from: ModernManaged::default(),
            req: MultiReq {
                references: vec![reference.clone()],
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![reference.clone()])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/SuccessfulResolveNamespaced
        Case {
            name: "SuccessfulResolveNamespaced",
            reason: "Resolve should be successful when a namespace is given",
            client: get_in_namespace("cool-ns", "coolv"),
            from: ModernManaged::default(),
            req: MultiReq {
                references: vec![reference.clone()],
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![reference.clone()])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/SuccessfulResolveInferredNamespace
        Case {
            name: "SuccessfulResolveInferredNamespace",
            reason: "Resolve should be successful when a namespace is given",
            client: get_in_namespace("from-ns", "coolv"),
            from: in_namespace("some-mr", "from-ns"),
            req: MultiReq {
                references: vec![ns_omitted_ref.clone()],
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![ns_omitted_ref])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/SuccessfulResolveCrossNamespace
        Case {
            name: "SuccessfulResolveCrossNamespace",
            reason: "Resolve should be successful when a namespace is given",
            client: get_in_namespace("cool-ns", "coolv"),
            from: in_namespace("some-mr", "from-ns"),
            req: MultiReq {
                references: vec![reference.clone()],
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![reference.clone()])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/OptionalReference
        Case {
            name: "OptionalReference",
            reason: "No error should be returned when the resolution policy is Optional",
            client: get_empty(),
            from: ModernManaged::default(),
            req: MultiReq {
                references: vec![optional_ref.clone()],
                extract: no_value(),
                ..Default::default()
            },
            want: Ok(multi_resp(&[""], vec![optional_ref])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/ListError
        Case {
            name: "ListError",
            reason: "Should return errors encountered while listing potential referenced resources",
            client: list_err(),
            from: ModernManaged::default(),
            req: MultiReq {
                selector: Some(NamespacedSelector::default()),
                ..Default::default()
            },
            want: Err(err_boom().wrap(ERR_LIST_MANAGED)),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/NoMatches
        Case {
            name: "NoMatches",
            reason: "Should return an error when no managed resources match the selector",
            client: list_of(vec![]),
            from: ModernManaged::default(),
            req: MultiReq {
                selector: Some(NamespacedSelector::default()),
                ..Default::default()
            },
            want: Err(Error::new(ERR_NO_MATCHES)),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/OptionalSelector
        Case {
            name: "OptionalSelector",
            reason: "No error should be returned when the resolution policy is Optional",
            client: list_of(vec![]),
            from: ModernManaged::default(),
            req: MultiReq {
                selector: Some(NamespacedSelector {
                    policy: Some(optional()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            want: Ok(multi_resp(&[], vec![])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/SuccessfulSelect
        Case {
            name: "SuccessfulSelect",
            reason: "A managed resource with a matching controller reference should be selected and returned",
            client: list_of(vec![ModernManaged::default(), controlled1.clone()]),
            from: controlled1.clone(),
            req: MultiReq {
                selector: Some(must_match_controller(None)),
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![self::reference(value, "")])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/SuccessfulSelectNamespaced
        Case {
            name: "SuccessfulSelectNamespaced",
            reason: "Resolve should be successful when a namespace is given",
            client: list_of(vec![ModernManaged::default(), controlled1.clone()]),
            from: controlled1.clone(),
            req: MultiReq {
                selector: Some(must_match_controller(None)),
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![self::reference(value, "")])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/AlwaysResolveSelector
        Case {
            name: "AlwaysResolveSelector",
            reason: "Should not return early if the current value is non-zero, when the resolve policy is set to Always",
            client: list_of(vec![ModernManaged::default(), controlled1.clone()]),
            from: controlled1.clone(),
            req: MultiReq {
                selector: Some(must_match_controller(Some(always()))),
                current_values: strings(&["oldValue"]),
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![self::reference(value, "")])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/BothReferenceSelector
        Case {
            name: "BothReferenceSelector",
            reason: "When both Reference and Selector fields set and Policy is not set, the Reference must be resolved",
            client: MockClient {
                list: Some(new_mock_list_err_fn(Error::new(
                    "unexpected call to List when resolving Refs only",
                ))),
                ..get_with_value("coolv")
            },
            from: ModernManaged::default(),
            req: MultiReq {
                references: vec![reference.clone()],
                selector: Some(must_match_controller(None)),
                ..Default::default()
            },
            want: Ok(multi_resp(&[value], vec![reference.clone()])),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/SelectorOrderOutput
        Case {
            name: "SelectorOrderOutput",
            reason: "Resolved values should be ordered when resolving a selector",
            client: list_of(vec![
                ModernManaged::default(),
                controlled1.clone(),
                ModernManaged::default(),
                controlled2.clone(),
            ]),
            from: controlled1.clone(),
            req: MultiReq {
                selector: Some(must_match_controller(None)),
                ..Default::default()
            },
            want: Ok(multi_resp(
                &[value2, value],
                vec![self::reference(value2, ""), self::reference(value, "")],
            )),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/NoSelectorOnlyRefs
        Case {
            name: "NoSelectorOnlyRefs",
            reason: "Refs should not be re-ordered when selector is omitted",
            client: MockClient {
                list: Some(new_mock_list_err_fn(Error::new(
                    "unexpected call to List when resolving Refs only",
                ))),
                get: Some(Box::new(|_, key| {
                    if !key.name.starts_with(TEST_RESOURCE_NAME_PREFIX) {
                        return Err(Error::new("test resource not found"));
                    }
                    let val = key.name.replacen(TEST_RESOURCE_NAME_PREFIX, TEST_VALUE_PREFIX, 1);
                    Ok(Box::new(named("", &val)))
                })),
                ..Default::default()
            },
            from: controlled1.clone(),
            req: MultiReq {
                references: vec![
                    test_refs[2].clone(),
                    test_refs[3].clone(),
                    test_refs[0].clone(),
                    test_refs[1].clone(),
                ],
                ..Default::default()
            },
            want: Ok(MultiNamespacedResolutionResponse {
                resolved_values: vec![
                    test_values[2].clone(),
                    test_values[3].clone(),
                    test_values[0].clone(),
                    test_values[1].clone(),
                ],
                resolved_references: vec![
                    test_refs[2].clone(),
                    test_refs[3].clone(),
                    test_refs[0].clone(),
                    test_refs[1].clone(),
                ],
            }),
        },
        // go: pkg/reference TestNamespacedResolveMultiple/AlwaysResolveSelector_NewValuesOrdered
        Case {
            name: "AlwaysResolveSelector_NewValuesOrdered",
            reason: "Must resolve new matches and reorder resolved values & refs, when Selector policy is Always and have existing refs and values",
            client: list_of(vec![
                ModernManaged::default(),
                test_controlled[2].clone(),
                ModernManaged::default(),
                test_controlled[4].clone(),
                test_controlled[1].clone(),
            ]),
            from: controlled1.clone(),
            req: MultiReq {
                current_values: vec![test_values[1].clone(), test_values[4].clone()],
                references: vec![test_refs[1].clone(), test_refs[4].clone()],
                selector: Some(must_match_controller(Some(always()))),
                ..Default::default()
            },
            want: Ok(MultiNamespacedResolutionResponse {
                resolved_values: vec![test_values[1].clone(), test_values[2].clone(), test_values[4].clone()],
                resolved_references: vec![test_refs[1].clone(), test_refs[2].clone(), test_refs[4].clone()],
            }),
        },
    ];

    for tc in cases {
        let r = ApiNamespacedResolver::new(&tc.client, &tc.from);
        let got = r.resolve_multiple(tc.req).await;
        assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

#[test]
fn namespaced_controllers_must_match() {
    struct Case {
        name: &'static str,
        s: Option<NamespacedSelector>,
        want: bool,
    }
    let cases = vec![
        // go: pkg/reference TestNamespacedControllersMustMatch/NilSelector
        Case {
            name: "NilSelector",
            s: None,
            want: false,
        },
        // go: pkg/reference TestNamespacedControllersMustMatch/NilMatchControllerRef
        Case {
            name: "NilMatchControllerRef",
            s: Some(NamespacedSelector::default()),
            want: false,
        },
        // go: pkg/reference TestNamespacedControllersMustMatch/False
        Case {
            name: "False",
            s: Some(NamespacedSelector {
                match_controller_ref: Some(false),
                ..Default::default()
            }),
            want: false,
        },
        // go: pkg/reference TestNamespacedControllersMustMatch/True
        Case {
            name: "True",
            s: Some(NamespacedSelector {
                match_controller_ref: Some(true),
                ..Default::default()
            }),
            want: true,
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, controllers_must_match_namespaced(tc.s.as_ref()), "{}", tc.name);
    }
}

#[test]
fn to_and_from_ptr() {
    struct Case {
        name: &'static str,
        want: Vec<&'static str>,
    }

    let cases = [
        // go: pkg/reference TestToAndFromPtr/Zero
        Case {
            name: "Zero",
            want: vec![""],
        },
        // go: pkg/reference TestToAndFromPtr/NonZero
        Case {
            name: "NonZero",
            want: vec!["pointy"],
        },
    ];
    for tc in cases {
        let got = from_ptr_value(to_ptr_value(tc.want[0]).as_deref());
        assert_eq!(tc.want[0], got, "{}", tc.name);
    }
}

#[test]
fn to_and_from_float_ptr() {
    struct Case {
        name: &'static str,
        want: Vec<&'static str>,
    }

    let cases = [
        // go: pkg/reference TestToAndFromFloatPtr/Zero
        Case {
            name: "Zero",
            want: vec![""],
        },
        // go: pkg/reference TestToAndFromFloatPtr/NonZero
        Case {
            name: "NonZero",
            want: vec!["1123581321"],
        },
    ];
    for tc in cases {
        let got = from_float_ptr_value(to_float_ptr_value(tc.want[0]));
        assert_eq!(tc.want[0], got, "{}", tc.name);
    }
}

#[test]
fn to_and_from_ptr_values() {
    struct Case {
        name: &'static str,
        want: Vec<&'static str>,
    }

    let cases = [
        // go: pkg/reference TestToAndFromPtrValues/Nil
        Case {
            name: "Nil",
            want: vec![],
        },
        // go: pkg/reference TestToAndFromPtrValues/Zero
        Case {
            name: "Zero",
            want: vec![""],
        },
        // go: pkg/reference TestToAndFromPtrValues/NonZero
        Case {
            name: "NonZero",
            want: vec!["pointy"],
        },
        // go: pkg/reference TestToAndFromPtrValues/Multiple
        Case {
            name: "Multiple",
            want: vec!["pointy", "pointers"],
        },
    ];
    for tc in cases {
        let want = strings(&tc.want);
        assert_eq!(want, from_ptr_values(&to_ptr_values(&want)), "{}", tc.name);
    }
}

#[test]
fn to_and_from_float_ptr_values() {
    struct Case {
        name: &'static str,
        want: Vec<&'static str>,
    }

    let cases = [
        // go: pkg/reference TestToAndFromFloatPtrValues/Nil
        Case {
            name: "Nil",
            want: vec![],
        },
        // go: pkg/reference TestToAndFromFloatPtrValues/Zero
        Case {
            name: "Zero",
            want: vec![""],
        },
        // go: pkg/reference TestToAndFromFloatPtrValues/NonZero
        Case {
            name: "NonZero",
            want: vec!["1123581321"],
        },
        // go: pkg/reference TestToAndFromFloatPtrValues/Multiple
        Case {
            name: "Multiple",
            want: vec!["1123581321", "1234567890"],
        },
    ];
    for tc in cases {
        let want = strings(&tc.want);
        assert_eq!(want, from_float_ptr_values(&to_float_ptr_values(&want)), "{}", tc.name);
    }
}

#[test]
fn to_and_from_int_ptr_values() {
    struct Case {
        name: &'static str,
        want: Vec<&'static str>,
    }

    let cases = [
        // go: pkg/reference TestToAndFromIntPtrValues/Nil
        Case {
            name: "Nil",
            want: vec![],
        },
        // go: pkg/reference TestToAndFromIntPtrValues/Zero
        Case {
            name: "Zero",
            want: vec![""],
        },
        // go: pkg/reference TestToAndFromIntPtrValues/NonZero
        Case {
            name: "NonZero",
            want: vec!["1123581321"],
        },
        // go: pkg/reference TestToAndFromIntPtrValues/Multiple
        Case {
            name: "Multiple",
            want: vec!["1123581321", "1234567890"],
        },
    ];
    for tc in cases {
        let want = strings(&tc.want);
        assert_eq!(want, from_int_ptr_values(&to_int_ptr_values(&want)), "{}", tc.name);
    }
}

#[test]
fn float_values_format_as_go_does() {
    assert_eq!("2", from_float_ptr_value(Some(2.5)));
    assert_eq!("4", from_float_ptr_value(Some(3.5)));
    assert_eq!("-0", from_float_ptr_value(Some(-0.0)));
    assert_eq!("+Inf", from_float_ptr_value(Some(f64::INFINITY)));
    assert_eq!(None, to_float_ptr_value("not a number"));
    assert_eq!(None, to_int_ptr_value("1.5"));
}

#[test]
fn reference_values_set_and_read_each_field_shape() {
    let mut s = Some("a".to_owned());
    assert_eq!("a", ReferenceValue::current_value(&s));
    ReferenceValue::set_resolved(&mut s, String::new());
    assert_eq!(None, s);

    let mut f: Option<f64> = None;
    ReferenceValue::set_resolved(&mut f, "7".into());
    assert_eq!((Some(7.0), "7".to_owned()), (f, ReferenceValue::current_value(&f)));

    let mut many: Vec<Option<String>> = vec![];
    ReferenceValues::set_resolved(&mut many, strings(&["x", ""]));
    assert_eq!(vec![Some("x".to_owned()), None], many);
    assert_eq!(strings(&["x", ""]), ReferenceValues::current_values(&many));
}

#[tokio::test]
async fn an_always_selector_discards_the_reference_it_is_sent_with() {
    let client = list_of(vec![controlled("picked", "picked-value")]);
    let from = ModernManaged::default();
    let req = Req {
        reference: Some(reference("stale", "")),
        selector: Some(NamespacedSelector {
            policy: Some(always()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let got = ApiNamespacedResolver::new(&client, &from).resolve(req).await;
    assert_eq!(Ok(resp("picked-value", Some(reference("picked", "")))), got);
}

#[tokio::test]
async fn only_an_optional_resolution_policy_swallows_errors() {
    let from = ModernManaged::default();
    let not_found = MockClient {
        get: Some(new_mock_get_err_fn(Error::not_found())),
        ..Default::default()
    };
    for (policy, want) in [
        (None, Err(Error::not_found().wrap(ERR_GET_MANAGED))),
        (Some(always()), Err(Error::not_found().wrap(ERR_GET_MANAGED))),
        (Some(optional()), Ok(resp("", None))),
    ] {
        let req = Req {
            reference: Some(NamespacedReference {
                policy,
                ..reference("gone", "")
            }),
            ..Default::default()
        };
        let got = ApiNamespacedResolver::new(&not_found, &from).resolve(req).await;
        assert_eq!(want, got);
    }

    let other = get_err();
    let req = Req {
        reference: Some(with_policy(reference("x", ""), optional())),
        ..Default::default()
    };
    let got = ApiNamespacedResolver::new(&other, &from).resolve(req).await;
    assert_eq!(Err(err_boom().wrap(ERR_GET_MANAGED)), got, "only NotFound is optional");
}

#[tokio::test]
async fn a_referencing_resource_without_a_namespace_reads_and_lists_across_namespaces() {
    let client = MockClient {
        get: Some(Box::new(|_, key| {
            assert_eq!(None, key.namespace);
            Ok(Box::new(named("", "v")))
        })),
        list: Some(Box::new(|_, options| {
            assert_eq!(None, options.namespace);
            Ok(vec![Box::new(named("n", "v"))])
        })),
        ..Default::default()
    };
    let from = ModernManaged::default();
    let r = ApiNamespacedResolver::new(&client, &from);
    r.resolve(Req {
        reference: Some(reference("n", "")),
        ..Default::default()
    })
    .await
    .unwrap();
    r.resolve(Req {
        selector: Some(NamespacedSelector::default()),
        ..Default::default()
    })
    .await
    .unwrap();
}

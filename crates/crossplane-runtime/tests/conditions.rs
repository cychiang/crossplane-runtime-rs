//! Port of `pkg/conditions/manager_test.go` `TestOGConditionSetMark`. It lives here and not in
//! `crossplane-core` because the fake kind is in `crossplane-test`, which depends on core.

use crossplane_core::resource::{Managed, mark_conditions};
use crossplane_core::v2::Condition;
use crossplane_test::fake::new_modern_managed;

#[test]
fn og_condition_set_mark() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        start: Vec<Condition>,
        mark: Vec<Condition>,
        want: Vec<Condition>,
    }
    let cases = vec![
        // go: pkg/conditions TestOGConditionSetMark/ProvideNoConditions
        Case {
            name: "ProvideNoConditions",
            reason: "If updating a resource without conditions with no new conditions, conditions should remain empty.",
            start: vec![],
            mark: vec![],
            want: vec![],
        },
        // go: pkg/conditions TestOGConditionSetMark/EmptyAppendCondition
        Case {
            name: "EmptyAppendCondition",
            reason: "If starting with a resource without conditions, and we mark a condition, it should propagate to conditions with the correct generation.",
            start: vec![],
            mark: vec![Condition::reconcile_success()],
            want: vec![Condition::reconcile_success().with_observed_generation(42)],
        },
        // go: pkg/conditions TestOGConditionSetMark/ExistingMarkNothing
        Case {
            name: "ExistingMarkNothing",
            reason: "If the resource has a condition and we update nothing, nothing should change.",
            start: vec![Condition::available().with_observed_generation(1)],
            mark: vec![],
            want: vec![Condition::available().with_observed_generation(1)],
        },
        // go: pkg/conditions TestOGConditionSetMark/ExistingUpdated
        Case {
            name: "ExistingUpdated",
            reason: "If a resource starts with a condition, and we update it, we should see the observedGeneration be updated",
            start: vec![Condition::reconcile_success().with_observed_generation(1)],
            mark: vec![Condition::reconcile_success()],
            want: vec![Condition::reconcile_success().with_observed_generation(42)],
        },
        // go: pkg/conditions TestOGConditionSetMark/ExistingAppended
        Case {
            name: "ExistingAppended",
            reason: "If a resource has an existing condition and we make another condition, the new condition should merge into the conditions list.",
            start: vec![Condition::available().with_observed_generation(1)],
            mark: vec![Condition::reconcile_success()],
            want: vec![
                Condition::available().with_observed_generation(1),
                Condition::reconcile_success().with_observed_generation(42),
            ],
        },
    ];

    for tc in cases {
        let mut mg = new_modern_managed(42);
        if !tc.start.is_empty() {
            mg.set_conditions(tc.start);
        }
        mark_conditions(&mut mg, tc.mark);

        let got = mg
            .managed_status()
            .map(|s| s.conditioned.conditions.clone())
            .unwrap_or_default();
        assert_eq!(tc.want.len(), got.len(), "{}: {}", tc.name, tc.reason);
        for (want, got) in tc.want.iter().zip(&got) {
            assert!(
                want.equal(got) && want.observed_generation == got.observed_generation,
                "{}: {}\n-want {want:?}\n+got  {got:?}",
                tc.name,
                tc.reason
            );
        }
    }
}

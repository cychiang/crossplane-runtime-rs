//! Port of the `ManagementPoliciesResolver` tables in `reconciler_modern_test.go`. Go's
//! `NewManagementPoliciesResolver(enabled, policy)` resolves against the default supported sets.

use crossplane_core::v2::{ManagementAction, ManagementAction::*, ManagementPolicies};

use super::policy::{PolicyResolver, default_supported_management_policies};

struct Case {
    name: &'static str,
    reason: &'static str,
    enabled: bool,
    policy: &'static [ManagementAction],
    want: bool,
}

fn check(cases: &[Case], method: fn(&PolicyResolver<'_>) -> bool) {
    let supported = default_supported_management_policies();
    for tc in cases {
        let r = PolicyResolver::new(tc.enabled, &ManagementPolicies(tc.policy.to_vec()), &supported);
        assert_eq!(tc.want, method(&r), "\n{}\nReason: {}", tc.name, tc.reason);
    }
}

#[test]
fn is_paused() {
    let cases = [
        // go: pkg/reconciler/managed TestManagementPoliciesResolverIsPaused/Disabled
        Case {
            name: "Disabled",
            reason: "Should return false if management policies are disabled",
            enabled: false,
            policy: &[],
            want: false,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverIsPaused/EnabledEmptyPolicies
        Case {
            name: "EnabledEmptyPolicies",
            reason: "Should return true if the management policies are enabled and empty",
            enabled: true,
            policy: &[],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverIsPaused/EnabledNonEmptyPolicies
        Case {
            name: "EnabledNonEmptyPolicies",
            reason: "Should return true if the management policies are enabled and non empty",
            enabled: true,
            policy: &[All],
            want: false,
        },
    ];
    check(&cases, |r| r.is_paused());
}

#[test]
fn validate() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        enabled: bool,
        policy: &'static [ManagementAction],
        want: Result<(), String>,
    }
    let cases = [
        // go: pkg/reconciler/managed TestManagementPoliciesResolverValidate/Enabled
        Case {
            name: "Enabled",
            reason: "Should return nil if the management policy is enabled.",
            enabled: true,
            policy: &[],
            want: Ok(()),
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverValidate/DisabledNonDefault
        Case {
            name: "DisabledNonDefault",
            reason: "Should return error if the management policy is non-default and disabled.",
            enabled: false,
            policy: &[Create],
            want: Err(
                "`spec.managementPolicies` is set to a non-default value but the feature is not enabled: [Create]"
                    .to_owned(),
            ),
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverValidate/DisabledDefault
        Case {
            name: "DisabledDefault",
            reason: "Should return nil if the management policy is default and disabled.",
            enabled: false,
            policy: &[All],
            want: Ok(()),
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverValidate/EnabledSupported
        Case {
            name: "EnabledSupported",
            reason: "Should return nil if the management policy is supported.",
            enabled: true,
            policy: &[All],
            want: Ok(()),
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverValidate/EnabledNotSupported
        Case {
            name: "EnabledNotSupported",
            reason: "Should return err if the management policy is not supported.",
            enabled: true,
            policy: &[Delete],
            want: Err(
                "`spec.managementPolicies` is set to a value([Delete]) which is not supported. Check docs for supported policies"
                    .to_owned(),
            ),
        },
    ];
    let supported = default_supported_management_policies();
    for tc in cases {
        let r = PolicyResolver::new(tc.enabled, &ManagementPolicies(tc.policy.to_vec()), &supported);
        assert_eq!(tc.want, r.validate(), "\n{}\nReason: {}", tc.name, tc.reason);
    }
}

#[test]
fn should_create() {
    let cases = [
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldCreate/ManagementPoliciesDisabled
        Case {
            name: "ManagementPoliciesDisabled",
            reason: "Should return true if management policies are disabled",
            enabled: false,
            policy: &[],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldCreate/ManagementPoliciesEnabledHasCreate
        Case {
            name: "ManagementPoliciesEnabledHasCreate",
            reason: "Should return true if management policies are enabled and managementPolicies has action Create",
            enabled: true,
            policy: &[Create],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldCreate/ManagementPoliciesEnabledHasCreateAll
        Case {
            name: "ManagementPoliciesEnabledHasCreateAll",
            reason: "Should return true if management policies are enabled and managementPolicies has action All",
            enabled: true,
            policy: &[All],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldCreate/ManagementPoliciesEnabledActionNotAllowed
        Case {
            name: "ManagementPoliciesEnabledActionNotAllowed",
            reason: "Should return false if management policies are enabled and managementPolicies does not have Create",
            enabled: true,
            policy: &[Observe],
            want: false,
        },
    ];
    check(&cases, |r| r.should_create());
}

#[test]
fn should_update() {
    let cases = [
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldUpdate/ManagementPoliciesDisabled
        Case {
            name: "ManagementPoliciesDisabled",
            reason: "Should return true if management policies are disabled",
            enabled: false,
            policy: &[],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldUpdate/ManagementPoliciesEnabledHasUpdate
        Case {
            name: "ManagementPoliciesEnabledHasUpdate",
            reason: "Should return true if management policies are enabled and managementPolicies has action Update",
            enabled: true,
            policy: &[Update],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldUpdate/ManagementPoliciesEnabledHasUpdateAll
        Case {
            name: "ManagementPoliciesEnabledHasUpdateAll",
            reason: "Should return true if management policies are enabled and managementPolicies has action All",
            enabled: true,
            policy: &[All],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldUpdate/ManagementPoliciesEnabledActionNotAllowed
        Case {
            name: "ManagementPoliciesEnabledActionNotAllowed",
            reason: "Should return false if management policies are enabled and managementPolicies does not have Update",
            enabled: true,
            policy: &[Observe],
            want: false,
        },
    ];
    check(&cases, |r| r.should_update());
}

#[test]
fn should_late_initialize() {
    let cases = [
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldLateInitialize/ManagementPoliciesDisabled
        Case {
            name: "ManagementPoliciesDisabled",
            reason: "Should return true if management policies are disabled",
            enabled: false,
            policy: &[],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldLateInitialize/ManagementPoliciesEnabledHasLateInitialize
        Case {
            name: "ManagementPoliciesEnabledHasLateInitialize",
            reason: "Should return true if management policies are enabled and managementPolicies has action LateInitialize",
            enabled: true,
            policy: &[LateInitialize],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldLateInitialize/ManagementPoliciesEnabledHasLateInitializeAll
        Case {
            name: "ManagementPoliciesEnabledHasLateInitializeAll",
            reason: "Should return true if management policies are enabled and managementPolicies has action All",
            enabled: true,
            policy: &[All],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverShouldLateInitialize/ManagementPoliciesEnabledActionNotAllowed
        Case {
            name: "ManagementPoliciesEnabledActionNotAllowed",
            reason: "Should return false if management policies are enabled and managementPolicies does not have LateInitialize",
            enabled: true,
            policy: &[Observe],
            want: false,
        },
    ];
    check(&cases, |r| r.should_late_initialize());
}

#[test]
fn should_only_observe() {
    let cases = [
        // go: pkg/reconciler/managed TestManagementPoliciesResolverOnlyObserve/ManagementPoliciesDisabled
        Case {
            name: "ManagementPoliciesDisabled",
            reason: "Should return false if management policies are disabled",
            enabled: false,
            policy: &[],
            want: false,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverOnlyObserve/ManagementPoliciesEnabledHasOnlyObserve
        Case {
            name: "ManagementPoliciesEnabledHasOnlyObserve",
            reason: "Should return true if management policies are enabled and managementPolicies has action LateInitialize",
            enabled: true,
            policy: &[Observe],
            want: true,
        },
        // go: pkg/reconciler/managed TestManagementPoliciesResolverOnlyObserve/ManagementPoliciesEnabledHasMultipleActions
        Case {
            name: "ManagementPoliciesEnabledHasMultipleActions",
            reason: "Should return false if management policies are enabled and managementPolicies has multiple actions",
            enabled: true,
            policy: &[LateInitialize, Observe],
            want: false,
        },
    ];
    check(&cases, |r| r.should_only_observe());
}

#[test]
fn should_delete() {
    let cases = [
        // go: pkg/reconciler/managed TestShouldDelete/ManagementPoliciesDisabled
        Case {
            name: "ManagementPoliciesDisabled",
            reason: "Should delete if management policies are disabled",
            enabled: false,
            policy: &[],
            want: true,
        },
        // go: pkg/reconciler/managed TestShouldDelete/DeleteManagementActionAll
        Case {
            name: "DeleteManagementActionAll",
            reason: "Should delete if management policies are enabled and management policy is set to All.",
            enabled: true,
            policy: &[All],
            want: true,
        },
        // go: pkg/reconciler/managed TestShouldDelete/ManagementActionDelete
        Case {
            name: "ManagementActionDelete",
            reason: "Should delete if management policies are enabled and management policy has action Delete.",
            enabled: true,
            policy: &[Delete],
            want: true,
        },
        // go: pkg/reconciler/managed TestShouldDelete/ManagementActionNoDelete
        Case {
            name: "ManagementActionNoDelete",
            reason: "Should orphan if management policies are enabled and management policy does not have action Delete.",
            enabled: true,
            policy: &[Observe],
            want: false,
        },
    ];
    check(&cases, |r| r.should_delete());
}

//! Management policy resolution (`policies.go`), as a bitset and pure functions. The truth table
//! in `docs/phase0/02-parity-spec.md` 2.2 is the table of these methods over
//! `default_supported_management_policies()`.

use crossplane_core::v2::{ManagementAction, ManagementPolicies};

/// Parsing collapses duplicates and order, so equality is set equality, as Go's `sets.Set` gives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ActionSet(u8);

impl ActionSet {
    const fn bit(a: ManagementAction) -> u8 {
        match a {
            ManagementAction::Observe => 1,
            ManagementAction::Create => 1 << 1,
            ManagementAction::Update => 1 << 2,
            ManagementAction::Delete => 1 << 3,
            ManagementAction::LateInitialize => 1 << 4,
            ManagementAction::All => 1 << 5,
        }
    }

    pub const fn of(actions: &[ManagementAction]) -> Self {
        let mut bits = 0;
        let mut i = 0;
        while i < actions.len() {
            bits |= Self::bit(actions[i]);
            i += 1;
        }
        Self(bits)
    }

    pub fn from_policies(p: &ManagementPolicies) -> Self {
        Self::of(&p.0)
    }

    pub const fn contains(self, a: ManagementAction) -> bool {
        self.0 & Self::bit(a) != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Allows `a` directly or through `*`.
    const fn allows(self, a: ManagementAction) -> bool {
        self.contains(a) || self.contains(ManagementAction::All)
    }
}

/// Go's `%s` of `UnsortedList()`: `[Observe Create]`.
impl std::fmt::Display for ActionSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ManagementAction::*;
        let names: Vec<&str> = [
            (Observe, "Observe"),
            (Create, "Create"),
            (Update, "Update"),
            (Delete, "Delete"),
            (LateInitialize, "LateInitialize"),
            (All, "*"),
        ]
        .into_iter()
        .filter(|(a, _)| self.contains(*a))
        .map(|(_, n)| n)
        .collect();
        write!(f, "[{}]", names.join(" "))
    }
}

/// The 15 sets at `policies.go:55-110`.
pub fn default_supported_management_policies() -> Vec<ActionSet> {
    use ManagementAction::*;
    [
        &[All][..],
        &[Observe, Create, Update, LateInitialize, Delete],
        &[Observe],
        &[],
        &[Observe, Create, Update, Delete],
        &[Observe, Create, Update, LateInitialize],
        &[Observe, Create, Update],
        &[Observe, Create, Delete, LateInitialize],
        &[Observe, Create, LateInitialize],
        &[Observe, Create, Delete],
        &[Observe, Create],
        &[Observe, Delete],
        &[Observe, Update],
        &[Observe, LateInitialize],
        &[Observe, Update, LateInitialize],
    ]
    .into_iter()
    .map(ActionSet::of)
    .collect()
}

/// Go `ManagementPoliciesResolver` (modern). Built per reconcile from the object's policies;
/// `enabled` is the beta feature flag.
#[derive(Clone, Debug)]
pub struct PolicyResolver<'a> {
    enabled: bool,
    policies: ActionSet,
    supported: &'a [ActionSet],
}

impl<'a> PolicyResolver<'a> {
    pub fn new(enabled: bool, policies: &ManagementPolicies, supported: &'a [ActionSet]) -> Self {
        Self {
            enabled,
            policies: ActionSet::from_policies(policies),
            supported,
        }
    }

    /// The error text is the condition and event message (B05). Go formats the set in hash order;
    /// this formats it in declaration order.
    pub fn validate(&self) -> Result<(), String> {
        let all = ActionSet::of(&[ManagementAction::All]);
        if !self.enabled {
            if self.policies != all && !self.policies.is_empty() {
                return Err(format!(
                    "`spec.managementPolicies` is set to a non-default value but the feature is not enabled: {}",
                    self.policies
                ));
            }
            return Ok(());
        }
        if self.supported.contains(&self.policies) {
            return Ok(());
        }
        Err(format!(
            "`spec.managementPolicies` is set to a value({}) which is not supported. Check docs for supported policies",
            self.policies
        ))
    }

    pub fn is_paused(&self) -> bool {
        self.enabled && self.policies.is_empty()
    }
    pub fn should_create(&self) -> bool {
        !self.enabled || self.policies.allows(ManagementAction::Create)
    }
    pub fn should_update(&self) -> bool {
        !self.enabled || self.policies.allows(ManagementAction::Update)
    }
    pub fn should_late_initialize(&self) -> bool {
        !self.enabled || self.policies.allows(ManagementAction::LateInitialize)
    }
    pub fn should_only_observe(&self) -> bool {
        self.enabled && self.policies == ActionSet::of(&[ManagementAction::Observe])
    }
    pub fn should_delete(&self) -> bool {
        !self.enabled || self.policies.allows(ManagementAction::Delete)
    }
}

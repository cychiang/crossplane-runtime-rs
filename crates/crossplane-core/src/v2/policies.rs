use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `xpv2.ManagementAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub enum ManagementAction {
    Observe,
    Create,
    Update,
    Delete,
    LateInitialize,
    #[serde(rename = "*")]
    All,
}

/// `xpv2.ManagementPolicies`. The wire form is a list; the managed reconciler parses it into a
/// set, so duplicates and order never reach the policy logic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ManagementPolicies(pub Vec<ManagementAction>);

impl ManagementPolicies {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

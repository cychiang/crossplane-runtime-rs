//! `github.com/crossplane/crossplane/apis/v2/core/v2` at v2.4.3.
//!
//! Every struct here is embedded into a provider's `#[derive(CustomResource)]` types with
//! `#[serde(flatten)]`, so field names, `omitempty` behaviour and schema markers must match the
//! Go source. `hack/crd-diff` checks the generated schema against the Go baseline's CRDs.
//!
//! `Default` is Go's zero value throughout. The kubebuilder defaults (`["*"]` for policies, the
//! `ClusterProviderConfig` reference) exist only in the schema, where the API server applies
//! them. A client-side default would turn an empty policy set, which means paused, into `["*"]`.

mod condition;
mod policies;
mod resource;
pub mod schema;

pub use condition::{
    Condition, ConditionReason, ConditionStatus, ConditionType, ConditionedStatus, is_system_condition_type,
};
pub use policies::{
    DeletionPolicy, ManagementAction, ManagementPolicies, ResolutionPolicy, ResolvePolicy, UpdatePolicy,
};
pub use resource::*;

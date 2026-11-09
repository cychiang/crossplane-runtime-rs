use futures::future::BoxFuture;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Resource;
use kube::core::NamespaceResourceScope;

use super::Kind;
use crate::client::Client;
use crate::errors::Result;
use crate::v2::{
    Condition, ConditionType, ConditionedStatus, LocalSecretReference, ManagedResourceSpec, ManagedResourceStatus,
    ManagementPolicies, ProviderConfigReference, ProviderConfigStatus, TypedProviderConfigUsage,
};

/// Go `resource.ModernManaged`. The four required accessors say where the kind keeps the two
/// structs every managed resource embeds; everything angryjet generated is a provided method.
pub trait Managed: Kind + Resource<Scope = NamespaceResourceScope> {
    /// Whether the reconciler records `status.lastHandledReconcileAt`. Go decides this by whether
    /// the root type has the reconcile-request tracker methods; an angryjet kind does not.
    const TRACKS_RECONCILE_REQUESTS: bool = false;

    fn managed_spec(&self) -> &ManagedResourceSpec;
    fn managed_spec_mut(&mut self) -> &mut ManagedResourceSpec;
    /// `None` when the root's `status` is `None`.
    fn managed_status(&self) -> Option<&ManagedResourceStatus>;
    /// Creates an empty status when absent.
    fn managed_status_mut(&mut self) -> &mut ManagedResourceStatus;

    /// Go `ResolveReferences(ctx, client.Reader)`, discovered in Go by type assertion. `None`
    /// means the kind has no references; the reference derive of unit 1.11 overrides it.
    fn resolve_references<'a>(&'a mut self, _client: &'a dyn Client) -> Option<BoxFuture<'a, Result<()>>> {
        None
    }

    fn provider_config_reference(&self) -> Option<&ProviderConfigReference> {
        self.managed_spec().provider_config_ref.as_ref()
    }

    fn write_connection_secret_to_reference(&self) -> Option<&LocalSecretReference> {
        self.managed_spec().write_connection_secret_to_ref.as_ref()
    }

    fn management_policies(&self) -> &ManagementPolicies {
        &self.managed_spec().management_policies
    }

    fn get_condition(&self, ct: &ConditionType) -> Condition {
        match self.managed_status() {
            Some(status) => status.conditioned.get_condition(ct),
            None => ConditionedStatus::default().get_condition(ct),
        }
    }

    fn set_conditions(&mut self, conditions: impl IntoIterator<Item = Condition>) {
        self.managed_status_mut().conditioned.set_conditions(conditions);
    }
}

/// Go `conditions.ObservedGenerationPropagationManager`: set conditions stamped with the object's
/// generation. The reconcilers use it; a provider's own `set_conditions` leaves
/// `observedGeneration` alone, as in Go.
pub fn mark_conditions<M: Managed>(mg: &mut M, conditions: impl IntoIterator<Item = Condition>) {
    let generation = mg.meta().generation.unwrap_or_default();
    mg.set_conditions(conditions.into_iter().map(|c| c.with_observed_generation(generation)));
}

/// Go `resource.ProviderConfig`.
pub trait ProviderConfig: Kind {
    fn provider_config_status(&self) -> Option<&ProviderConfigStatus>;
    fn provider_config_status_mut(&mut self) -> &mut ProviderConfigStatus;
}

/// Go `resource.TypedProviderConfigUsage`.
pub trait ProviderConfigUsage: Kind {
    /// The usage tracker creates usages and never reads one into an empty value, so the kind has
    /// a constructor and needs no `Default`.
    fn from_usage(metadata: ObjectMeta, usage: TypedProviderConfigUsage) -> Self;
    fn usage(&self) -> &TypedProviderConfigUsage;
    fn usage_mut(&mut self) -> &mut TypedProviderConfigUsage;
}

use std::marker::PhantomData;
use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::{Error, ErrorKind, Result};
use crossplane_core::meta;
use crossplane_core::resource::{Managed, ObjectType, ProviderConfigUsage};
use crossplane_core::v2::{
    LABEL_KEY_PROVIDER_KIND, LABEL_KEY_PROVIDER_NAME, ProviderConfigReference, TypedProviderConfigUsage, TypedReference,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::applicator::{ApiUpdatingApplicator, Applicator};
use super::apply_option::{ApplyOption, is_not_allowed, new_not_allowed};

pub const ERR_MISSING_PC_REF: &str = "managed resource does not reference a ProviderConfig";
pub const ERR_MISSING_PC_KIND: &str = "managed resource ProviderConfig reference has no Kind";
pub const ERR_APPLY_PCU: &str = "cannot apply ProviderConfigUsage";

/// Go `resource.ProviderConfigUsageTracker` (the typed, modern one).
pub struct ProviderConfigUsageTracker<U> {
    applicator: Arc<dyn Applicator>,
    _usage: PhantomData<fn() -> U>,
}

impl<U: ProviderConfigUsage> ProviderConfigUsageTracker<U> {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self::with_applicator(Arc::new(ApiUpdatingApplicator::new(client)))
    }

    /// For a table that replaces the applicator, as Go's tests set the tracker's `c`.
    pub fn with_applicator(applicator: Arc<dyn Applicator>) -> Self {
        Self {
            applicator,
            _usage: PhantomData,
        }
    }

    /// Ensures a usage named after the MR's UID, in the MR's namespace, labelled with the config
    /// name and kind, controller-owned by the MR and pointing back at it. Steady state is one Get
    /// and no write.
    pub async fn track<M: Managed>(&self, mg: &M) -> Result<()> {
        let gvk = ObjectType::of::<M>().gvk();
        let r = mg
            .provider_config_reference()
            .ok_or_else(|| Error::with_kind(ErrorKind::MissingReference, ERR_MISSING_PC_REF))?;
        if r.kind.is_empty() {
            return Err(Error::with_kind(ErrorKind::MissingReference, ERR_MISSING_PC_KIND));
        }

        let owner = meta::typed_reference_to(mg, &gvk);
        let pc_ref = ProviderConfigReference {
            name: r.name.clone(),
            kind: r.kind.clone(),
        };
        let mut pcu = U::from_usage(
            ObjectMeta {
                name: Some(owner.uid.clone()),
                namespace: mg.meta().namespace.clone(),
                labels: Some(
                    [
                        (LABEL_KEY_PROVIDER_NAME.to_owned(), r.name.clone()),
                        (LABEL_KEY_PROVIDER_KIND.to_owned(), r.kind.clone()),
                    ]
                    .into(),
                ),
                owner_references: Some(vec![meta::as_controller(&owner)]),
                ..Default::default()
            },
            TypedProviderConfigUsage {
                provider_config_ref: pc_ref.clone(),
                resource_ref: TypedReference {
                    api_version: owner.api_version.clone(),
                    kind: owner.kind.clone(),
                    name: owner.name.clone(),
                    uid: String::new(),
                },
            },
        );

        let uid = owner.uid;
        let opts = [
            ApplyOption::MustBeControllableBy(uid),
            ApplyOption::Custom(Box::new(move |current, _| {
                let current = current
                    .downcast_ref::<U>()
                    .expect("the applicator hands back the usage kind it was given");
                if *current.provider_config_reference() != pc_ref {
                    Ok(())
                } else {
                    Err(new_not_allowed("update not allowed"))
                }
            })),
        ];

        match self.applicator.apply(&mut pcu, &opts).await {
            Err(err) if !is_not_allowed(&err) => Err(err.wrap(ERR_APPLY_PCU)),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "tracker_test.rs"]
mod tests;

//! Go `internal/controller/mytype/external.go`. THIS FILE IS YOURS.

use crossplane_core::errors::Result;
use crossplane_core::meta;
use crossplane_core::resource::Managed;
use crossplane_core::v2::Condition;
use crossplane_runtime::controller::Options;
use crossplane_runtime::manager::Manager;
use crossplane_runtime::reconciler::managed::{
    ExternalClient, ExternalCreation, ExternalDelete, ExternalObservation, ExternalUpdate, ReconcilerBuilder,
};

use crate::apis::sample_v1alpha1::MyType;
use crate::provider::{Client, Thing};

pub struct External {
    client: Client,
}

impl External {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

/// Go `ReconcilerOptions`: extra reconciler options for this kind.
pub fn reconciler_options(
    b: ReconcilerBuilder<MyType>,
    _mgr: &Manager,
    _o: &Options,
) -> Result<ReconcilerBuilder<MyType>> {
    Ok(b)
}

fn desired(cr: &MyType) -> Thing {
    Thing {
        configurable_field: cr.spec.for_provider.configurable_field.clone(),
    }
}

impl ExternalClient<MyType> for External {
    async fn observe(&self, cr: &mut MyType) -> Result<ExternalObservation> {
        let Some(thing) = self.client.get(meta::external_name(cr)).await? else {
            return Ok(ExternalObservation::default());
        };
        let up_to_date = thing.configurable_field == cr.spec.for_provider.configurable_field;
        cr.status.get_or_insert_default().at_provider.configurable_field = thing.configurable_field;
        cr.set_conditions([Condition::available()]);
        Ok(ExternalObservation {
            resource_exists: true,
            resource_up_to_date: up_to_date,
            ..Default::default()
        })
    }

    async fn create(&self, cr: &mut MyType) -> Result<ExternalCreation> {
        cr.set_conditions([Condition::creating()]);
        let thing = self.client.put(meta::external_name(cr), desired(cr)).await?;
        cr.status.get_or_insert_default().at_provider.configurable_field = thing.configurable_field;
        Ok(ExternalCreation::default())
    }

    async fn update(&self, cr: &mut MyType) -> Result<ExternalUpdate> {
        let thing = self.client.put(meta::external_name(cr), desired(cr)).await?;
        cr.status.get_or_insert_default().at_provider.configurable_field = thing.configurable_field;
        Ok(ExternalUpdate::default())
    }

    async fn delete(&self, cr: &mut MyType) -> Result<ExternalDelete> {
        cr.set_conditions([Condition::deleting()]);
        self.client.delete(meta::external_name(cr)).await?;
        Ok(ExternalDelete::default())
    }
}

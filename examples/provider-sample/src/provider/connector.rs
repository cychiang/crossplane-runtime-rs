//! Go `internal/provider/connector.go`.

use std::sync::Arc;

use crossplane_core::errors::{Error, Result, ResultExt};
use crossplane_core::meta;
use crossplane_core::resource::{Managed, ObjectKey};
use crossplane_core::v2::CommonCredentialSelectors;
use crossplane_runtime::manager::Manager;
use crossplane_runtime::reconciler::managed::{ExternalClient, ExternalConnector};
use crossplane_runtime::resource::{self, ProviderConfigUsageTracker};

use super::{Client, ClientConfig};
use crate::apis::v1alpha1::{ProviderConfig, ProviderConfigUsage};

const ERR_NO_PC_REF: &str = "managed resource has no provider config reference";
const ERR_TRACK_PC_USAGE: &str = "cannot track ProviderConfig usage";
const ERR_GET_PC: &str = "cannot get ProviderConfig";
const ERR_GET_CREDS: &str = "cannot get credentials";
const ERR_NEW_CLIENT: &str = "cannot create client";

/// Resolves a managed resource's ProviderConfig into a [`Client`] and hands it to `external`,
/// which builds the kind's external client. One connector type serves every kind.
pub struct Connector<F> {
    kube: Arc<dyn crossplane_core::client::Client>,
    usage: ProviderConfigUsageTracker<ProviderConfigUsage>,
    external: F,
}

impl<F> Connector<F> {
    pub fn new(mgr: &Manager, external: F) -> Self {
        Self {
            kube: mgr.client(),
            usage: ProviderConfigUsageTracker::new(mgr.client()),
            external,
        }
    }
}

impl<M, F, E> ExternalConnector<M> for Connector<F>
where
    M: Managed,
    F: Fn(Client) -> E + Send + Sync + 'static,
    E: ExternalClient<M>,
{
    type Client = E;

    async fn connect(&self, mg: &M) -> Result<E> {
        self.usage.track(mg).await.wrap_err(ERR_TRACK_PC_USAGE)?;
        let cfg = self.client_config(mg).await?;
        let cl = Client::new(cfg).await.wrap_err(ERR_NEW_CLIENT)?;
        Ok((self.external)(cl))
    }
}

impl<F> Connector<F> {
    async fn client_config<M: Managed>(&self, mg: &M) -> Result<ClientConfig> {
        let r = mg
            .provider_config_reference()
            .ok_or_else(|| Error::new(ERR_NO_PC_REF))?;
        if !r.kind.is_empty() && r.kind != "ProviderConfig" {
            return Err(Error::new(format!("unsupported provider config kind: {}", r.kind)));
        }
        let key = ObjectKey::namespaced(meta::namespace(mg), &r.name);
        let pc = self.kube.get_as::<ProviderConfig>(&key).await.wrap_err(ERR_GET_PC)?;
        let selectors = CommonCredentialSelectors {
            secret_ref: pc
                .spec
                .credentials
                .secret_ref
                .as_ref()
                .map(|s| s.to_secret_key_selector(meta::namespace(&pc))),
            ..Default::default()
        };
        let creds = resource::common_credential_extractor(&pc.spec.credentials.source, &*self.kube, &selectors)
            .await
            .wrap_err(ERR_GET_CREDS)?;
        Ok(ClientConfig {
            spec: pc.spec,
            credentials: creds,
            kube: self.kube.clone(),
        })
    }
}

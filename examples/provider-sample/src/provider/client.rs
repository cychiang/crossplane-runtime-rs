//! Go `internal/provider/client.go`. THIS FILE IS YOURS: the template never overwrites it.
//!
//! The Go baseline's client calls the fake external service under `parity/fake-external`. The
//! HTTP calls land with the Rust template in unit 2.3.

use std::sync::Arc;

use crossplane_core::errors::Result;
use serde::{Deserialize, Serialize};

use crate::apis::v1alpha1::ProviderConfigSpec;

/// What `Client::new` gets from the resolved ProviderConfig.
pub struct ClientConfig {
    pub spec: ProviderConfigSpec,
    /// Extracted per the ProviderConfig's credentials source.
    pub credentials: Vec<u8>,
    pub kube: Arc<dyn crossplane_core::client::Client>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thing {
    pub configurable_field: String,
}

/// Talks to the external API this provider manages.
pub struct Client {
    endpoint: String,
}

impl Client {
    pub async fn new(_cfg: ClientConfig) -> Result<Self> {
        Ok(Self {
            endpoint: super::options::endpoint(),
        })
    }

    /// `None` when the thing does not exist.
    pub async fn get(&self, name: &str) -> Result<Option<Thing>> {
        todo!("GET {}/things/{name}", self.endpoint)
    }

    pub async fn put(&self, name: &str, thing: Thing) -> Result<Thing> {
        todo!("PUT {}/things/{name} {thing:?}", self.endpoint)
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        todo!("DELETE {}/things/{name}", self.endpoint)
    }
}

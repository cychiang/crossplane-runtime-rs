//! The Kubernetes client seam: controller-runtime's `client.Client`, reduced to the seven
//! operations in-scope Go code calls (`docs/phase0/02-parity-spec.md` 6.2), over type-erased
//! objects.
//!
//! Object safe, so one `Arc<dyn Client>` serves every reconciler, collaborator and connector.
//! Production code gets one from `crossplane_runtime::manager::Manager::client`. Tests build a
//! `crossplane_test::MockClient`.

use std::collections::BTreeMap;

use futures::future::BoxFuture;

use crate::errors::Result;
use crate::resource::{Kind, Object, ObjectKey, ObjectType};

/// Every write decodes the server response back into `obj`, as Go's client does. The late-init
/// status reset (parity spec S5) depends on it.
pub trait Client: Send + Sync + 'static {
    /// Returns the object instead of filling one in, so no caller needs an empty `K`.
    fn get<'a>(&'a self, object_type: &'a ObjectType, key: &'a ObjectKey) -> BoxFuture<'a, Result<Box<dyn Object>>>;
    fn list<'a>(
        &'a self,
        object_type: &'a ObjectType,
        options: &'a ListOptions,
    ) -> BoxFuture<'a, Result<Vec<Box<dyn Object>>>>;
    fn create<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>>;
    /// PUT. Go `Update`.
    fn update<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>>;
    /// PUT to `/status`. Go `Status().Update`.
    fn update_status<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>>;
    fn patch<'a>(&'a self, obj: &'a mut dyn Object, patch: Patch) -> BoxFuture<'a, Result<()>>;
    fn delete<'a>(&'a self, obj: &'a dyn Object) -> BoxFuture<'a, Result<()>>;
}

impl dyn Client {
    /// The one place a read is downcast back to its type.
    pub async fn get_as<K: Kind>(&self, key: &ObjectKey) -> Result<K> {
        let object_type = ObjectType::of::<K>();
        let obj = self.get(&object_type, key).await?;
        Ok(*obj
            .downcast::<K>()
            .unwrap_or_else(|other| panic!("client returned {other:?} for a get of {object_type:?}")))
    }

    pub async fn list_as<K: Kind>(&self, options: &ListOptions) -> Result<Vec<K>> {
        let object_type = ObjectType::of::<K>();
        let items = self.list(&object_type, options).await?;
        Ok(items
            .into_iter()
            .map(|obj| {
                *obj.downcast::<K>()
                    .unwrap_or_else(|other| panic!("client returned {other:?} in a list of {object_type:?}"))
            })
            .collect())
    }
}

/// Go `client.MatchingLabels` plus `client.InNamespace`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListOptions {
    /// `None` lists across all namespaces.
    pub namespace: Option<String>,
    pub labels: BTreeMap<String, String>,
}

/// The two patch shapes in-scope Go code sends. The caller builds the body, so a mock sees the
/// exact bytes a table wants to assert.
#[derive(Clone, Debug, PartialEq)]
pub enum Patch {
    /// `application/merge-patch+json`. The Secret publisher sends the whole desired object.
    Merge(serde_json::Value),
    /// `application/apply-patch+yaml` with a field owner, as the reference resolver sends.
    Apply {
        body: serde_json::Value,
        field_manager: String,
        force: bool,
    },
}

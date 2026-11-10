//! The replaceable collaborators of the managed reconciler (`api.go` and the interfaces in
//! `reconciler.go`). Each trait is object safe. Each has a `*Fn` or `*Fns` adapter over
//! synchronous closures for tables, and a default that talks to the API server through the
//! client seam.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::{Client, Patch};
use crossplane_core::errors::{Result, ResultExt};
use crossplane_core::meta;
use crossplane_core::resource::{Managed, Object};
use crossplane_core::v2::ConnectionDetails;
use futures::FutureExt;
use futures::future::BoxFuture;

use super::{
    ERR_MARSHAL_EXISTING, ERR_MARSHAL_RESOLVED, ERR_PATCH_MANAGED, ERR_RESOLVE_REFERENCES,
    ERR_UPDATE_CRITICAL_ANNOTATIONS, ERR_UPDATE_MANAGED, REFERENCE_RESOLVER_FIELD_OWNER,
};

/// Go `Initializer`. The reconciler runs a chain of them.
pub trait Initializer<M>: Send + Sync + 'static {
    fn initialize<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>>;
}

/// Go `InitializerFn`.
pub struct InitializerFn<M>(Box<dyn Fn(&mut M) -> Result<()> + Send + Sync>);

impl<M> InitializerFn<M> {
    pub fn new(f: impl Fn(&mut M) -> Result<()> + Send + Sync + 'static) -> Self {
        Self(Box::new(f))
    }
}

impl<M: Send + 'static> Initializer<M> for InitializerFn<M> {
    fn initialize<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>> {
        futures::future::ready((self.0)(mg)).boxed()
    }
}

/// Go `NameAsExternalName`, the default initializer.
pub struct NameAsExternalName {
    client: Arc<dyn Client>,
}

impl NameAsExternalName {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }
}

impl<M: Managed> Initializer<M> for NameAsExternalName {
    fn initialize<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>> {
        async move {
            if !meta::external_name(mg).is_empty() {
                return Ok(());
            }
            let name = meta::name(mg).to_owned();
            meta::set_external_name(mg, &name);
            self.client.update(mg).await.wrap_err(ERR_UPDATE_MANAGED)
        }
        .boxed()
    }
}

/// Go `ReferenceResolver`.
pub trait ReferenceResolver<M>: Send + Sync + 'static {
    fn resolve_references<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>>;
}

/// Go `ReferenceResolverFn`.
pub struct ReferenceResolverFn<M>(Box<dyn Fn(&mut M) -> Result<()> + Send + Sync>);

impl<M> ReferenceResolverFn<M> {
    pub fn new(f: impl Fn(&mut M) -> Result<()> + Send + Sync + 'static) -> Self {
        Self(Box::new(f))
    }
}

impl<M: Send + 'static> ReferenceResolver<M> for ReferenceResolverFn<M> {
    fn resolve_references<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>> {
        futures::future::ready((self.0)(mg)).boxed()
    }
}

/// Go `APISimpleReferenceResolver`.
pub struct ApiSimpleReferenceResolver {
    client: Arc<dyn Client>,
}

impl ApiSimpleReferenceResolver {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }
}

impl<M: Managed> ReferenceResolver<M> for ApiSimpleReferenceResolver {
    fn resolve_references<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>> {
        async move {
            let existing = mg.to_json().wrap_err(ERR_MARSHAL_EXISTING)?;
            let Some(resolving) = mg.resolve_references(&*self.client) else {
                return Ok(());
            };
            resolving.await.wrap_err(ERR_RESOLVE_REFERENCES)?;
            let resolved = mg.to_json().wrap_err(ERR_MARSHAL_RESOLVED)?;
            if without_empty(existing.clone()) == without_empty(resolved.clone()) {
                return Ok(());
            }
            let patch = Patch::Apply {
                body: prepare_json_merge(existing, &resolved),
                field_manager: REFERENCE_RESOLVER_FIELD_OWNER.to_owned(),
                force: true,
            };
            self.client.patch(mg, patch).await.wrap_err(ERR_PATCH_MANAGED)
        }
        .boxed()
    }
}

/// Go `cmpopts.EquateEmpty` on the JSON forms: an empty list or map equals an absent one.
fn without_empty(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(k, v)| (k, without_empty(v)))
                .filter(|(_, v)| !is_empty(v))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(without_empty).collect()),
        other => other,
    }
}

fn is_empty(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::Object(fields) => fields.is_empty(),
        serde_json::Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

/// Go `prepareJSONMerge` (`A:210-233`): the JSON merge patch from `existing` to `resolved`.
/// `apiVersion` and `kind` are dropped from `existing` first, so the patch always carries them,
/// as a server-side apply body must.
pub(crate) fn prepare_json_merge(mut existing: serde_json::Value, resolved: &serde_json::Value) -> serde_json::Value {
    if let Some(fields) = existing.as_object_mut() {
        fields.remove("apiVersion");
        fields.remove("kind");
    }
    create_merge_patch(&existing, resolved)
}

/// `jsonpatch.CreateMergePatch` (RFC 7386): changed and added keys carry their new value,
/// removed keys `null`, and a list that differs is replaced whole.
fn create_merge_patch(from: &serde_json::Value, to: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    let (Value::Object(from), Value::Object(to)) = (from, to) else {
        return to.clone();
    };
    let mut patch = serde_json::Map::new();
    for (key, new) in to {
        match from.get(key) {
            Some(old) if old == new => {}
            Some(old) if old.is_object() && new.is_object() => {
                patch.insert(key.clone(), create_merge_patch(old, new));
            }
            _ => {
                patch.insert(key.clone(), new.clone());
            }
        }
    }
    for key in from.keys().filter(|key| !to.contains_key(*key)) {
        patch.insert(key.clone(), Value::Null);
    }
    Value::Object(patch)
}

/// Go `CriticalAnnotationUpdater`.
pub trait CriticalAnnotationUpdater<M>: Send + Sync + 'static {
    fn update_critical_annotations<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>>;
}

/// Go `CriticalAnnotationUpdateFn`.
pub struct CriticalAnnotationUpdateFn<M>(Box<dyn Fn(&mut M) -> Result<()> + Send + Sync>);

impl<M> CriticalAnnotationUpdateFn<M> {
    pub fn new(f: impl Fn(&mut M) -> Result<()> + Send + Sync + 'static) -> Self {
        Self(Box::new(f))
    }
}

impl<M: Send + 'static> CriticalAnnotationUpdater<M> for CriticalAnnotationUpdateFn<M> {
    fn update_critical_annotations<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>> {
        futures::future::ready((self.0)(mg)).boxed()
    }
}

/// Go `RetryingCriticalAnnotationUpdater`.
pub struct RetryingCriticalAnnotationUpdater {
    client: Arc<dyn Client>,
}

impl RetryingCriticalAnnotationUpdater {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }
}

// client-go `retry.DefaultRetry` (`util/retry/util.go:28-33`); its factor is 1.0, so every wait
// is the base duration plus jitter.
const DEFAULT_RETRY_STEPS: u32 = 5;
const DEFAULT_RETRY_DURATION: Duration = Duration::from_millis(10);
const DEFAULT_RETRY_JITTER: f64 = 0.1;

impl RetryingCriticalAnnotationUpdater {
    /// One attempt (`A:287-297`). After a conflict the object is re-read and the saved
    /// annotations re-applied, so the next attempt writes them onto the latest version; the
    /// conflict is still returned so the loop retries. Go retries every error but
    /// `context.Canceled`, and cancellation here is the future being dropped.
    async fn update_once<M: Managed>(&self, mg: &mut M, annotations: &BTreeMap<String, String>) -> Result<()> {
        let Err(e) = self.client.update(mg).await else {
            return Ok(());
        };
        if e.is_conflict() {
            *mg = self.client.get_as::<M>(&(&*mg as &dyn Object).key()).await?;
            meta::add_annotations(mg, annotations.clone());
        }
        Err(e)
    }
}

impl<M: Managed> CriticalAnnotationUpdater<M> for RetryingCriticalAnnotationUpdater {
    fn update_critical_annotations<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>> {
        async move {
            let annotations = mg.meta().annotations.clone().unwrap_or_default();
            let mut attempt = 1;
            loop {
                let Err(e) = self.update_once(mg, &annotations).await else {
                    return Ok(());
                };
                if attempt == DEFAULT_RETRY_STEPS {
                    return Err(e.wrap(ERR_UPDATE_CRITICAL_ANNOTATIONS));
                }
                attempt += 1;
                tokio::time::sleep(DEFAULT_RETRY_DURATION.mul_f64(1.0 + rand::random::<f64>() * DEFAULT_RETRY_JITTER))
                    .await;
            }
        }
        .boxed()
    }
}

/// Go `LocalConnectionPublisher`. Modern kinds only, so there is one publisher and no
/// `ConnectionSecretOwner` dispatch.
pub trait ConnectionPublisher<M>: Send + Sync + 'static {
    /// Returns whether the Secret was written.
    fn publish_connection<'a>(&'a self, mg: &'a M, c: &'a ConnectionDetails) -> BoxFuture<'a, Result<bool>>;
    fn unpublish_connection<'a>(&'a self, mg: &'a M, c: &'a ConnectionDetails) -> BoxFuture<'a, Result<()>>;
}

type PublishFn<M> = Box<dyn Fn(&M, &ConnectionDetails) -> Result<bool> + Send + Sync>;
type UnpublishFn<M> = Box<dyn Fn(&M, &ConnectionDetails) -> Result<()> + Send + Sync>;

/// Go `LocalConnectionPublisherFns`. A `None` field panics when called.
pub struct ConnectionPublisherFns<M> {
    pub publish_connection: Option<PublishFn<M>>,
    pub unpublish_connection: Option<UnpublishFn<M>>,
}

impl<M> Default for ConnectionPublisherFns<M> {
    fn default() -> Self {
        Self {
            publish_connection: None,
            unpublish_connection: None,
        }
    }
}

impl<M: Sync + 'static> ConnectionPublisher<M> for ConnectionPublisherFns<M> {
    fn publish_connection<'a>(&'a self, mg: &'a M, c: &'a ConnectionDetails) -> BoxFuture<'a, Result<bool>> {
        let f = self
            .publish_connection
            .as_ref()
            .expect("unexpected call to ConnectionPublisherFns::publish_connection");
        futures::future::ready(f(mg, c)).boxed()
    }
    fn unpublish_connection<'a>(&'a self, mg: &'a M, c: &'a ConnectionDetails) -> BoxFuture<'a, Result<()>> {
        let f = self
            .unpublish_connection
            .as_ref()
            .expect("unexpected call to ConnectionPublisherFns::unpublish_connection");
        futures::future::ready(f(mg, c)).boxed()
    }
}

/// Go `APILocalSecretPublisher`. Unpublish is a no-op: garbage collection removes the Secret.
pub struct ApiLocalSecretPublisher {
    client: Arc<dyn Client>,
}

impl ApiLocalSecretPublisher {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }
}

impl<M: Managed> ConnectionPublisher<M> for ApiLocalSecretPublisher {
    fn publish_connection<'a>(&'a self, mg: &'a M, c: &'a ConnectionDetails) -> BoxFuture<'a, Result<bool>> {
        async move {
            if mg.write_connection_secret_to_reference().is_none() {
                return Ok(false);
            }
            let _ = (&self.client, c);
            // TODO (A:163): apply local_connection_secret_for with ApiPatchingApplicator, the
            // controllable-by check, and a skip when `data` is unchanged.
            todo!()
        }
        .boxed()
    }
    fn unpublish_connection<'a>(&'a self, _: &'a M, _: &'a ConnectionDetails) -> BoxFuture<'a, Result<()>> {
        futures::future::ready(Ok(())).boxed()
    }
}

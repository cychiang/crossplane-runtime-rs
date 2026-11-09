//! The replaceable collaborators of the managed reconciler (`api.go` and the interfaces in
//! `reconciler.go`). Each trait is object safe. Each has a `*Fn` or `*Fns` adapter over
//! synchronous closures for tables, and a default that talks to the API server through the
//! client seam.

use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::Result;
use crossplane_core::resource::Managed;
use crossplane_core::v2::ConnectionDetails;
use futures::FutureExt;
use futures::future::BoxFuture;

use super::{ERR_UPDATE_MANAGED, REFERENCE_RESOLVER_FIELD_OWNER};

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
            let _ = (&self.client, mg, ERR_UPDATE_MANAGED);
            // TODO (A:69): external name set -> Ok; else set it to metadata.name and Update,
            // wrapped in ERR_UPDATE_MANAGED.
            todo!()
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
            let _ = (&self.client, mg, REFERENCE_RESOLVER_FIELD_OWNER);
            // TODO (A:237): before = mg.to_json(); Some(fut) = mg.resolve_references(client) else
            // return Ok; after = mg.to_json(); equal with empty == nil -> Ok; body = the JSON
            // merge diff from before to after, always carrying apiVersion and kind;
            // client.patch(mg, Patch::Apply { body, field_manager, force: true }).
            todo!()
        }
        .boxed()
    }
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

impl<M: Managed> CriticalAnnotationUpdater<M> for RetryingCriticalAnnotationUpdater {
    fn update_critical_annotations<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<()>> {
        async move {
            let _ = (&self.client, mg);
            // TODO (A:283): Update; on any error except cancellation, Get, re-apply the saved
            // annotations and retry with client-go's DefaultRetry (5 steps, 10ms, factor 1.0,
            // jitter 0.1).
            todo!()
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
            let _ = (&self.client, mg, c);
            // TODO (A:158): no writeConnectionSecretToRef -> Ok(false); else apply
            // local_connection_secret_for with ApiPatchingApplicator, the controllable-by check,
            // and a skip when `data` is unchanged.
            todo!()
        }
        .boxed()
    }
    fn unpublish_connection<'a>(&'a self, _: &'a M, _: &'a ConnectionDetails) -> BoxFuture<'a, Result<()>> {
        futures::future::ready(Ok(())).boxed()
    }
}

//! The provider-facing half of the managed reconciler: connect to the external system and drive
//! the external resource.

use std::collections::BTreeMap;
use std::future::Future;

use crossplane_core::errors::Result;
use crossplane_core::resource::Managed;
use crossplane_core::v2::ConnectionDetails;
use futures::FutureExt;
use futures::future::BoxFuture;

/// Go `AdditionalDetails`, for the change log.
pub type AdditionalDetails = BTreeMap<String, String>;

/// Go `ExternalObservation`. `Default` is "does not exist".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalObservation {
    pub resource_exists: bool,
    pub resource_up_to_date: bool,
    pub resource_late_initialized: bool,
    pub connection_details: ConnectionDetails,
    /// Debug-logged only (S6).
    pub diff: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalCreation {
    pub connection_details: ConnectionDetails,
    pub additional_details: AdditionalDetails,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalUpdate {
    pub connection_details: ConnectionDetails,
    pub additional_details: AdditionalDetails,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalDelete {
    pub additional_details: AdditionalDetails,
}

/// Go `ExternalConnector` plus `ExternalDisconnector`. Typed by kind, so provider code has no
/// type assertion at the top of each method, and written with plain `async fn`. `Client` is an
/// associated type: one connector type may serve many kinds.
pub trait ExternalConnector<M: Managed>: Send + Sync + 'static {
    type Client: ExternalClient<M>;

    /// Runs under the external deadline (Go `externalCtx`, `Timing::timeout`). Exceeding it drops
    /// the future at an await point; the reconciler reports
    /// `connect failed: context deadline exceeded`. That drop is the cancellation, so the traits
    /// take no context.
    fn connect(&self, mg: &M) -> impl Future<Output = Result<Self::Client>> + Send;

    /// Called at every return after a successful connect, before the client's own `disconnect`
    /// (X5).
    fn disconnect(&self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
}

/// Go `ExternalClient`. `&mut M` because Go passes the managed resource by pointer and providers
/// set conditions, annotations and `atProvider` on it.
pub trait ExternalClient<M: Managed>: Send + Sync + 'static {
    fn observe(&self, mg: &mut M) -> impl Future<Output = Result<ExternalObservation>> + Send;
    fn create(&self, mg: &mut M) -> impl Future<Output = Result<ExternalCreation>> + Send;
    fn update(&self, mg: &mut M) -> impl Future<Output = Result<ExternalUpdate>> + Send;
    fn delete(&self, mg: &mut M) -> impl Future<Output = Result<ExternalDelete>> + Send;
    fn disconnect(&self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
}

// Object-safe mirrors, so the reconciler stores `Arc<dyn DynConnector<M>>` without a type
// parameter per collaborator, and authors never see boxed futures.

pub(crate) trait DynConnector<M>: Send + Sync {
    fn connect<'a>(&'a self, mg: &'a M) -> BoxFuture<'a, Result<Box<dyn DynExternal<M>>>>;
    fn disconnect(&self) -> BoxFuture<'_, Result<()>>;
}

pub(crate) trait DynExternal<M>: Send + Sync {
    fn observe<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalObservation>>;
    fn create<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalCreation>>;
    fn update<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalUpdate>>;
    fn delete<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalDelete>>;
    fn disconnect(&self) -> BoxFuture<'_, Result<()>>;
}

impl<M: Managed, C: ExternalConnector<M>> DynConnector<M> for C {
    fn connect<'a>(&'a self, mg: &'a M) -> BoxFuture<'a, Result<Box<dyn DynExternal<M>>>> {
        async move {
            let c = ExternalConnector::connect(self, mg).await?;
            Ok(Box::new(c) as Box<dyn DynExternal<M>>)
        }
        .boxed()
    }
    fn disconnect(&self) -> BoxFuture<'_, Result<()>> {
        ExternalConnector::disconnect(self).boxed()
    }
}

impl<M: Managed, E: ExternalClient<M>> DynExternal<M> for E {
    fn observe<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalObservation>> {
        ExternalClient::observe(self, mg).boxed()
    }
    fn create<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalCreation>> {
        ExternalClient::create(self, mg).boxed()
    }
    fn update<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalUpdate>> {
        ExternalClient::update(self, mg).boxed()
    }
    fn delete<'a>(&'a self, mg: &'a mut M) -> BoxFuture<'a, Result<ExternalDelete>> {
        ExternalClient::delete(self, mg).boxed()
    }
    fn disconnect(&self) -> BoxFuture<'_, Result<()>> {
        ExternalClient::disconnect(self).boxed()
    }
}

/// Go `NopConnector`, the default.
pub struct NopConnector;

/// Go `NopClient`.
pub struct NopClient;

impl<M: Managed> ExternalConnector<M> for NopConnector {
    type Client = NopClient;
    async fn connect(&self, _: &M) -> Result<NopClient> {
        Ok(NopClient)
    }
}

impl<M: Managed> ExternalClient<M> for NopClient {
    async fn observe(&self, _: &mut M) -> Result<ExternalObservation> {
        Ok(ExternalObservation::default())
    }
    async fn create(&self, _: &mut M) -> Result<ExternalCreation> {
        Ok(ExternalCreation::default())
    }
    async fn update(&self, _: &mut M) -> Result<ExternalUpdate> {
        Ok(ExternalUpdate::default())
    }
    async fn delete(&self, _: &mut M) -> Result<ExternalDelete> {
        Ok(ExternalDelete::default())
    }
}

type Op<M, T> = Box<dyn Fn(&mut M) -> Result<T> + Send + Sync>;

/// Go `ExternalClientFns`, for tables. Closures are synchronous because no Go table awaits
/// inside one. A `None` field panics when called, which pins "this operation must not run".
pub struct ExternalClientFns<M> {
    pub observe: Option<Op<M, ExternalObservation>>,
    pub create: Option<Op<M, ExternalCreation>>,
    pub update: Option<Op<M, ExternalUpdate>>,
    pub delete: Option<Op<M, ExternalDelete>>,
    pub disconnect: Option<Box<dyn Fn() -> Result<()> + Send + Sync>>,
}

impl<M> Default for ExternalClientFns<M> {
    fn default() -> Self {
        Self {
            observe: None,
            create: None,
            update: None,
            delete: None,
            disconnect: None,
        }
    }
}

fn called<'a, T: ?Sized>(f: &'a Option<Box<T>>, name: &str) -> &'a T {
    f.as_deref()
        .unwrap_or_else(|| panic!("unexpected call to ExternalClientFns::{name}"))
}

impl<M: Managed> ExternalClient<M> for ExternalClientFns<M> {
    async fn observe(&self, mg: &mut M) -> Result<ExternalObservation> {
        called(&self.observe, "observe")(mg)
    }
    async fn create(&self, mg: &mut M) -> Result<ExternalCreation> {
        called(&self.create, "create")(mg)
    }
    async fn update(&self, mg: &mut M) -> Result<ExternalUpdate> {
        called(&self.update, "update")(mg)
    }
    async fn delete(&self, mg: &mut M) -> Result<ExternalDelete> {
        called(&self.delete, "delete")(mg)
    }
    async fn disconnect(&self) -> Result<()> {
        match &self.disconnect {
            Some(f) => f(),
            None => Ok(()),
        }
    }
}

/// Go `ExternalConnectorFn`.
pub struct ExternalConnectorFn<M>(pub Box<dyn Fn(&M) -> Result<ExternalClientFns<M>> + Send + Sync>);

impl<M> ExternalConnectorFn<M> {
    pub fn new(f: impl Fn(&M) -> Result<ExternalClientFns<M>> + Send + Sync + 'static) -> Self {
        Self(Box::new(f))
    }
}

impl<M: Managed> ExternalConnector<M> for ExternalConnectorFn<M> {
    type Client = ExternalClientFns<M>;
    async fn connect(&self, mg: &M) -> Result<ExternalClientFns<M>> {
        (self.0)(mg)
    }
}

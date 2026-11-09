//! The production `Client`: reads from the informer cache, writes straight to the API server
//! with the verbs Go uses, so the audit trace keeps Go's verbs.
//!
//! This file and `cache.rs` are the only ones that touch `kube::Api`, and [`convert_error`] is
//! the only place a `kube::Error` becomes an `Error`.

use crossplane_core::client::{Client, ListOptions, Patch};
use crossplane_core::errors::{Error, Result, StatusReason};
use crossplane_core::resource::{Object, ObjectKey, ObjectType};
use futures::FutureExt;
use futures::future::BoxFuture;
use kube::api::{Api, DeleteParams, DynamicObject, Patch as KubePatch, PatchParams, PostParams};

use crate::cache::Cache;

pub(crate) struct KubeClient {
    kube: kube::Client,
    cache: Cache,
}

impl KubeClient {
    pub(crate) fn new(kube: kube::Client, cache: Cache) -> Self {
        Self { kube, cache }
    }

    fn api(&self, obj: &dyn Object) -> Api<DynamicObject> {
        let object_type = obj.object_type();
        match obj.meta().namespace.as_deref() {
            Some(namespace) => Api::namespaced_with(self.kube.clone(), namespace, object_type.api_resource()),
            None => Api::all_with(self.kube.clone(), object_type.api_resource()),
        }
    }
}

fn name(obj: &dyn Object) -> Result<String> {
    obj.meta().name.clone().ok_or_else(|| Error::new("object has no name"))
}

fn body(obj: &dyn Object) -> Result<DynamicObject> {
    serde_json::from_value(obj.to_json()?).map_err(Error::foreign)
}

fn reflect(obj: &mut dyn Object, response: DynamicObject) -> Result<()> {
    obj.set_from_json(serde_json::to_value(response).map_err(Error::foreign)?)
}

fn convert_error(err: kube::Error) -> Error {
    match &err {
        kube::Error::Api(status) => {
            let reason = match status.reason.as_str() {
                "NotFound" => StatusReason::NotFound,
                "AlreadyExists" => StatusReason::AlreadyExists,
                "Conflict" => StatusReason::Conflict,
                "Forbidden" => StatusReason::Forbidden,
                "Invalid" => StatusReason::Invalid,
                other => StatusReason::Other(other.to_owned()),
            };
            Error::status(reason, status.message.clone())
        }
        _ => Error::foreign(err),
    }
}

impl Client for KubeClient {
    fn get<'a>(&'a self, object_type: &'a ObjectType, key: &'a ObjectKey) -> BoxFuture<'a, Result<Box<dyn Object>>> {
        self.cache.get(object_type, key).boxed()
    }

    fn list<'a>(
        &'a self,
        object_type: &'a ObjectType,
        options: &'a ListOptions,
    ) -> BoxFuture<'a, Result<Vec<Box<dyn Object>>>> {
        self.cache.list(object_type, options).boxed()
    }

    fn create<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            let response = self
                .api(obj)
                .create(&PostParams::default(), &body(obj)?)
                .await
                .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn update<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            let response = self
                .api(obj)
                .replace(&name(obj)?, &PostParams::default(), &body(obj)?)
                .await
                .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn update_status<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            let response = self
                .api(obj)
                .replace_status(&name(obj)?, &PostParams::default(), &body(obj)?)
                .await
                .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn patch<'a>(&'a self, obj: &'a mut dyn Object, patch: Patch) -> BoxFuture<'a, Result<()>> {
        async move {
            let (name, api) = (name(obj)?, self.api(obj));
            let response = match patch {
                Patch::Merge(body) => api.patch(&name, &PatchParams::default(), &KubePatch::Merge(body)).await,
                Patch::Apply {
                    body,
                    field_manager,
                    force,
                } => {
                    let mut params = PatchParams::apply(&field_manager);
                    params.force = force;
                    api.patch(&name, &params, &KubePatch::Apply(body)).await
                }
            }
            .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn delete<'a>(&'a self, obj: &'a dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            self.api(obj)
                .delete(&name(obj)?, &DeleteParams::default())
                .await
                .map(|_| ())
                .map_err(convert_error)
        }
        .boxed()
    }
}

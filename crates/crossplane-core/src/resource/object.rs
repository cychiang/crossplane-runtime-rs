use std::any::Any;
use std::fmt;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Resource;
use kube::core::{ApiResource, GroupVersionKind};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::errors::{Error, Result};

/// Go `client.Object`: a Kubernetes object with its concrete type erased. Blanket-implemented for
/// every typed kube resource, so nobody implements it by hand.
pub trait Object: Any + fmt::Debug + Send + Sync {
    fn object_type(&self) -> ObjectType;
    fn meta(&self) -> &ObjectMeta;
    fn meta_mut(&mut self) -> &mut ObjectMeta;
    /// The wire form, always carrying `apiVersion` and `kind`. Called by `Client` implementations
    /// and by comparison helpers.
    fn to_json(&self) -> Result<serde_json::Value>;
    /// Replaces `self` with a decoded server response, as Go's client decodes the response into
    /// the object passed to `Update`.
    fn set_from_json(&mut self, document: serde_json::Value) -> Result<()>;
}

/// An [`Object`] whose concrete type is known at compile time. Deliberately without `Default`: a
/// provider's spec may have required fields with no sensible empty value.
pub trait Kind:
    Resource<DynamicType = ()> + Serialize + DeserializeOwned + Clone + fmt::Debug + Send + Sync + 'static
{
}

impl<K> Kind for K where
    K: Resource<DynamicType = ()> + Serialize + DeserializeOwned + Clone + fmt::Debug + Send + Sync + 'static
{
}

impl<K: Kind> Object for K {
    fn object_type(&self) -> ObjectType {
        ObjectType::of::<K>()
    }

    fn meta(&self) -> &ObjectMeta {
        Resource::meta(self)
    }

    fn meta_mut(&mut self) -> &mut ObjectMeta {
        Resource::meta_mut(self)
    }

    fn to_json(&self) -> Result<serde_json::Value> {
        let mut document = serde_json::to_value(self).map_err(Error::foreign)?;
        if let Some(fields) = document.as_object_mut() {
            fields.insert("apiVersion".into(), K::api_version(&()).into_owned().into());
            fields.insert("kind".into(), K::kind(&()).into_owned().into());
        }
        Ok(document)
    }

    fn set_from_json(&mut self, document: serde_json::Value) -> Result<()> {
        *self = serde_json::from_value(document).map_err(Error::foreign)?;
        Ok(())
    }
}

impl dyn Object {
    pub fn downcast_ref<T: Object>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }

    pub fn downcast_mut<T: Object>(&mut self) -> Option<&mut T> {
        (self as &mut dyn Any).downcast_mut()
    }

    pub fn downcast<T: Object>(self: Box<Self>) -> Result<Box<T>, Box<dyn Object>> {
        if (&*self as &dyn Any).is::<T>() {
            Ok((self as Box<dyn Any>).downcast().expect("checked by is"))
        } else {
            Err(self)
        }
    }

    pub fn key(&self) -> ObjectKey {
        ObjectKey {
            namespace: self.meta().namespace.clone(),
            name: self.meta().name.clone().unwrap_or_default(),
        }
    }
}

/// A kind as a value: its names plus how to decode a JSON document into the Rust type. Replaces
/// Go's scheme. Built only by [`ObjectType::of`], so the names and the decoder cannot disagree.
#[derive(Clone)]
pub struct ObjectType {
    resource: ApiResource,
    decode: fn(serde_json::Value) -> Result<Box<dyn Object>>,
}

impl ObjectType {
    pub fn of<K: Kind>() -> Self {
        Self {
            resource: ApiResource::erase::<K>(&()),
            decode: |document| Ok(Box::new(serde_json::from_value::<K>(document).map_err(Error::foreign)?)),
        }
    }

    pub fn api_resource(&self) -> &ApiResource {
        &self.resource
    }

    pub fn gvk(&self) -> GroupVersionKind {
        GroupVersionKind::gvk(&self.resource.group, &self.resource.version, &self.resource.kind)
    }

    /// Go `schema.GroupKind.String()`: `<Kind>.<group>`.
    pub fn group_kind(&self) -> String {
        format!("{}.{}", self.resource.kind, self.resource.group)
    }

    /// Called by `Client` implementations only.
    pub fn decode(&self, document: serde_json::Value) -> Result<Box<dyn Object>> {
        (self.decode)(document)
    }
}

impl PartialEq for ObjectType {
    fn eq(&self, other: &Self) -> bool {
        self.resource == other.resource
    }
}

impl Eq for ObjectType {}

impl std::hash::Hash for ObjectType {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.resource.hash(state);
    }
}

impl fmt::Debug for ObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, Kind={}", self.resource.api_version, self.resource.kind)
    }
}

/// Go `types.NamespacedName`. Also the reconcile request.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectKey {
    pub namespace: Option<String>,
    pub name: String,
}

impl ObjectKey {
    pub fn namespaced(namespace: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            namespace: Some(namespace.into()),
            name: name.into(),
        }
    }

    pub fn cluster(name: impl Into<String>) -> Self {
        Self {
            namespace: None,
            name: name.into(),
        }
    }
}

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

    /// Go `schema.GroupKind.String()`: `<Kind>.<group>`, or `<Kind>` for the core group.
    pub fn group_kind(&self) -> String {
        if self.resource.group.is_empty() {
            self.resource.kind.clone()
        } else {
            format!("{}.{}", self.resource.kind, self.resource.group)
        }
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

/// Go `schema.GroupVersionKind.String()`.
impl fmt::Debug for ObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = &self.resource;
        write!(f, "{}/{}, Kind={}", r.group, r.version, r.kind)
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

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::HashSet;

    use k8s_openapi::api::core::v1::ConfigMap;
    use kube::core::NamespaceResourceScope;
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    /// A kind whose Rust form carries no `apiVersion` or `kind`, as kube's derive emits.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Widget {
        metadata: ObjectMeta,
        spec: WidgetSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<WidgetStatus>,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct WidgetSpec {
        size: i64,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct WidgetStatus {
        phase: String,
    }

    impl Resource for Widget {
        type DynamicType = ();
        type Scope = NamespaceResourceScope;

        fn kind(_: &()) -> Cow<'_, str> {
            "Widget".into()
        }
        fn group(_: &()) -> Cow<'_, str> {
            "test.example.com".into()
        }
        fn version(_: &()) -> Cow<'_, str> {
            "v1".into()
        }
        fn plural(_: &()) -> Cow<'_, str> {
            "widgets".into()
        }
        fn meta(&self) -> &ObjectMeta {
            &self.metadata
        }
        fn meta_mut(&mut self) -> &mut ObjectMeta {
            &mut self.metadata
        }
    }

    fn widget() -> Widget {
        Widget {
            metadata: ObjectMeta {
                name: Some("w".into()),
                namespace: Some("ns".into()),
                resource_version: Some("1".into()),
                labels: Some([("local".to_owned(), "edit".to_owned())].into()),
                ..Default::default()
            },
            spec: WidgetSpec { size: 3 },
            status: Some(WidgetStatus { phase: "local".into() }),
        }
    }

    #[test]
    fn to_json_carries_api_version_and_kind() {
        let got = widget().to_json().unwrap();
        assert_eq!(got["apiVersion"], "test.example.com/v1");
        assert_eq!(got["kind"], "Widget");
        assert_eq!(got["spec"], json!({"size": 3}));
        assert_eq!(got["metadata"]["name"], "w");
    }

    #[test]
    fn json_bridge_round_trips_through_object_type() {
        let original = widget();
        let decoded = ObjectType::of::<Widget>().decode(original.to_json().unwrap()).unwrap();
        assert_eq!(decoded.object_type(), ObjectType::of::<Widget>());
        assert_eq!(decoded.downcast_ref::<Widget>(), Some(&original));
    }

    #[test]
    fn decode_of_a_document_that_does_not_fit_the_kind_is_an_error() {
        let got = ObjectType::of::<Widget>().decode(json!({"metadata": {}, "spec": {"size": "big"}}));
        assert!(got.is_err(), "a string size cannot decode into a Widget");
    }

    /// Go's client decodes the server response into the object passed to `Update`, so a status
    /// the server did not persist and a label the server dropped are gone afterwards, and the
    /// resourceVersion is the server's. Parity spec S5 depends on this.
    #[test]
    fn set_from_json_overwrites_the_whole_object() {
        let mut local = widget();
        let response = json!({
            "apiVersion": "test.example.com/v1",
            "kind": "Widget",
            "metadata": {"name": "w", "namespace": "ns", "resourceVersion": "2"},
            "spec": {"size": 3},
            "status": {"phase": "server"},
        });
        let obj: &mut dyn Object = &mut local;
        obj.set_from_json(response).unwrap();
        assert_eq!(local.metadata.resource_version.as_deref(), Some("2"));
        assert_eq!(
            local.metadata.labels, None,
            "fields absent from the response are cleared"
        );
        assert_eq!(local.status, Some(WidgetStatus { phase: "server".into() }));

        let obj: &mut dyn Object = &mut local;
        obj.set_from_json(json!({"metadata": {"name": "w"}, "spec": {"size": 3}}))
            .unwrap();
        assert_eq!(local.status, None, "a response without status clears the local status");
    }

    #[test]
    fn set_from_json_leaves_the_object_alone_when_the_response_does_not_decode() {
        let mut local = widget();
        let obj: &mut dyn Object = &mut local;
        assert!(obj.set_from_json(json!({"spec": {"size": "big"}})).is_err());
        assert_eq!(local, widget());
    }

    #[test]
    fn downcasts_succeed_only_for_the_concrete_type() {
        let mut obj: Box<dyn Object> = Box::new(widget());
        assert!(obj.downcast_ref::<ConfigMap>().is_none());
        obj.downcast_mut::<Widget>().unwrap().spec.size = 4;
        assert_eq!(obj.downcast_ref::<Widget>().map(|w| w.spec.size), Some(4));

        let obj = obj.downcast::<ConfigMap>().expect_err("a Widget is not a ConfigMap");
        assert_eq!(
            obj.meta().name.as_deref(),
            Some("w"),
            "a failed downcast hands the object back"
        );
        assert_eq!(obj.downcast::<Widget>().unwrap().spec.size, 4);
    }

    #[test]
    fn key_reads_namespace_and_name() {
        let obj: &dyn Object = &widget();
        assert_eq!(obj.key(), ObjectKey::namespaced("ns", "w"));
        let unnamed: &dyn Object = &ConfigMap::default();
        assert_eq!(unnamed.key(), ObjectKey::default());
    }

    #[test]
    fn object_type_names_match_go_schema_strings() {
        let widget = ObjectType::of::<Widget>();
        assert_eq!(widget.group_kind(), "Widget.test.example.com");
        assert_eq!(format!("{widget:?}"), "test.example.com/v1, Kind=Widget");
        assert_eq!(widget.gvk(), GroupVersionKind::gvk("test.example.com", "v1", "Widget"));
        assert_eq!(widget.api_resource().plural, "widgets");

        let config_map = ObjectType::of::<ConfigMap>();
        assert_eq!(config_map.group_kind(), "ConfigMap");
        assert_eq!(format!("{config_map:?}"), "/v1, Kind=ConfigMap");
    }

    #[test]
    fn object_types_compare_and_hash_by_kind() {
        assert_eq!(ObjectType::of::<Widget>(), ObjectType::of::<Widget>());
        assert_ne!(ObjectType::of::<Widget>(), ObjectType::of::<ConfigMap>());
        let set: HashSet<_> = [
            ObjectType::of::<Widget>(),
            ObjectType::of::<Widget>(),
            ObjectType::of::<ConfigMap>(),
        ]
        .into();
        assert_eq!(set.len(), 2);
    }
}

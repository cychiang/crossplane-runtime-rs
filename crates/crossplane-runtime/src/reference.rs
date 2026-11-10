//! Go `pkg/reference`: what angryjet-generated `ResolveReferences` methods call. Namespaced
//! resources only; Go's cluster-scoped `ResolutionRequest` family has no counterpart here.
//!
//! Go's `To{Managed, List}` names the referenced kind twice, once for `Get` and once for `List`.
//! The Rust client lists and gets typed kinds, so the request is generic over the referenced kind
//! and that one type parameter replaces both. Go's `Namespace` request field is ignored by the
//! namespaced resolver, so it is not here either.

use std::collections::BTreeMap;

use crossplane_core::client::{Client, ListOptions};
use crossplane_core::errors::{Error, Result};
use crossplane_core::meta;
use crossplane_core::resource::{Managed, ObjectKey};
use crossplane_core::v2::{NamespacedReference, NamespacedSelector, Policy};

pub const ERR_GET_MANAGED: &str = "cannot get referenced resource";
pub const ERR_LIST_MANAGED: &str = "cannot list resources that match selector";
pub const ERR_NO_MATCHES: &str = "no resources matched selector";
pub const ERR_NO_VALUE: &str = "referenced field was empty (referenced resource may not yet be ready)";

/// Go `FromPtrValue`.
pub fn from_ptr_value(v: Option<&str>) -> String {
    v.unwrap_or_default().to_owned()
}

/// Go `FromFloatPtrValue`: `strconv.FormatFloat(v, 'f', 0, 64)`.
pub fn from_float_ptr_value(v: Option<f64>) -> String {
    match v {
        None => String::new(),
        Some(v) if v.is_nan() => "NaN".into(),
        Some(v) if v.is_infinite() => if v > 0.0 { "+Inf" } else { "-Inf" }.into(),
        Some(v) => format!("{v:.0}"),
    }
}

/// Go `FromIntPtrValue`.
pub fn from_int_ptr_value(v: Option<i64>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

/// Go `ToPtrValue`.
pub fn to_ptr_value(v: &str) -> Option<String> {
    (!v.is_empty()).then(|| v.to_owned())
}

/// Go `ToFloatPtrValue`: `None` for the empty string and for anything that does not parse.
pub fn to_float_ptr_value(v: &str) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.parse().ok()
}

/// Go `ToIntPtrValue`.
pub fn to_int_ptr_value(v: &str) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.parse().ok()
}

/// Go `FromPtrValues`.
pub fn from_ptr_values(v: &[Option<String>]) -> Vec<String> {
    v.iter().map(|v| from_ptr_value(v.as_deref())).collect()
}

/// Go `FromFloatPtrValues`.
pub fn from_float_ptr_values(v: &[Option<f64>]) -> Vec<String> {
    v.iter().map(|v| from_float_ptr_value(*v)).collect()
}

/// Go `FromIntPtrValues`.
pub fn from_int_ptr_values(v: &[Option<i64>]) -> Vec<String> {
    v.iter().map(|v| from_int_ptr_value(*v)).collect()
}

/// Go `ToPtrValues`.
pub fn to_ptr_values(v: &[String]) -> Vec<Option<String>> {
    v.iter().map(|v| to_ptr_value(v)).collect()
}

/// Go `ToFloatPtrValues`.
pub fn to_float_ptr_values(v: &[String]) -> Vec<Option<f64>> {
    v.iter().map(|v| to_float_ptr_value(v)).collect()
}

/// Go `ToIntPtrValues`.
pub fn to_int_ptr_values(v: &[String]) -> Vec<Option<i64>> {
    v.iter().map(|v| to_int_ptr_value(v)).collect()
}

/// A field a single reference resolves into. Go has one `CurrentValue` string and generates the
/// pointer conversion at each call site; the generated Rust asks the field type instead.
pub trait ReferenceValue {
    fn current_value(&self) -> String;
    fn set_resolved(&mut self, v: String);
}

impl ReferenceValue for String {
    fn current_value(&self) -> String {
        self.clone()
    }
    fn set_resolved(&mut self, v: String) {
        *self = v;
    }
}

impl ReferenceValue for Option<String> {
    fn current_value(&self) -> String {
        from_ptr_value(self.as_deref())
    }
    fn set_resolved(&mut self, v: String) {
        *self = to_ptr_value(&v);
    }
}

impl ReferenceValue for Option<f64> {
    fn current_value(&self) -> String {
        from_float_ptr_value(*self)
    }
    fn set_resolved(&mut self, v: String) {
        *self = to_float_ptr_value(&v);
    }
}

impl ReferenceValue for Option<i64> {
    fn current_value(&self) -> String {
        from_int_ptr_value(*self)
    }
    fn set_resolved(&mut self, v: String) {
        *self = to_int_ptr_value(&v);
    }
}

/// A field a list of references resolves into.
pub trait ReferenceValues {
    fn current_values(&self) -> Vec<String>;
    fn set_resolved(&mut self, v: Vec<String>);
}

impl ReferenceValues for Vec<String> {
    fn current_values(&self) -> Vec<String> {
        self.clone()
    }
    fn set_resolved(&mut self, v: Vec<String>) {
        *self = v;
    }
}

impl ReferenceValues for Vec<Option<String>> {
    fn current_values(&self) -> Vec<String> {
        from_ptr_values(self)
    }
    fn set_resolved(&mut self, v: Vec<String>) {
        *self = to_ptr_values(&v);
    }
}

impl ReferenceValues for Vec<Option<f64>> {
    fn current_values(&self) -> Vec<String> {
        from_float_ptr_values(self)
    }
    fn set_resolved(&mut self, v: Vec<String>) {
        *self = to_float_ptr_values(&v);
    }
}

impl ReferenceValues for Vec<Option<i64>> {
    fn current_values(&self) -> Vec<String> {
        from_int_ptr_values(self)
    }
    fn set_resolved(&mut self, v: Vec<String>) {
        *self = to_int_ptr_values(&v);
    }
}

/// Go `ExtractValueFn`: how to read the value out of the resolved managed resource.
pub type ExtractValueFn<T> = Box<dyn Fn(&T) -> String + Send + Sync>;

/// Go `ExternalName()`.
pub fn external_name<T: Managed>() -> ExtractValueFn<T> {
    Box::new(|mg| meta::external_name(mg).to_owned())
}

/// Go `NamespacedResolutionRequest`. `Default` extracts the external name, which is what
/// angryjet generates when a marker names no extractor.
pub struct NamespacedResolutionRequest<T: Managed> {
    pub current_value: String,
    pub reference: Option<NamespacedReference>,
    pub selector: Option<NamespacedSelector>,
    pub extract: ExtractValueFn<T>,
}

impl<T: Managed> Default for NamespacedResolutionRequest<T> {
    fn default() -> Self {
        Self {
            current_value: String::new(),
            reference: None,
            selector: None,
            extract: external_name(),
        }
    }
}

fn policy_is_always(p: &Option<Policy>) -> bool {
    p.as_ref().is_some_and(Policy::is_resolve_policy_always)
}

impl<T: Managed> NamespacedResolutionRequest<T> {
    /// True if the request cannot or should not be processed. An Always selector discards the
    /// request's reference, as in Go.
    pub fn is_no_op(&mut self) -> bool {
        let mut is_always = false;
        if let Some(selector) = &self.selector {
            if policy_is_always(&selector.policy) {
                self.reference = None;
                is_always = true;
            }
        } else if let Some(reference) = &self.reference {
            is_always = policy_is_always(&reference.policy);
        }
        if !self.current_value.is_empty() && !is_always {
            return true;
        }
        self.reference.is_none() && self.selector.is_none()
    }
}

/// Go `NamespacedResolutionResponse`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamespacedResolutionResponse {
    pub resolved_value: String,
    pub resolved_reference: Option<NamespacedReference>,
}

impl NamespacedResolutionResponse {
    pub fn validate(&self) -> Result<()> {
        if self.resolved_value.is_empty() {
            return Err(Error::new(ERR_NO_VALUE));
        }
        Ok(())
    }
}

/// Go `MultiNamespacedResolutionRequest`.
pub struct MultiNamespacedResolutionRequest<T: Managed> {
    pub current_values: Vec<String>,
    pub references: Vec<NamespacedReference>,
    pub selector: Option<NamespacedSelector>,
    pub extract: ExtractValueFn<T>,
}

impl<T: Managed> Default for MultiNamespacedResolutionRequest<T> {
    fn default() -> Self {
        Self {
            current_values: Vec::new(),
            references: Vec::new(),
            selector: None,
            extract: external_name(),
        }
    }
}

impl<T: Managed> MultiNamespacedResolutionRequest<T> {
    /// True if the request cannot or should not be processed. An Always selector discards the
    /// request's references, as in Go.
    pub fn is_no_op(&mut self) -> bool {
        let mut is_always = false;
        if let Some(selector) = &self.selector {
            if policy_is_always(&selector.policy) {
                self.references.clear();
                is_always = true;
            }
        } else {
            is_always = self.references.iter().any(|r| policy_is_always(&r.policy));
        }
        if !self.current_values.is_empty() && !is_always {
            return true;
        }
        self.references.is_empty() && self.selector.is_none()
    }
}

/// Go `MultiNamespacedResolutionResponse`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MultiNamespacedResolutionResponse {
    pub resolved_values: Vec<String>,
    pub resolved_references: Vec<NamespacedReference>,
}

impl MultiNamespacedResolutionResponse {
    pub fn validate(&self) -> Result<()> {
        if self.resolved_values.is_empty() {
            return Err(Error::new(ERR_NO_MATCHES));
        }
        for (value, reference) in self.resolved_values.iter().zip(&self.resolved_references) {
            if value.is_empty() {
                return get_resolution_error(&reference.policy, Err(Error::new(ERR_NO_VALUE)));
            }
        }
        Ok(())
    }
}

fn get_resolution_error(policy: &Option<Policy>, result: Result<()>) -> Result<()> {
    match policy {
        Some(p) if p.is_resolution_policy_optional() => Ok(()),
        _ => result,
    }
}

/// Go `ControllersMustMatchNamespaced`.
pub fn controllers_must_match_namespaced(s: Option<&NamespacedSelector>) -> bool {
    s.is_some_and(|s| s.match_controller_ref == Some(true))
}

/// Go `APINamespacedResolver`. Borrows the referencing resource instead of holding it, so the
/// generated code can assign the resolved fields between calls by building a resolver per call.
pub struct ApiNamespacedResolver<'a, F: Managed> {
    client: &'a dyn Client,
    from: &'a F,
}

impl<'a, F: Managed> ApiNamespacedResolver<'a, F> {
    pub fn new(client: &'a dyn Client, from: &'a F) -> Self {
        Self { client, from }
    }

    fn namespace_or_from(&self, namespace: &str) -> Option<String> {
        let ns = if namespace.is_empty() {
            self.from.meta().namespace.as_deref().unwrap_or_default()
        } else {
            namespace
        };
        (!ns.is_empty()).then(|| ns.to_owned())
    }

    async fn get<T: Managed>(&self, reference: &NamespacedReference) -> Result<T> {
        let key = ObjectKey {
            namespace: self.namespace_or_from(&reference.namespace),
            name: reference.name.clone(),
        };
        self.client.get_as::<T>(&key).await
    }

    async fn select<T: Managed>(&self, selector: &NamespacedSelector) -> Result<(Vec<T>, String)> {
        let ns = self.namespace_or_from(&selector.namespace);
        let options = ListOptions {
            namespace: ns.clone(),
            labels: selector
                .match_labels
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<_, _>>(),
        };
        let items = self
            .client
            .list_as::<T>(&options)
            .await
            .map_err(|e| e.wrap(ERR_LIST_MANAGED))?;
        let items = items
            .into_iter()
            .filter(|to| {
                !controllers_must_match_namespaced(Some(selector)) || meta::have_same_controller(self.from, to)
            })
            .collect();
        Ok((items, ns.unwrap_or_default()))
    }

    /// Resolves the request. Unlike Go, an error carries no response: generated code never reads
    /// one next to an error.
    pub async fn resolve<T: Managed>(
        &self,
        mut req: NamespacedResolutionRequest<T>,
    ) -> Result<NamespacedResolutionResponse> {
        if meta::was_deleted(self.from) || req.is_no_op() {
            return Ok(NamespacedResolutionResponse {
                resolved_value: req.current_value,
                resolved_reference: req.reference,
            });
        }

        if let Some(reference) = req.reference {
            let to = match self.get::<T>(&reference).await {
                Ok(to) => to,
                Err(e) if e.is_not_found() => {
                    return get_resolution_error(&reference.policy, Err(e.wrap(ERR_GET_MANAGED)))
                        .map(|()| Default::default());
                }
                Err(e) => return Err(e.wrap(ERR_GET_MANAGED)),
            };
            let rsp = NamespacedResolutionResponse {
                resolved_value: (req.extract)(&to),
                resolved_reference: Some(reference.clone()),
            };
            return get_resolution_error(&reference.policy, rsp.validate()).map(|()| rsp);
        }

        let selector = req
            .selector
            .expect("a request that is not a no-op has a reference or a selector");
        let (items, ns) = self.select::<T>(&selector).await?;
        if let Some(to) = items.first() {
            let rsp = NamespacedResolutionResponse {
                resolved_value: (req.extract)(to),
                resolved_reference: Some(NamespacedReference {
                    name: to.meta().name.clone().unwrap_or_default(),
                    namespace: ns,
                    policy: None,
                }),
            };
            return get_resolution_error(&selector.policy, rsp.validate()).map(|()| rsp);
        }

        get_resolution_error(&selector.policy, Err(Error::new(ERR_NO_MATCHES))).map(|()| Default::default())
    }

    /// Resolves the request; see [`Self::resolve`] for the shape of an error.
    pub async fn resolve_multiple<T: Managed>(
        &self,
        mut req: MultiNamespacedResolutionRequest<T>,
    ) -> Result<MultiNamespacedResolutionResponse> {
        if meta::was_deleted(self.from) || req.is_no_op() {
            return Ok(MultiNamespacedResolutionResponse {
                resolved_values: req.current_values,
                resolved_references: req.references,
            });
        }

        if !req.references.is_empty() {
            let mut resolved_values = Vec::with_capacity(req.references.len());
            for reference in &req.references {
                match self.get::<T>(reference).await {
                    Ok(to) => resolved_values.push((req.extract)(&to)),
                    Err(e) if e.is_not_found() => {
                        return get_resolution_error(&reference.policy, Err(e.wrap(ERR_GET_MANAGED)))
                            .map(|()| Default::default());
                    }
                    Err(e) => return Err(e.wrap(ERR_GET_MANAGED)),
                }
            }
            let rsp = MultiNamespacedResolutionResponse {
                resolved_values,
                resolved_references: req.references,
            };
            return rsp.validate().map(|()| rsp);
        }

        let selector = req
            .selector
            .expect("a request that is not a no-op has references or a selector");
        let (items, ns) = self.select::<T>(&selector).await?;
        let by_value: BTreeMap<String, NamespacedReference> = items
            .iter()
            .map(|to| {
                (
                    (req.extract)(to),
                    NamespacedReference {
                        name: to.meta().name.clone().unwrap_or_default(),
                        namespace: ns.clone(),
                        policy: None,
                    },
                )
            })
            .collect();
        let (resolved_values, resolved_references) = by_value.into_iter().unzip();
        let rsp = MultiNamespacedResolutionResponse {
            resolved_values,
            resolved_references,
        };
        get_resolution_error(&selector.policy, rsp.validate()).map(|()| rsp)
    }
}

#[cfg(test)]
#[path = "reference_test.rs"]
mod tests;

//! Derives that replace angryjet's generated method sets.
//!
//! Each sits next to kube's `#[derive(CustomResource)]` on the spec struct, reads `kind`, `root`
//! and `status` from the `#[kube(...)]` attribute, finds the embedded core/v2 struct by type (as
//! angryjet matches embedded structs), and implements the trait in `crossplane_core::resource`
//! for the generated root struct. Re-exported at the root of `crossplane_core`, which generated
//! code names as `::crossplane_core`.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{Data, DeriveInput, Expr, Field, Ident, LitStr, Type, parse_macro_input};

/// `impl resource::Managed for <Kind>`. Requires `#[kube(status = "S")]` with
/// `S: AsRef<ManagedResourceStatus> + AsMut<..> + Default`, and one `#[serde(flatten)]` field of
/// type `ManagedResourceSpec`.
///
/// `#[crossplane(track_reconcile_requests)]` opts the kind into recording
/// `status.lastHandledReconcileAt`. Off by default, matching the Go baseline `MyType`, whose
/// named `Status` field hides `ObservedStatus`'s methods.
///
/// One `#[crossplane(reference(...))]` per cross-resource reference also generates
/// `Managed::resolve_references`, as angryjet does from `+crossplane:generate:reference`
/// markers. The value field is named by a dotted path of plain struct fields from the spec, so
/// the marker's place in the Go struct becomes an attribute on the spec:
///
/// ```text
/// #[crossplane(reference(path = "for_provider.vpc_id", to = Vpc))]
/// #[crossplane(reference(path = "for_provider.subnet_ids", to = Subnet, list))]
/// ```
///
/// `to` is the referenced kind, which stands for Go's `To{Managed, List}`. `list` selects the
/// `ResolveMultiple` form. The reference and selector fields sit beside the value field and
/// default to `<field>_ref` (`<field>_refs` for a list) and `<field>_selector`; `ref_field`
/// and `selector_field` rename them, as the `refFieldName` and `selectorFieldName` markers do.
/// `extractor = "expr"` names an `ExtractValueFn<To>` expression and defaults to the external
/// name, as the `extractor` marker does.
///
/// The value field is a `String`, or an `Option` of `String`, `f64` or `i64` (Go's pointers); a
/// list is a `Vec` of the same. A reference field is `Option<NamespacedReference>` (a list:
/// `Vec<NamespacedReference>`) and a selector field `Option<NamespacedSelector>`. Parents that are
/// optional or lists of structs, which angryjet walks, are not supported; such a kind overrides
/// `resolve_references` by hand. Generated code names `::crossplane_runtime`, so a kind with a
/// reference needs that crate as a dependency.
#[proc_macro_derive(Managed, attributes(crossplane))]
pub fn derive_managed(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input, Target::Managed)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// `impl resource::ProviderConfig for <Kind>`. Requires a status type with
/// `AsRef<ProviderConfigStatus> + AsMut<..> + Default`.
#[proc_macro_derive(ProviderConfig, attributes(crossplane))]
pub fn derive_provider_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input, Target::ProviderConfig)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// `impl resource::ProviderConfigUsage for <Kind>`, on a `#[kube(no_spec)]` struct whose only
/// field is a `#[serde(flatten)]` `TypedProviderConfigUsage`.
#[proc_macro_derive(ProviderConfigUsage, attributes(crossplane))]
pub fn derive_provider_config_usage(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input, Target::Usage)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// `AsRef<T>` and `AsMut<T>` for every `#[serde(flatten)]` field: the Rust spelling of a Go
/// `json:",inline"` embed, used on status structs.
#[proc_macro_derive(Inline)]
pub fn derive_inline(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_inline(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[derive(Clone, Copy)]
enum Target {
    Managed,
    ProviderConfig,
    Usage,
}

struct KubeAttrs {
    root: Ident,
    status: Option<Type>,
    namespaced: bool,
}

fn kube_attrs(input: &DeriveInput) -> syn::Result<KubeAttrs> {
    let mut kind = None;
    let mut root = None;
    let mut status = None;
    let mut namespaced = false;
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("kube")) {
        attr.parse_nested_meta(|meta| {
            let is = |n: &str| meta.path.is_ident(n);
            if is("kind") || is("root") || is("status") {
                let v: LitStr = meta.value()?.parse()?;
                if is("kind") {
                    kind = Some(v.value());
                } else if is("root") {
                    root = Some(v.value());
                } else {
                    status = Some(v.parse::<Type>()?);
                }
            } else if is("namespaced") {
                namespaced = true;
            } else if meta.input.peek(syn::Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            } else if meta.input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in meta.input);
                let _: TokenStream2 = content.parse()?;
            }
            Ok(())
        })?;
    }
    let root = root.or(kind).ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "expected #[kube(kind = \"...\")] next to #[derive(CustomResource)]: the crossplane derives go on the spec struct of a kube custom resource",
        )
    })?;
    Ok(KubeAttrs {
        root: Ident::new(&root, Span::call_site()),
        status,
        namespaced,
    })
}

/// One `#[crossplane(reference(...))]`.
struct Reference {
    path: Vec<Ident>,
    to: Type,
    list: bool,
    ref_field: Ident,
    selector_field: Ident,
    extractor: Option<Expr>,
}

#[derive(Default)]
struct CrossplaneAttrs {
    track_reconcile_requests: bool,
    references: Vec<Reference>,
}

fn parse_reference(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<Reference> {
    let mut path = None;
    let mut to = None;
    let mut list = false;
    let mut ref_field = None;
    let mut selector_field = None;
    let mut extractor = None;
    meta.parse_nested_meta(|m| {
        let is = |n: &str| m.path.is_ident(n);
        if is("path") {
            let lit: LitStr = m.value()?.parse()?;
            let idents = lit
                .value()
                .split('.')
                .map(|segment| field_ident(segment, lit.span()))
                .collect::<syn::Result<Vec<_>>>()?;
            path = Some(idents);
        } else if is("to") {
            to = Some(m.value()?.parse::<Type>()?);
        } else if is("list") {
            list = true;
        } else if is("ref_field") || is("selector_field") {
            let lit: LitStr = m.value()?.parse()?;
            let ident = lit.parse::<Ident>()?;
            if is("ref_field") {
                ref_field = Some(ident);
            } else {
                selector_field = Some(ident);
            }
        } else if is("extractor") {
            let lit: LitStr = m.value()?.parse()?;
            extractor = Some(lit.parse::<Expr>()?);
        } else {
            return Err(m.error(
                "unknown reference option, expected one of path, to, list, ref_field, selector_field, extractor",
            ));
        }
        Ok(())
    })?;
    let path = path.ok_or_else(|| {
        meta.error("reference needs path = \"<field>.<field>...\", the dotted path to the value field")
    })?;
    let to = to.ok_or_else(|| meta.error("reference needs to = <Kind>, the referenced managed resource"))?;
    let last = path.last().expect("split yields one segment").to_string();
    let named = |suffix: &str| Ident::new(&format!("{last}{suffix}"), Span::call_site());
    Ok(Reference {
        ref_field: ref_field.unwrap_or_else(|| named(if list { "_refs" } else { "_ref" })),
        selector_field: selector_field.unwrap_or_else(|| named("_selector")),
        path,
        to,
        list,
        extractor,
    })
}

fn field_ident(segment: &str, span: Span) -> syn::Result<Ident> {
    let mut ident = syn::parse_str::<Ident>(segment)
        .map_err(|_| syn::Error::new(span, format!("`{segment}` in a reference path is not a field name")))?;
    ident.set_span(span);
    Ok(ident)
}

fn crossplane_attrs(input: &DeriveInput, target: Target) -> syn::Result<CrossplaneAttrs> {
    let mut attrs = CrossplaneAttrs::default();
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("crossplane")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("track_reconcile_requests") && matches!(target, Target::Managed) {
                attrs.track_reconcile_requests = true;
            } else if meta.path.is_ident("reference") && matches!(target, Target::Managed) {
                attrs.references.push(parse_reference(&meta)?);
            } else {
                return Err(meta.error(match target {
                    Target::Managed => {
                        "unknown crossplane attribute, expected track_reconcile_requests or reference(...)"
                    }
                    _ => "this derive takes no #[crossplane(...)] attributes",
                }));
            }
            Ok(())
        })?;
    }
    Ok(attrs)
}

fn fields(input: &DeriveInput) -> syn::Result<Vec<&Field>> {
    match &input.data {
        Data::Struct(s) => Ok(s.fields.iter().collect()),
        _ => Err(syn::Error::new_spanned(&input.ident, "expected a struct")),
    }
}

fn is_flattened(f: &Field) -> bool {
    f.attrs.iter().filter(|a| a.path().is_ident("serde")).any(|a| {
        let mut flat = false;
        let _ = a.parse_nested_meta(|meta| {
            if meta.path.is_ident("flatten") {
                flat = true;
            } else if meta.input.peek(syn::Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            }
            Ok(())
        });
        flat
    })
}

fn type_ends_with(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == name))
}

fn embedded_field<'a>(input: &'a DeriveInput, derive: &str, type_name: &str) -> syn::Result<&'a Ident> {
    let candidates: Vec<&Field> = fields(input)?
        .into_iter()
        .filter(|f| f.ident.is_some() && type_ends_with(&f.ty, type_name))
        .collect();
    match candidates.iter().find(|f| is_flattened(f)) {
        Some(f) => Ok(f.ident.as_ref().expect("filtered on named")),
        None => Err(syn::Error::new_spanned(
            &input.ident,
            match candidates.first() {
                Some(f) => format!(
                    "{derive} needs the `{type_name}` field `{}` to be `#[serde(flatten)]`, as Go inlines the embedded struct",
                    f.ident.as_ref().expect("filtered on named")
                ),
                None => format!(
                    "{derive} needs a named `#[serde(flatten)]` field of type `{type_name}` (not wrapped in Option or a reference)"
                ),
            },
        )),
    }
}

fn expand(input: &DeriveInput, target: Target) -> syn::Result<TokenStream2> {
    let KubeAttrs {
        root,
        status,
        namespaced,
    } = kube_attrs(input)?;
    let attrs = crossplane_attrs(input, target)?;
    let rt = quote!(::crossplane_core);
    match target {
        Target::Managed => {
            if !namespaced {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "Managed needs #[kube(namespaced)]: only namespaced managed resources are supported",
                ));
            }
            let field = embedded_field(input, "Managed", "ManagedResourceSpec")?;
            let status = status.ok_or_else(|| {
                syn::Error::new_spanned(
                    &input.ident,
                    "Managed needs #[kube(status = \"...\")]: the status type embeds ManagedResourceStatus",
                )
            })?;
            let track = attrs.track_reconcile_requests;
            let resolve = resolve_references(&attrs.references);
            Ok(quote! {
                impl #rt::resource::Managed for #root {
                    const TRACKS_RECONCILE_REQUESTS: bool = #track;
                    fn managed_spec(&self) -> &#rt::v2::ManagedResourceSpec {
                        &self.spec.#field
                    }
                    fn managed_spec_mut(&mut self) -> &mut #rt::v2::ManagedResourceSpec {
                        &mut self.spec.#field
                    }
                    fn managed_status(&self) -> ::core::option::Option<&#rt::v2::ManagedResourceStatus> {
                        self.status.as_ref().map(<#status as ::core::convert::AsRef<#rt::v2::ManagedResourceStatus>>::as_ref)
                    }
                    fn managed_status_mut(&mut self) -> &mut #rt::v2::ManagedResourceStatus {
                        <#status as ::core::convert::AsMut<#rt::v2::ManagedResourceStatus>>::as_mut(
                            self.status.get_or_insert_with(::core::default::Default::default),
                        )
                    }
                    #resolve
                }
            })
        }
        Target::ProviderConfig => {
            let status = status.ok_or_else(|| {
                syn::Error::new_spanned(
                    &input.ident,
                    "ProviderConfig needs #[kube(status = \"...\")]: the status type embeds ProviderConfigStatus",
                )
            })?;
            Ok(quote! {
                impl #rt::resource::ProviderConfig for #root {
                    fn provider_config_status(&self) -> ::core::option::Option<&#rt::v2::ProviderConfigStatus> {
                        self.status.as_ref().map(<#status as ::core::convert::AsRef<#rt::v2::ProviderConfigStatus>>::as_ref)
                    }
                    fn provider_config_status_mut(&mut self) -> &mut #rt::v2::ProviderConfigStatus {
                        <#status as ::core::convert::AsMut<#rt::v2::ProviderConfigStatus>>::as_mut(
                            self.status.get_or_insert_with(::core::default::Default::default),
                        )
                    }
                }
            })
        }
        Target::Usage => {
            let field = embedded_field(input, "ProviderConfigUsage", "TypedProviderConfigUsage")?;
            if fields(input)?.len() != 1 {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "ProviderConfigUsage expects the flattened `TypedProviderConfigUsage` to be the only field",
                ));
            }
            let fields_struct = &input.ident;
            Ok(quote! {
                impl #rt::resource::ProviderConfigUsage for #root {
                    fn from_usage(
                        metadata: ::k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
                        usage: #rt::v2::TypedProviderConfigUsage,
                    ) -> Self {
                        let mut root = Self::new("", #fields_struct { #field: usage });
                        root.metadata = metadata;
                        root
                    }
                    fn usage(&self) -> &#rt::v2::TypedProviderConfigUsage {
                        &self.spec.#field
                    }
                    fn usage_mut(&mut self) -> &mut #rt::v2::TypedProviderConfigUsage {
                        &mut self.spec.#field
                    }
                }
            })
        }
    }
}

/// The body of angryjet's `ResolveReferences`: one `Resolve` or `ResolveMultiple` per reference,
/// in declaration order, stopping at the first error, which is wrapped with the value field's
/// path.
fn resolve_references(references: &[Reference]) -> TokenStream2 {
    if references.is_empty() {
        return quote!();
    }
    let reference = quote!(::crossplane_runtime::reference);
    let calls = references.iter().map(|r| {
        let Reference {
            path,
            to,
            list,
            ref_field,
            selector_field,
            extractor,
        } = r;
        let parent = &path[..path.len() - 1];
        let value = quote!(self.spec #(.#path)*);
        let refs = quote!(self.spec #(.#parent)* .#ref_field);
        let selector = quote!(self.spec #(.#parent)* .#selector_field);
        let extract = extractor
            .as_ref()
            .map_or_else(|| quote!(#reference::external_name::<#to>()), |e| quote!(#e));
        let wrap = format!(
            "spec.{}",
            path.iter().map(Ident::to_string).collect::<Vec<_>>().join(".")
        );
        if *list {
            quote! {
                let rsp = #reference::ApiNamespacedResolver::new(client, &*self)
                    .resolve_multiple::<#to>(#reference::MultiNamespacedResolutionRequest {
                        current_values: #reference::ReferenceValues::current_values(&#value),
                        references: ::core::clone::Clone::clone(&#refs),
                        selector: ::core::clone::Clone::clone(&#selector),
                        extract: #extract,
                    })
                    .await
                    .map_err(|e| e.wrap(#wrap))?;
                #reference::ReferenceValues::set_resolved(&mut #value, rsp.resolved_values);
                #refs = rsp.resolved_references;
            }
        } else {
            quote! {
                let rsp = #reference::ApiNamespacedResolver::new(client, &*self)
                    .resolve::<#to>(#reference::NamespacedResolutionRequest {
                        current_value: #reference::ReferenceValue::current_value(&#value),
                        reference: ::core::clone::Clone::clone(&#refs),
                        selector: ::core::clone::Clone::clone(&#selector),
                        extract: #extract,
                    })
                    .await
                    .map_err(|e| e.wrap(#wrap))?;
                #reference::ReferenceValue::set_resolved(&mut #value, rsp.resolved_value);
                #refs = rsp.resolved_reference;
            }
        }
    });
    quote! {
        fn resolve_references<'a>(
            &'a mut self,
            client: &'a dyn ::crossplane_core::client::Client,
        ) -> ::core::option::Option<
            ::core::pin::Pin<
                ::std::boxed::Box<
                    dyn ::core::future::Future<Output = ::crossplane_core::errors::Result<()>> + ::core::marker::Send + 'a,
                >,
            >,
        > {
            ::core::option::Option::Some(::std::boxed::Box::pin(async move {
                #(#calls)*
                ::core::result::Result::Ok(())
            }))
        }
    }
}

fn expand_inline(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let impls = fields(input)?.into_iter().filter(|f| is_flattened(f)).map(|f| {
        let ident = f.ident.as_ref().expect("named field");
        let ty = &f.ty;
        quote! {
            impl ::core::convert::AsRef<#ty> for #name {
                fn as_ref(&self) -> &#ty { &self.#ident }
            }
            impl ::core::convert::AsMut<#ty> for #name {
                fn as_mut(&mut self) -> &mut #ty { &mut self.#ident }
            }
        }
    });
    Ok(quote!(#(#impls)*))
}

/// Misuse that only rustc can see, because the derive checks the shape of the struct and not the
/// types behind its paths. Each example fails with the named error code.
///
/// A status type that does not expose the embedded `ManagedResourceStatus`:
///
/// ```compile_fail,E0277
/// use crossplane_core::v2::ManagedResourceSpec;
/// use crossplane_core::Managed;
/// use kube::CustomResource;
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
/// struct Status {
///     phase: String,
/// }
///
/// #[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
/// #[kube(group = "x.example.com", version = "v1", kind = "W", namespaced, status = "Status")]
/// struct WSpec {
///     #[serde(flatten)]
///     managed: ManagedResourceSpec,
/// }
/// ```
///
/// A provider config whose status does not embed `ProviderConfigStatus`:
///
/// ```compile_fail,E0277
/// use crossplane_core::ProviderConfig;
/// use kube::CustomResource;
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
/// struct Status {}
///
/// #[derive(CustomResource, ProviderConfig, Clone, Debug, Serialize, Deserialize, JsonSchema)]
/// #[kube(group = "x.example.com", version = "v1", kind = "C", namespaced, status = "Status")]
/// struct CSpec {
///     region: String,
/// }
/// ```
///
/// `Inline` only exposes `#[serde(flatten)]` fields, so a field that is not flattened is not an
/// `AsRef` target:
///
/// ```compile_fail,E0277
/// use crossplane_core::v2::ManagedResourceStatus;
/// use crossplane_core::Inline;
///
/// #[derive(Inline, Default)]
/// struct Status {
///     inner: ManagedResourceStatus,
/// }
///
/// fn needs<S: AsRef<ManagedResourceStatus>>(_: &S) {}
/// needs(&Status::default());
/// ```
///
/// A reference whose referenced kind is not a managed resource:
///
/// ```compile_fail,E0277
/// use crossplane_core::v2::{ManagedResourceSpec, ManagedResourceStatus};
/// use crossplane_core::{Inline, Managed};
/// use kube::CustomResource;
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Inline, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
/// struct Status {
///     #[serde(flatten)]
///     managed: ManagedResourceStatus,
/// }
///
/// #[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
/// struct Params {
///     target: Option<String>,
/// }
///
/// #[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
/// #[kube(group = "x.example.com", version = "v1", kind = "W", namespaced, status = "Status")]
/// #[crossplane(reference(path = "params.target", to = String))]
/// struct WSpec {
///     #[serde(flatten)]
///     managed: ManagedResourceSpec,
///     params: Params,
/// }
/// ```
#[cfg(doctest)]
mod compile_fail_doctests {}

#[cfg(test)]
#[path = "expand_test.rs"]
mod tests;

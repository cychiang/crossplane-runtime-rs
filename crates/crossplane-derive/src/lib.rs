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
use syn::{Data, DeriveInput, Field, Ident, LitStr, Type, parse_macro_input};

/// `impl resource::Managed for <Kind>`. Requires `#[kube(status = "S")]` with
/// `S: AsRef<ManagedResourceStatus> + AsMut<..> + Default`, and one `#[serde(flatten)]` field of
/// type `ManagedResourceSpec`.
///
/// `#[crossplane(track_reconcile_requests)]` opts the kind into recording
/// `status.lastHandledReconcileAt`. Off by default, matching the Go baseline `MyType`, whose
/// named `Status` field hides `ObservedStatus`'s methods.
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
}

fn kube_attrs(input: &DeriveInput) -> syn::Result<KubeAttrs> {
    let mut kind = None;
    let mut root = None;
    let mut status = None;
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
        syn::Error::new(
            Span::call_site(),
            "expected #[kube(kind = \"...\")] next to #[derive(CustomResource)]",
        )
    })?;
    Ok(KubeAttrs {
        root: Ident::new(&root, Span::call_site()),
        status,
    })
}

fn has_flag(input: &DeriveInput, flag: &str) -> syn::Result<bool> {
    let mut found = false;
    for attr in input.attrs.iter().filter(|a| a.path().is_ident("crossplane")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident(flag) {
                found = true;
                Ok(())
            } else {
                Err(meta.error("unknown crossplane attribute"))
            }
        })?;
    }
    Ok(found)
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

fn embedded_field<'a>(input: &'a DeriveInput, type_name: &str) -> syn::Result<&'a Ident> {
    fields(input)?
        .into_iter()
        .find(|f| is_flattened(f) && type_ends_with(&f.ty, type_name))
        .and_then(|f| f.ident.as_ref())
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &input.ident,
                format!("expected a named `#[serde(flatten)]` field of type `{type_name}`"),
            )
        })
}

fn expand(input: &DeriveInput, target: Target) -> syn::Result<TokenStream2> {
    let KubeAttrs { root, status } = kube_attrs(input)?;
    let rt = quote!(::crossplane_core);
    match target {
        Target::Managed => {
            let field = embedded_field(input, "ManagedResourceSpec")?;
            let status = status
                .ok_or_else(|| syn::Error::new_spanned(&input.ident, "Managed requires #[kube(status = \"...\")]"))?;
            let track = has_flag(input, "track_reconcile_requests")?;
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
                }
            })
        }
        Target::ProviderConfig => {
            let status = status.ok_or_else(|| {
                syn::Error::new_spanned(&input.ident, "ProviderConfig requires #[kube(status = \"...\")]")
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
            let field = embedded_field(input, "TypedProviderConfigUsage")?;
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

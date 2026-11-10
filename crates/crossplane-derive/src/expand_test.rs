use syn::parse_quote;

use super::*;

fn managed_error(input: DeriveInput) -> String {
    expand(&input, Target::Managed)
        .expect_err("the derive should reject this input")
        .to_string()
}

fn error(input: DeriveInput, target: Target) -> String {
    expand(&input, target)
        .expect_err("the derive should reject this input")
        .to_string()
}

#[test]
fn a_struct_that_is_not_a_kube_spec_is_rejected() {
    let msg = managed_error(parse_quote! {
        struct NotASpec {
            #[serde(flatten)]
            managed: ManagedResourceSpec,
        }
    });
    assert!(
        msg.contains("expected #[kube(kind = \"...\")] next to #[derive(CustomResource)]"),
        "{msg}"
    );
    assert!(msg.contains("spec struct of a kube custom resource"), "{msg}");

    for target in [Target::ProviderConfig, Target::Usage] {
        let msg = error(
            parse_quote!(
                struct NotASpec {
                    x: u8,
                }
            ),
            target,
        );
        assert!(msg.contains("expected #[kube(kind"), "{msg}");
    }
}

#[test]
fn an_enum_is_rejected() {
    let msg = managed_error(parse_quote! {
        #[kube(kind = "E", namespaced, status = "S")]
        enum E { A }
    });
    assert_eq!("expected a struct", msg);
}

#[test]
fn a_missing_embedded_spec_is_rejected() {
    let msg = managed_error(parse_quote! {
        #[kube(kind = "W", namespaced, status = "S")]
        struct WSpec { size: i64 }
    });
    assert_eq!(
        "Managed needs a named `#[serde(flatten)]` field of type `ManagedResourceSpec` (not wrapped in Option or a reference)",
        msg
    );
}

#[test]
fn an_embedded_spec_of_the_wrong_type_is_rejected() {
    for ty in [
        "Option<ManagedResourceSpec>",
        "String",
        "Box<ManagedResourceSpec>",
        "Vec<ManagedResourceSpec>",
        "&'static ManagedResourceSpec",
    ] {
        let ty: Type = syn::parse_str(ty).unwrap();
        let msg = managed_error(parse_quote! {
            #[kube(kind = "W", namespaced, status = "S")]
            struct WSpec {
                #[serde(flatten)]
                managed: #ty,
            }
        });
        assert!(
            msg.contains("needs a named `#[serde(flatten)]` field of type `ManagedResourceSpec`"),
            "{msg}"
        );
    }
}

#[test]
fn an_embedded_spec_that_is_not_flattened_is_rejected_and_named() {
    let msg = managed_error(parse_quote! {
        #[kube(kind = "W", namespaced, status = "S")]
        struct WSpec { managed: ManagedResourceSpec }
    });
    assert_eq!(
        "Managed needs the `ManagedResourceSpec` field `managed` to be `#[serde(flatten)]`, as Go inlines the embedded struct",
        msg
    );
}

#[test]
fn a_tuple_struct_has_no_named_embedded_spec() {
    let msg = managed_error(parse_quote! {
        #[kube(kind = "W", namespaced, status = "S")]
        struct WSpec(#[serde(flatten)] ManagedResourceSpec);
    });
    assert!(msg.contains("named `#[serde(flatten)]` field"), "{msg}");
}

#[test]
fn a_missing_status_is_rejected() {
    let msg = managed_error(parse_quote! {
        #[kube(kind = "W", namespaced)]
        struct WSpec { #[serde(flatten)] managed: ManagedResourceSpec }
    });
    assert_eq!(
        "Managed needs #[kube(status = \"...\")]: the status type embeds ManagedResourceStatus",
        msg
    );
    let msg = error(
        parse_quote! {
            #[kube(kind = "C", namespaced)]
            struct CSpec { region: String }
        },
        Target::ProviderConfig,
    );
    assert_eq!(
        "ProviderConfig needs #[kube(status = \"...\")]: the status type embeds ProviderConfigStatus",
        msg
    );
}

#[test]
fn a_cluster_scoped_managed_resource_is_rejected() {
    let msg = managed_error(parse_quote! {
        #[kube(kind = "W", status = "S")]
        struct WSpec { #[serde(flatten)] managed: ManagedResourceSpec }
    });
    assert_eq!(
        "Managed needs #[kube(namespaced)]: only namespaced managed resources are supported",
        msg
    );
}

#[test]
fn a_usage_must_be_exactly_the_flattened_usage() {
    let msg = error(
        parse_quote! {
            #[kube(kind = "U", namespaced, no_spec)]
            struct UFields { other: String }
        },
        Target::Usage,
    );
    assert!(
        msg.contains("named `#[serde(flatten)]` field of type `TypedProviderConfigUsage`"),
        "{msg}"
    );
    let msg = error(
        parse_quote! {
            #[kube(kind = "U", namespaced, no_spec)]
            struct UFields {
                #[serde(flatten)]
                usage: TypedProviderConfigUsage,
                extra: String,
            }
        },
        Target::Usage,
    );
    assert!(msg.contains("to be the only field"), "{msg}");
}

#[test]
fn unknown_and_misplaced_crossplane_attributes_are_rejected() {
    let managed = |attr: syn::Attribute| -> DeriveInput {
        parse_quote! {
            #attr
            #[kube(kind = "W", namespaced, status = "S")]
            struct WSpec { #[serde(flatten)] managed: ManagedResourceSpec }
        }
    };
    let msg = managed_error(managed(parse_quote!(#[crossplane(track)])));
    assert!(
        msg.contains("unknown crossplane attribute, expected track_reconcile_requests or reference(...)"),
        "{msg}"
    );

    let msg = error(
        parse_quote! {
            #[crossplane(reference(path = "a", to = X))]
            #[kube(kind = "C", namespaced, status = "S")]
            struct CSpec { region: String }
        },
        Target::ProviderConfig,
    );
    assert!(
        msg.contains("this derive takes no #[crossplane(...)] attributes"),
        "{msg}"
    );
}

#[test]
fn a_malformed_reference_is_rejected_with_what_is_missing() {
    let with = |attr: syn::Attribute| -> String {
        managed_error(parse_quote! {
            #attr
            #[kube(kind = "W", namespaced, status = "S")]
            struct WSpec { #[serde(flatten)] managed: ManagedResourceSpec }
        })
    };
    assert!(with(parse_quote!(#[crossplane(reference(to = Vpc))])).contains("needs path ="));
    assert!(with(parse_quote!(#[crossplane(reference(path = "a.b"))])).contains("needs to = <Kind>"));
    assert!(with(parse_quote!(#[crossplane(reference(path = "a..b", to = Vpc))])).contains("not a field name"));
    assert!(
        with(parse_quote!(#[crossplane(reference(path = "a", to = Vpc, many))])).contains("unknown reference option")
    );
}

#[test]
fn a_well_formed_managed_derive_expands() {
    let out = expand(
        &parse_quote! {
            #[kube(kind = "W", namespaced, status = "S")]
            #[crossplane(reference(path = "for_provider.vpc_id", to = Vpc))]
            struct WSpec { #[serde(flatten)] managed: ManagedResourceSpec }
        },
        Target::Managed,
    )
    .expect("valid input expands")
    .to_string();
    assert!(out.contains("resolve_references"), "{out}");
}

#[test]
fn inline_rejects_an_enum() {
    let err = expand_inline(&parse_quote!(
        enum E {
            A,
        }
    ))
    .expect_err("an enum has no fields to inline");
    assert_eq!("expected a struct", err.to_string());
}

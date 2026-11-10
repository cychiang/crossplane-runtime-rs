use crossplane_core::resource::ProviderConfigUsage as _;
use crossplane_core::v2::ProviderConfigReference;
use crossplane_test::fake::ProviderConfigUsage;

use super::*;

fn usage(namespace: &str, kind: &str, name: &str) -> ProviderConfigUsage {
    let mut u = ProviderConfigUsage::default();
    u.metadata.name = Some("some-pcu".into());
    u.metadata.namespace = Some(namespace.into());
    u.set_provider_config_reference(ProviderConfigReference {
        name: name.into(),
        kind: kind.into(),
    });
    u
}

#[test]
fn add_provider_config() {
    struct Case {
        name: &'static str,
        handler: EnqueueRequestForProviderConfig,
        obj: ProviderConfigUsage,
        want: Vec<Request>,
    }
    let name = "coolname";
    let cases = vec![
        // go: pkg/resource TestAddProviderConfig/IsTypedProviderConfigReferencer
        Case {
            name: "IsTypedProviderConfigReferencer",
            handler: EnqueueRequestForProviderConfig::new(""),
            obj: usage("foo", "ProviderConfig", name),
            want: vec![Request::namespaced("foo", name)],
        },
        // go: pkg/resource TestAddProviderConfig/ClusterScopedProviderConfigOmitsNamespace
        Case {
            name: "ClusterScopedProviderConfigOmitsNamespace",
            handler: EnqueueRequestForProviderConfig::new("ClusterProviderConfig"),
            obj: usage("foo", "ClusterProviderConfig", name),
            want: vec![Request::cluster(name)],
        },
        // go: pkg/resource TestAddProviderConfig/KindFilterMatchesKind
        Case {
            name: "KindFilterMatchesKind",
            handler: EnqueueRequestForProviderConfig::new("ProviderConfig"),
            obj: usage("bar", "ProviderConfig", name),
            want: vec![Request::namespaced("bar", name)],
        },
        // go: pkg/resource TestAddProviderConfig/KindFilterSkipsNonMatchingKind
        Case {
            name: "KindFilterSkipsNonMatchingKind",
            handler: EnqueueRequestForProviderConfig::new("ProviderConfig"),
            obj: usage("bar", "OtherProviderConfig", name),
            want: vec![],
        },
        // go: pkg/resource TestAddProviderConfig/EmptyRefKindDefaultsToProviderConfig
        Case {
            name: "EmptyRefKindDefaultsToProviderConfig",
            handler: EnqueueRequestForProviderConfig::new("ProviderConfig"),
            obj: usage("baz", "", name),
            want: vec![Request::namespaced("baz", name)],
        },
    ];
    for tc in cases {
        let got = tc.handler.requests(&WatchEvent::Create(&tc.obj));
        assert_eq!(tc.want, got, "{}", tc.name);
    }
}

#[test]
fn update_enqueues_for_old_and_new_and_delete_for_the_object() {
    let h = EnqueueRequestForProviderConfig::new("");
    let (old, new) = (usage("ns", "ProviderConfig", "a"), usage("ns", "ProviderConfig", "b"));
    assert_eq!(
        vec![Request::namespaced("ns", "a"), Request::namespaced("ns", "b")],
        h.requests(&WatchEvent::Update { old: &old, new: &new })
    );
    assert_eq!(
        vec![Request::namespaced("ns", "a")],
        h.requests(&WatchEvent::Delete(&old))
    );
}

use super::*;

#[test]
fn reference_status_type_string() {
    struct Case {
        name: &'static str,
        t: ReferenceStatusType,
        want: &'static str,
    }
    let cases = vec![
        // go: pkg/resource TestReferenceStatusType_String/ReferenceStatusUnknown
        Case {
            name: "ReferenceStatusUnknown",
            t: ReferenceStatusType::Unknown,
            want: "Unknown",
        },
        // go: pkg/resource TestReferenceStatusType_String/ReferenceNotFound
        Case {
            name: "ReferenceNotFound",
            t: ReferenceStatusType::NotFound,
            want: "NotFound",
        },
        // go: pkg/resource TestReferenceStatusType_String/ReferenceNotReady
        Case {
            name: "ReferenceNotReady",
            t: ReferenceStatusType::NotReady,
            want: "NotReady",
        },
        // go: pkg/resource TestReferenceStatusType_String/ReferenceReady
        Case {
            name: "ReferenceReady",
            t: ReferenceStatusType::Ready,
            want: "Ready",
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, tc.t.to_string(), "{}", tc.name);
    }
}

#[test]
fn reference_status_string() {
    struct Case {
        name: &'static str,
        rs: ReferenceStatus,
        want: String,
    }
    let cases = vec![
        // go: pkg/resource TestReferenceStatus_String/test-name-ready
        Case {
            name: "test-name-ready",
            rs: ReferenceStatus {
                name: "test-name".into(),
                status: ReferenceStatusType::Ready,
            },
            want: format!("{{reference:test-name status:{}}}", ReferenceStatusType::Ready),
        },
    ];
    for tc in cases {
        assert_eq!(tc.want, tc.rs.to_string(), "{}", tc.name);
    }
}

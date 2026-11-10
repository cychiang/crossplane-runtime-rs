use crossplane_core::errors::Error;
use crossplane_core::resource::ObjectType;
use crossplane_core::v2::{EnvSelector, FsSelector, SecretKeySelector, SecretReference};
use crossplane_test::{self as test, MockClient};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;

use super::*;

const CREDENTIALS: &[u8] = b"supersecretcreds";

struct Case {
    name: &'static str,
    reason: &'static str,
    selectors: CommonCredentialSelectors,
    want: Result<Vec<u8>, Error>,
}

fn secret_selector() -> CommonCredentialSelectors {
    CommonCredentialSelectors {
        secret_ref: Some(SecretKeySelector {
            secret: SecretReference {
                name: "super".into(),
                namespace: "secret".into(),
            },
            key: "creds".into(),
        }),
        ..Default::default()
    }
}

#[test]
fn extract_env_table() {
    let cases = vec![
        // go: pkg/resource TestExtractEnv/EnvVarSuccess
        Case {
            name: "EnvVarSuccess",
            reason: "Successful extraction of credentials from environment variable",
            selectors: CommonCredentialSelectors {
                env: Some(EnvSelector {
                    name: "SECRET_CREDS".into(),
                }),
                ..Default::default()
            },
            want: Ok(CREDENTIALS.to_vec()),
        },
        // go: pkg/resource TestExtractEnv/EnvVarFail
        Case {
            name: "EnvVarFail",
            reason: "Failed extraction of credentials from environment variable",
            selectors: CommonCredentialSelectors::default(),
            want: Err(Error::new(ERR_EXTRACT_ENV)),
        },
    ];
    for tc in cases {
        let got = extract_env(|_| String::from_utf8(CREDENTIALS.to_vec()).unwrap(), &tc.selectors);
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

#[test]
fn extract_fs_table() {
    let path = std::env::temp_dir().join(format!("xprs-credentials-{}.txt", std::process::id()));
    std::fs::write(&path, CREDENTIALS).unwrap();
    let cases = vec![
        // go: pkg/resource TestExtractFs/FsSuccess
        Case {
            name: "FsSuccess",
            reason: "Successful extraction of credentials from filesystem",
            selectors: CommonCredentialSelectors {
                fs: Some(FsSelector {
                    path: path.to_str().unwrap().to_owned(),
                }),
                ..Default::default()
            },
            want: Ok(CREDENTIALS.to_vec()),
        },
        // go: pkg/resource TestExtractFs/FsFailure
        Case {
            name: "FsFailure",
            reason: "Failed extraction of credentials from filesystem",
            selectors: CommonCredentialSelectors::default(),
            want: Err(Error::new(ERR_EXTRACT_FS)),
        },
    ];
    for tc in cases {
        test::assert_result(tc.name, tc.reason, &tc.want, &extract_fs(&tc.selectors));
    }
    std::fs::remove_file(&path).unwrap();
}

#[tokio::test]
async fn extract_secret_table() {
    let with_creds = || Secret {
        data: Some([("creds".to_owned(), ByteString(CREDENTIALS.to_vec()))].into()),
        ..Default::default()
    };
    struct Case {
        name: &'static str,
        reason: &'static str,
        selectors: CommonCredentialSelectors,
        client: MockClient,
        want: Result<Vec<u8>, Error>,
    }
    let cases = vec![
        // go: pkg/resource TestExtractSecret/SecretSuccess
        Case {
            name: "SecretSuccess",
            reason: "Successful extraction of credentials from Secret",
            selectors: secret_selector(),
            want: Ok(CREDENTIALS.to_vec()),
            client: MockClient {
                get: Some(Box::new(move |_, key| {
                    assert_eq!(key, &ObjectKey::namespaced("secret", "super"));
                    test::new_mock_get_fn(with_creds)(&ObjectType::of::<Secret>(), key)
                })),
                ..Default::default()
            },
        },
        // go: pkg/resource TestExtractSecret/SecretFailureNotDefined
        Case {
            name: "SecretFailureNotDefined",
            reason: "Failed extraction of credentials from Secret when key not defined",
            selectors: CommonCredentialSelectors::default(),
            want: Err(Error::new(ERR_EXTRACT_SECRET_KEY)),
            client: MockClient::default(),
        },
        // go: pkg/resource TestExtractSecret/SecretFailureGet
        Case {
            name: "SecretFailureGet",
            reason: "Failed extraction of credentials from Secret when client fails",
            selectors: secret_selector(),
            want: Err(Error::new("boom").wrap(ERR_GET_CREDENTIALS_SECRET)),
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(Error::new("boom"))),
                ..Default::default()
            },
        },
    ];
    for tc in cases {
        let got = extract_secret(&tc.client, &tc.selectors).await;
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

#[tokio::test]
async fn the_dispatcher_follows_go() {
    let client = MockClient::default();
    let none = CommonCredentialSelectors::default();
    let got = common_credential_extractor(&CredentialsSource::NONE, &client, &none).await;
    assert_eq!(got, Ok(vec![]), "None is empty");
    for source in [CredentialsSource::INJECTED_IDENTITY, CredentialsSource("Other".into())] {
        let got = common_credential_extractor(&source, &client, &none).await;
        let want = Err(Error::new(format!(
            "no extraction handler registered for source: {}",
            source.0
        )));
        assert_eq!(got, want);
    }
    let got = common_credential_extractor(&CredentialsSource::ENVIRONMENT, &client, &none).await;
    assert_eq!(got, Err(Error::new(ERR_EXTRACT_ENV)));
}

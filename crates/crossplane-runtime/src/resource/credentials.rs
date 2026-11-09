use crossplane_core::client::Client;
use crossplane_core::errors::Result;
use crossplane_core::v2::{CommonCredentialSelectors, CredentialsSource};

/// Go `resource.CommonCredentialExtractor`. Filesystem reads the local file system directly; Go
/// uses afero, and tests point this at a temp dir.
pub async fn common_credential_extractor(
    source: &CredentialsSource,
    client: &dyn Client,
    selectors: &CommonCredentialSelectors,
) -> Result<Vec<u8>> {
    let _ = (source, client, selectors);
    // TODO (pkg/resource/providerconfig.go:96): None/InjectedIdentity -> empty;
    // Secret -> Get Secret, data[key]; Environment -> env var; Filesystem -> read file;
    // anything else -> "no extraction handler registered for source: <s>".
    todo!()
}

use crossplane_core::client::Client;
use crossplane_core::errors::{Error, Result, ResultExt};
use crossplane_core::resource::ObjectKey;
use crossplane_core::v2::{CommonCredentialSelectors, CredentialsSource};
use k8s_openapi::api::core::v1::Secret;

pub(super) const ERR_EXTRACT_ENV: &str = "cannot extract from environment variable when none specified";
pub(super) const ERR_EXTRACT_FS: &str = "cannot extract from filesystem when no path specified";
pub(super) const ERR_EXTRACT_SECRET_KEY: &str = "cannot extract from secret key when none specified";
pub(super) const ERR_GET_CREDENTIALS_SECRET: &str = "cannot get credentials secret";

/// Go `resource.ExtractEnv`. `lookup` is Go's `EnvLookupFn`: an unset variable is an empty string.
pub fn extract_env(lookup: impl Fn(&str) -> String, selectors: &CommonCredentialSelectors) -> Result<Vec<u8>> {
    let env = selectors.env.as_ref().ok_or_else(|| Error::new(ERR_EXTRACT_ENV))?;
    Ok(lookup(&env.name).into_bytes())
}

/// Go `resource.ExtractFs`, over the local file system where Go takes an afero one.
pub fn extract_fs(selectors: &CommonCredentialSelectors) -> Result<Vec<u8>> {
    let fs = selectors.fs.as_ref().ok_or_else(|| Error::new(ERR_EXTRACT_FS))?;
    std::fs::read(&fs.path).map_err(Error::foreign)
}

/// Go `resource.ExtractSecret`. A missing key is an empty value, as a nil map lookup is in Go.
pub async fn extract_secret(client: &dyn Client, selectors: &CommonCredentialSelectors) -> Result<Vec<u8>> {
    let sel = selectors
        .secret_ref
        .as_ref()
        .ok_or_else(|| Error::new(ERR_EXTRACT_SECRET_KEY))?;
    let key = ObjectKey::namespaced(&sel.secret.namespace, &sel.secret.name);
    let secret = client
        .get_as::<Secret>(&key)
        .await
        .wrap_err(ERR_GET_CREDENTIALS_SECRET)?;
    Ok(secret
        .data
        .and_then(|mut d| d.remove(&sel.key))
        .map_or_else(Vec::new, |b| b.0))
}

/// Go `resource.CommonCredentialExtractor`. Only `None` yields nothing: `InjectedIdentity` has no
/// common extractor, so it and any unknown source are errors.
pub async fn common_credential_extractor(
    source: &CredentialsSource,
    client: &dyn Client,
    selectors: &CommonCredentialSelectors,
) -> Result<Vec<u8>> {
    match source.0.as_ref() {
        "Environment" => extract_env(|name| std::env::var(name).unwrap_or_default(), selectors),
        "Filesystem" => extract_fs(selectors),
        "Secret" => extract_secret(client, selectors).await,
        "None" => Ok(Vec::new()),
        other => Err(Error::new(format!(
            "no extraction handler registered for source: {other}"
        ))),
    }
}

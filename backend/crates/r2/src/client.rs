use aws_config::{BehaviorVersion, Region};
use aws_sdk_s3::{Client, config::Credentials};

/// Builds an S3 client pointed at R2 via an explicit endpoint URL (R2
/// dashboards give you the full `https://<account_id>.r2.cloudflarestorage.com`
/// or a custom S3 API endpoint directly — read it from `S3_ENDPOINT` rather
/// than reconstructing it from an account id, so custom/jurisdiction-specific
/// endpoints work without code changes).
pub async fn build_client(endpoint: &str, access_key_id: &str, secret_access_key: &str) -> Client {
    let credentials = Credentials::new(access_key_id, secret_access_key, None, None, "r2");

    let config = aws_config::defaults(BehaviorVersion::latest())
        .endpoint_url(endpoint)
        .credentials_provider(credentials)
        .region(Region::new("auto"))
        .load()
        .await;

    // R2 (and most non-AWS S3-compatible providers) need path-style
    // requests — virtual-hosted-style (bucket.endpoint/key) only reliably
    // resolves against AWS's own DNS.
    Client::from_conf(
        aws_sdk_s3::config::Builder::from(&config)
            .force_path_style(true)
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // No network I/O here — explicit credentials/region mean `.load()`
    // never needs to hit IMDS or any other provider, so this is safe to
    // run as a plain unit test.
    #[tokio::test]
    async fn builds_a_client_from_explicit_credentials_without_panicking() {
        let client = build_client(
            "https://example-account.r2.cloudflarestorage.com",
            "dummy-access-key-id",
            "dummy-secret-access-key",
        )
        .await;

        assert_eq!(client.config().region().map(|r| r.as_ref()), Some("auto"));
    }
}

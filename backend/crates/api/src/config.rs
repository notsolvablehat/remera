#[derive(Clone)]
pub struct AppConfig {
    pub port: u16,
    pub database_url: String,
    pub auth_secret: String,
    // better-auth uses the below to generate verification urls for signups.
    pub base_url: String,
    pub frontend_allow_origins: Vec<String>,
    // R2 (S3-compatible object storage) — see state.rs for the client
    // built from these. R2_API_KEY is intentionally not read here: it's
    // a Cloudflare account-level API token, not an S3 credential, and
    // isn't needed for the presigned-URL flow this config supports.
    pub s3_endpoint: String,
    pub r2_access_key_id: String,
    pub r2_secret_access_key: String,
    pub r2_bucket_name: String,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),

            database_url: std::env::var("DATABASE_URL").expect("DATABSE_URL is not set."),

            auth_secret: std::env::var("AUTH_SECRET").expect("AUTH_SECRET is not set."),

            base_url: std::env::var("BASE_URL").expect("BASE_URL is not set."),

            frontend_allow_origins: std::env::var("FRONTEND_ALLOW_ORIGINS")
                .unwrap_or_else(|_| "http://localhost:5173".to_string())
                .trim()
                .split(',')
                .map(|url| url.trim().to_string())
                .collect(),

            s3_endpoint: std::env::var("S3_ENDPOINT").expect("S3_ENDPOINT is not set."),

            r2_access_key_id: std::env::var("R2_ACCESS_KEY_ID")
                .expect("R2_ACCESS_KEY_ID is not set."),

            r2_secret_access_key: std::env::var("R2_SECRET_ACCESS_KEY")
                .expect("R2_SECRET_ACCESS_KEY is not set."),

            r2_bucket_name: std::env::var("R2_BUCKET_NAME").expect("R2_BUCKET_NAME is not set."),
        }
    }
}

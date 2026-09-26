use std::sync::Arc;

use better_auth::{
    AuthBuilder, AuthConfig, BetterAuth,
    adapters::SqlxAdapter,
    plugins::{EmailPasswordPlugin, SessionManagementPlugin},
};
use sqlx::PgPool;

use crate::{
    config::AppConfig, extractors::container_access::ContainerStatus, share_link::ShareLinkCodec,
};

// NOTE: this was originally going to be HookedDatabaseAdapter<SqlxAdapter>
// so AppAuthHooks (invite resolution on signup) could run via better-auth's
// hook mechanism. That doesn't compile on better-auth-core 0.10.0:
// HookedDatabaseAdapter implements every *Ops trait EXCEPT PasskeyOps, so it
// can never satisfy the blanket `impl<T: UserOps + ... + PasskeyOps> DatabaseAdapter for T`
// — this is a real gap in this crate version (confirmed against the crate's
// own source; upstream's `master` branch has since removed
// HookedDatabaseAdapter from hooks.rs entirely, i.e. it was reworked after
// 0.10.0). Until better-auth is upgraded past 0.10.0, invite resolution
// (AppAuthHooks) has to happen explicitly in our own signup flow instead of
// as a DatabaseHooks callback — see auth_hooks.rs.
pub type AppDb = SqlxAdapter;

#[derive(Clone)]
pub struct AppState {
    #[allow(dead_code)]
    pub config: AppConfig,
    pub db: PgPool,
    pub auth: Arc<BetterAuth<AppDb>>,
    pub cache: Arc<moka::future::Cache<(uuid::Uuid, String), domain::Role>>,
    pub container_status_cache: Arc<moka::future::Cache<uuid::Uuid, ContainerStatus>>,
    pub share_link_codec: Arc<ShareLinkCodec>,
    pub r2: r2::Client,
    pub r2_bucket: String,
}

impl AppState {
    pub async fn new(config: AppConfig) -> Self {
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .expect("Failed to connect to database.");

        let auth_config = AuthConfig::new(&config.auth_secret)
            .base_url(&config.base_url)
            .password_min_length(8);

        let auth = Arc::new(
            AuthBuilder::new(auth_config)
                .database(SqlxAdapter::from_pool(db.clone()))
                .plugin(EmailPasswordPlugin::new().enable_signup(true))
                .plugin(SessionManagementPlugin::new())
                .build()
                .await
                .expect("Failed to intialise AUTH"),
        );

        let cache = Arc::new(
            moka::future::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(std::time::Duration::from_secs(300))
                .build(),
        );

        let container_status_cache = Arc::new(
            moka::future::Cache::builder()
                .max_capacity(10_000)
                .time_to_live(std::time::Duration::from_secs(300))
                .build(),
        );

        let share_link_codec = Arc::new(ShareLinkCodec::new(&config.auth_secret));

        let r2 = r2::build_client(
            &config.s3_endpoint,
            &config.r2_access_key_id,
            &config.r2_secret_access_key,
        )
        .await;
        let r2_bucket = config.r2_bucket_name.clone();

        Self {
            config,
            auth,
            db,
            cache,
            container_status_cache,
            share_link_codec,
            r2,
            r2_bucket,
        }
    }
}

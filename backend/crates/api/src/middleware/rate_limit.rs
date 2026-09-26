use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use governor::middleware::StateInformationMiddleware;
use tower_governor::{
    GovernorLayer, governor::GovernorConfigBuilder, key_extractor::PeerIpKeyExtractor,
};

// Coarse, global-per-IP limit applied to every route (rate limiting runs
// before auth extraction, per docs/architecture/howisthebackendstructured-1.md
// §"Summary of the access-control shape" — unauthenticated abuse shouldn't
// even reach the DB). One flat limit for now; splitting into stricter
// per-route limits (e.g. tighter on /auth/sign-up) is future work, not
// done here.
const REPLENISH_PERIOD: Duration = Duration::from_secs(1);
const BURST_SIZE: u32 = 30;

/// Builds the rate-limiting layer and spawns the background task Governor's
/// own docs recommend: without periodically pruning stale per-IP entries,
/// the in-memory rate-limiter state grows forever (one entry per distinct
/// IP ever seen).
pub fn layer() -> GovernorLayer<PeerIpKeyExtractor, StateInformationMiddleware, Body> {
    let config = Arc::new(
        GovernorConfigBuilder::default()
            .period(REPLENISH_PERIOD)
            .burst_size(BURST_SIZE)
            .use_headers()
            .finish()
            .expect("invalid rate limiter configuration"),
    );

    let limiter = config.limiter().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            limiter.retain_recent();
        }
    });

    GovernorLayer::new(config)
}

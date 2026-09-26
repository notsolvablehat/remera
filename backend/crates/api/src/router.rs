use crate::{routes, state::AppState};
use axum::Router;
use better_auth::AxumIntegration;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(info(title = "remera-api-docs", version = "0.1.0"))]
struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    // .axum_router() returns Router<Arc<BetterAuth<AppDb>>> — it does NOT
    // resolve its own state. axum's `.nest()` requires the nested router's
    // state type to match the outer router's exactly, so we resolve it here
    // with a second `.with_state(...)` call before nesting (the standard
    // axum pattern for merging a sub-router built against its own state
    // into a router built against a different outer state).
    let auth_router = state
        .auth
        .clone()
        .axum_router()
        .with_state(state.auth.clone());

    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(routes::health::router())
        .merge(routes::me::router())
        .split_for_parts();

    let origins: Vec<_> = state
        .config
        .frontend_allow_origins
        .iter()
        .map(|o| o.parse().expect("invalid origin in FRONTEND_ALLOW_ORIGINS"))
        .collect();

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers(Any);

    router
        .nest("/auth", auth_router)
        .merge(SwaggerUi::new("/docs").url("/openapi.json", api))
        .layer(cors)
        .with_state(state)
}

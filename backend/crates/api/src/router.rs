use crate::{routes, state::AppState};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(info(title = "remera-api-docs", version = "0.1.0"))]
struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(routes::health::router())
        .split_for_parts();

    let cors = CorsLayer::new()
        .allow_origin(
            "http://localhost:5173"
                .parse::<axum::http::HeaderValue>()
                .unwrap(),
        )
        .allow_methods(Any)
        .allow_headers(Any);

    router
        .merge(SwaggerUi::new("/docs").url("/openapi.json", api))
        .layer(cors)
        .with_state(state)
}

use axum::{
    Json, Router,
    http::{StatusCode, header},
};
use serde_json::json;
use tower::ServiceBuilder;
use vercel_runtime::axum::VercelLayer;

fn configuration_response(status: StatusCode) -> impl axum::response::IntoResponse {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({
            "service": "pulse-api",
            "status": "configuration_required",
            "configured": false,
            "health": "/healthz",
        })),
    )
}

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    let router = match pulse_api::router() {
        Ok(router) => router,
        Err(error) => {
            eprintln!("Pulse API initialization failed: {error}");
            Router::new()
                .route(
                    "/",
                    axum::routing::get(|| async { configuration_response(StatusCode::OK) }),
                )
                .fallback(|| async { configuration_response(StatusCode::SERVICE_UNAVAILABLE) })
        }
    };
    let app = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(router);
    vercel_runtime::run(app).await
}

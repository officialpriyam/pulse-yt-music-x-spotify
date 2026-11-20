use tower::ServiceBuilder;
use vercel_runtime::axum::VercelLayer;

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    let app = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(pulse_api::router()?);
    vercel_runtime::run(app).await
}

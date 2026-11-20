#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    let address = std::env::var("PULSE_BIND").unwrap_or_else(|_| "127.0.0.1:4000".into());
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("Pulse API listening on {}", listener.local_addr()?);
    axum::serve(listener, pulse_api::router()?).await?;
    Ok(())
}

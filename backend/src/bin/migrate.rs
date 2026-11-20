use pulse_api::migration;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    if let Err(error) = run().await {
        if let Some(db) = error.downcast_ref::<deadpool_postgres::tokio_postgres::Error>() {
            // Report SQLSTATE without echoing a database URL, SQL parameters, or credentials.
            if let Some(code) = db.code() {
                eprintln!("Pulse database operation failed (SQLSTATE {})", code.code());
            } else {
                eprintln!("Pulse database operation failed");
            }
        } else {
            eprintln!("{error}");
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let command = std::env::args().nth(1).unwrap_or_default();
    if !matches!(command.as_str(), "apply" | "check" | "prune") {
        return Err("Usage: cargo run --bin migrate -- apply|check|prune".into());
    }

    let mut client = migration::connect().await?;
    match command.as_str() {
        "apply" => println!(
            "Applied {} migration(s)",
            migration::apply(&mut client).await?
        ),
        "check" => {
            migration::check(&client).await?;
            println!("Pulse migrations are current");
        }
        "prune" => println!(
            "Removed {} expired row(s)",
            migration::prune(&client).await?
        ),
        _ => unreachable!("command validated"),
    }
    Ok(())
}

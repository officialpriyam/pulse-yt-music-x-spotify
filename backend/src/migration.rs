use std::time::Duration;

use deadpool_postgres::tokio_postgres::{Client, GenericClient};
use sha2::{Digest, Sha256};

use crate::AnyError;

const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../migrations/0001_initial.sql")),
    (2, include_str!("../migrations/0002_prune_expired.sql")),
];

fn checksum(sql: &str) -> Vec<u8> {
    Sha256::digest(sql.replace("\r\n", "\n").as_bytes()).to_vec()
}

async fn applied(client: &impl GenericClient) -> Result<usize, AnyError> {
    let rows = client
        .query(
            "SELECT version, checksum FROM pulse_schema_migrations ORDER BY version",
            &[],
        )
        .await?;
    for (index, row) in rows.iter().enumerate() {
        let version: i32 = row.get(0);
        let saved: Vec<u8> = row.get(1);
        let Some((expected, sql)) = MIGRATIONS.get(index) else {
            return Err("Database contains migrations newer than this build".into());
        };
        if *expected != version || saved != checksum(sql) {
            return Err(format!("Migration history mismatch at version {version}").into());
        }
    }
    Ok(rows.len())
}

pub async fn check(client: &Client) -> Result<(), AnyError> {
    if applied(client).await? != MIGRATIONS.len() {
        return Err("Database migrations are pending".into());
    }
    Ok(())
}

pub const LATEST: i32 = MIGRATIONS[MIGRATIONS.len() - 1].0;

pub async fn apply(client: &mut Client) -> Result<usize, AnyError> {
    let tx = client.transaction().await?;
    tx.batch_execute(
        "SET LOCAL lock_timeout = '10s';
         SET LOCAL statement_timeout = '60s';
         SELECT pg_advisory_xact_lock(182673013, 1415540077);
         CREATE TABLE IF NOT EXISTS pulse_schema_migrations (
             version integer PRIMARY KEY,
             checksum bytea NOT NULL,
             applied_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .await?;
    let installed = applied(&tx).await?;
    for (version, sql) in &MIGRATIONS[installed..] {
        tx.batch_execute(sql).await?;
        tx.execute(
            "INSERT INTO pulse_schema_migrations (version, checksum) VALUES ($1, $2)",
            &[version, &checksum(sql)],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(MIGRATIONS.len() - installed)
}

pub async fn prune(client: &Client) -> Result<i64, AnyError> {
    check(client).await?;
    let row = client
        .query_one("SELECT pulse_prune_expired()", &[])
        .await?;
    Ok(row.get(0))
}

pub async fn connect() -> Result<Client, AnyError> {
    let url = crate::env_value("DATABASE_URL_DIRECT")
        .or_else(|| crate::env_value("DATABASE_URL"))
        .ok_or("Set DATABASE_URL_DIRECT to the direct Postgres connection string")?;
    let config = crate::postgres_config(&url)?;
    let connector = postgres_native_tls::MakeTlsConnector::new(native_tls::TlsConnector::new()?);
    let (client, connection) =
        tokio::time::timeout(Duration::from_secs(8), config.connect(connector))
            .await
            .map_err(|_| "Postgres connection timed out")?
            .map_err(|_| "Could not connect to Postgres")?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

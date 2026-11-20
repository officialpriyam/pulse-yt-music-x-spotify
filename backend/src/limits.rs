use std::time::Duration;

use redis::{
    AsyncCommands, Script,
    aio::{ConnectionManager, ConnectionManagerConfig},
};
use reqwest::Url;
use tokio::sync::OnceCell;

use crate::{AnyError, ApiResult, STORAGE, digest, random};

const LIMIT: &str = r#"
local count = redis.call('INCR', KEYS[1])
if count == 1 then
    redis.call('EXPIRE', KEYS[1], ARGV[2])
end
if count > tonumber(ARGV[1]) then
    return math.max(1, redis.call('TTL', KEYS[1]))
end
return 0
"#;

const RELEASE: &str = r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
end
return 0
"#;

pub struct Limits {
    client: redis::Client,
    connection: OnceCell<ConnectionManager>,
    namespace: String,
    pub global: u32,
    pub oauth: u32,
    pub per_session: u32,
}

impl Limits {
    pub fn from_env(origin: &Url) -> Result<Self, AnyError> {
        let raw =
            crate::env_value("REDIS_URL").ok_or("Set REDIS_URL when enabling Google sign-in")?;
        let url = Url::parse(&raw).map_err(|_| "REDIS_URL is invalid")?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if origin.scheme() == "https" && url.password().is_none_or(str::is_empty) {
            return Err("Production Redis requires an authenticated TLS connection".into());
        }
        if url.fragment().is_some()
            || !matches!(url.scheme(), "rediss" | "redis")
            || (url.scheme() != "rediss" && (!local || origin.scheme() == "https"))
        {
            return Err("Use a rediss:// URL for production Redis. Plain Redis is limited to localhost development.".into());
        }
        let identity = digest(&origin.origin().ascii_serialization());
        let tag = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &identity[..12],
        );
        Ok(Self {
            client: redis::Client::open(raw).map_err(|_| "REDIS_URL is invalid")?,
            connection: OnceCell::new(),
            namespace: format!("pulse:{tag}"),
            global: crate::env_limit("PULSE_REQUESTS_PER_MINUTE", 6_000, 1, 100_000)?,
            oauth: crate::env_limit("PULSE_OAUTH_PER_MINUTE", 180, 1, 10_000)?,
            per_session: crate::env_limit("PULSE_SESSION_PER_MINUTE", 120, 1, 10_000)?,
        })
    }

    async fn connection(&self) -> ApiResult<ConnectionManager> {
        let initialise = self.connection.get_or_try_init(|| async {
            let config = ConnectionManagerConfig::new()
                .set_connection_timeout(Duration::from_millis(800))
                .set_response_timeout(Duration::from_millis(700))
                .set_number_of_retries(1);
            ConnectionManager::new_with_config(self.client.clone(), config).await
        });
        tokio::time::timeout(Duration::from_millis(1400), initialise)
            .await
            .map_err(|_| STORAGE)?
            .cloned()
            .map_err(|_| STORAGE)
    }

    pub async fn check(&self, bucket: &str, limit: u32) -> ApiResult<u32> {
        let key = format!("{}:limit:{bucket}", self.namespace);
        Script::new(LIMIT)
            .key(key)
            .arg(limit)
            .arg(60)
            .invoke_async(&mut self.connection().await?)
            .await
            .map_err(|_| STORAGE)
    }

    pub async fn ping(&self) -> ApiResult<()> {
        redis::cmd("PING")
            .query_async::<String>(&mut self.connection().await?)
            .await
            .map(|_| ())
            .map_err(|_| STORAGE)
    }

    pub async fn lock_refresh(&self, session: &str) -> ApiResult<Option<String>> {
        let key = format!("{}:refresh:{session}", self.namespace);
        let token = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            random::<24>()?,
        );
        let locked: Option<String> = self
            .connection()
            .await?
            .set_options(
                key,
                &token,
                redis::SetOptions::default()
                    .conditional_set(redis::ExistenceCheck::NX)
                    .with_expiration(redis::SetExpiry::EX(30)),
            )
            .await
            .map_err(|_| STORAGE)?;
        Ok(locked.map(|_| token))
    }

    pub async fn release_refresh(&self, session: &str, token: &str) {
        if let Ok(mut connection) = self.connection().await {
            let key = format!("{}:refresh:{session}", self.namespace);
            let _ = Script::new(RELEASE)
                .key(key)
                .arg(token)
                .invoke_async::<i64>(&mut connection)
                .await;
        }
    }
}

use std::time::Duration;

use redis::{
    AsyncCommands, Script,
    aio::{ConnectionManager, ConnectionManagerConfig},
};
use reqwest::{Client, Url, header};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

use crate::{AnyError, ApiResult, STORAGE, digest, env_value, random};

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

struct RedisTcp {
    client: redis::Client,
    connection: OnceCell<ConnectionManager>,
}

impl RedisTcp {
    fn new(raw: String, origin: &Url) -> Result<Self, AnyError> {
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
        Ok(Self {
            client: redis::Client::open(raw).map_err(|_| "REDIS_URL is invalid")?,
            connection: OnceCell::new(),
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
}

struct Upstash {
    client: Client,
    url: Url,
}

impl Upstash {
    fn new(raw: &str, token: &str) -> Result<Self, AnyError> {
        let url = Url::parse(raw).map_err(|_| "UPSTASH_REDIS_REST_URL is invalid")?;
        if url.scheme() != "https"
            || !url
                .host_str()
                .is_some_and(|host| host.ends_with(".upstash.io"))
            || url.port().is_some()
            || url.path() != "/"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("UPSTASH_REDIS_REST_URL must be an HTTPS Upstash origin".into());
        }
        let mut auth = header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "UPSTASH_REDIS_REST_TOKEN is invalid")?;
        auth.set_sensitive(true);
        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION, auth);
        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(2))
            .connect_timeout(Duration::from_secs(1))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { client, url })
    }

    async fn command<T: DeserializeOwned>(&self, command: Value) -> ApiResult<T> {
        let mut response = self
            .client
            .post(self.url.clone())
            .json(&command)
            .send()
            .await
            .map_err(|_| STORAGE)?
            .error_for_status()
            .map_err(|_| STORAGE)?
            .json::<serde_json::Map<String, Value>>()
            .await
            .map_err(|_| STORAGE)?;
        if response.contains_key("error") {
            return Err(STORAGE);
        }
        serde_json::from_value(response.remove("result").ok_or(STORAGE)?).map_err(|_| STORAGE)
    }
}

enum RedisBackend {
    Tcp(RedisTcp),
    Rest(Upstash),
}

pub struct Limits {
    backend: RedisBackend,
    namespace: String,
    pub global: u32,
    pub oauth: u32,
    pub per_session: u32,
}

impl Limits {
    pub fn from_env(origin: &Url) -> Result<Self, AnyError> {
        let backend = match (
            env_value("UPSTASH_REDIS_REST_URL"),
            env_value("UPSTASH_REDIS_REST_TOKEN"),
        ) {
            (Some(url), Some(token)) => RedisBackend::Rest(Upstash::new(&url, &token)?),
            (None, None) => {
                let url = env_value("REDIS_URL")
                    .ok_or("Configure Upstash REST or REDIS_URL when enabling Google sign-in")?;
                RedisBackend::Tcp(RedisTcp::new(url, origin)?)
            }
            _ => {
                return Err(
                    "Set UPSTASH_REDIS_REST_URL and UPSTASH_REDIS_REST_TOKEN together".into(),
                );
            }
        };
        let identity = digest(&origin.origin().ascii_serialization());
        let tag = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            &identity[..12],
        );
        Ok(Self {
            backend,
            namespace: format!("pulse:{tag}"),
            global: crate::env_limit("PULSE_REQUESTS_PER_MINUTE", 6_000, 1, 100_000)?,
            oauth: crate::env_limit("PULSE_OAUTH_PER_MINUTE", 180, 1, 10_000)?,
            per_session: crate::env_limit("PULSE_SESSION_PER_MINUTE", 120, 1, 10_000)?,
        })
    }

    pub async fn check(&self, bucket: &str, limit: u32) -> ApiResult<u32> {
        let key = format!("{}:limit:{bucket}", self.namespace);
        match &self.backend {
            RedisBackend::Rest(rest) => {
                rest.command(json!(["EVAL", LIMIT, 1, key, limit, 60]))
                    .await
            }
            RedisBackend::Tcp(tcp) => Script::new(LIMIT)
                .key(key)
                .arg(limit)
                .arg(60)
                .invoke_async(&mut tcp.connection().await?)
                .await
                .map_err(|_| STORAGE),
        }
    }

    pub async fn ping(&self) -> ApiResult<()> {
        let result: String = match &self.backend {
            RedisBackend::Rest(rest) => rest.command(json!(["PING"])).await?,
            RedisBackend::Tcp(tcp) => redis::cmd("PING")
                .query_async(&mut tcp.connection().await?)
                .await
                .map_err(|_| STORAGE)?,
        };
        (result == "PONG").then_some(()).ok_or(STORAGE)
    }

    pub async fn lock_refresh(&self, session: &str) -> ApiResult<Option<String>> {
        let key = format!("{}:refresh:{session}", self.namespace);
        let token = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            random::<24>()?,
        );
        let locked: Option<String> = match &self.backend {
            RedisBackend::Rest(rest) => {
                rest.command(json!(["SET", key, &token, "NX", "EX", 30]))
                    .await?
            }
            RedisBackend::Tcp(tcp) => tcp
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
                .map_err(|_| STORAGE)?,
        };
        Ok(locked.map(|_| token))
    }

    pub async fn release_refresh(&self, session: &str, token: &str) {
        let key = format!("{}:refresh:{session}", self.namespace);
        match &self.backend {
            RedisBackend::Rest(rest) => {
                let _ = rest
                    .command::<u32>(json!(["EVAL", RELEASE, 1, key, token]))
                    .await;
            }
            RedisBackend::Tcp(tcp) => {
                if let Ok(mut connection) = tcp.connection().await {
                    let _ = Script::new(RELEASE)
                        .key(key)
                        .arg(token)
                        .invoke_async::<u32>(&mut connection)
                        .await;
                }
            }
        }
    }
}

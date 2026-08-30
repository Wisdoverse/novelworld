use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use aws_config::{retry::RetryConfig, timeout::TimeoutConfig, BehaviorVersion};
use aws_sdk_s3::{
    config::{Credentials, Region},
    error::ProvideErrorMetadata,
    primitives::ByteStream,
    Client,
};
use bytes::Bytes;
use std::time::Duration;
use tokio::io::AsyncReadExt;

use crate::domain::ports::{ReadinessProbe, SourceFileStorage};

/// Hard ceiling for a retained source object read during import replay. The
/// upload contract bounds every accepted object at 20 MiB; this fails closed
/// far above that without ever trusting attacker-influenced metadata.
const MAX_RETAINED_SOURCE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct S3StorageConfig {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub force_path_style: bool,
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
    pub session_token: Option<String>,
}

impl S3StorageConfig {
    pub fn from_env() -> Result<Option<Self>> {
        if !env_bool("S3_ENABLED", false)? {
            return Ok(None);
        }
        let bucket = required_env("S3_BUCKET")?;
        if bucket.len() > 255
            || bucket
                .chars()
                .any(|ch| ch.is_control() || ch.is_whitespace() || ch == '/')
        {
            bail!("S3_BUCKET is invalid");
        }

        let endpoint = optional_env("S3_ENDPOINT")
            .map(validate_endpoint)
            .transpose()?;
        let access_key = optional_env("S3_ACCESS_KEY");
        let secret_key = optional_env("S3_SECRET_KEY");
        if access_key.is_some() != secret_key.is_some() {
            bail!("S3_ACCESS_KEY and S3_SECRET_KEY must be set together");
        }
        let session_token = optional_env("S3_SESSION_TOKEN");
        if session_token.is_some() && access_key.is_none() {
            bail!("S3_SESSION_TOKEN requires explicit S3 credentials");
        }

        Ok(Some(Self {
            bucket,
            region: optional_env("S3_REGION").unwrap_or_else(|| "us-east-1".into()),
            endpoint,
            force_path_style: env_bool("S3_FORCE_PATH_STYLE", false)?,
            access_key,
            secret_key,
            session_token,
        }))
    }
}

pub struct S3SourceFileStorage {
    client: Client,
    bucket: String,
}

impl S3SourceFileStorage {
    pub async fn new(config: S3StorageConfig) -> Result<Self> {
        #[cfg(test)]
        let uses_plain_http_test_endpoint = config
            .endpoint
            .as_deref()
            .is_some_and(|endpoint| endpoint.starts_with("http://"));
        let timeout = TimeoutConfig::builder()
            .connect_timeout(Duration::from_secs(3))
            .read_timeout(Duration::from_secs(15))
            .operation_attempt_timeout(Duration::from_secs(15))
            .operation_timeout(Duration::from_secs(30))
            .build();
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(config.region))
            .retry_config(RetryConfig::standard().with_max_attempts(3))
            .timeout_config(timeout);
        #[cfg(test)]
        if uses_plain_http_test_endpoint {
            // The loopback fake S3 server is deliberately HTTP-only. Supplying
            // an HTTP-only Smithy client keeps these adapter tests independent
            // of the host certificate store while production continues to use
            // the SDK's normal verified TLS client.
            loader = loader.http_client(aws_smithy_http_client::Builder::new().build_http());
        }
        if let (Some(access_key), Some(secret_key)) = (config.access_key, config.secret_key) {
            loader = loader.credentials_provider(Credentials::new(
                access_key,
                secret_key,
                config.session_token,
                None,
                "novelworld-s3",
            ));
        }
        let shared = loader.load().await;
        let mut service =
            aws_sdk_s3::config::Builder::from(&shared).force_path_style(config.force_path_style);
        if let Some(endpoint) = config.endpoint {
            service = service.endpoint_url(endpoint);
        }
        Ok(Self {
            client: Client::from_conf(service.build()),
            bucket: config.bucket,
        })
    }
}

#[async_trait]
impl SourceFileStorage for S3SourceFileStorage {
    async fn put(&self, key: &str, data: Bytes) -> Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type("application/octet-stream")
            .content_length(data.len() as i64)
            .body(ByteStream::from(data))
            .send()
            .await
            .with_context(|| format!("failed to store source object {key}"))?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        let output = match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(output) => output,
            Err(error) => {
                if error
                    .as_service_error()
                    .is_some_and(|service| service.code() == Some("NoSuchKey"))
                {
                    return Ok(None);
                }
                return Err(error).with_context(|| format!("failed to read source object {key}"));
            }
        };
        let mut reader = output.body.into_async_read();
        let mut data = Vec::new();
        tokio::io::AsyncReadExt::take(&mut reader, (MAX_RETAINED_SOURCE_BYTES + 1) as u64)
            .read_to_end(&mut data)
            .await
            .with_context(|| format!("failed to read source object {key}"))?;
        if data.len() > MAX_RETAINED_SOURCE_BYTES {
            bail!("source object {key} exceeds the bounded replay limit");
        }
        Ok(Some(Bytes::from(data)))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .with_context(|| format!("failed to delete source object {key}"))?;
        Ok(())
    }
}

#[async_trait]
impl ReadinessProbe for S3SourceFileStorage {
    async fn is_ready(&self) -> bool {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
    }
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn required_env(name: &str) -> Result<String> {
    optional_env(name).with_context(|| format!("{name} must be set when S3_ENABLED=true"))
}

fn env_bool(name: &str, default: bool) -> Result<bool> {
    match optional_env(name).as_deref() {
        None => Ok(default),
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        Some(_) => bail!("{name} must be true or false"),
    }
}

fn validate_endpoint(value: String) -> Result<String> {
    let endpoint = reqwest::Url::parse(&value).context("S3_ENDPOINT must be an absolute URL")?;
    if !matches!(endpoint.scheme(), "http" | "https")
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        bail!(
            "S3_ENDPOINT must be an HTTP(S) origin without credentials, path, query, or fragment"
        );
    }
    Ok(value.trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Bytes as RequestBytes,
        extract::{Path, State},
        http::{header, StatusCode},
        response::IntoResponse,
        routing::{head, put},
        Router,
    };
    use std::{collections::HashMap, sync::Arc};
    use tokio::sync::Mutex;

    #[test]
    fn endpoint_and_boolean_configuration_fail_closed() {
        assert_eq!(
            validate_endpoint("https://s3.example.test/".into()).unwrap(),
            "https://s3.example.test"
        );
        for invalid in [
            "ftp://s3.example.test",
            "https://user:pass@s3.example.test",
            "https://s3.example.test/path",
            "https://s3.example.test?bucket=x",
        ] {
            assert!(validate_endpoint(invalid.into()).is_err());
        }
    }

    #[derive(Clone, Default)]
    struct FakeS3 {
        objects: Arc<Mutex<HashMap<String, RequestBytes>>>,
    }

    async fn put_object(
        State(state): State<FakeS3>,
        Path((_bucket, key)): Path<(String, String)>,
        body: RequestBytes,
    ) -> StatusCode {
        state.objects.lock().await.insert(key, body);
        StatusCode::OK
    }

    async fn get_object(
        State(state): State<FakeS3>,
        Path((_bucket, key)): Path<(String, String)>,
    ) -> impl IntoResponse {
        match state.objects.lock().await.get(&key).cloned() {
            Some(bytes) => (StatusCode::OK, bytes).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                [(
                    header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("application/xml"),
                )],
                concat!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
                    "<Error><Code>NoSuchKey</Code>",
                    "<Message>The specified key does not exist.</Message></Error>"
                ),
            )
                .into_response(),
        }
    }

    async fn delete_object(
        State(state): State<FakeS3>,
        Path((_bucket, key)): Path<(String, String)>,
    ) -> StatusCode {
        state.objects.lock().await.remove(&key);
        StatusCode::NO_CONTENT
    }

    #[tokio::test]
    async fn adapter_puts_checks_and_deletes_private_objects() {
        let state = FakeS3::default();
        let app = Router::new()
            .route("/{bucket}", head(|| async { StatusCode::OK }))
            .route("/{bucket}/", head(|| async { StatusCode::OK }))
            .route(
                "/{bucket}/{*key}",
                put(put_object).get(get_object).delete(delete_object),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let storage = S3SourceFileStorage::new(S3StorageConfig {
            bucket: "source-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some(format!("http://{address}")),
            force_path_style: true,
            access_key: Some("test-access".into()),
            secret_key: Some("test-secret".into()),
            session_token: None,
        })
        .await
        .unwrap();

        assert!(storage.is_ready().await);
        storage
            .put("source-files/user/novel", Bytes::from_static(b"novel"))
            .await
            .unwrap();
        assert_eq!(
            state
                .objects
                .lock()
                .await
                .get("source-files/user/novel")
                .map(RequestBytes::as_ref),
            Some(b"novel".as_slice())
        );
        storage.delete("source-files/user/novel").await.unwrap();
        assert!(state.objects.lock().await.is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn get_replays_stored_bytes_and_maps_missing_keys_to_none() {
        let state = FakeS3::default();
        let app = Router::new()
            .route("/{bucket}", head(|| async { StatusCode::OK }))
            .route("/{bucket}/", head(|| async { StatusCode::OK }))
            .route(
                "/{bucket}/{*key}",
                put(put_object).get(get_object).delete(delete_object),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let storage = S3SourceFileStorage::new(S3StorageConfig {
            bucket: "source-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some(format!("http://{address}")),
            force_path_style: true,
            access_key: Some("test-access".into()),
            secret_key: Some("test-secret".into()),
            session_token: None,
        })
        .await
        .unwrap();

        assert_eq!(
            storage.get("source-files/user/missing").await.unwrap(),
            None
        );
        storage
            .put(
                "source-files/user/novel",
                Bytes::from_static(b"retained upload bytes"),
            )
            .await
            .unwrap();
        assert_eq!(
            storage.get("source-files/user/novel").await.unwrap(),
            Some(Bytes::from_static(b"retained upload bytes"))
        );
        storage.delete("source-files/user/novel").await.unwrap();
        assert_eq!(storage.get("source-files/user/novel").await.unwrap(), None);
        server.abort();
    }

    #[tokio::test]
    async fn get_fails_closed_when_the_object_exceeds_the_replay_bound() {
        let state = FakeS3::default();
        state.objects.lock().await.insert(
            "source-files/user/oversize".into(),
            RequestBytes::from(vec![b'x'; MAX_RETAINED_SOURCE_BYTES + 1]),
        );
        let app = Router::new()
            .route("/{bucket}", head(|| async { StatusCode::OK }))
            .route("/{bucket}/", head(|| async { StatusCode::OK }))
            .route(
                "/{bucket}/{*key}",
                put(put_object).get(get_object).delete(delete_object),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let storage = S3SourceFileStorage::new(S3StorageConfig {
            bucket: "source-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some(format!("http://{address}")),
            force_path_style: true,
            access_key: Some("test-access".into()),
            secret_key: Some("test-secret".into()),
            session_token: None,
        })
        .await
        .unwrap();

        let error = storage
            .get("source-files/user/oversize")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("bounded replay limit"),
            "unexpected error: {error}"
        );
        server.abort();
    }
}

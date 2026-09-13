use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::domain::ports::EmbeddingGenerator;

pub struct EmbeddingAdapter {
    client: Arc<llm_client::LlmClient>,
    model: String,
    pad_to_dimensions: Option<(usize, usize)>,
}

pub struct NoopEmbeddingGenerator;

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub provider: String,
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}

impl EmbeddingConfig {
    pub fn from_environment() -> Result<Option<Self>> {
        Self::parse(
            std::env::var("EMBEDDING_PROVIDER").ok(),
            std::env::var("EMBEDDING_API_URL").ok(),
            std::env::var("EMBEDDING_API_KEY").ok(),
            std::env::var("EMBEDDING_MODEL").ok(),
        )
    }

    fn parse(
        provider: Option<String>,
        api_url: Option<String>,
        api_key: Option<String>,
        model: Option<String>,
    ) -> Result<Option<Self>> {
        let provider = clean(provider)?;
        let api_url = clean(api_url)?;
        let api_key = clean(api_key)?.unwrap_or_default();
        let model = clean(model)?;

        if provider.is_none() && api_url.is_none() && model.is_none() {
            anyhow::ensure!(api_key.is_empty(), "incomplete embedding configuration");
            return Ok(None);
        }
        let provider = provider.ok_or_else(|| anyhow::anyhow!("EMBEDDING_PROVIDER is required"))?;
        let mut api_url =
            api_url.ok_or_else(|| anyhow::anyhow!("EMBEDDING_API_URL is required"))?;
        let model = model.ok_or_else(|| anyhow::anyhow!("EMBEDDING_MODEL is required"))?;
        anyhow::ensure!(
            provider.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || b"._-".contains(&byte)),
            "EMBEDDING_PROVIDER must use lowercase letters, digits, '.', '_' or '-'"
        );
        let parsed = api_url
            .parse::<axum::http::Uri>()
            .map_err(|_| anyhow::anyhow!("EMBEDDING_API_URL must be an absolute HTTP(S) URL"))?;
        anyhow::ensure!(
            matches!(parsed.scheme_str(), Some("http" | "https"))
                && parsed.host().is_some()
                && parsed
                    .authority()
                    .is_some_and(|authority| !authority.as_str().contains('@'))
                && parsed
                    .path_and_query()
                    .is_none_or(|value| value.query().is_none()),
            "EMBEDDING_API_URL must be an absolute HTTP(S) URL without credentials, query or fragment"
        );
        while api_url.ends_with('/') {
            api_url.pop();
        }

        Ok(Some(Self {
            provider,
            api_url,
            api_key,
            model,
        }))
    }

    pub fn qualified_model(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

fn clean(value: Option<String>) -> Result<Option<String>> {
    match value {
        Some(value) if value.is_empty() => Ok(None),
        Some(value) if value == value.trim() && !value.chars().any(char::is_control) => {
            Ok(Some(value))
        }
        Some(_) => anyhow::bail!("embedding configuration values must be trimmed and printable"),
        None => Ok(None),
    }
}

impl EmbeddingAdapter {
    pub fn new(
        client: Arc<llm_client::LlmClient>,
        model: String,
        pad_to_dimensions: Option<(usize, usize)>,
    ) -> Self {
        Self {
            client,
            model,
            pad_to_dimensions,
        }
    }
}

fn pad_embedding(mut embedding: Vec<f32>, dimensions: Option<(usize, usize)>) -> Result<Vec<f32>> {
    if let Some((source, target)) = dimensions {
        anyhow::ensure!(
            embedding.len() == source && target >= source,
            "embedding dimension mismatch"
        );
        embedding.resize(target, 0.0);
    }
    Ok(embedding)
}

#[async_trait]
impl EmbeddingGenerator for EmbeddingAdapter {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let req = llm_client::EmbeddingRequest {
            model: self.model.clone(),
            input: text.to_string(),
        };
        let response = self.client.embed(req).await?;
        pad_embedding(response.embedding, self.pad_to_dimensions)
    }
}

#[async_trait]
impl EmbeddingGenerator for NoopEmbeddingGenerator {
    async fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>> {
        Err(anyhow::anyhow!(
            "Embedding not configured — semantic search disabled"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{pad_embedding, EmbeddingConfig};

    #[test]
    fn explicit_config_supports_local_and_external_endpoints() {
        assert!(EmbeddingConfig::parse(None, None, None, None)
            .unwrap()
            .is_none());
        let local = EmbeddingConfig::parse(
            Some("local-tei".into()),
            Some("http://embedding:80/".into()),
            None,
            Some("Qwen/Qwen3-Embedding-0.6B".into()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(local.provider, "local-tei");
        assert_eq!(local.api_url, "http://embedding:80");
        assert!(local.api_key.is_empty());
        assert_eq!(local.model, "Qwen/Qwen3-Embedding-0.6B");
        let external = EmbeddingConfig::parse(
            Some("external".into()),
            Some("https://embeddings.example".into()),
            Some("secret".into()),
            Some("model".into()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(external.qualified_model(), "external/model");
        assert_eq!(external.api_key, "secret");
    }

    #[test]
    fn partial_or_malformed_config_fails_closed() {
        for values in [
            (Some("local"), None, None, Some("model")),
            (None, Some("http://embedding"), None, Some("model")),
            (Some("local"), Some("http://embedding"), None, None),
            (None, None, Some("secret"), None),
            (Some("LOCAL"), Some("http://embedding"), None, Some("model")),
            (Some("local"), Some("not-a-url"), None, Some("model")),
        ] {
            assert!(EmbeddingConfig::parse(
                values.0.map(str::to_owned),
                values.1.map(str::to_owned),
                values.2.map(str::to_owned),
                values.3.map(str::to_owned),
            )
            .is_err());
        }
    }

    #[test]
    fn local_vector_is_zero_padded_to_storage_dimensions() {
        let embedding = pad_embedding(vec![1.0; 1024], Some((1024, 1536))).unwrap();
        assert_eq!(embedding.len(), 1536);
        assert!(embedding[..1024].iter().all(|value| *value == 1.0));
        assert!(embedding[1024..].iter().all(|value| *value == 0.0));
        assert!(pad_embedding(vec![1.0; 1536], None).is_ok());
        assert!(pad_embedding(vec![1.0; 1023], Some((1024, 1536))).is_err());
    }
}

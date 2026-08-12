const MAX_API_KEY_BYTES: usize = 4_096;

#[derive(Clone)]
pub struct RuntimeLlmConfig {
    pub provider: String,
    pub api_url: String,
    pub model: String,
    pub api_key: String,
}

impl RuntimeLlmConfig {
    pub fn for_provider(provider: &str, api_key: &str) -> Result<Self, String> {
        let api_key = api_key.trim();
        if api_key.is_empty() || api_key.len() > MAX_API_KEY_BYTES {
            return Err("A valid API key is required".into());
        }
        if api_key.chars().any(char::is_control) {
            return Err("API key contains unsupported characters".into());
        }

        let (provider, api_url, model) = match provider.trim().to_lowercase().as_str() {
            "deepseek" => ("deepseek", "https://api.deepseek.com", "deepseek-v4-flash"),
            "openai" => ("openai", "https://api.openai.com", "gpt-4o-mini"),
            _ => return Err("Choose a supported AI provider".into()),
        };

        Ok(Self {
            provider: provider.into(),
            api_url: api_url.into(),
            model: model.into(),
            api_key: api_key.into(),
        })
    }

    pub fn from_environment(api_url: String, model: String, api_key: String) -> Option<Self> {
        let api_key = api_key.trim();
        if api_key.is_empty() || api_key == "sk-your-api-key" {
            return None;
        }
        Some(Self {
            provider: "environment".into(),
            api_url,
            model,
            api_key: api_key.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_presets_cannot_be_used_for_ssrf() {
        let deepseek = RuntimeLlmConfig::for_provider("deepseek", "secret").unwrap();
        assert_eq!(deepseek.api_url, "https://api.deepseek.com");
        assert_eq!(deepseek.model, "deepseek-v4-flash");
        assert!(RuntimeLlmConfig::for_provider("http://127.0.0.1", "secret").is_err());
    }
}

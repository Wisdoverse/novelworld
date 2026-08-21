const MAX_API_KEY_BYTES: usize = 4_096;

#[derive(Clone)]
pub struct RuntimeLlmConfig {
    pub provider: String,
    pub api_url: String,
    pub model: String,
    pub thinking_enabled: bool,
    pub api_key: String,
}

impl RuntimeLlmConfig {
    pub fn for_provider(provider: &str, api_key: &str) -> Result<Self, String> {
        let default_model = match provider.trim().to_lowercase().as_str() {
            "deepseek" => "deepseek-v4-flash",
            "openai" => "gpt-4o-mini",
            _ => return Err("Choose a supported AI provider".into()),
        };
        Self::for_settings(provider, default_model, api_key, false)
    }

    pub fn for_settings(
        provider: &str,
        model: &str,
        api_key: &str,
        thinking_enabled: bool,
    ) -> Result<Self, String> {
        let api_key = api_key.trim();
        if api_key.is_empty() || api_key.len() > MAX_API_KEY_BYTES {
            return Err("A valid API key is required".into());
        }
        if api_key.chars().any(char::is_control) {
            return Err("API key contains unsupported characters".into());
        }

        let model = model.trim();
        let (provider, api_url) = match (provider.trim().to_lowercase().as_str(), model) {
            (
                "deepseek",
                "deepseek-v4-flash"
                | "deepseek-v4-flash-vision-exp"
                | "deepseek-v4-pro",
            ) => {
                ("deepseek", "https://api.deepseek.com")
            }
            ("openai", "gpt-4o-mini") => ("openai", "https://api.openai.com"),
            ("deepseek", _) | ("openai", _) => {
                return Err("Choose a model supported by the selected provider".into())
            }
            _ => return Err("Choose a supported AI provider".into()),
        };

        Ok(Self {
            provider: provider.into(),
            api_url: api_url.into(),
            model: model.into(),
            thinking_enabled: provider == "deepseek" && thinking_enabled,
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
            thinking_enabled: false,
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
        assert!(!deepseek.thinking_enabled);
        assert!(
            RuntimeLlmConfig::for_settings("deepseek", "deepseek-v4-pro", "secret", true)
                .unwrap()
                .thinking_enabled
        );
        assert!(RuntimeLlmConfig::for_provider("http://127.0.0.1", "secret").is_err());
    }

    #[test]
    fn experimental_vision_model_uses_the_fixed_deepseek_endpoint() {
        let config = RuntimeLlmConfig::for_settings(
            "deepseek",
            "deepseek-v4-flash-vision-exp",
            "secret",
            false,
        )
        .unwrap();

        assert_eq!(config.api_url, "https://api.deepseek.com");
        assert_eq!(config.model, "deepseek-v4-flash-vision-exp");
    }
}

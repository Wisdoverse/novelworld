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
        if model.is_empty()
            || model.len() > 200
            || model.chars().any(char::is_whitespace)
            || model.chars().any(char::is_control)
        {
            return Err("A valid model ID is required (at most 200 bytes)".into());
        }
        let provider = provider.trim().to_lowercase();
        let api_url = Self::preset_url(&provider).ok_or("Choose a supported AI provider")?;
        match (provider.as_str(), model) {
            (
                "deepseek",
                "deepseek-flash"
                | "deepseek-v4-flash"
                | "deepseek-v4-flash-vision-exp"
                | "deepseek-v4-pro",
            )
            | ("openai", "gpt-4o-mini") => {}
            ("deepseek", _) | ("openai", _) => {
                return Err("Choose a model supported by the selected provider".into())
            }
            _ => {}
        }

        Ok(Self {
            provider: provider.clone(),
            api_url: api_url.into(),
            model: model.into(),
            thinking_enabled: provider == "deepseek" && thinking_enabled,
            api_key: api_key.into(),
        })
    }

    /// Only fixed official origins are accepted from browser settings.
    pub fn preset_url(provider: &str) -> Option<&'static str> {
        Some(match provider {
            "deepseek" => "https://api.deepseek.com",
            "openai" => "https://api.openai.com",
            "zhipu" => "https://open.bigmodel.cn/api/paas/v4",
            "zai" => "https://api.z.ai/api/paas/v4",
            "zhipu_coding" => "https://open.bigmodel.cn/api/coding/paas/v4",
            "zai_coding" => "https://api.z.ai/api/coding/paas/v4",
            "minimax_cn" | "minimax_coding_cn" => "https://api.minimax.cn/v1",
            "minimax_global" | "minimax_coding_global" => "https://api.minimax.io/v1",
            "qwen_cn" => "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "qwen_global" => "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
            "aliyun_coding_cn" => "https://coding.dashscope.aliyuncs.com/v1",
            "aliyun_coding_global" => "https://coding-intl.dashscope.aliyuncs.com/v1",
            "kimi_cn" => "https://api.moonshot.cn/v1",
            "kimi_global" => "https://api.moonshot.ai/v1",
            "kimi_coding_cn" => "https://api.kimi.com/coding/v1",
            "kimi_coding_global" => "https://api.kimi.ai/coding/v1",
            "stepfun" => "https://api.stepfun.com/v1",
            "stepfun_global" => "https://api.stepfun.ai/v1",
            "stepfun_coding" => "https://api.stepfun.com/step_plan/v1",
            "stepfun_coding_global" => "https://api.stepfun.ai/step_plan/v1",
            "hunyuan" => "https://api.hunyuan.cloud.tencent.com/v1",
            "doubao" => "https://ark.cn-beijing.volces.com/api/v3",
            "volcengine_coding" => "https://ark.cn-beijing.volces.com/api/coding/v3",
            "qianfan" => "https://qianfan.baidubce.com/v2",
            "qianfan_coding" => "https://qianfan.baidubce.com/v2/coding",
            "siliconflow" => "https://api.siliconflow.cn/v1",
            _ => return None,
        })
    }

    pub fn reusable_key_for(&self, provider: &str) -> Option<&str> {
        let provider = provider.trim().to_lowercase();
        (self.provider == provider && Self::preset_url(&provider) == Some(self.api_url.as_str()))
            .then_some(self.api_key.as_str())
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
    fn regional_plan_endpoints_and_key_reuse_are_isolated() {
        for (provider, url) in [
            (
                "zhipu_coding",
                "https://open.bigmodel.cn/api/coding/paas/v4",
            ),
            ("zai_coding", "https://api.z.ai/api/coding/paas/v4"),
            ("minimax_coding_cn", "https://api.minimax.cn/v1"),
            ("minimax_coding_global", "https://api.minimax.io/v1"),
            (
                "aliyun_coding_cn",
                "https://coding.dashscope.aliyuncs.com/v1",
            ),
            (
                "aliyun_coding_global",
                "https://coding-intl.dashscope.aliyuncs.com/v1",
            ),
            ("siliconflow", "https://api.siliconflow.cn/v1"),
        ] {
            let config =
                RuntimeLlmConfig::for_settings(provider, "account-model", "secret", true).unwrap();
            assert_eq!(config.api_url, url);
            assert!(!config.thinking_enabled);
            assert_eq!(config.reusable_key_for(provider), Some("secret"));
            assert!(config.reusable_key_for("deepseek").is_none());
        }
        let config =
            RuntimeLlmConfig::for_settings("minimax_cn", "MiniMax-M3", "secret", false).unwrap();
        assert!(config.reusable_key_for("minimax_coding_cn").is_none());
        assert!(config.reusable_key_for("minimax_global").is_none());
        for model in ["", "bad model", "bad\nmodel", &"x".repeat(201)] {
            assert!(RuntimeLlmConfig::for_settings("zhipu", model, "secret", false).is_err());
        }
        let config = RuntimeLlmConfig::for_settings(
            "siliconflow",
            "Qwen/Qwen3-235B-A22B-Instruct-2507",
            "secret",
            false,
        )
        .unwrap();
        assert_eq!(config.model, "Qwen/Qwen3-235B-A22B-Instruct-2507");
    }

    #[test]
    fn browser_catalog_matches_owner_endpoints() {
        let catalog = include_str!("../../../../../frontend/src/pages/settings/model/providers.ts");
        let mut count = 0;
        for line in catalog.lines() {
            let Some((provider, details)) = line.trim().split_once(": { label: ") else {
                continue;
            };
            let endpoint = details
                .split_once("endpoint: '")
                .unwrap()
                .1
                .split('\'')
                .next()
                .unwrap();
            assert_eq!(
                RuntimeLlmConfig::preset_url(provider),
                Some(endpoint),
                "{provider}"
            );
            count += 1;
        }
        assert_eq!(count, 28);
    }

    #[test]
    fn provider_presets_cannot_be_used_for_ssrf() {
        let deepseek =
            RuntimeLlmConfig::for_settings("deepseek", "deepseek-flash", "secret", false).unwrap();
        assert_eq!(deepseek.api_url, "https://api.deepseek.com");
        assert_eq!(deepseek.model, "deepseek-flash");
        assert!(!deepseek.thinking_enabled);
        assert!(
            RuntimeLlmConfig::for_settings("deepseek", "deepseek-v4-pro", "secret", true)
                .unwrap()
                .thinking_enabled
        );
        assert!(RuntimeLlmConfig::for_settings(
            "http://127.0.0.1",
            "deepseek-flash",
            "secret",
            false,
        )
        .is_err());
        assert!(RuntimeLlmConfig::for_settings(
            "deepseek",
            "deepseek-flash-custom",
            "secret",
            false
        )
        .is_err());
    }

    #[test]
    fn legacy_deepseek_models_keep_the_fixed_endpoint() {
        for model in [
            "deepseek-v4-flash",
            "deepseek-v4-flash-vision-exp",
            "deepseek-v4-pro",
        ] {
            let config =
                RuntimeLlmConfig::for_settings("deepseek", model, "secret", false).unwrap();
            assert_eq!(config.api_url, "https://api.deepseek.com");
            assert_eq!(config.model, model);
        }
    }
}

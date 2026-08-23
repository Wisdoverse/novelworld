use std::{collections::HashMap, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use crate::domain::{
    entities::llm_usage::{
        BillableTokenClass, BillableTokenUsage, LlmPricingCatalog, LlmUsageSnapshot, TokenPrices,
    },
    ports::LlmUsageReader,
};

const MAX_WINDOW_DAYS: u16 = 90;

pub struct PrometheusLlmUsageReader {
    client: reqwest::Client,
    query_url: reqwest::Url,
    window_days: u16,
}

impl PrometheusLlmUsageReader {
    pub fn new(base_url: &str, window_days: u16) -> Result<Self> {
        if !(1..=MAX_WINDOW_DAYS).contains(&window_days) {
            bail!("LLM usage window must be between 1 and {MAX_WINDOW_DAYS} days");
        }
        let mut query_url = reqwest::Url::parse(base_url)?.join("/api/v1/query")?;
        if !matches!(query_url.scheme(), "http" | "https")
            || !query_url.username().is_empty()
            || query_url.password().is_some()
            || query_url.host_str().is_none()
        {
            bail!("PROMETHEUS_URL must be an HTTP(S) origin without credentials");
        }
        query_url.set_query(None);
        query_url.set_fragment(None);
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?,
            query_url,
            window_days,
        })
    }
}

#[async_trait]
impl LlmUsageReader for PrometheusLlmUsageReader {
    async fn read(&self) -> Result<LlmUsageSnapshot> {
        let query = format!(
            "round(sum by (provider, model, class) (increase(novelworld_llm_billable_tokens_total[{}d])))",
            self.window_days
        );
        let mut query_url = self.query_url.clone();
        query_url.query_pairs_mut().append_pair("query", &query);
        let response = self
            .client
            .get(query_url)
            .send()
            .await?
            .error_for_status()?;
        parse_snapshot(response.json().await?, self.window_days)
    }
}

#[derive(Deserialize)]
struct PrometheusResponse {
    status: String,
    data: PrometheusData,
}

#[derive(Deserialize)]
struct PrometheusData {
    #[serde(rename = "resultType")]
    result_type: String,
    result: Vec<PrometheusSample>,
}

#[derive(Deserialize)]
struct PrometheusSample {
    metric: HashMap<String, String>,
    value: (f64, String),
}

fn parse_snapshot(response: PrometheusResponse, window_days: u16) -> Result<LlmUsageSnapshot> {
    if response.status != "success" || response.data.result_type != "vector" {
        bail!("Prometheus returned an unexpected LLM usage response");
    }
    let usage = response
        .data
        .result
        .into_iter()
        .map(|sample| {
            let provider = required_label(&sample.metric, "provider")?;
            let model = required_label(&sample.metric, "model")?;
            let class = BillableTokenClass::from_str(required_label(&sample.metric, "class")?)
                .ok_or_else(|| anyhow!("Prometheus returned an unknown billable token class"))?;
            let value: f64 = sample.value.1.parse()?;
            if !value.is_finite() || value < 0.0 || value > u64::MAX as f64 {
                bail!("Prometheus returned an invalid billable token count");
            }
            Ok(BillableTokenUsage {
                provider: provider.into(),
                model: model.into(),
                class,
                tokens: value.round() as u64,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(LlmUsageSnapshot { window_days, usage })
}

fn required_label<'a>(labels: &'a HashMap<String, String>, key: &str) -> Result<&'a str> {
    labels
        .get(key)
        .filter(|value| !value.is_empty() && value.len() <= 200)
        .map(String::as_str)
        .ok_or_else(|| anyhow!("Prometheus LLM usage is missing the {key} label"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTokenPrices {
    cached_input: Option<String>,
    uncached_input: String,
    output: String,
}

pub fn pricing_from_config(json: &str, usd_cny_rate: Option<&str>) -> Result<LlmPricingCatalog> {
    let raw: HashMap<String, RawTokenPrices> =
        serde_json::from_str(json).context("invalid LLM_PRICING_USD_PER_MILLION JSON")?;
    let mut prices = HashMap::with_capacity(raw.len());
    for (model, value) in raw {
        let (provider, model_name) = model
            .split_once('/')
            .filter(|(provider, model)| !provider.is_empty() && !model.is_empty())
            .ok_or_else(|| anyhow!("LLM pricing keys must use provider/model"))?;
        if model.len() > 401 || provider.len() > 200 || model_name.len() > 200 {
            bail!("LLM pricing key is too long");
        }
        prices.insert(
            model,
            TokenPrices {
                cached_input_microusd_per_million: value
                    .cached_input
                    .as_deref()
                    .map(parse_decimal_micros)
                    .transpose()?,
                uncached_input_microusd_per_million: parse_decimal_micros(&value.uncached_input)?,
                output_microusd_per_million: parse_decimal_micros(&value.output)?,
            },
        );
    }
    let usd_cny_micros_per_usd = usd_cny_rate
        .filter(|value| !value.trim().is_empty())
        .map(parse_decimal_micros)
        .transpose()?;
    Ok(LlmPricingCatalog::new(prices, usd_cny_micros_per_usd))
}

fn parse_decimal_micros(value: &str) -> Result<u64> {
    let value = value.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 6
    {
        bail!("prices and exchange rates must be non-negative decimals with at most 6 places");
    }
    let whole: u64 = whole.parse()?;
    let fraction: u64 = if fraction.is_empty() {
        0
    } else {
        format!("{fraction:0<6}").parse()?
    };
    whole
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(fraction))
        .ok_or_else(|| anyhow!("price or exchange rate is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_config_uses_exact_decimal_microunits() {
        let catalog = pricing_from_config(
            r#"{"openai/gpt-4o-mini":{"cached_input":"0.075","uncached_input":"0.15","output":"0.6"}}"#,
            Some("7.2"),
        )
        .unwrap();
        let summary = catalog.summarize(LlmUsageSnapshot {
            window_days: 30,
            usage: vec![BillableTokenUsage {
                provider: "openai".into(),
                model: "gpt-4o-mini".into(),
                class: BillableTokenClass::UncachedInput,
                tokens: 1_000_000,
            }],
        });
        assert_eq!(summary.usd_micros, Some(150_000));
        assert_eq!(summary.cny_micros, Some(1_080_000));
    }

    #[test]
    fn prometheus_response_requires_bounded_accounting_labels() {
        let response: PrometheusResponse = serde_json::from_str(
            r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"provider":"openai","model":"gpt-4o-mini","class":"output"},"value":[1,"42"]}]}}"#,
        )
        .unwrap();
        let snapshot = parse_snapshot(response, 30).unwrap();
        assert_eq!(snapshot.usage[0].tokens, 42);
        assert_eq!(snapshot.usage[0].class, BillableTokenClass::Output);
    }
}

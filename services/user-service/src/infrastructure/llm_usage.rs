use std::{collections::HashMap, time::Duration};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

use crate::domain::{
    entities::llm_usage::{
        BillableTokenClass, BillableTokenUsage, BillingKind, LlmPricingCatalog, LlmUsageSnapshot,
        NativeTokenPrices, PriceRange, PricingCurrency, PricingReference, SubscriptionPlan,
        TokenPrices,
    },
    entities::runtime_config::RuntimeLlmConfig,
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
    async fn read(&self, config: &RuntimeLlmConfig) -> Result<LlmUsageSnapshot> {
        let query = usage_query(self.window_days, &config.api_key);
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

fn usage_query(window_days: u16, api_key: &str) -> String {
    let usage_key = llm_client::usage_key_fingerprint(api_key);
    format!(
        "round(sum by (provider, model, class) (increase(novelworld_llm_billable_tokens_total{{usage_key=\"{usage_key}\"}}[{window_days}d])))"
    )
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfficialPrices {
    verified_on: String,
    #[serde(deserialize_with = "unique_map")]
    models: HashMap<String, OfficialModelPrice>,
    #[serde(deserialize_with = "unique_map")]
    subscriptions: HashMap<String, OfficialSubscription>,
    #[serde(deserialize_with = "unique_map")]
    unavailable: HashMap<String, OfficialUnavailable>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct PriceOverrides(#[serde(deserialize_with = "unique_map")] HashMap<String, RawTokenPrices>);

fn unique_map<'de, D, T>(deserializer: D) -> std::result::Result<HashMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct UniqueMap<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for UniqueMap<T> {
        type Value = HashMap<String, T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a price object with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut values = HashMap::new();
            while let Some((key, value)) = map.next_entry()? {
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate pricing key"));
                }
            }
            Ok(values)
        }
    }
    deserializer.deserialize_map(UniqueMap(std::marker::PhantomData))
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum RawCurrency {
    Usd,
    Cny,
}
impl From<RawCurrency> for PricingCurrency {
    fn from(value: RawCurrency) -> Self {
        match value {
            RawCurrency::Usd => Self::Usd,
            RawCurrency::Cny => Self::Cny,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfficialModelPrice {
    currency: RawCurrency,
    cached_input: Option<[String; 2]>,
    uncached_input: [String; 2],
    output: [String; 2],
    source_url: String,
    note: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfficialSubscription {
    source_url: String,
    note: String,
    plans: Vec<OfficialPlan>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfficialPlan {
    name: String,
    currency: RawCurrency,
    monthly: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OfficialUnavailable {
    source_url: String,
    note: String,
}

/// Offline public price snapshot. Operator overrides never modify Diagnostic pricing.
pub fn pricing_from_snapshot(
    snapshot: &str,
    usd_overrides: &str,
    cny_overrides: &str,
    usd_cny_rate: Option<&str>,
) -> Result<LlmPricingCatalog> {
    let snapshot: OfficialPrices =
        serde_json::from_str(snapshot).context("invalid official LLM price snapshot")?;
    let date = chrono::NaiveDate::parse_from_str(&snapshot.verified_on, "%Y-%m-%d")?;
    if snapshot.verified_on.len() != 10
        || date.format("%Y-%m-%d").to_string() != snapshot.verified_on
    {
        bail!("official price verification date must use YYYY-MM-DD");
    }
    let mut prices = HashMap::new();
    let mut references = HashMap::new();
    let mut subscriptions = HashMap::new();
    for (provider, value) in snapshot.subscriptions {
        validate_label(&provider)?;
        validate_source(&value.source_url, &value.note)?;
        let plans = value
            .plans
            .into_iter()
            .map(|plan| {
                if plan.name.is_empty()
                    || plan.name.len() > 100
                    || plan.name.chars().any(char::is_control)
                {
                    bail!("invalid subscription plan name");
                }
                Ok(SubscriptionPlan {
                    name: plan.name,
                    currency: plan.currency.into(),
                    monthly_micros: parse_decimal_micros(&plan.monthly)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        subscriptions.insert(
            provider.clone(),
            PricingReference {
                provider,
                model: String::new(),
                source_url: Some(value.source_url),
                verified_on: Some(snapshot.verified_on.clone()),
                note: value.note,
                billing_kind: BillingKind::Subscription,
                plans,
            },
        );
    }
    for (key, value) in snapshot.models {
        let (provider, model) = split_price_key(&key)?;
        if subscriptions.contains_key(provider) {
            bail!("subscription providers cannot have API token prices");
        }
        validate_source(&value.source_url, &value.note)?;
        references.insert(
            key.clone(),
            PricingReference {
                provider: provider.into(),
                model: model.into(),
                source_url: Some(value.source_url),
                verified_on: Some(snapshot.verified_on.clone()),
                note: value.note,
                billing_kind: BillingKind::Api,
                plans: Vec::new(),
            },
        );
        prices.insert(
            key,
            NativeTokenPrices {
                currency: value.currency.into(),
                cached_input: value
                    .cached_input
                    .as_ref()
                    .map(parse_price_range)
                    .transpose()?,
                uncached_input: parse_price_range(&value.uncached_input)?,
                output: parse_price_range(&value.output)?,
            },
        );
    }
    for (key, value) in snapshot.unavailable {
        let (provider, model) = split_price_key(&key)?;
        if references.contains_key(&key) || subscriptions.contains_key(provider) {
            bail!("conflicting official pricing entries");
        }
        validate_source(&value.source_url, &value.note)?;
        references.insert(
            key.clone(),
            PricingReference {
                provider: provider.into(),
                model: model.into(),
                source_url: Some(value.source_url),
                verified_on: Some(snapshot.verified_on.clone()),
                note: value.note,
                billing_kind: BillingKind::Unavailable,
                plans: Vec::new(),
            },
        );
    }
    let usd = serde_json::from_str::<PriceOverrides>(usd_overrides)
        .context("invalid LLM_PRICING_USD_PER_MILLION JSON")?
        .0;
    let cny = serde_json::from_str::<PriceOverrides>(cny_overrides)
        .context("invalid LLM_PRICING_CNY_PER_MILLION JSON")?
        .0;
    if usd.keys().any(|key| cny.contains_key(key)) {
        bail!("USD and CNY price overrides conflict for the same provider/model");
    }
    for (currency, overrides) in [(PricingCurrency::Usd, usd), (PricingCurrency::Cny, cny)] {
        for (key, value) in overrides {
            let (provider, model) = split_price_key(&key)?;
            if subscriptions.contains_key(provider) {
                bail!("subscription token prices cannot be overridden as ordinary API prices");
            }
            references.insert(key.clone(), PricingReference {
                provider: provider.into(), model: model.into(), source_url: None, verified_on: None,
                note: "Operator-configured price; replaces the public snapshot and is not an official account bill.".into(),
                billing_kind: BillingKind::Operator, plans: Vec::new(),
            });
            prices.insert(
                key,
                NativeTokenPrices {
                    currency,
                    cached_input: value
                        .cached_input
                        .as_deref()
                        .map(parse_decimal_micros)
                        .transpose()?
                        .map(PriceRange::fixed),
                    uncached_input: PriceRange::fixed(parse_decimal_micros(&value.uncached_input)?),
                    output: PriceRange::fixed(parse_decimal_micros(&value.output)?),
                },
            );
        }
    }
    let fx = usd_cny_rate
        .filter(|value| !value.trim().is_empty())
        .map(parse_decimal_micros)
        .transpose()?;
    if fx == Some(0) {
        bail!("USD_CNY_RATE must be positive");
    }
    Ok(LlmPricingCatalog::from_native(
        prices,
        references,
        subscriptions,
        fx,
    ))
}

pub fn pricing_from_environment(
    usd_overrides: &str,
    cny_overrides: &str,
    usd_cny_rate: Option<&str>,
) -> Result<LlmPricingCatalog> {
    pricing_from_snapshot(
        include_str!("llm-prices.json"),
        usd_overrides,
        cny_overrides,
        usd_cny_rate,
    )
}

fn split_price_key(key: &str) -> Result<(&str, &str)> {
    let (provider, model) = key
        .split_once('/')
        .ok_or_else(|| anyhow!("LLM pricing keys must use provider/model"))?;
    validate_label(provider)?;
    validate_label(model)?;
    Ok((provider, model))
}

fn validate_label(label: &str) -> Result<()> {
    if label.is_empty()
        || label.len() > 200
        || !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'/' | b':'))
    {
        bail!("invalid price provider/model label");
    }
    Ok(())
}

fn validate_source(source: &str, note: &str) -> Result<()> {
    let url = reqwest::Url::parse(source)?;
    if source.len() > 2048
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        bail!("price sources must be HTTPS URLs without credentials or fragments");
    }
    if note.is_empty() || note.len() > 4096 || note.chars().any(|c| c.is_control() && c != '\n') {
        bail!("invalid price note");
    }
    Ok(())
}

fn parse_price_range(value: &[String; 2]) -> Result<PriceRange> {
    let range = PriceRange {
        minimum: parse_decimal_micros(&value[0])?,
        maximum: parse_decimal_micros(&value[1])?,
    };
    if range.minimum > range.maximum {
        bail!("price minimum exceeds maximum");
    }
    Ok(range)
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
    use crate::domain::entities::llm_usage::LlmUsageSummary;

    fn fixture_snapshot() -> serde_json::Value {
        serde_json::json!({
            "verified_on": "2026-09-26",
            "models": {
                "vendor_cn/model": {"currency":"CNY","cached_input":["0.1","0.1"],"uncached_input":["2","2"],"output":["8","8"],"source_url":"https://example.com/cn","note":"CN list price"},
                "vendor_global/model": {"currency":"USD","cached_input":null,"uncached_input":["0.3","0.6"],"output":["1.2","2.4"],"source_url":"https://example.com/global","note":"Input-length tiers"},
                "vendor_cn/free": {"currency":"CNY","cached_input":null,"uncached_input":["0","0"],"output":["0","0"],"source_url":"https://example.com/free","note":"Verified free model"}
            },
            "subscriptions": {"vendor_coding":{"source_url":"https://example.com/plan","note":"Published subscription quote; account tier unknown","plans":[{"name":"Plus","currency":"CNY","monthly":"49"}]}},
            "unavailable": {"vendor_cn/unknown":{"source_url":"https://example.com/models","note":"No verified model price"}}
        })
    }

    fn token(
        provider: &str,
        model: &str,
        class: BillableTokenClass,
        tokens: u64,
    ) -> BillableTokenUsage {
        BillableTokenUsage {
            provider: provider.into(),
            model: model.into(),
            class,
            tokens,
        }
    }

    fn summarize_fixture(usage: Vec<BillableTokenUsage>) -> LlmUsageSummary {
        pricing_from_snapshot(&fixture_snapshot().to_string(), "{}", "{}", None)
            .unwrap()
            .summarize(LlmUsageSnapshot {
                window_days: 30,
                usage,
            })
    }

    #[test]
    fn official_prices_keep_native_currencies_ranges_and_unknown_usage_separate() {
        let summary = summarize_fixture(vec![
            token(
                "vendor_cn",
                "model",
                BillableTokenClass::UncachedInput,
                1_000_000,
            ),
            token(
                "vendor_global",
                "model",
                BillableTokenClass::UncachedInput,
                1_000_000,
            ),
            token("vendor_cn", "unknown", BillableTokenClass::Output, 12),
            token("vendor_global", "model", BillableTokenClass::CachedInput, 3),
        ]);
        assert_eq!(
            summary.estimates,
            vec![
                crate::domain::entities::llm_usage::CostEstimate {
                    currency: PricingCurrency::Usd,
                    minimum_micros: 300_000,
                    maximum_micros: 600_000
                },
                crate::domain::entities::llm_usage::CostEstimate {
                    currency: PricingCurrency::Cny,
                    minimum_micros: 2_000_000,
                    maximum_micros: 2_000_000
                },
            ]
        );
        assert_eq!(summary.unpriced_tokens, 15);
        assert_eq!((summary.usd_micros, summary.cny_micros), (None, None));
        assert_eq!(summary.pricing_references.len(), 3);
    }

    #[test]
    fn native_cny_does_not_need_fx_and_free_is_not_unknown() {
        let summary = summarize_fixture(vec![token(
            "vendor_cn",
            "model",
            BillableTokenClass::UncachedInput,
            1_000_000,
        )]);
        assert_eq!(
            (summary.usd_micros, summary.cny_micros),
            (None, Some(2_000_000))
        );
        let free = summarize_fixture(vec![token(
            "vendor_cn",
            "free",
            BillableTokenClass::UncachedInput,
            42,
        )]);
        assert_eq!(free.cny_micros, Some(0));
        assert_eq!(free.unpriced_tokens, 0);
        let unknown = summarize_fixture(vec![token(
            "vendor_cn",
            "unknown",
            BillableTokenClass::UncachedInput,
            42,
        )]);
        assert_eq!(unknown.cny_micros, None);
        assert_eq!(unknown.unpriced_tokens, 42);
    }

    #[test]
    fn active_subscription_has_quotes_without_fabricating_a_zero_bill() {
        let catalog =
            pricing_from_snapshot(&fixture_snapshot().to_string(), "{}", "{}", None).unwrap();
        let summary = catalog.summarize_for(
            LlmUsageSnapshot {
                window_days: 30,
                usage: Vec::new(),
            },
            "vendor_coding",
            "model",
        );
        assert!(summary.estimates.is_empty());
        assert_eq!((summary.usd_micros, summary.cny_micros), (None, None));
        assert_eq!(
            summary.pricing_references[0].billing_kind,
            BillingKind::Subscription
        );
        assert_eq!(
            summary.pricing_references[0].plans[0].monthly_micros,
            49_000_000
        );
        let used = catalog.summarize_for(
            LlmUsageSnapshot {
                window_days: 30,
                usage: vec![token(
                    "vendor_coding",
                    "model",
                    BillableTokenClass::Output,
                    99,
                )],
            },
            "vendor_coding",
            "model",
        );
        assert_eq!(used.unpriced_tokens, 99);
        assert!(used.estimates.is_empty());
    }

    #[test]
    fn overrides_replace_snapshot_and_unknown_prices_with_operator_provenance() {
        let overrides = r#"{"vendor_cn/model":{"uncached_input":"1","output":"3"},"vendor_cn/unknown":{"uncached_input":"0","output":"2"}}"#;
        let catalog = pricing_from_snapshot(
            &fixture_snapshot().to_string(),
            "{}",
            overrides,
            Some("7.2"),
        )
        .unwrap();
        let summary = catalog.summarize(LlmUsageSnapshot {
            window_days: 30,
            usage: vec![token(
                "vendor_cn",
                "model",
                BillableTokenClass::UncachedInput,
                1_000_000,
            )],
        });
        assert_eq!(summary.cny_micros, Some(1_000_000));
        assert_eq!(summary.usd_micros, Some(138_889));
        let reference = &summary.pricing_references[0];
        assert_eq!(reference.billing_kind, BillingKind::Operator);
        assert!(reference.source_url.is_none() && reference.verified_on.is_none());
        let unknown = catalog.summarize(LlmUsageSnapshot {
            window_days: 30,
            usage: vec![token(
                "vendor_cn",
                "unknown",
                BillableTokenClass::Output,
                1_000_000,
            )],
        });
        assert_eq!(unknown.cny_micros, Some(2_000_000));
        assert!(
            pricing_from_snapshot(&fixture_snapshot().to_string(), overrides, overrides, None)
                .is_err()
        );
        let subscription = r#"{"vendor_coding/model":{"uncached_input":"1","output":"2"}}"#;
        assert!(
            pricing_from_snapshot(&fixture_snapshot().to_string(), subscription, "{}", None)
                .is_err()
        );
    }

    #[test]
    fn active_configuration_only_adds_reference_without_repricing_historical_provider() {
        let catalog =
            pricing_from_snapshot(&fixture_snapshot().to_string(), "{}", "{}", None).unwrap();
        let summary = catalog.summarize_for(
            LlmUsageSnapshot {
                window_days: 30,
                usage: vec![token(
                    "vendor_cn",
                    "model",
                    BillableTokenClass::UncachedInput,
                    1_000_000,
                )],
            },
            "vendor_global",
            "model",
        );
        assert_eq!(summary.cny_micros, Some(2_000_000));
        assert_eq!(summary.usd_micros, None);
        assert_eq!(summary.pricing_references.len(), 2);
        assert_eq!(summary.estimates.len(), 1);
    }

    #[test]
    fn mixed_fixed_native_prices_need_explicit_fx_for_legacy_total() {
        let mut snapshot = fixture_snapshot();
        snapshot["models"]["vendor_global/model"]["uncached_input"] = serde_json::json!(["1", "1"]);
        let usage = vec![
            token(
                "vendor_cn",
                "model",
                BillableTokenClass::UncachedInput,
                1_000_000,
            ),
            token(
                "vendor_global",
                "model",
                BillableTokenClass::UncachedInput,
                1_000_000,
            ),
        ];
        let without = pricing_from_snapshot(&snapshot.to_string(), "{}", "{}", None)
            .unwrap()
            .summarize(LlmUsageSnapshot {
                window_days: 30,
                usage: usage.clone(),
            });
        assert_eq!((without.usd_micros, without.cny_micros), (None, None));
        let with = pricing_from_snapshot(&snapshot.to_string(), "{}", "{}", Some("2"))
            .unwrap()
            .summarize(LlmUsageSnapshot {
                window_days: 30,
                usage,
            });
        assert_eq!(
            (with.usd_micros, with.cny_micros),
            (Some(2_000_000), Some(4_000_000))
        );
    }

    #[test]
    fn snapshot_rejects_invalid_ranges_dates_sources_and_json() {
        for (field, value) in [
            ("source_url", serde_json::json!("http://example.com")),
            (
                "source_url",
                serde_json::json!("https://secret@example.com"),
            ),
            ("uncached_input", serde_json::json!(["2", "1"])),
            ("output", serde_json::json!(["NaN", "1"])),
            ("unexpected", serde_json::json!(true)),
        ] {
            let mut snapshot = fixture_snapshot();
            snapshot["models"]["vendor_cn/model"][field] = value;
            assert!(pricing_from_snapshot(&snapshot.to_string(), "{}", "{}", None).is_err());
        }
        let mut snapshot = fixture_snapshot();
        snapshot["verified_on"] = serde_json::json!("2026-02-30");
        assert!(pricing_from_snapshot(&snapshot.to_string(), "{}", "{}", None).is_err());
        assert!(
            pricing_from_snapshot(&fixture_snapshot().to_string(), "{}", "{}", Some("0")).is_err()
        );
    }

    #[test]
    fn operator_slash_model_key_is_exact_and_keeps_unknown_cache_unpriced() {
        let overrides = r#"{"siliconflow/vendor/model":{"uncached_input":"0.1","output":"0.2"}}"#;
        let summary = pricing_from_snapshot(&fixture_snapshot().to_string(), overrides, "{}", None)
            .unwrap()
            .summarize(LlmUsageSnapshot {
                window_days: 30,
                usage: vec![
                    token(
                        "siliconflow",
                        "vendor/model",
                        BillableTokenClass::UncachedInput,
                        1_000_000,
                    ),
                    token(
                        "siliconflow",
                        "vendor/model",
                        BillableTokenClass::CachedInput,
                        10,
                    ),
                ],
            });
        assert_eq!(summary.usd_micros, Some(100_000));
        assert_eq!(summary.unpriced_tokens, 10);
        assert_eq!(summary.pricing_references[0].model, "vendor/model");
    }

    #[test]
    fn duplicate_pricing_keys_are_rejected() {
        let duplicate = r#"{"vendor_cn/model":{"uncached_input":"1","output":"1"},"vendor_cn/model":{"uncached_input":"0","output":"0"}}"#;
        assert!(
            pricing_from_snapshot(&fixture_snapshot().to_string(), duplicate, "{}", None).is_err()
        );
    }

    #[test]
    fn overflowing_amount_is_unpriced_instead_of_clamped_or_wrapped() {
        let overrides =
            r#"{"vendor_cn/model":{"uncached_input":"18446744073709.551615","output":"1"}}"#;
        let summary = pricing_from_snapshot(&fixture_snapshot().to_string(), "{}", overrides, None)
            .unwrap()
            .summarize(LlmUsageSnapshot {
                window_days: 30,
                usage: vec![token(
                    "vendor_cn",
                    "model",
                    BillableTokenClass::UncachedInput,
                    u64::MAX,
                )],
            });
        assert!(summary.estimates.is_empty());
        assert_eq!(summary.cny_micros, None);
        assert_eq!(summary.unpriced_tokens, u64::MAX);
    }

    #[test]
    fn compiled_official_snapshot_is_valid_and_keeps_subscription_token_prices_absent() {
        let raw = include_str!("llm-prices.json");
        let catalog = pricing_from_snapshot(raw, "{}", "{}", None).unwrap();
        let snapshot: OfficialPrices = serde_json::from_str(raw).unwrap();
        for provider in snapshot.subscriptions.keys() {
            let summary = catalog.summarize_for(
                LlmUsageSnapshot {
                    window_days: 30,
                    usage: Vec::new(),
                },
                provider,
                "account-model",
            );
            assert!(summary.estimates.is_empty());
            assert!(summary.usd_micros.is_none() && summary.cny_micros.is_none());
            assert_eq!(
                summary.pricing_references[0].billing_kind,
                BillingKind::Subscription
            );
        }
    }

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

    #[test]
    fn usage_query_filters_by_a_non_secret_key_fingerprint() {
        let query = usage_query(30, "top-secret-api-key");
        assert!(query.contains("usage_key=\""));
        assert!(!query.contains("top-secret-api-key"));
    }
}

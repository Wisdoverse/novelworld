use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BillableTokenClass {
    CachedInput,
    UncachedInput,
    Output,
}

impl BillableTokenClass {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "cached_input" => Some(Self::CachedInput),
            "uncached_input" => Some(Self::UncachedInput),
            "output" => Some(Self::Output),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillableTokenUsage {
    pub provider: String,
    pub model: String,
    pub class: BillableTokenClass,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmUsageSnapshot {
    pub window_days: u16,
    pub usage: Vec<BillableTokenUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenPrices {
    pub cached_input_microusd_per_million: Option<u64>,
    pub uncached_input_microusd_per_million: u64,
    pub output_microusd_per_million: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PricingCurrency {
    Usd,
    Cny,
}
impl PricingCurrency {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usd => "USD",
            Self::Cny => "CNY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriceRange {
    pub minimum: u64,
    pub maximum: u64,
}
impl PriceRange {
    pub const fn fixed(value: u64) -> Self {
        Self {
            minimum: value,
            maximum: value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeTokenPrices {
    pub currency: PricingCurrency,
    pub cached_input: Option<PriceRange>,
    pub uncached_input: PriceRange,
    pub output: PriceRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingKind {
    Api,
    Subscription,
    Unavailable,
    Operator,
}
impl BillingKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Subscription => "subscription",
            Self::Unavailable => "unavailable",
            Self::Operator => "operator",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionPlan {
    pub name: String,
    pub currency: PricingCurrency,
    pub monthly_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricingReference {
    pub provider: String,
    pub model: String,
    pub source_url: Option<String>,
    pub verified_on: Option<String>,
    pub note: String,
    pub billing_kind: BillingKind,
    pub plans: Vec<SubscriptionPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostEstimate {
    pub currency: PricingCurrency,
    pub minimum_micros: u64,
    pub maximum_micros: u64,
}

#[derive(Debug, Clone, Default)]
pub struct LlmPricingCatalog {
    prices: HashMap<String, NativeTokenPrices>,
    references: HashMap<String, PricingReference>,
    subscriptions: HashMap<String, PricingReference>,
    usd_cny_micros_per_usd: Option<u64>,
}

impl LlmPricingCatalog {
    pub fn new(prices: HashMap<String, TokenPrices>, usd_cny_micros_per_usd: Option<u64>) -> Self {
        Self::from_native(
            prices
                .into_iter()
                .map(|(key, value)| {
                    (
                        key,
                        NativeTokenPrices {
                            currency: PricingCurrency::Usd,
                            cached_input: value
                                .cached_input_microusd_per_million
                                .map(PriceRange::fixed),
                            uncached_input: PriceRange::fixed(
                                value.uncached_input_microusd_per_million,
                            ),
                            output: PriceRange::fixed(value.output_microusd_per_million),
                        },
                    )
                })
                .collect(),
            HashMap::new(),
            HashMap::new(),
            usd_cny_micros_per_usd,
        )
    }

    pub fn from_native(
        prices: HashMap<String, NativeTokenPrices>,
        references: HashMap<String, PricingReference>,
        subscriptions: HashMap<String, PricingReference>,
        usd_cny_micros_per_usd: Option<u64>,
    ) -> Self {
        Self {
            prices,
            references,
            subscriptions,
            usd_cny_micros_per_usd,
        }
    }

    pub fn summarize(&self, snapshot: LlmUsageSnapshot) -> LlmUsageSummary {
        self.summarize_inner(snapshot, None)
    }

    pub fn summarize_for(
        &self,
        snapshot: LlmUsageSnapshot,
        provider: &str,
        model: &str,
    ) -> LlmUsageSummary {
        self.summarize_inner(snapshot, Some((provider, model)))
    }

    fn reference(&self, provider: &str, model: &str) -> PricingReference {
        if let Some(reference) = self.subscriptions.get(provider) {
            let mut reference = reference.clone();
            reference.model = model.into();
            return reference;
        }
        self.references
            .get(&format!("{provider}/{model}"))
            .cloned()
            .unwrap_or_else(|| PricingReference {
                provider: provider.into(),
                model: model.into(),
                source_url: None,
                verified_on: None,
                note: if self.prices.contains_key(&format!("{provider}/{model}")) {
                    "Operator-configured price; not an official account bill.".into()
                } else {
                    "No verified price for this provider/model.".into()
                },
                billing_kind: if self.prices.contains_key(&format!("{provider}/{model}")) {
                    BillingKind::Operator
                } else {
                    BillingKind::Unavailable
                },
                plans: Vec::new(),
            })
    }

    fn summarize_inner(
        &self,
        snapshot: LlmUsageSnapshot,
        active: Option<(&str, &str)>,
    ) -> LlmUsageSummary {
        let empty = snapshot.usage.is_empty();
        let mut summary = LlmUsageSummary {
            window_days: snapshot.window_days,
            ..Default::default()
        };
        // Accumulate wide amounts; an unrepresentable currency remains unpriced rather than wrapping.
        let mut totals: HashMap<PricingCurrency, (u128, u128, u64)> = HashMap::new();
        let mut references = HashMap::new();
        for item in snapshot.usage {
            match item.class {
                BillableTokenClass::CachedInput => {
                    summary.cached_input_tokens =
                        summary.cached_input_tokens.saturating_add(item.tokens)
                }
                BillableTokenClass::UncachedInput => {
                    summary.uncached_input_tokens =
                        summary.uncached_input_tokens.saturating_add(item.tokens)
                }
                BillableTokenClass::Output => {
                    summary.output_tokens = summary.output_tokens.saturating_add(item.tokens)
                }
            }
            let key = format!("{}/{}", item.provider, item.model);
            references.insert(key.clone(), self.reference(&item.provider, &item.model));
            let price = self
                .prices
                .get(&key)
                .filter(|_| !self.subscriptions.contains_key(&item.provider));
            let rate = price.and_then(|price| match item.class {
                BillableTokenClass::CachedInput => price.cached_input,
                BillableTokenClass::UncachedInput => Some(price.uncached_input),
                BillableTokenClass::Output => Some(price.output),
            });
            if let (Some(price), Some(rate)) = (price, rate) {
                let total = totals.entry(price.currency).or_default();
                total.0 = total
                    .0
                    .saturating_add(multiply_million_wide(item.tokens, rate.minimum));
                total.1 = total
                    .1
                    .saturating_add(multiply_million_wide(item.tokens, rate.maximum));
                total.2 = total.2.saturating_add(item.tokens);
            } else {
                summary.unpriced_tokens = summary.unpriced_tokens.saturating_add(item.tokens);
            }
        }
        if let Some((provider, model)) = active {
            references
                .entry(format!("{provider}/{model}"))
                .or_insert_with(|| self.reference(provider, model));
            if empty && !self.subscriptions.contains_key(provider) {
                if let Some(price) = self.prices.get(&format!("{provider}/{model}")) {
                    totals.entry(price.currency).or_default();
                }
            }
        } else if empty {
            // Preserve the legacy constructor's empty-usage result.
            totals.entry(PricingCurrency::Usd).or_default();
        }
        for currency in [PricingCurrency::Usd, PricingCurrency::Cny] {
            if let Some((minimum, maximum, tokens)) = totals.get(&currency) {
                match (u64::try_from(*minimum), u64::try_from(*maximum)) {
                    (Ok(minimum_micros), Ok(maximum_micros)) => {
                        summary.estimates.push(CostEstimate {
                            currency,
                            minimum_micros,
                            maximum_micros,
                        })
                    }
                    _ => summary.unpriced_tokens = summary.unpriced_tokens.saturating_add(*tokens),
                }
            }
        }
        summary.pricing_references = references.into_values().collect();
        summary
            .pricing_references
            .sort_by(|a, b| (&a.provider, &a.model).cmp(&(&b.provider, &b.model)));
        // Legacy fields remain point estimates, never a flattened range or incomplete mixed-currency total.
        if summary.estimates.len() == totals.len()
            && summary
                .estimates
                .iter()
                .all(|e| e.minimum_micros == e.maximum_micros)
        {
            let usd = summary
                .estimates
                .iter()
                .find(|e| e.currency == PricingCurrency::Usd)
                .map(|e| e.minimum_micros);
            let cny = summary
                .estimates
                .iter()
                .find(|e| e.currency == PricingCurrency::Cny)
                .map(|e| e.minimum_micros);
            match (
                usd,
                cny,
                self.usd_cny_micros_per_usd.filter(|rate| *rate > 0),
            ) {
                (Some(usd), None, _) => {
                    summary.usd_micros = Some(usd);
                    summary.cny_micros = self
                        .usd_cny_micros_per_usd
                        .and_then(|rate| u64::try_from(multiply_million_wide(usd, rate)).ok());
                }
                (None, Some(cny), rate) => {
                    summary.cny_micros = Some(cny);
                    summary.usd_micros = rate.and_then(|rate| divide_million(cny, rate));
                }
                (Some(usd), Some(cny), Some(rate)) => {
                    summary.usd_micros =
                        divide_million(cny, rate).and_then(|converted| usd.checked_add(converted));
                    summary.cny_micros = u64::try_from(multiply_million_wide(usd, rate))
                        .ok()
                        .and_then(|converted| cny.checked_add(converted));
                }
                _ => {}
            }
        }
        summary
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LlmUsageSummary {
    pub window_days: u16,
    pub cached_input_tokens: u64,
    pub uncached_input_tokens: u64,
    pub output_tokens: u64,
    pub unpriced_tokens: u64,
    pub usd_micros: Option<u64>,
    pub cny_micros: Option<u64>,
    pub estimates: Vec<CostEstimate>,
    pub pricing_references: Vec<PricingReference>,
}

impl LlmUsageSummary {
    pub fn input_tokens(&self) -> u64 {
        self.cached_input_tokens
            .saturating_add(self.uncached_input_tokens)
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens().saturating_add(self.output_tokens)
    }
}

fn multiply_million_wide(value: u64, rate: u64) -> u128 {
    ((value as u128) * (rate as u128) + 500_000) / 1_000_000
}

fn divide_million(value: u64, rate: u64) -> Option<u64> {
    u64::try_from(((value as u128) * 1_000_000 + (rate as u128) / 2) / (rate as u128)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_prices_known_models_and_keeps_unknown_tokens_visible() {
        let catalog = LlmPricingCatalog::new(
            HashMap::from([(
                "openai/gpt-4o-mini".into(),
                TokenPrices {
                    cached_input_microusd_per_million: Some(75_000),
                    uncached_input_microusd_per_million: 150_000,
                    output_microusd_per_million: 600_000,
                },
            )]),
            Some(7_200_000),
        );
        let summary = catalog.summarize(LlmUsageSnapshot {
            window_days: 30,
            usage: vec![
                BillableTokenUsage {
                    provider: "openai".into(),
                    model: "gpt-4o-mini".into(),
                    class: BillableTokenClass::UncachedInput,
                    tokens: 1_000_000,
                },
                BillableTokenUsage {
                    provider: "openai".into(),
                    model: "gpt-4o-mini".into(),
                    class: BillableTokenClass::Output,
                    tokens: 500_000,
                },
                BillableTokenUsage {
                    provider: "unknown".into(),
                    model: "model".into(),
                    class: BillableTokenClass::CachedInput,
                    tokens: 42,
                },
            ],
        });

        assert_eq!(summary.total_tokens(), 1_500_042);
        assert_eq!(summary.unpriced_tokens, 42);
        assert_eq!(summary.usd_micros, Some(450_000));
        assert_eq!(summary.cny_micros, Some(3_240_000));
    }
}

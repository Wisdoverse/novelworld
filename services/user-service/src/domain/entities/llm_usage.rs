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

#[derive(Debug, Clone, Default)]
pub struct LlmPricingCatalog {
    prices: HashMap<String, TokenPrices>,
    usd_cny_micros_per_usd: Option<u64>,
}

impl LlmPricingCatalog {
    pub fn new(prices: HashMap<String, TokenPrices>, usd_cny_micros_per_usd: Option<u64>) -> Self {
        Self {
            prices,
            usd_cny_micros_per_usd,
        }
    }

    pub fn summarize(&self, snapshot: LlmUsageSnapshot) -> LlmUsageSummary {
        let mut summary = LlmUsageSummary {
            window_days: snapshot.window_days,
            ..LlmUsageSummary::default()
        };
        let mut priced_tokens = 0_u64;
        let mut usd_micros = 0_u64;

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
            let rate = self.prices.get(&key).and_then(|prices| match item.class {
                BillableTokenClass::CachedInput => prices.cached_input_microusd_per_million,
                BillableTokenClass::UncachedInput => {
                    Some(prices.uncached_input_microusd_per_million)
                }
                BillableTokenClass::Output => Some(prices.output_microusd_per_million),
            });
            if let Some(rate) = rate {
                priced_tokens = priced_tokens.saturating_add(item.tokens);
                usd_micros = usd_micros.saturating_add(multiply_million(item.tokens, rate));
            } else {
                summary.unpriced_tokens = summary.unpriced_tokens.saturating_add(item.tokens);
            }
        }

        if priced_tokens > 0 || summary.total_tokens() == 0 {
            summary.usd_micros = Some(usd_micros);
            summary.cny_micros = self
                .usd_cny_micros_per_usd
                .map(|rate| multiply_million(usd_micros, rate));
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

fn multiply_million(value: u64, rate: u64) -> u64 {
    let rounded = (value as u128)
        .saturating_mul(rate as u128)
        .saturating_add(500_000)
        / 1_000_000;
    rounded.min(u64::MAX as u128) as u64
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

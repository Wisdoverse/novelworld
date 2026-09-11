use std::sync::Mutex;

use llm_client::{MetricsHandle, Usage};
use serde::Serialize;

pub const MODEL: &str = "deepseek-flash";
pub const PROFILE: &str = "vision-diagnostic-budget-v2";
const INPUT_CEILING: u64 = 1 << 20;
const ATTEMPTS_PER_CALL: u64 = 5;
const INPUT_MICRO_CNY: u64 = 4;
const OUTPUT_MICRO_CNY: u64 = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Amount {
    logical_calls: u64,
    attempts: u64,
    tokens: u64,
    cost_micro_cny: u64,
}

const LIMITS: Amount = Amount {
    logical_calls: 40,
    attempts: 200,
    tokens: 20_000_000,
    cost_micro_cny: 35_000_000,
};

impl Amount {
    fn add(self, other: Self) -> Option<Self> {
        Some(Self {
            logical_calls: self.logical_calls.checked_add(other.logical_calls)?,
            attempts: self.attempts.checked_add(other.attempts)?,
            tokens: self.tokens.checked_add(other.tokens)?,
            cost_micro_cny: self.cost_micro_cny.checked_add(other.cost_micro_cny)?,
        })
    }

    fn subtract(self, other: Self) -> Option<Self> {
        Some(Self {
            logical_calls: self.logical_calls.checked_sub(other.logical_calls)?,
            attempts: self.attempts.checked_sub(other.attempts)?,
            tokens: self.tokens.checked_sub(other.tokens)?,
            cost_micro_cny: self.cost_micro_cny.checked_sub(other.cost_micro_cny)?,
        })
    }

    fn within(self, limit: Self) -> bool {
        self.logical_calls <= limit.logical_calls
            && self.attempts <= limit.attempts
            && self.tokens <= limit.tokens
            && self.cost_micro_cny <= limit.cost_micro_cny
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Counters {
    started: u64,
    terminal: u64,
    attempts: u64,
}

impl Counters {
    fn parse(metrics: &str) -> Result<Self, &'static str> {
        let mut counters = Self::default();
        for line in metrics.lines() {
            let target = if line.starts_with("novelworld_llm_requests_started_total{") {
                &mut counters.started
            } else if line.starts_with("novelworld_llm_requests_total{") {
                &mut counters.terminal
            } else if line.starts_with("novelworld_llm_attempts_total{") {
                &mut counters.attempts
            } else {
                continue;
            };
            let value = line
                .rsplit_once(' ')
                .and_then(|(_, value)| value.parse::<u64>().ok())
                .ok_or("diagnostic_metrics_invalid")?;
            *target = target
                .checked_add(value)
                .ok_or("diagnostic_metrics_invalid")?;
        }
        Ok(counters)
    }
}

pub struct Ticket {
    before: Counters,
    output_limit: u32,
}

#[derive(Default)]
struct Ledger {
    // Charged = settled consumption + every unreleased reservation.
    charged: Amount,
    pending: Option<Amount>,
    stopped: Option<&'static str>,
}

impl Ledger {
    fn reserve(&mut self, output_limit: u32, before: Counters) -> Result<Ticket, &'static str> {
        if let Some(code) = self.stopped {
            return Err(code);
        }
        if self.pending.is_some() {
            return Err("diagnostic_concurrent_dispatch");
        }
        if !(1..=8192).contains(&output_limit) {
            return Err("diagnostic_request_invalid");
        }
        if before.started != self.charged.logical_calls
            || before.terminal != before.started
            || before.attempts != self.charged.attempts
        {
            return Err("diagnostic_metrics_invalid");
        }
        let output = u64::from(output_limit);
        // These factors are fixed and bounded above; ledger accumulation is checked.
        let reservation = Amount {
            logical_calls: 1,
            attempts: ATTEMPTS_PER_CALL,
            tokens: ATTEMPTS_PER_CALL * (INPUT_CEILING + output),
            cost_micro_cny: ATTEMPTS_PER_CALL
                * (INPUT_CEILING * INPUT_MICRO_CNY + output * OUTPUT_MICRO_CNY),
        };
        let charged = self
            .charged
            .add(reservation)
            .filter(|charged| charged.within(LIMITS))
            .ok_or("diagnostic_budget_exhausted")?;
        self.charged = charged;
        self.pending = Some(reservation);
        Ok(Ticket {
            before,
            output_limit,
        })
    }

    fn settle(
        &mut self,
        ticket: Ticket,
        after: Counters,
        usages: &[Usage],
    ) -> Result<(), &'static str> {
        if let Some(code) = self.stopped {
            return Err(code);
        }
        let reserved = self.pending.ok_or("diagnostic_metrics_invalid")?;
        let attempts = after
            .attempts
            .checked_sub(ticket.before.attempts)
            .filter(|count| (1..=ATTEMPTS_PER_CALL).contains(count))
            .ok_or("diagnostic_metrics_invalid")?;
        if after.started.checked_sub(ticket.before.started) != Some(1)
            || after.terminal.checked_sub(ticket.before.terminal) != Some(1)
            || u64::try_from(usages.len()).ok() != Some(attempts)
        {
            return Err("diagnostic_evidence_unaccounted");
        }
        let mut actual = Amount {
            logical_calls: 1,
            attempts,
            ..Amount::default()
        };
        for usage in usages {
            let input = u64::from(usage.input_tokens);
            let output = u64::from(usage.output_tokens);
            if input > INPUT_CEILING
                || usage.output_tokens > ticket.output_limit
                || usage
                    .cached_input_tokens
                    .is_some_and(|cached| cached > usage.input_tokens)
            {
                return Err("diagnostic_usage_out_of_bounds");
            }
            actual = actual
                .add(Amount {
                    tokens: input + output,
                    cost_micro_cny: input * INPUT_MICRO_CNY + output * OUTPUT_MICRO_CNY,
                    ..Amount::default()
                })
                .ok_or("diagnostic_usage_out_of_bounds")?;
        }
        if !actual.within(reserved) {
            return Err("diagnostic_usage_out_of_bounds");
        }
        self.charged = self
            .charged
            .subtract(reserved)
            .and_then(|charged| charged.add(actual))
            .filter(|charged| charged.within(LIMITS))
            .ok_or("diagnostic_usage_out_of_bounds")?;
        self.pending = None;
        Ok(())
    }
}

pub struct Control {
    ledger: Mutex<Ledger>,
    metrics: MetricsHandle,
}

#[derive(Debug, Serialize)]
pub struct Report {
    profile: &'static str,
    limits: Amount,
    charged: Amount,
    unreleased_reservation: Option<Amount>,
    stopped: Option<&'static str>,
}

impl Control {
    pub fn new(metrics: MetricsHandle) -> Self {
        Self {
            ledger: Mutex::new(Ledger::default()),
            metrics,
        }
    }

    pub fn begin(&self, output_limit: u32) -> Result<Ticket, &'static str> {
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| "diagnostic_budget_lock_failed")?;
        let result = Counters::parse(&self.metrics.render())
            .and_then(|before| ledger.reserve(output_limit, before));
        if let Err(code) = result {
            ledger.stopped.get_or_insert(code);
        }
        result
    }

    pub fn finish(&self, ticket: Ticket, usages: &[Usage]) -> Result<(), &'static str> {
        let mut ledger = self
            .ledger
            .lock()
            .map_err(|_| "diagnostic_budget_lock_failed")?;
        let result = Counters::parse(&self.metrics.render())
            .and_then(|after| ledger.settle(ticket, after, usages));
        if let Err(code) = result {
            ledger.stopped.get_or_insert(code);
        }
        result
    }

    pub fn stop(&self, code: &'static str) {
        if let Ok(mut ledger) = self.ledger.lock() {
            ledger.stopped.get_or_insert(code);
        }
    }

    pub fn report(&self) -> Result<Report, &'static str> {
        let ledger = self
            .ledger
            .lock()
            .map_err(|_| "diagnostic_budget_lock_failed")?;
        Ok(Report {
            profile: PROFILE,
            limits: LIMITS,
            charged: ledger.charged,
            unreleased_reservation: ledger.pending,
            stopped: ledger.stopped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(attempts: u64) -> Counters {
        Counters {
            started: 1,
            terminal: 1,
            attempts,
        }
    }

    fn usage(input: u32, output: u32) -> Usage {
        Usage::new(input, output, None).unwrap()
    }

    #[test]
    fn reserves_before_dispatch_and_only_settles_complete_evidence() {
        assert_eq!(MODEL, "deepseek-flash");
        assert_eq!(PROFILE, "vision-diagnostic-budget-v2");
        let mut ledger = Ledger::default();
        let ticket = ledger.reserve(8192, Counters::default()).unwrap();
        assert_eq!(ledger.charged.tokens, 5_283_840);
        assert_eq!(ledger.charged.cost_micro_cny, 21_463_040);
        assert_eq!(ledger.charged.attempts, 5);
        assert!(ledger.reserve(8192, Counters::default()).is_err());
        ledger
            .settle(ticket, counters(2), &[usage(3, 0), usage(4, 2)])
            .unwrap();
        assert_eq!(
            ledger.charged,
            Amount {
                logical_calls: 1,
                attempts: 2,
                tokens: 9,
                cost_micro_cny: 52,
            }
        );
        assert!(ledger.pending.is_none());

        for (after, usages) in [
            (counters(2), vec![usage(3, 2)]), // a missing-header attempt before success
            (counters(1), vec![]),            // missing usage
            (counters(1), vec![usage(1 << 20, 8193)]),
            (
                Counters {
                    terminal: 0,
                    ..counters(1)
                },
                vec![usage(3, 2)],
            ),
            (counters(6), vec![usage(3, 2); 6]),
        ] {
            let mut ledger = Ledger::default();
            let ticket = ledger.reserve(8192, Counters::default()).unwrap();
            let charged = ledger.charged;
            assert!(ledger.settle(ticket, after, &usages).is_err());
            assert_eq!(ledger.charged, charged);
            assert!(ledger.pending.is_some());
        }
    }

    #[test]
    fn every_ceiling_allows_equality_and_rejects_one_more() {
        let mut initial = Ledger::default();
        initial.reserve(8192, Counters::default()).unwrap();
        let reservation = initial.charged;
        for dimension in 0..4 {
            for extra in [0, 1] {
                let mut charged = Amount::default();
                match dimension {
                    0 => {
                        charged.logical_calls =
                            LIMITS.logical_calls - reservation.logical_calls + extra
                    }
                    1 => charged.attempts = LIMITS.attempts - reservation.attempts + extra,
                    2 => charged.tokens = LIMITS.tokens - reservation.tokens + extra,
                    _ => {
                        charged.cost_micro_cny =
                            LIMITS.cost_micro_cny - reservation.cost_micro_cny + extra
                    }
                }
                let before = Counters {
                    started: charged.logical_calls,
                    terminal: charged.logical_calls,
                    attempts: charged.attempts,
                };
                let mut ledger = Ledger {
                    charged,
                    ..Ledger::default()
                };
                assert_eq!(ledger.reserve(8192, before).is_ok(), extra == 0);
                if extra == 1 {
                    assert_eq!(ledger.charged, charged);
                    assert!(ledger.pending.is_none());
                }
            }
        }
        assert!(Amount {
            tokens: u64::MAX,
            ..Amount::default()
        }
        .add(Amount {
            tokens: 1,
            ..Amount::default()
        })
        .is_none());
    }

    #[test]
    fn metrics_require_integer_monotonic_accounting() {
        let parsed = Counters::parse(
            "novelworld_llm_requests_started_total{mode=\"sync\"} 1\n\
             novelworld_llm_requests_total{status=\"success\"} 1\n\
             novelworld_llm_attempts_total{status=\"success\"} 1\n\
             novelworld_llm_attempts_total{status=\"empty_json_mode\"} 1\n",
        )
        .unwrap();
        assert_eq!(
            (parsed.started, parsed.terminal, parsed.attempts),
            (1, 1, 2)
        );
        for invalid in ["NaN", "inf", "-1", "1.5", "18446744073709551616"] {
            assert!(
                Counters::parse(&format!("novelworld_llm_attempts_total{{}} {invalid}")).is_err()
            );
        }
        assert!(Ledger::default().reserve(8192, parsed).is_err());
    }

    #[test]
    fn bounded_chat_enforces_network_and_evidence_boundaries() {
        use crate::tests::{envelope, evidence_server};
        use crate::{private_request, Mode, PrivateResponseSink, RunConfig};
        use llm_client::{ChatRequest, LlmOperation, RuntimeLlmClient};
        use std::{collections::BTreeSet, env, process::Command, sync::atomic::Ordering};

        const CHILD: &str = "NOVELWORLD_BUDGET_TEST_CASE";
        let Ok(case) = env::var(CHILD) else {
            // Each child owns a fresh real recorder; unrelated parallel tests cannot
            // change its counters. No new test dependency or runtime metric hook.
            for case in [
                "success",
                "fallback",
                "exhausted",
                "missing_usage",
                "missing_headers",
                "metrics_mismatch",
                "write_failure",
                "cancelled",
            ] {
                let output = Command::new(env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "budget::tests::bounded_chat_enforces_network_and_evidence_boundaries",
                        "--nocapture",
                    ])
                    .env(CHILD, case)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{case}: {} {}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        };
        let metrics = llm_client::install_metrics("h1-eval").unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let success = Some(envelope(MODEL, "{}", true));
            let bodies = match case.as_str() {
                "fallback" => vec![Some(envelope(MODEL, "", true)), success],
                "missing_usage" => vec![Some(envelope(MODEL, "{}", false))],
                "missing_headers" | "cancelled" => vec![None, success],
                _ => vec![success],
            };
            let (_, server, calls, url) = evidence_server(bodies).await;
            let path = env::temp_dir().join(format!("h1-budget-{}.jsonl", uuid::Uuid::new_v4()));
            let sink = PrivateResponseSink::create(&path).unwrap();
            let control = Control::new(metrics);
            if case == "exhausted" {
                control.ledger.lock().unwrap().charged.cost_micro_cny = LIMITS.cost_micro_cny;
            } else if case == "metrics_mismatch" {
                control.ledger.lock().unwrap().charged.logical_calls = 1;
            } else if case == "write_failure" {
                sink.0.lock().unwrap().writer =
                    std::io::BufWriter::new(std::fs::File::open(&path).unwrap());
            }
            let config = RunConfig {
                mode: Mode::Live,
                provider: "deepseek".into(),
                model: MODEL.into(),
                allowed_response_models: BTreeSet::from([MODEL.into()]),
                client: Some(RuntimeLlmClient::static_config(
                    url,
                    MODEL.into(),
                    "synthetic-key".into(),
                    false,
                )),
                budget: Some(control),
            };
            let request = || {
                private_request(
                    Some(&sink),
                    "budget-case",
                    1,
                    &config.allowed_response_models,
                    ChatRequest::new(LlmOperation::CharacterExtraction, MODEL)
                        .max_tokens(20)
                        .json(),
                )
                .unwrap()
            };
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(if case == "cancelled" { 100 } else { 10_000 }),
                config.chat(Some(&sink), request()),
            )
            .await;
            let healthy = matches!(case.as_str(), "success" | "fallback");
            if case == "cancelled" {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap().is_ok(), healthy);
            }
            let count = calls.load(Ordering::SeqCst);
            let control = config.budget.as_ref().unwrap();
            let report = control.report().unwrap();
            if healthy {
                let expected = if case == "fallback" { 2 } else { 1 };
                assert_eq!(count, expected);
                assert_eq!(report.charged.logical_calls, 1);
                assert_eq!(report.charged.attempts, expected as u64);
                assert_eq!(report.charged.cost_micro_cny, expected as u64 * 36);
                assert_eq!(report.charged.tokens, expected as u64 * 5);
                assert!(report.unreleased_reservation.is_none());
                assert!(report.stopped.is_none());
                config.chat(Some(&sink), request()).await.unwrap();
                assert_eq!(calls.load(Ordering::SeqCst), count + 1);
            } else {
                if matches!(case.as_str(), "exhausted" | "metrics_mismatch") {
                    assert_eq!(count, 0);
                } else {
                    assert!(report.unreleased_reservation.is_some());
                    assert_eq!(report.charged.attempts, 5);
                    if case == "missing_headers" {
                        assert_eq!(count, 2);
                    }
                }
                // Bypass the existing private_request fail guard deliberately:
                // the budget boundary itself must refuse the next dispatch.
                assert!(config
                    .chat(
                        Some(&sink),
                        ChatRequest::new(LlmOperation::CharacterExtraction, MODEL).max_tokens(20)
                    )
                    .await
                    .is_err());
                assert_eq!(calls.load(Ordering::SeqCst), count);
                assert_eq!(control.report().unwrap().charged, report.charged);
                assert!(control.report().unwrap().stopped.is_some());
            }
            server.abort();
            drop(sink);
            std::fs::remove_file(path).unwrap();
        });
    }
}

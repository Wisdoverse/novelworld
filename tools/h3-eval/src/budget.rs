use std::{fs::File, io::Write, sync::Mutex};

use llm_client::{MetricsHandle, Usage};
use serde::Serialize;

pub const MODEL: &str = "deepseek-flash";
pub const PROFILE: &str = "h3-vision-calibration-diagnostic-v2";
pub const INPUT_CEILING: u64 = 1 << 20;
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
    logical_calls: 8,
    attempts: 40,
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

#[derive(Clone, Default)]
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
        if output_limit != 800 {
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
    state: Mutex<State>,
    metrics: MetricsHandle,
}

struct State {
    ledger: Ledger,
    journal: File,
    records: usize,
    #[cfg(test)]
    fail_sync: bool,
}

impl State {
    fn record(&mut self, event: &str, case: &str, ledger: &Ledger) -> Result<(), &'static str> {
        if self.records >= 32 || case.len() > 100 {
            return Err("diagnostic_journal_bound");
        }
        let record = serde_json::json!({
            "schema_version": 1, "sequence": self.records + 1, "event": event,
            "case_id": case, "ticket": ledger.charged.logical_calls,
            "charged": ledger.charged, "unreleased_reservation": ledger.pending,
            "stopped": ledger.stopped,
        });
        let mut bytes = super::diagnostic::bounded_json(&record, 4095)
            .map_err(|_| "diagnostic_journal_bound")?;
        bytes.push(b'\n');
        self.journal
            .write_all(&bytes)
            .map_err(|_| "diagnostic_journal_write_failed")?;
        #[cfg(test)]
        if self.fail_sync {
            return Err("diagnostic_journal_sync_failed");
        }
        self.journal
            .sync_all()
            .map_err(|_| "diagnostic_journal_sync_failed")?;
        self.records += 1;
        Ok(())
    }
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
    pub fn new(metrics: MetricsHandle, journal: File) -> Self {
        Self {
            state: Mutex::new(State {
                ledger: Ledger::default(),
                journal,
                records: 0,
                #[cfg(test)]
                fail_sync: false,
            }),
            metrics,
        }
    }

    pub fn begin(&self, case: &str, output_limit: u32) -> Result<Ticket, &'static str> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "diagnostic_budget_lock_failed")?;
        let result = Counters::parse(&self.metrics.render())
            .and_then(|before| state.ledger.reserve(output_limit, before))
            .and_then(|ticket| {
                let pending = state.ledger.clone();
                state.record("reserve", case, &pending)?;
                Ok(ticket)
            });
        if let Err(code) = result {
            state.ledger.stopped.get_or_insert(code);
        }
        result
    }

    pub fn finish(&self, case: &str, ticket: Ticket, usages: &[Usage]) -> Result<(), &'static str> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "diagnostic_budget_lock_failed")?;
        // Do not refund the in-memory ticket until the settlement is durable.
        let mut settled = state.ledger.clone();
        let result = Counters::parse(&self.metrics.render())
            .and_then(|after| settled.settle(ticket, after, usages))
            .and_then(|()| state.record("settle", case, &settled));
        match result {
            Ok(()) => state.ledger = settled,
            Err(code) => {
                state.ledger.stopped.get_or_insert(code);
            }
        }
        result
    }

    pub fn stop(&self, code: &'static str) {
        if let Ok(mut state) = self.state.lock() {
            if state.ledger.stopped.is_none() {
                state.ledger.stopped = Some(code);
                let stopped = state.ledger.clone();
                let _ = state.record("stop", "", &stopped);
            }
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.state
            .lock()
            .map_or(true, |state| state.ledger.stopped.is_some())
    }

    pub fn report(&self) -> Result<Report, &'static str> {
        let state = self
            .state
            .lock()
            .map_err(|_| "diagnostic_budget_lock_failed")?;
        Ok(Report {
            profile: PROFILE,
            limits: LIMITS,
            charged: state.ledger.charged,
            unreleased_reservation: state.ledger.pending,
            stopped: state.ledger.stopped,
        })
    }
}

#[cfg(test)]
impl Control {
    pub(super) fn test_fault(&self, fault: &str, path: &std::path::Path) {
        if fault == "poison" {
            let _ = std::panic::catch_unwind(|| {
                let _guard = self.state.lock().unwrap();
                panic!("synthetic poisoned ledger");
            });
            return;
        }
        let mut state = self.state.lock().unwrap();
        match fault {
            "reserve_write" => state.journal = File::open(path).unwrap(),
            "reserve_sync" => state.fail_sync = true,
            "exhausted" => state.ledger.charged.cost_micro_cny = LIMITS.cost_micro_cny,
            _ => panic!("unknown synthetic fault"),
        }
    }
}

#[cfg(test)]
impl Control {
    pub(super) fn test_settle_without_provider(&self, case: &str) {
        let mut state = self.state.lock().unwrap();
        let mut settled = state.ledger.clone();
        settled
            .settle(
                Ticket {
                    before: Counters::default(),
                    output_limit: 800,
                },
                Counters {
                    started: 1,
                    terminal: 1,
                    attempts: 1,
                },
                &[Usage::new(3, 2, None).unwrap()],
            )
            .unwrap();
        state.record("settle", case, &settled).unwrap();
        state.ledger = settled;
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
        assert_eq!(PROFILE, "h3-vision-calibration-diagnostic-v2");
        let mut ledger = Ledger::default();
        let ticket = ledger.reserve(800, Counters::default()).unwrap();
        assert_eq!(ledger.charged.tokens, 5_246_880);
        assert_eq!(ledger.charged.cost_micro_cny, 21_019_520);
        assert_eq!(ledger.charged.attempts, 5);
        assert!(ledger.reserve(800, Counters::default()).is_err());
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
            (counters(1), vec![usage(1 << 20, 801)]),
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
            let ticket = ledger.reserve(800, Counters::default()).unwrap();
            let charged = ledger.charged;
            assert!(ledger.settle(ticket, after, &usages).is_err());
            assert_eq!(ledger.charged, charged);
            assert!(ledger.pending.is_some());
        }
    }

    #[test]
    fn every_ceiling_allows_equality_and_rejects_one_more() {
        let mut initial = Ledger::default();
        initial.reserve(800, Counters::default()).unwrap();
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
                assert_eq!(ledger.reserve(800, before).is_ok(), extra == 0);
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
        assert!(Ledger::default().reserve(800, parsed).is_err());
    }
}

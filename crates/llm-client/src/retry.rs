use std::time::Duration;

const MAX_RETRIES: u32 = 3;
const RETRY_DELAYS: [u64; 3] = [1, 2, 4];

pub struct RetryPolicy;

impl RetryPolicy {
    pub fn should_retry(status: u16, attempt: u32) -> bool {
        attempt < MAX_RETRIES && (status == 429 || status >= 500)
    }

    pub fn delay(status: u16, attempt: u32, retry_after: Option<&str>) -> Duration {
        if status == 429 {
            if let Some(secs) = retry_after.and_then(|v| v.parse::<u64>().ok()) {
                return Duration::from_secs(secs);
            }
        }
        Duration::from_secs(RETRY_DELAYS[attempt as usize])
    }

    pub fn max_retries() -> u32 {
        MAX_RETRIES
    }
}

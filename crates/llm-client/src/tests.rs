#[cfg(test)]
mod tests {
    use crate::retry::RetryPolicy;

    #[test]
    fn test_should_retry_on_429() {
        assert!(RetryPolicy::should_retry(429, 0));
        assert!(RetryPolicy::should_retry(429, 1));
        assert!(RetryPolicy::should_retry(429, 2));
        assert!(!RetryPolicy::should_retry(429, 3)); // exceeds max
    }

    #[test]
    fn test_should_retry_on_5xx() {
        assert!(RetryPolicy::should_retry(500, 0));
        assert!(RetryPolicy::should_retry(502, 0));
        assert!(RetryPolicy::should_retry(503, 0));
    }

    #[test]
    fn test_should_not_retry_on_4xx() {
        assert!(!RetryPolicy::should_retry(400, 0));
        assert!(!RetryPolicy::should_retry(401, 0));
        assert!(!RetryPolicy::should_retry(403, 0));
        assert!(!RetryPolicy::should_retry(404, 0));
    }

    #[test]
    fn test_retry_delay() {
        let d = RetryPolicy::delay(500, 0, None);
        assert_eq!(d.as_secs(), 1);
        let d = RetryPolicy::delay(500, 1, None);
        assert_eq!(d.as_secs(), 2);
        let d = RetryPolicy::delay(500, 2, None);
        assert_eq!(d.as_secs(), 4);
    }

    #[test]
    fn test_retry_after_header() {
        let d = RetryPolicy::delay(429, 0, Some("30"));
        assert_eq!(d.as_secs(), 30);
    }
}

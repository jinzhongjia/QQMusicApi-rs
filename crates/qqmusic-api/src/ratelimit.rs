//! Token bucket rate limiter applied to every outgoing request.
//!
//! Bursty traffic is one of the triggers of QQ Music's risk control
//! (`code 2001`). The default (10 req/s, burst 50) matches upstream.

use std::time::Duration;

use std::sync::Mutex;

use tokio::time::Instant;

use crate::utils::lock_sync;

/// Rate limit settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimit {
    /// Tokens added per second.
    pub rate: f64,
    /// Bucket capacity (maximum burst).
    pub capacity: f64,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self { rate: 10.0, capacity: 50.0 }
    }
}

/// Async token bucket.
///
/// The bucket state sits behind a `std` mutex held only for the arithmetic,
/// never across `.await`, so contended callers do not queue on async lock
/// hand-offs.
#[derive(Debug)]
pub struct TokenBucket {
    limit: RateLimit,
    state: Mutex<(f64, Instant)>,
}

impl TokenBucket {
    /// Full bucket.
    ///
    /// # Panics
    ///
    /// Panics if `rate` or `capacity` is not positive.
    pub fn new(limit: RateLimit) -> Self {
        assert!(limit.rate > 0.0 && limit.capacity >= 1.0, "invalid rate limit");
        Self { limit, state: Mutex::new((limit.capacity, Instant::now())) }
    }

    /// Wait for one token.
    pub async fn acquire(&self) {
        let wait = {
            let mut state = lock_sync(&self.state);
            let now = Instant::now();
            let elapsed = now.duration_since(state.1).as_secs_f64();
            state.0 = (state.0 + elapsed * self.limit.rate).min(self.limit.capacity);
            state.1 = now;
            state.0 -= 1.0;
            if state.0 >= 0.0 { None } else { Some(Duration::from_secs_f64(-state.0 / self.limit.rate)) }
        };
        if let Some(wait) = wait {
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn burst_then_throttle() {
        let bucket = TokenBucket::new(RateLimit { rate: 2.0, capacity: 2.0 });
        let start = Instant::now();
        bucket.acquire().await;
        bucket.acquire().await;
        assert!(start.elapsed() < Duration::from_millis(10));
        bucket.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(490));
        bucket.acquire().await;
        assert!(start.elapsed() >= Duration::from_millis(990));
    }

    #[tokio::test(start_paused = true)]
    async fn refills_over_time() {
        let bucket = TokenBucket::new(RateLimit { rate: 1.0, capacity: 1.0 });
        bucket.acquire().await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        let start = Instant::now();
        bucket.acquire().await;
        assert!(start.elapsed() < Duration::from_millis(10));
    }

    #[test]
    #[should_panic(expected = "invalid rate limit")]
    fn rejects_invalid() {
        let _ = TokenBucket::new(RateLimit { rate: 0.0, capacity: 1.0 });
    }
}

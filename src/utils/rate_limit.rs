use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<u64, Instant>>>,
    cooldown: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cooldown {
    /// Seconds the caller should wait before retrying.
    pub retry_after_secs: u64,
}

impl RateLimiter {
    pub fn new(cooldown_secs: u64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            cooldown: Duration::from_secs(cooldown_secs.max(1)),
        }
    }

    /// Returns `Ok(())` if `user_id` may proceed, otherwise the remaining cooldown.
    pub async fn check(&self, user_id: u64) -> Result<(), Cooldown> {
        let mut guard = self.inner.lock().await;
        let now = Instant::now();
        if let Some(last) = guard.get(&user_id) {
            if let Some(remaining) = self.cooldown.checked_sub(now.duration_since(*last)) {
                if !remaining.is_zero() {
                    return Err(Cooldown {
                        retry_after_secs: remaining.as_secs().max(1),
                    });
                }
            }
        }
        guard.insert(user_id, now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn second_immediate_call_is_limited() {
        let limiter = RateLimiter::new(60);
        assert!(limiter.check(1).await.is_ok());
        assert!(limiter.check(1).await.is_err());
        // Different user is unaffected.
        assert!(limiter.check(2).await.is_ok());
    }
}

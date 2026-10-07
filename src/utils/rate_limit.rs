use dashmap::DashMap;
use std::time::{Duration, Instant};

/// Per-user cooldown tracker. Lock-free across users (sharded map) so it
/// never becomes a bottleneck under heavy concurrent load.
#[derive(Debug)]
pub struct RateLimiter {
    last_seen: DashMap<u64, Instant>,
    default_cooldown: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cooldown {
    pub retry_after_secs: u64,
}

const PRUNE_THRESHOLD: usize = 10_000;
const PRUNE_AGE: Duration = Duration::from_secs(3600);

impl RateLimiter {
    pub fn new(cooldown_secs: u64) -> Self {
        Self {
            last_seen: DashMap::new(),
            default_cooldown: Duration::from_secs(cooldown_secs.max(1)),
        }
    }

    pub fn default_cooldown(&self) -> Duration {
        self.default_cooldown
    }

    /// `Ok(())` when `user_id` may proceed (and records the hit), otherwise
    /// the remaining cooldown. `cooldown` overrides the default when set.
    pub fn check(&self, user_id: u64, cooldown: Option<Duration>) -> Result<(), Cooldown> {
        let cooldown = cooldown.unwrap_or(self.default_cooldown);
        let now = Instant::now();
        let mut entry = self.last_seen.entry(user_id).or_insert(now - cooldown * 2);
        let elapsed = now.duration_since(*entry);
        if elapsed < cooldown {
            let remaining = cooldown - elapsed;
            return Err(Cooldown {
                retry_after_secs: remaining.as_secs().max(1),
            });
        }
        *entry = now;
        drop(entry);
        if self.last_seen.len() > PRUNE_THRESHOLD {
            self.last_seen
                .retain(|_, t| now.duration_since(*t) < PRUNE_AGE);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_immediate_call_is_limited() {
        let limiter = RateLimiter::new(60);
        assert!(limiter.check(1, None).is_ok());
        assert!(limiter.check(1, None).is_err());
        assert!(limiter.check(2, None).is_ok());
    }

    #[test]
    fn override_cooldown_applies() {
        let limiter = RateLimiter::new(60);
        assert!(limiter.check(1, Some(Duration::from_millis(1))).is_ok());
        std::thread::sleep(Duration::from_millis(5));
        assert!(limiter.check(1, Some(Duration::from_millis(1))).is_ok());
    }
}

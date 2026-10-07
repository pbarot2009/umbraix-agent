use dashmap::DashMap;
use std::{
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Admission control for agent turns.
///
/// - Global semaphore caps concurrent turns (default 128) so a traffic burst
///   queues instead of exhausting memory/sockets.
/// - One active turn per user prevents a single user from flooding.
/// - Queued requests wait up to `queue_timeout` before being told to retry.
pub struct TurnLimiter {
    global: Arc<Semaphore>,
    capacity: usize,
    active_users: Arc<DashMap<u64, ()>>,
    queue_timeout: Duration,
    queued: AtomicUsize,
}

pub struct UserSlot {
    users: Arc<DashMap<u64, ()>>,
    user_id: u64,
}

impl Drop for UserSlot {
    fn drop(&mut self) {
        self.users.remove(&self.user_id);
    }
}

pub enum Admission {
    Immediate(OwnedSemaphorePermit),
    Queued,
}

impl TurnLimiter {
    pub fn new(capacity: usize, queue_timeout: Duration) -> Self {
        Self {
            global: Arc::new(Semaphore::new(capacity)),
            capacity,
            active_users: Arc::new(DashMap::new()),
            queue_timeout,
            queued: AtomicUsize::new(0),
        }
    }

    pub fn try_claim_user(&self, user_id: u64) -> Option<UserSlot> {
        use dashmap::mapref::entry::Entry;
        match self.active_users.entry(user_id) {
            Entry::Occupied(_) => None,
            Entry::Vacant(v) => {
                v.insert(());
                Some(UserSlot {
                    users: self.active_users.clone(),
                    user_id,
                })
            }
        }
    }

    pub fn try_admit(&self) -> Admission {
        match self.global.clone().try_acquire_owned() {
            Ok(p) => Admission::Immediate(p),
            Err(_) => Admission::Queued,
        }
    }

    /// Wait in line for a slot. `None` on timeout or shutdown.
    pub async fn wait_admit(&self) -> Option<OwnedSemaphorePermit> {
        self.queued.fetch_add(1, Ordering::Relaxed);
        let res =
            tokio::time::timeout(self.queue_timeout, self.global.clone().acquire_owned()).await;
        self.queued.fetch_sub(1, Ordering::Relaxed);
        res.ok().and_then(|r| r.ok())
    }

    pub fn active(&self) -> usize {
        self.capacity - self.global.available_permits()
    }

    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Wait for in-flight turns to finish (graceful shutdown).
    pub async fn drain(&self, timeout: Duration) -> bool {
        let all = self.capacity as u32;
        let res = tokio::time::timeout(timeout, self.global.acquire_many(all)).await;
        match res {
            Ok(Ok(permit)) => {
                permit.forget();
                self.global.close();
                true
            }
            _ => false,
        }
    }
}

/// Process-wide counters surfaced by `!stats` and logs.
#[derive(Debug, Default)]
pub struct Metrics {
    pub turns_started: AtomicU64,
    pub turns_ok: AtomicU64,
    pub turns_failed: AtomicU64,
    pub tool_calls: AtomicU64,
    pub tool_denied: AtomicU64,
    pub commands: AtomicU64,
    pub queued_total: AtomicU64,
    pub rejected_busy: AtomicU64,
}

impl Metrics {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_turn_per_user() {
        let l = TurnLimiter::new(4, Duration::from_secs(1));
        let slot = l.try_claim_user(1).unwrap();
        assert!(l.try_claim_user(1).is_none());
        assert!(l.try_claim_user(2).is_some());
        drop(slot);
        assert!(l.try_claim_user(1).is_some());
    }

    #[tokio::test]
    async fn global_capacity_queues_then_times_out() {
        let l = TurnLimiter::new(1, Duration::from_millis(50));
        let p = match l.try_admit() {
            Admission::Immediate(p) => p,
            Admission::Queued => panic!("should admit"),
        };
        assert_eq!(l.active(), 1);
        assert!(matches!(l.try_admit(), Admission::Queued));
        assert!(l.wait_admit().await.is_none());
        drop(p);
        assert!(l.wait_admit().await.is_some());
    }

    #[tokio::test]
    async fn drain_waits_for_inflight() {
        let l = Arc::new(TurnLimiter::new(2, Duration::from_secs(1)));
        let p = match l.try_admit() {
            Admission::Immediate(p) => p,
            Admission::Queued => panic!(),
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(p);
        });
        assert!(l.drain(Duration::from_secs(2)).await);
    }
}

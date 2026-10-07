use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::Mutex;

/// Conversation key: `(channel_id, user_id)`.
///
/// History is per user *and* per channel so one user's conversation is
/// never replayed to Gemini under another user's API key.
pub type MemoryKey = (u64, u64);

/// Bounded multi-turn history replayed verbatim to Gemini.
///
/// Stores raw Gemini `Content` objects and guarantees the retained window
/// always starts at a plain user turn, so a trimmed history never begins
/// with an orphaned `functionResponse` (which Gemini rejects with HTTP 400).
#[derive(Debug, Clone)]
pub struct ConversationMemory {
    inner: Arc<Mutex<State>>,
    limit: usize,
}

#[derive(Debug, Default)]
struct State {
    map: HashMap<MemoryKey, VecDeque<Value>>,
    touched: VecDeque<MemoryKey>,
}

const MAX_CONVERSATIONS: usize = 5_000;

fn is_plain_user_turn(v: &Value) -> bool {
    v.get("role").and_then(|r| r.as_str()) == Some("user")
        && v.get("parts")
            .and_then(|p| p.as_array())
            .is_some_and(|parts| {
                !parts.is_empty() && parts.iter().all(|p| p.get("functionResponse").is_none())
            })
}

fn trim_to_user_start(queue: &mut VecDeque<Value>) {
    while queue.front().is_some_and(|v| !is_plain_user_turn(v)) {
        queue.pop_front();
    }
}

impl ConversationMemory {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State::default())),
            limit,
        }
    }

    pub async fn history(&self, key: MemoryKey) -> Vec<Value> {
        let guard = self.inner.lock().await;
        guard
            .map
            .get(&key)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Append one turn's entries (user turn, model calls, tool responses,
    /// final answer) in chronological order.
    pub async fn record(&self, key: MemoryKey, new_entries: Vec<Value>) {
        if self.limit == 0 || new_entries.is_empty() {
            return;
        }
        // Bound a single turn: cap entries and per-entry size so one huge
        // tool dump can't blow memory (large outputs are truncated upstream,
        // this is defence-in-depth).
        const MAX_ENTRY_CHARS: usize = 24_000;
        let mut entries = new_entries;
        if entries.len() > 512 {
            entries.truncate(512);
        }
        for e in &mut entries {
            let s = e.to_string();
            if s.len() > MAX_ENTRY_CHARS {
                *e = serde_json::json!({"role": e.get("role").cloned().unwrap_or_default(), "parts": [{"text": "[truncated: entry too large]"}]});
            }
        }
        let mut guard = self.inner.lock().await;
        if !guard.map.contains_key(&key) {
            // Evict until under cap (stale `touched` entries may no-op).
            while guard.map.len() >= MAX_CONVERSATIONS {
                let Some(victim) = guard.touched.pop_front() else {
                    break;
                };
                guard.map.remove(&victim);
            }
        }
        guard.touched.retain(|k| *k != key);
        guard.touched.push_back(key);
        // Periodic prune of stale touched keys (keeps retain() cheap).
        if guard.touched.len() > MAX_CONVERSATIONS * 2 {
            let live: std::collections::HashSet<MemoryKey> = guard.map.keys().copied().collect();
            guard.touched.retain(|k| live.contains(k));
        }
        let limit = self.limit;
        let queue = guard.map.entry(key).or_default();
        queue.extend(entries);
        while queue.len() > limit {
            queue.pop_front();
        }
        trim_to_user_start(queue);
        if queue.is_empty() {
            guard.map.remove(&key);
            guard.touched.retain(|k| *k != key);
        }
    }

    pub async fn len(&self, key: MemoryKey) -> usize {
        self.inner.lock().await.map.get(&key).map_or(0, |q| q.len())
    }

    pub async fn clear(&self, key: MemoryKey) {
        let mut guard = self.inner.lock().await;
        guard.map.remove(&key);
        guard.touched.retain(|k| *k != key);
    }

    pub async fn conversations(&self) -> usize {
        self.inner.lock().await.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(t: &str) -> Value {
        json!({"role": "user", "parts": [{"text": t}]})
    }
    fn model_call() -> Value {
        json!({"role": "model", "parts": [{"functionCall": {"name": "x", "args": {}}}]})
    }
    fn tool_resp() -> Value {
        json!({"role": "user", "parts": [{"functionResponse": {"name": "x", "response": {}}}]})
    }
    fn model_text() -> Value {
        json!({"role": "model", "parts": [{"text": "ok"}]})
    }

    #[tokio::test]
    async fn history_is_bounded_and_starts_with_user() {
        let mem = ConversationMemory::new(4);
        mem.record((1, 1), vec![user("a"), model_text()]).await;
        mem.record(
            (1, 1),
            vec![user("b"), model_call(), tool_resp(), model_text()],
        )
        .await;
        let h = mem.history((1, 1)).await;
        assert!(h.len() <= 4);
        assert!(is_plain_user_turn(&h[0]));
    }

    #[tokio::test]
    async fn never_starts_with_orphan_tool_response() {
        let mem = ConversationMemory::new(3);
        mem.record(
            (1, 1),
            vec![
                user("a"),
                model_call(),
                tool_resp(),
                model_call(),
                tool_resp(),
                model_text(),
            ],
        )
        .await;
        let h = mem.history((1, 1)).await;
        assert!(h.is_empty() || is_plain_user_turn(&h[0]));
    }

    #[tokio::test]
    async fn users_and_channels_are_isolated() {
        let mem = ConversationMemory::new(10);
        mem.record((1, 1), vec![user("a")]).await;
        assert_eq!(mem.len((1, 2)).await, 0);
        assert_eq!(mem.len((2, 1)).await, 0);
        assert_eq!(mem.len((1, 1)).await, 1);
        mem.clear((1, 1)).await;
        assert_eq!(mem.len((1, 1)).await, 0);
    }
}

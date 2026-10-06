use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::Mutex;

/// Per-channel conversation history used to give Gemini multi-turn context.
///
/// Stores raw Gemini `Content` objects (`{"role": ..., "parts": [...]}`) so
/// they can be replayed verbatim in the next `generateContent` call.
/// Bounded by `limit` entries per channel (each user/model round-trip counts
/// as two entries).
#[derive(Debug, Clone)]
pub struct ConversationMemory {
    inner: Arc<Mutex<HashMap<u64, VecDeque<Value>>>>,
    limit: usize,
}

impl ConversationMemory {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            limit,
        }
    }

    /// Build the `contents` array for the next model call:
    /// retained history followed by the new user turn.
    pub async fn build_contents(&self, channel_id: u64, user_turn: Value) -> Vec<Value> {
        let guard = self.inner.lock().await;
        let mut contents: Vec<Value> = guard
            .get(&channel_id)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default();
        contents.push(user_turn);
        contents
    }

    /// Append a full round-trip (user turn already sent + model + tool parts).
    /// `new_entries` should be in chronological order.
    pub async fn record(&self, channel_id: u64, new_entries: Vec<Value>) {
        if self.limit == 0 || new_entries.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().await;
        let queue = guard.entry(channel_id).or_insert_with(VecDeque::new);
        for entry in new_entries {
            queue.push_back(entry);
            while queue.len() > self.limit {
                queue.pop_front();
            }
        }
    }

    /// Number of retained entries for a channel (for tests / diagnostics).
    pub async fn len(&self, channel_id: u64) -> usize {
        self.inner
            .lock()
            .await
            .get(&channel_id)
            .map_or(0, |q| q.len())
    }

    pub async fn clear(&self, channel_id: u64) {
        self.inner.lock().await.remove(&channel_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn history_is_bounded() {
        let mem = ConversationMemory::new(4);
        mem.record(1, vec![json!({"a": 1}), json!({"a": 2})]).await;
        mem.record(1, vec![json!({"a": 3}), json!({"a": 4}), json!({"a": 5})])
            .await;
        assert_eq!(mem.len(1).await, 4);
        let contents = mem.build_contents(1, json!({"new": true})).await;
        assert_eq!(contents.len(), 5); // 4 retained + 1 new
    }

    #[tokio::test]
    async fn channels_are_isolated() {
        let mem = ConversationMemory::new(10);
        mem.record(1, vec![serde_json::json!({})]).await;
        assert_eq!(mem.len(2).await, 0);
    }
}

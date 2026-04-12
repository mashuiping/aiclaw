//! Streaming output buffer — accumulates tokens and flushes on size/timeout

use std::collections::HashMap;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

/// Max characters to buffer before flushing
const DEFAULT_BUFFER_SIZE: usize = 50;
/// Default flush timeout (ms)
const DEFAULT_FLUSH_TIMEOUT_MS: u64 = 500;
/// Rate limit: max updates per minute (飞书限制)
const MAX_UPDATES_PER_MINUTE: usize = 60;

/// A pending update for a single message
#[derive(Debug)]
pub enum BufferEntry {
    /// Text content waiting to be flushed
    Text(String),
    /// Interactive card waiting to be flushed
    Card(serde_json::Value),
}

/// Manages streaming buffers for multiple in-flight messages
pub struct StreamingBuffer {
    entries: RwLock<HashMap<String, BufferEntry>>,
    last_flush: RwLock<HashMap<String, Instant>>,
    buffer_size: usize,
    flush_timeout: Duration,
    rate_limit_reset: RwLock<Instant>,
    updates_this_minute: RwLock<usize>,
}

impl StreamingBuffer {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            last_flush: RwLock::new(HashMap::new()),
            buffer_size: DEFAULT_BUFFER_SIZE,
            flush_timeout: Duration::from_millis(DEFAULT_FLUSH_TIMEOUT_MS),
            rate_limit_reset: RwLock::new(Instant::now() + Duration::from_secs(60)),
            updates_this_minute: RwLock::new(0),
        }
    }

    pub fn with_limits(buffer_size: usize, flush_timeout_ms: u64) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            last_flush: RwLock::new(HashMap::new()),
            buffer_size,
            flush_timeout: Duration::from_millis(flush_timeout_ms),
            rate_limit_reset: RwLock::new(Instant::now() + Duration::from_secs(60)),
            updates_this_minute: RwLock::new(0),
        }
    }

    /// Append text token to a message's buffer. Returns true if flush is needed.
    pub async fn append_text(&self, message_id: &str, text: &str) -> bool {
        let mut entries = self.entries.write().await;
        let entry = entries.entry(message_id.to_string()).or_insert_with(|| {
            BufferEntry::Text(String::new())
        });

        let current_len = match entry {
            BufferEntry::Text(s) => s.len(),
            _ => 0,
        };

        if let BufferEntry::Text(s) = entry {
            s.push_str(text);
            current_len + text.len() >= self.buffer_size
        } else {
            false
        }
    }

    /// Set card content for a message (overwrites).
    pub async fn set_card(&self, message_id: &str, card: serde_json::Value) {
        let mut entries = self.entries.write().await;
        entries.insert(message_id.to_string(), BufferEntry::Card(card));
    }

    /// Take and clear the buffer entry for a message, if ready to flush.
    /// Returns None if rate-limited or not yet time to flush.
    pub async fn take_pending(&self, message_id: &str) -> Option<BufferEntry> {
        // Rate limit: check and increment atomically under a single lock.
        // Also extend the reset window if expired.
        let dominated;
        {
            let mut updates = self.updates_this_minute.write().await;
            if Instant::now() > *self.rate_limit_reset.read().await {
                *updates = 0;
            }
            dominated = *updates >= MAX_UPDATES_PER_MINUTE;
            if !dominated {
                *updates += 1;
                // Extend the rate limit window
                *self.rate_limit_reset.write().await = Instant::now() + Duration::from_secs(60);
            }
        }

        if dominated {
            return None;
        }

        // Check if there's a pending entry and if it's ready to flush
        let entry = {
            let mut entries = self.entries.write().await;
            let entry = entries.remove(message_id)?;
            let dominated = {
                let mut last_flush = self.last_flush.write().await;
                let dominated = last_flush.get(message_id)
                    .map(|t| t.elapsed() >= self.flush_timeout)
                    .unwrap_or(true);
                if dominated {
                    last_flush.insert(message_id.to_string(), Instant::now());
                }
                dominated
            };
            if !dominated {
                // Put it back — not time to flush yet
                entries.insert(message_id.to_string(), entry);
                return None;
            }
            entry
        };

        Some(entry)
    }

    /// Force flush — ignore rate limit (use sparingly)
    pub async fn force_take_pending(&self, message_id: &str) -> Option<BufferEntry> {
        let mut entries = self.entries.write().await;
        entries.remove(message_id)
    }

    /// Get current buffered text length for a message
    pub async fn buffered_len(&self, message_id: &str) -> usize {
        let entries = self.entries.read().await;
        match entries.get(message_id) {
            Some(BufferEntry::Text(s)) => s.len(),
            _ => 0,
        }
    }
}

impl Default for StreamingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

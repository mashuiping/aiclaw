//! Session management

use aiclaw_types::agent::{InteractionRecord, Session, SessionContext, SessionState};
use chrono::Utc;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// Session manager - manages agent sessions
pub struct SessionManager {
    sessions: DashMap<String, Arc<Session>>,
    user_sessions: DashMap<String, Vec<String>>,
    timeout: Duration,
}

impl SessionManager {
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            sessions: DashMap::new(),
            user_sessions: DashMap::new(),
            timeout: Duration::from_secs(timeout_secs),
        }
    }

    /// Create a new session
    pub fn create_session(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Arc<Session> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now();

        let session = Session {
            id: session_id.clone(),
            user_id: user_id.to_string(),
            channel: channel.to_string(),
            thread_id: thread_id.map(String::from),
            created_at: now,
            last_activity: now,
            state: SessionState::Active,
            context: SessionContext::default(),
        };

        let session = Arc::new(session);

        self.sessions.insert(session_id.clone(), session.clone());

        self.user_sessions
            .entry(user_id.to_string())
            .or_insert_with(Vec::new)
            .push(session_id);

        debug!("Created session {} for user {}", session.id, user_id);

        session
    }

    /// Get or create session
    pub fn get_or_create(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Option<&str>,
    ) -> Arc<Session> {
        let key = self.session_key(user_id, channel, thread_id);

        if let Some(session) = self.sessions.get(&key) {
            if self.is_session_valid(&session) {
                return session.value().clone();
            }
        }

        self.create_session(user_id, channel, thread_id)
    }

    /// Get session by ID
    pub fn get(&self, session_id: &str) -> Option<Arc<Session>> {
        self.sessions.get(session_id).map(|r| r.value().clone())
    }

    /// Update session activity
    pub fn touch(&self, session_id: &str) -> Option<Arc<Session>> {
        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            entry.last_activity = Utc::now();
            return Some(entry.value().clone());
        }
        None
    }

    /// Update session state
    pub fn update_state(&self, session_id: &str, state: SessionState) -> Option<Arc<Session>> {
        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            entry.state = state;
            entry.last_activity = Utc::now();
            return Some(entry.value().clone());
        }
        None
    }

    /// Add interaction to session
    pub fn add_interaction(
        &self,
        session_id: &str,
        intent: &str,
        skill: Option<&str>,
        result: Option<&str>,
        success: bool,
    ) -> Option<Arc<Session>> {
        if let Some(mut entry) = self.sessions.get_mut(session_id) {
            let record = InteractionRecord {
                timestamp: Utc::now(),
                intent: intent.to_string(),
                skill: skill.map(String::from),
                result: result.map(String::from),
                success,
            };

            entry.context.history.push(record);
            entry.context.last_skill = skill.map(String::from);
            entry.last_activity = Utc::now();

            return Some(entry.value().clone());
        }
        None
    }

    /// Clean up expired sessions
    pub fn cleanup_expired(&self) -> usize {
        let now = Utc::now();
        let mut removed = 0;

        let expired: Vec<String> = self
            .sessions
            .iter()
            .filter(|entry| {
                let age = now.signed_duration_since(entry.last_activity);
                age.num_seconds() as u64 > self.timeout.as_secs()
            })
            .map(|entry| entry.key().clone())
            .collect();

        for session_id in expired {
            if let Some((_, entry)) = self.sessions.remove(&session_id) {
                if let Some(sessions) = self.user_sessions.get_mut(&entry.user_id) {
                    sessions.retain(|s| s != &session_id);
                }
                removed += 1;
            }
        }

        if removed > 0 {
            info!("Cleaned up {} expired sessions", removed);
        }

        removed
    }

    /// Get active session count
    pub fn active_count(&self) -> usize {
        self.sessions.len()
    }

    /// Generate session key
    fn session_key(&self, user_id: &str, channel: &str, thread_id: Option<&str>) -> String {
        match thread_id {
            Some(tid) => format!("{}:{}:{}", user_id, channel, tid),
            None => format!("{}:{}", user_id, channel),
        }
    }

    /// Check if session is valid
    fn is_session_valid(&self, session: &Session) -> bool {
        if session.state == SessionState::Expired || session.state == SessionState::Completed {
            return false;
        }

        let age = Utc::now().signed_duration_since(session.last_activity);
        age.num_seconds() as u64 <= self.timeout.as_secs()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new(3600)
    }
}

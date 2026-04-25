//! SQLite-backed session store with WAL mode and FTS5.

pub mod error;
pub mod schema;

use chrono::{DateTime, TimeZone, Utc};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, info};

use crate::config::schema::SessionState;
use aiclaw_types::agent::{ChatMessage, MessageRole, Session, SessionContext};
pub use error::SessionStoreError;
pub use schema::SCHEMA_VERSION;

/// A stored message row from the database.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: i64,
    pub session_id: String,
    pub role: String,
    pub content: Option<String>,
    pub timestamp: f64,
}

impl From<StoredMessage> for ChatMessage {
    fn from(m: StoredMessage) -> Self {
        ChatMessage {
            role: match m.role.as_str() {
                "system" => MessageRole::System,
                "user" => MessageRole::User,
                _ => MessageRole::Assistant,
            },
            content: m.content.unwrap_or_default(),
            timestamp: Utc.timestamp_opt((m.timestamp as i64), 0).unwrap(),
        }
    }
}

/// A search result from FTS5.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub session_id: String,
    pub snippet: String,
    pub rank: f64,
}

pub struct SessionStore {
    pool: Pool<SqliteConnectionManager>,
    db_path: PathBuf,
}

impl SessionStore {
    /// Open (or create) the session database at the given path.
    pub fn new(db_path: PathBuf) -> Result<Self, SessionStoreError> {
        let manager = SqliteConnectionManager::file(&db_path);
        let pool = Pool::builder()
            .max_size(10)
            .build(manager)?;

        // Enable WAL mode
        let conn = pool.get()?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        // Initialize schema
        let store = Self { pool, db_path };
        store.init_schema()?;
        Ok(store)
    }

    fn get_conn(&self) -> Result<PooledConnection<SqliteConnectionManager>, SessionStoreError> {
        Ok(self.pool.get()?)
    }

    fn init_schema(&self) -> Result<(), SessionStoreError> {
        let conn = self.get_conn()?;

        // Check current version
        let version: Option<i32> = conn
            .query_row(
                "SELECT version FROM schema_version LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();

        let current = version.unwrap_or(0);

        if current < SCHEMA_VERSION {
            // Apply all schema statements
            for sql in schema::ALL_SCHEMA {
                conn.execute_batch(sql).map_err(|e| {
                    SessionStoreError::Migration(format!("failed SQL: {e}"))
                })?;
            }
            // Update version
            if current == 0 {
                conn.execute(
                    "INSERT INTO schema_version (version) VALUES (?1)",
                    params![SCHEMA_VERSION],
                )?;
            } else {
                conn.execute(
                    "UPDATE schema_version SET version = ?1",
                    params![SCHEMA_VERSION],
                )?;
            }
            info!(
                "session_store: schema migrated from {} to {}",
                current, SCHEMA_VERSION
            );
        }

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Session operations
    // -------------------------------------------------------------------------

    /// Persist a new session. Session must not already exist in DB.
    pub fn create_session(&self, session: &Session) -> Result<(), SessionStoreError> {
        let conn = self.get_conn()?;
        let state = match session.state {
            SessionState::Active => "active",
            SessionState::Waiting => "waiting",
            SessionState::Completed => "completed",
            SessionState::Expired => "expired",
        };
        conn.execute(
            r#"INSERT INTO sessions
               (id, user_id, channel, thread_id, created_at, last_activity, state,
                current_cluster, current_namespace, pending_question, kubeconfig_path)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                session.id,
                session.user_id,
                session.channel,
                session.thread_id,
                session.created_at.timestamp(),
                session.last_activity.timestamp(),
                state,
                session.context.current_cluster,
                session.context.current_namespace,
                session.context.pending_question,
                session.context.kubeconfig_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            ],
        )?;
        Ok(())
    }

    /// Load a session by ID. Returns None if not found.
    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>, SessionStoreError> {
        let conn = self.get_conn()?;
        let mut rows = conn.query(
            "SELECT id, user_id, channel, thread_id, created_at, last_activity, state,
                    current_cluster, current_namespace, pending_question, kubeconfig_path
             FROM sessions WHERE id = ?1",
            params![session_id],
        )?;

        if let Some(row) = rows.next()? {
            Ok(Some(self.row_to_session(row)?))
        } else {
            Ok(None)
        }
    }

    /// Update session metadata (last_activity, state, context fields).
    pub fn update_session(&self, session: &Session) -> Result<(), SessionStoreError> {
        let conn = self.get_conn()?;
        let state = match session.state {
            SessionState::Active => "active",
            SessionState::Waiting => "waiting",
            SessionState::Completed => "completed",
            SessionState::Expired => "expired",
        };
        conn.execute(
            r#"UPDATE sessions SET
               last_activity = ?2, state = ?3,
               current_cluster = ?4, current_namespace = ?5,
               pending_question = ?6, kubeconfig_path = ?7
               WHERE id = ?1"#,
            params![
                session.id,
                session.last_activity.timestamp(),
                state,
                session.context.current_cluster,
                session.context.current_namespace,
                session.context.pending_question,
                session.context.kubeconfig_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            ],
        )?;
        Ok(())
    }

    /// Touch last_activity without rebuilding the full session.
    pub fn touch_session(&self, session_id: &str) -> Result<(), SessionStoreError> {
        let conn = self.get_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE sessions SET last_activity = ?2 WHERE id = ?1",
            params![session_id, now],
        )?;
        Ok(())
    }

    /// Mark a session as ended.
    pub fn end_session(&self, session_id: &str, reason: &str) -> Result<(), SessionStoreError> {
        let conn = self.get_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE sessions SET last_activity = ?2, state = ?3 WHERE id = ?1",
            params![session_id, now, reason],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Message operations
    // -------------------------------------------------------------------------

    /// Append a message to a session. Returns the auto-generated message id.
    pub fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> Result<i64, SessionStoreError> {
        let conn = self.get_conn()?;
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, role, content, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Load all messages for a session in order.
    pub fn get_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>, SessionStoreError> {
        let conn = self.get_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, timestamp
             FROM messages WHERE session_id = ?1 ORDER BY timestamp ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(StoredMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                timestamp: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(From::from)
    }

    // -------------------------------------------------------------------------
    // FTS
    // -------------------------------------------------------------------------

    /// Full-text search across all messages. Returns snippets with match markers.
    pub fn search_messages(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, SessionStoreError> {
        let conn = self.get_conn()?;
        let safe_query = query.replace('"', "\"\"");
        let fts_query = format!("\"{}\"", safe_query);

        let mut stmt = conn.prepare(
            r#"SELECT m.session_id,
                      snippet(messages_fts, 0, '>>>', '<<<', '...', 32) AS snippet,
                      rank
               FROM messages_fts
               JOIN messages m ON messages_fts.rowid = m.id
               WHERE messages_fts MATCH ?1
               ORDER BY rank
               LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
            Ok(SearchResult {
                session_id: row.get(0)?,
                snippet: row.get(1)?,
                rank: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(From::from)
    }

    // -------------------------------------------------------------------------
    // Cleanup
    // -------------------------------------------------------------------------

    /// Delete sessions older than `days` that are not active.
    pub fn prune_sessions(&self, older_than_days: u32) -> Result<usize, SessionStoreError> {
        let conn = self.get_conn()?;
        let cutoff = Utc::now().timestamp() - (older_than_days as i64) * 86400;
        let deleted = conn.execute(
            "DELETE FROM sessions WHERE last_activity < ?1 AND state != 'active'",
            params![cutoff],
        )?;
        debug!("session_store: pruned {deleted} expired sessions");
        Ok(deleted)
    }

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn row_to_session(
        &self,
        row: &rusqlite::Row<'_>,
    ) -> Result<Session, SessionStoreError> {
        use rusqlite::types::Value;

        let id: String = row.get(0)?;
        let user_id: String = row.get(1)?;
        let channel: String = row.get(2)?;
        let thread_id: Option<String> = row.get(3)?;
        let created_at_ts: f64 = row.get(4)?;
        let last_activity_ts: f64 = row.get(5)?;
        let state_str: String = row.get(6)?;
        let current_cluster: Option<String> = row.get(7)?;
        let current_namespace: Option<String> = row.get(8)?;
        let pending_question: Option<String> = row.get(9)?;
        let kubeconfig_path_str: Option<String> = row.get(10)?;

        let state = match state_str.as_str() {
            "active" => SessionState::Active,
            "waiting" => SessionState::Waiting,
            "completed" => SessionState::Completed,
            "expired" => SessionState::Expired,
            _ => SessionState::Active,
        };

        let created_at = Utc.timestamp_opt(created_at_ts as i64, 0).unwrap();
        let last_activity = Utc.timestamp_opt(last_activity_ts as i64, 0).unwrap();

        // Load conversation history from messages table
        let messages = self.get_messages(&id).unwrap_or_default();
        let conversation_history: Vec<ChatMessage> =
            messages.into_iter().map(ChatMessage::from).collect();

        Ok(Session {
            id,
            user_id,
            channel,
            thread_id,
            created_at,
            last_activity,
            state,
            context: SessionContext {
                last_skill: None,
                last_parameters: Default::default(),
                history: vec![],
                conversation_history,
                current_cluster,
                current_namespace,
                pending_question,
                kubeconfig_path: kubeconfig_path_str.map(PathBuf::from),
            },
        })
    }
}

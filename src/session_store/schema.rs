/// Current schema version
pub const SCHEMA_VERSION: i32 = 1;

pub const CREATE_SCHEMA_VERSION: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);
"#;

pub const CREATE_SESSIONS: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    thread_id TEXT,
    created_at REAL NOT NULL,
    last_activity REAL NOT NULL,
    state TEXT NOT NULL,
    current_cluster TEXT,
    current_namespace TEXT,
    pending_question TEXT,
    kubeconfig_path TEXT
);
"#;

pub const CREATE_SESSIONS_USER_CHANNEL_IDX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_sessions_user_channel ON sessions(user_id, channel);
"#;

pub const CREATE_SESSIONS_LAST_ACTIVITY_IDX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_sessions_last_activity ON sessions(last_activity DESC);
"#;

pub const CREATE_MESSAGES: &str = r#"
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT,
    timestamp REAL NOT NULL
);
"#;

pub const CREATE_MESSAGES_SESSION_IDX: &str = r#"
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);
"#;

pub const CREATE_MESSAGES_FTS: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content,
    content=messages,
    content_rowid=id
);
"#;

pub const TRIGGER_INSERT_FTS: &str = r#"
CREATE TRIGGER IF NOT EXISTS messages_fts_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;
"#;

pub const TRIGGER_DELETE_FTS: &str = r#"
CREATE TRIGGER IF NOT EXISTS messages_fts_delete AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
END;
"#;

pub const TRIGGER_UPDATE_FTS: &str = r#"
CREATE TRIGGER IF NOT EXISTS messages_fts_update AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;
"#;

pub const ALL_SCHEMA: &[&str] = &[
    CREATE_SCHEMA_VERSION,
    CREATE_SESSIONS,
    CREATE_SESSIONS_USER_CHANNEL_IDX,
    CREATE_SESSIONS_LAST_ACTIVITY_IDX,
    CREATE_MESSAGES,
    CREATE_MESSAGES_SESSION_IDX,
    CREATE_MESSAGES_FTS,
    TRIGGER_INSERT_FTS,
    TRIGGER_DELETE_FTS,
    TRIGGER_UPDATE_FTS,
];

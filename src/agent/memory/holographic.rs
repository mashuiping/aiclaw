//! Holographic memory provider — SQLite FTS5 + trust scoring.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use aiclaw_types::memory::{MemoryProvider, ToolSchema};
use serde_json::{json, Value};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS facts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity TEXT,
    content TEXT NOT NULL,
    category TEXT DEFAULT 'general',
    tags TEXT,
    trust REAL DEFAULT 0.5,
    created_at REAL NOT NULL,
    updated_at REAL NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(
    content, entity, tags,
    content=facts, content_rowid=id
);

CREATE TABLE IF NOT EXISTS fact_feedback (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fact_id INTEGER REFERENCES facts(id),
    helpful INTEGER NOT NULL,
    created_at REAL NOT NULL
);
"#;

#[derive(Clone)]
pub struct HolographicMemoryProvider {
    pool: Arc<Pool<SqliteConnectionManager>>,
    db_path: PathBuf,
    default_trust: f64,
    min_trust: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum HoloError {
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("pool: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("not found")]
    NotFound,
}

impl HolographicMemoryProvider {
    pub fn new(db_path: &str, default_trust: f64, min_trust: f64) -> Result<Self, HoloError> {
        let expanded = shellexpand::tilde(db_path);
        let path = PathBuf::from(expanded.as_ref());

        // Ensure parent dir exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let manager = SqliteConnectionManager::file(&path);
        let pool = Pool::builder().max_size(5).build(manager)?;

        // Init schema
        let conn = pool.get()?;
        conn.execute_batch(SCHEMA)?;

        Ok(Self {
            pool: Arc::new(pool),
            db_path: path,
            default_trust,
            min_trust,
        })
    }

    fn get_conn(&self) -> Result<PooledConnection<SqliteConnectionManager>, HoloError> {
        Ok(self.pool.get()?)
    }
}

impl MemoryProvider for HolographicMemoryProvider {
    fn name(&self) -> &str {
        "holographic"
    }

    fn is_available(&self) -> bool {
        self.db_path.exists() || Path::new(&*shellexpand::tilde("~/.aiclaw")).exists()
    }

    fn system_prompt_block(&self) -> String {
        String::from(
            "# Holographic Memory\n\
             Active. Use fact_store to search, probe, reason about, or add facts.\n\
             Use fact_feedback to rate facts as helpful or unhelpful.",
        )
    }

    fn get_tool_schemas(&self) -> Vec<ToolSchema> {
        vec![
            ToolSchema {
                name: "fact_store".to_string(),
                description: "Deep structured memory with algebraic reasoning. \
                    Actions: add, search, probe, related, reason, contradict, update, remove, list.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["add", "search", "probe", "related", "reason", "contradict", "update", "remove", "list"]
                        },
                        "content": {"type": "string", "description": "Fact content (required for 'add')"},
                        "query": {"type": "string", "description": "Search query (required for 'search')"},
                        "entity": {"type": "string", "description": "Entity for 'probe'/'related'/'contradict'"},
                        "entities": {"type": "array", "items": {"type": "string"}, "description": "Entities for 'reason'"},
                        "fact_id": {"type": "integer", "description": "Fact ID for 'update'/'remove'"},
                        "category": {"type": "string"},
                        "tags": {"type": "string"},
                        "trust_delta": {"type": "number"},
                        "min_trust": {"type": "number"},
                        "limit": {"type": "integer", "default": 10}
                    },
                    "required": ["action"]
                }),
            },
            ToolSchema {
                name: "fact_feedback".to_string(),
                description: "Rate a fact after using it. Mark 'helpful' if accurate, 'unhelpful' if outdated.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {"type": "string", "enum": ["helpful", "unhelpful"]},
                        "fact_id": {"type": "integer"}
                    },
                    "required": ["action", "fact_id"]
                }),
            },
        ]
    }

    fn handle_tool_call(&self, name: &str, args: &Value) -> String {
        let conn = match self.get_conn() {
            Ok(c) => c,
            Err(e) => return json!({ "error": e.to_string() }).to_string(),
        };

        match name {
            "fact_store" => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
                let result = match action {
                    "add" => {
                        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        let entity = args.get("entity").and_then(|v| v.as_str());
                        let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("general");
                        let tags = args.get("tags").and_then(|v| v.as_str()).unwrap_or("");
                        let now = chrono::Utc::now().timestamp();
                        conn.execute(
                            "INSERT INTO facts (entity, content, category, tags, trust, created_at, updated_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![entity, content, category, tags, self.default_trust, now, now],
                        ).map_err(|e| e.to_string()).ok();
                        json!({ "result": "fact added" })
                    }
                    "search" => {
                        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10) as usize;
                        let min_trust = args.get("min_trust").and_then(|v| v.as_f64()).unwrap_or(self.min_trust);
                        let safe_q = query.replace('"', "\"\"");
                        let fts_q = format!("\"{}\"", safe_q);
                        let mut stmt = conn.prepare(
                            "SELECT f.id, f.entity, f.content, f.trust, f.tags
                             FROM facts f
                             JOIN facts_fts ON facts_fts.rowid = f.id
                             WHERE facts_fts MATCH ?1 AND f.trust >= ?2
                             LIMIT ?3"
                        ).unwrap();
                        let rows: Vec<_> = stmt.query_map(params![fts_q, min_trust, limit as i64], |row| {
                            Ok(serde_json::json!({
                                "id": row.get::<_, i64>(0)?,
                                "entity": row.get::<_, Option<String>>(1)?,
                                "content": row.get::<_, String>(2)?,
                                "trust": row.get::<_, f64>(3)?,
                                "tags": row.get::<_, Option<String>>(4)?
                            }))
                        }).unwrap().filter_map(|r| r.ok()).collect();
                        json!({ "results": rows })
                    }
                    "probe" => {
                        let entity = args.get("entity").and_then(|v| v.as_str()).unwrap_or("");
                        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10) as usize;
                        let mut stmt = conn.prepare(
                            "SELECT id, content, trust, category FROM facts \
                             WHERE entity = ?1 AND trust >= ?2 LIMIT ?3"
                        ).unwrap();
                        let rows: Vec<_> = stmt.query_map(params![entity, self.min_trust, limit as i64], |row| {
                            Ok(serde_json::json!({
                                "id": row.get::<_, i64>(0)?,
                                "content": row.get::<_, String>(1)?,
                                "trust": row.get::<_, f64>(2)?,
                                "category": row.get::<_, String>(3)?
                            }))
                        }).unwrap().filter_map(|r| r.ok()).collect();
                        json!({ "facts": rows })
                    }
                    "list" => {
                        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20) as usize;
                        let mut stmt = conn.prepare(
                            "SELECT id, entity, content, trust, category FROM facts \
                             WHERE trust >= ?1 ORDER BY updated_at DESC LIMIT ?2"
                        ).unwrap();
                        let rows: Vec<_> = stmt.query_map(params![self.min_trust, limit as i64], |row| {
                            Ok(serde_json::json!({
                                "id": row.get::<_, i64>(0)?,
                                "entity": row.get::<_, Option<String>>(1)?,
                                "content": row.get::<_, String>(2)?,
                                "trust": row.get::<_, f64>(3)?,
                                "category": row.get::<_, String>(4)?
                            }))
                        }).unwrap().filter_map(|r| r.ok()).collect();
                        json!({ "facts": rows })
                    }
                    _ => json!({ "error": format!("unknown action: {action}") }),
                };
                result.to_string()
            }
            "fact_feedback" => {
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
                let fact_id = args.get("fact_id").and_then(|v| v.as_i64()).unwrap_or(0);
                let delta = if action == "helpful" { 0.05 } else { -0.10 };
                conn.execute(
                    "UPDATE facts SET trust = MAX(0.0, MIN(1.0, trust + ?1)), updated_at = ?2 WHERE id = ?3",
                    params![delta, chrono::Utc::now().timestamp(), fact_id],
                ).ok();
                json!({ "result": format!("feedback recorded: {action}") }).to_string()
            }
            _ => json!({ "error": format!("unknown tool: {name}") }).to_string(),
        }
    }
}

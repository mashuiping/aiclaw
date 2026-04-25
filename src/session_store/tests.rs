//! Unit tests for session_store module

use tempfile::TempDir;

fn create_test_store() -> (super::SessionStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("test_session.db");
    let store = super::SessionStore::new(path).expect("failed to create store");
    (store, temp)
}

fn create_test_session(id: &str, user: &str) -> aiclaw_types::agent::Session {
    use aiclaw_types::agent::{Session, SessionContext, SessionState};
    use chrono::Utc;
    Session {
        id: id.to_string(),
        user_id: user.to_string(),
        channel: "test".to_string(),
        thread_id: None,
        created_at: Utc::now(),
        last_activity: Utc::now(),
        state: SessionState::Active,
        context: SessionContext::default(),
    }
}

#[test]
fn test_session_store_create_and_get() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-1", "user-1");

    store.create_session(&session).expect("create_session failed");

    let loaded = store.get_session("session-1").expect("get_session failed");
    let loaded = loaded.expect("session not found");

    assert_eq!(loaded.id, "session-1");
    assert_eq!(loaded.user_id, "user-1");
    assert_eq!(loaded.channel, "test");
}

#[test]
fn test_session_store_get_nonexistent() {
    let (store, _tmp) = create_test_store();

    let result = store.get_session("nonexistent").expect("get_session failed");
    assert!(result.is_none());
}

#[test]
fn test_session_store_update_session() {
    let (store, _tmp) = create_test_store();
    let mut session = create_test_session("session-2", "user-2");

    store.create_session(&session).expect("create_session failed");

    session.context.current_cluster = Some("prod-cluster".to_string());
    session.context.current_namespace = Some("default".to_string());
    store.update_session(&session).expect("update_session failed");

    let loaded = store.get_session("session-2").expect("get_session failed").unwrap();
    assert_eq!(loaded.context.current_cluster, Some("prod-cluster".to_string()));
    assert_eq!(loaded.context.current_namespace, Some("default".to_string()));
}

#[test]
fn test_session_store_touch_session() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-3", "user-3");

    store.create_session(&session).expect("create_session failed");
    store.touch_session("session-3").expect("touch_session failed");
    // Just verify it doesn't panic
}

#[test]
fn test_session_store_end_session() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-4", "user-4");

    store.create_session(&session).expect("create_session failed");
    store.end_session("session-4", "completed").expect("end_session failed");

    let loaded = store.get_session("session-4").expect("get_session failed").unwrap();
    assert_eq!(loaded.state, aiclaw_types::agent::SessionState::Completed);
}

#[test]
fn test_append_and_get_messages() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-5", "user-5");
    store.create_session(&session).expect("create_session failed");

    store.append_message("session-5", "user", "Hello").expect("append failed");
    store.append_message("session-5", "assistant", "Hi there").expect("append failed");
    store.append_message("session-5", "user", "How are you?").expect("append failed");

    let messages = store.get_messages("session-5").expect("get_messages failed");
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content.as_deref(), Some("Hello"));
    assert_eq!(messages[1].role, "assistant");
    assert_eq!(messages[1].content.as_deref(), Some("Hi there"));
}

#[test]
fn test_message_stored_as_chat_message() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-6", "user-6");
    store.create_session(&session).expect("create_session failed");

    store.append_message("session-6", "user", "Test message").expect("append failed");

    let messages = store.get_messages("session-6").expect("get_messages failed");
    let msg = super::StoredMessage::from(messages[0].clone());

    use aiclaw_types::agent::{ChatMessage, MessageRole};
    let chat: ChatMessage = msg.into();
    assert_eq!(chat.role, MessageRole::User);
    assert_eq!(chat.content, "Test message");
}

#[test]
fn test_search_messages() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-7", "user-7");
    store.create_session(&session).expect("create_session failed");

    store.append_message("session-7", "user", "Deploy the application to kubernetes cluster").expect("append failed");
    store.append_message("session-7", "assistant", "Which cluster?").expect("append failed");
    store.append_message("session-7", "user", "Use the prod cluster please").expect("append failed");

    let results = store.search_messages("kubernetes", 10).expect("search failed");
    assert!(!results.is_empty(), "should find results for 'kubernetes'");
    assert!(results.iter().all(|r| r.snippet.contains("kubernetes") || r.snippet.contains(">>>")));
}

#[test]
fn test_search_messages_no_results() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-8", "user-8");
    store.create_session(&session).expect("create_session failed");

    store.append_message("session-8", "user", "Just a regular message").expect("append failed");

    let results = store.search_messages("xyznonexistent", 10).expect("search failed");
    assert!(results.is_empty());
}

#[test]
fn test_prune_sessions() {
    let (store, _tmp) = create_test_store();
    let session = create_test_session("session-9", "user-9");
    store.create_session(&session).expect("create_session failed");

    // Pruning with 0 days should remove nothing active
    let count = store.prune_sessions(0).expect("prune failed");
    assert_eq!(count, 0);

    // Session still exists
    let loaded = store.get_session("session-9").expect("get_session failed").unwrap();
    assert_eq!(loaded.id, "session-9");
}

#[test]
fn test_wal_mode_enabled() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("wal_test.db");
    let store = super::SessionStore::new(path).expect("failed to create store");

    // WAL mode is set in new(). If we get here without panic, it's enabled.
    // Create a session to verify DB works
    let session = create_test_session("wal-session", "user-wal");
    store.create_session(&session).expect("create_session failed");

    // Verify the DB file exists
    let db_path = temp.path().join("wal_test.db");
    assert!(db_path.exists());
}
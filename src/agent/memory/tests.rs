//! Unit tests for memory module

use aiclaw_types::memory::MemoryProvider;
use std::sync::Arc;

type BuiltinProvider = crate::agent::memory::builtin::BuiltinMemoryProvider;
type MemoryManager = crate::agent::memory::MemoryManager;

// =============================================================================
// BuiltinMemoryProvider tests
// =============================================================================

#[test]
fn test_builtin_memory_provider_name() {
    let provider = BuiltinProvider::new();
    assert_eq!(provider.name(), "builtin");
}

#[test]
fn test_builtin_memory_provider_is_available() {
    let provider = BuiltinProvider::new();
    assert!(provider.is_available());
}

#[test]
fn test_builtin_memory_provider_no_tools() {
    let provider = BuiltinProvider::new();
    assert!(provider.get_tool_schemas().is_empty());
}

#[test]
fn test_builtin_memory_provider_system_prompt_empty_when_no_files() {
    let provider = BuiltinProvider::new();
    let block = provider.system_prompt_block();
    // Without files, memory_content is empty
    assert_eq!(block, "");
}

// =============================================================================
// MemoryManager tests
// =============================================================================

#[test]
fn test_memory_manager_new() {
    let mm = MemoryManager::new();
    // Should have builtin provider
    assert_eq!(mm.build_system_prompt(), "");
}

#[test]
fn test_memory_manager_no_external_provider() {
    let mm = MemoryManager::new();
    // External is None by default
    assert_eq!(mm.prefetch_all("test query"), String::new());
}

#[test]
fn test_memory_manager_has_tool_false() {
    let mm = MemoryManager::new();
    assert!(!mm.has_tool("nonexistent_tool"));
    assert!(!mm.has_tool("fact_store"));
    assert!(!mm.has_tool("brv_query"));
}

#[test]
fn test_memory_manager_unknown_tool_call() {
    let mm = MemoryManager::new();
    let result = mm.handle_tool_call("unknown", &serde_json::json!({}));
    assert!(result.contains("unknown memory tool"));
}

#[test]
fn test_memory_manager_get_all_tool_schemas_empty() {
    let mm = MemoryManager::new();
    // Builtin has no tools
    assert!(mm.get_all_tool_schemas().is_empty());
}

#[test]
fn test_memory_manager_sync_noop() {
    let mm = MemoryManager::new();
    mm.sync_all("user message", "assistant response");
}

#[test]
fn test_memory_manager_queue_prefetch_noop() {
    let mm = MemoryManager::new();
    mm.queue_prefetch_all("next query");
}

#[test]
fn test_memory_manager_on_turn_start_noop() {
    let mm = MemoryManager::new();
    mm.on_turn_start(1, "test message");
}

#[test]
fn test_memory_manager_on_session_end_noop() {
    let mm = MemoryManager::new();
    mm.on_session_end(&[]);
}

#[test]
fn test_memory_manager_on_pre_compress_noop() {
    let mm = MemoryManager::new();
    let result = mm.on_pre_compress(&[("user".to_string(), "assistant".to_string())]);
    assert_eq!(result, String::new());
}

#[test]
fn test_memory_manager_shutdown_noop() {
    let mm = MemoryManager::new();
    mm.shutdown_all();
}

// =============================================================================
// ToolSchema tests
// =============================================================================

#[test]
fn test_tool_schema_debug() {
    use aiclaw_types::memory::ToolSchema;
    let schema = ToolSchema {
        name: "test_tool".to_string(),
        description: "A test tool".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    };
    let debug = format!("{:?}", schema);
    assert!(debug.contains("test_tool"));
}

#[test]
fn test_tool_schema_clone() {
    use aiclaw_types::memory::ToolSchema;
    let schema = ToolSchema {
        name: "test_tool".to_string(),
        description: "A test tool".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    };
    let cloned = schema.clone();
    assert_eq!(cloned.name, schema.name);
    assert_eq!(cloned.description, schema.description);
}

// =============================================================================
// MemoryProvider trait object tests
// =============================================================================

#[test]
fn test_memory_provider_trait_object() {
    let provider: Arc<dyn aiclaw_types::memory::MemoryProvider> =
        Arc::new(BuiltinProvider::new());

    assert_eq!(provider.name(), "builtin");
    assert!(provider.is_available());
    assert!(provider.get_tool_schemas().is_empty());
}

#[test]
fn test_memory_provider_trait_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BuiltinProvider>();
}
//! MCP protocol types and helpers

use aiclaw_types::mcp::{
    JSONRPCError, JSONRPCRequest, JSONRPCResponse, MCPServerInfo, ResourceInfo, ToolInfo,
};
use serde_json::Value;
use std::collections::HashMap;

/// MCP protocol methods
pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const TOOLS_LIST: &str = "tools/list";
    pub const TOOLS_CALL: &str = "tools/call";
    pub const RESOURCES_LIST: &str = "resources/list";
    pub const RESOURCES_READ: &str = "resources/read";
    pub const PROMPTS_LIST: &str = "prompts/list";
    pub const PROMPTS_GET: &str = "prompts/get";
    pub const SHUTDOWN: &str = "shutdown";
}

/// JSON-RPC request builder
pub struct RequestBuilder {
    method: String,
    params: Option<Value>,
    id: Value,
}

impl RequestBuilder {
    pub fn new(method: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            params: None,
            id: Value::Number(serde_json::Number::from(1)),
        }
    }

    pub fn with_params(mut self, params: Value) -> Self {
        self.params = Some(params);
        self
    }

    pub fn with_id(mut self, id: Value) -> Self {
        self.id = id;
        self
    }

    pub fn build(self) -> JSONRPCRequest {
        JSONRPCRequest {
            jsonrpc: "2.0".to_string(),
            id: self.id,
            method: self.method,
            params: self.params,
        }
    }
}

/// Parse a JSON-RPC response
pub fn parse_response(response_body: &[u8]) -> anyhow::Result<JSONRPCResponse> {
    serde_json::from_slice(response_body).map_err(|e| anyhow::anyhow!("Failed to parse JSON-RPC response: {}", e))
}

/// Parse a JSON-RPC request
pub fn parse_request(request_body: &[u8]) -> anyhow::Result<JSONRPCRequest> {
    serde_json::from_slice(request_body).map_err(|e| anyhow::anyhow!("Failed to parse JSON-RPC request: {}", e))
}

/// Create an initialize request
pub fn create_initialize_request(client_name: &str, client_version: &str) -> JSONRPCRequest {
    let params = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "roots": {
                "listChanged": true
            },
            "sampling": {}
        },
        "clientInfo": {
            "name": client_name,
            "version": client_version
        }
    });

    RequestBuilder::new(methods::INITIALIZE)
        .with_params(params)
        .with_id(Value::String("1".to_string()))
        .build()
}

/// Create a tools/list request
pub fn create_list_tools_request() -> JSONRPCRequest {
    RequestBuilder::new(methods::TOOLS_LIST)
        .with_id(Value::String("2".to_string()))
        .build()
}

/// Create a tools/call request
pub fn create_call_tool_request(tool_name: &str, arguments: HashMap<String, Value>) -> JSONRPCRequest {
    let params = serde_json::json!({
        "name": tool_name,
        "arguments": arguments
    });

    RequestBuilder::new(methods::TOOLS_CALL)
        .with_params(params)
        .with_id(Value::String("3".to_string()))
        .build()
}

/// Create a resources/list request
pub fn create_list_resources_request() -> JSONRPCRequest {
    RequestBuilder::new(methods::RESOURCES_LIST)
        .with_id(Value::String("4".to_string()))
        .build()
}

/// Create a shutdown request
pub fn create_shutdown_request() -> JSONRPCRequest {
    RequestBuilder::new(methods::SHUTDOWN)
        .with_id(Value::String("5".to_string()))
        .build()
}

/// Parse tool info from JSON-RPC result
pub fn parse_tool_info(result: &Value) -> anyhow::Result<Vec<ToolInfo>> {
    let tools = result
        .get("tools")
        .and_then(|t| t.as_array())
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'tools' field in result"))?;

    tools
        .iter()
        .map(|t| serde_json::from_value(t.clone()).map_err(|e| anyhow::anyhow!("{}", e)))
        .collect()
}

/// Parse server info from JSON-RPC result
pub fn parse_server_info(result: &Value) -> anyhow::Result<MCPServerInfo> {
    let server_info = result
        .get("serverInfo")
        .ok_or_else(|| anyhow::anyhow!("Missing 'serverInfo' field in result"))?;

    serde_json::from_value(server_info.clone())
        .map_err(|e| anyhow::anyhow!("Failed to parse serverInfo: {}", e))
}

/// Parse resource info from JSON-RPC result
pub fn parse_resource_info(result: &Value) -> anyhow::Result<Vec<ResourceInfo>> {
    let resources = result
        .get("resources")
        .and_then(|r| r.as_array())
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'resources' field in result"))?;

    resources
        .iter()
        .map(|r| serde_json::from_value(r.clone()).map_err(|e| anyhow::anyhow!("{}", e)))
        .collect()
}

/// Create an error response
pub fn create_error_response(id: Value, code: i32, message: &str) -> JSONRPCResponse {
    JSONRPCResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: None,
        error: Some(JSONRPCError {
            code,
            message: message.to_string(),
            data: None,
        }),
    }
}

/// Create a success response
pub fn create_success_response<T: serde::Serialize>(id: Value, result: T) -> anyhow::Result<JSONRPCResponse> {
    Ok(JSONRPCResponse {
        jsonrpc: "2.0".to_string(),
        id,
        result: Some(serde_json::to_value(result)?),
        error: None,
    })
}

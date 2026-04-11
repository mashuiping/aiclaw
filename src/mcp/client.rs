//! MCP client implementation

use aiclaw_types::mcp::{JSONRPCRequest, JSONRPCResponse, MCPServerInfo, ToolInfo};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::RwLock as TokioRwLock;
use tracing::{debug, error, info};

use super::protocol::{self};

/// MCP client errors
#[derive(Debug, thiserror::Error)]
pub enum MCPError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Request failed: {0}")]
    RequestFailed(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Timeout")]
    Timeout,

    #[error("Tool not found: {0}")]
    ToolNotFound(String),
}

/// MCP client for connecting to MCP servers
pub struct MCPClient {
    name: String,
    server_info: TokioRwLock<Option<MCPServerInfo>>,
    tools: TokioRwLock<Vec<ToolInfo>>,
    process: TokioRwLock<Option<Child>>,
    stdin: Arc<TokioRwLock<Option<tokio::process::ChildStdin>>>,
    stdout: Arc<TokioRwLock<Option<tokio::process::ChildStdout>>>,
}

impl MCPClient {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            server_info: TokioRwLock::new(None),
            tools: TokioRwLock::new(Vec::new()),
            process: TokioRwLock::new(None),
            stdin: Arc::new(TokioRwLock::new(None)),
            stdout: Arc::new(TokioRwLock::new(None)),
        }
    }

    /// Start the MCP server process (stdio transport)
    pub async fn start_stdio(&self, command: &str, args: &[String], env: &HashMap<String, String>) -> anyhow::Result<()> {
        info!("Starting MCP server via stdio: {} {:?}", command, args);

        let mut cmd = Command::new(command);
        cmd.args(args)
            .envs(env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        *self.stdin.write().await = stdin;
        *self.stdout.write().await = stdout;
        *self.process.write().await = Some(child);

        self.initialize().await?;

        info!("MCP server {} started successfully", self.name);
        Ok(())
    }

    /// Initialize the MCP server
    async fn initialize(&self) -> anyhow::Result<()> {
        let request = protocol::create_initialize_request("aiclaw", env!("CARGO_PKG_VERSION"));

        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(anyhow::anyhow!("Server error: {} - {}", error.code, error.message));
        }

        let result = response.result
            .ok_or_else(|| anyhow::anyhow!("No result in initialize response"))?;

        let server_info = protocol::parse_server_info(&result)?;
        *self.server_info.write().await = Some(server_info);

        self.list_tools().await?;

        Ok(())
    }

    /// Send a JSON-RPC request
    async fn send_request(&self, request: JSONRPCRequest) -> anyhow::Result<JSONRPCResponse> {
        let mut stdin_guard = self.stdin.write().await;
        let mut stdout_guard = self.stdout.write().await;

        let stdin = stdin_guard.as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP stdin not available"))?;

        let stdout = stdout_guard.as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP stdout not available"))?;

        let request_json = serde_json::to_string(&request)?;
        let request_line = format!("{}\n", request_json);

        debug!("Sending MCP request: {}", request.method);

        stdin.write_all(request_line.as_bytes()).await?;

        let mut response_line = String::new();
        BufReader::new(stdout)
            .read_line(&mut response_line)
            .await?;
        let response_line = response_line.trim_end_matches(['\r', '\n']);

        let response: JSONRPCResponse = serde_json::from_str(response_line)?;

        debug!("Received MCP response for {}", request.method);

        Ok(response)
    }

    /// List available tools from the server
    pub async fn list_tools(&self) -> anyhow::Result<Vec<ToolInfo>> {
        let request = protocol::create_list_tools_request();
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(anyhow::anyhow!("Server error: {} - {}", error.code, error.message));
        }

        let result = response.result
            .ok_or_else(|| anyhow::anyhow!("No result in list_tools response"))?;

        let tools = protocol::parse_tool_info(&result)?;
        *self.tools.write().await = tools.clone();

        Ok(tools)
    }

    /// Call a tool on the server
    pub async fn call_tool(&self, tool_name: &str, arguments: HashMap<String, Value>) -> anyhow::Result<Value> {
        let tools = self.tools.read().await;
        if !tools.iter().any(|t| t.name == tool_name) {
            return Err(anyhow::anyhow!("Tool not found: {}", tool_name).into());
        }
        drop(tools);

        let request = protocol::create_call_tool_request(tool_name, arguments);
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(anyhow::anyhow!("Tool call error: {} - {}", error.code, error.message).into());
        }

        response.result
            .ok_or_else(|| anyhow::anyhow!("No result in call_tool response").into())
    }

    /// Get server info
    pub fn server_info(&self) -> Option<MCPServerInfo> {
        self.server_info.try_read().ok().and_then(|g| g.clone())
    }

    /// Get cached tools
    pub fn cached_tools(&self) -> Vec<ToolInfo> {
        self.tools.try_read().ok().map(|g| g.clone()).unwrap_or_default()
    }

    /// Health check
    pub async fn health_check(&self) -> bool {
        if self.process.read().await.is_none() {
            return false;
        }

        if let Ok(tools) = self.list_tools().await {
            !tools.is_empty()
        } else {
            false
        }
    }

    /// Shutdown the MCP server
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let request = protocol::create_shutdown_request();
        let _ = self.send_request(request).await;

        if let Some(mut child) = self.process.write().await.take() {
            let _ = child.kill();
        }

        *self.stdin.write().await = None;
        *self.stdout.write().await = None;

        info!("MCP server {} shutdown", self.name);
        Ok(())
    }
}

impl Drop for MCPClient {
    fn drop(&mut self) {
        // Use try_write because we can't block in Drop
        if let Ok(mut guard) = self.process.try_write() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
            }
        }
    }
}

/// MCP client pool for managing multiple MCP server connections
pub struct MCPClientPool {
    clients: HashMap<String, Arc<MCPClient>>,
}

impl MCPClientPool {
    pub fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    /// Add a client to the pool
    pub fn add(&mut self, name: String, client: Arc<MCPClient>) {
        self.clients.insert(name, client);
    }

    /// Get a client by name
    pub fn get(&self, name: &str) -> Option<Arc<MCPClient>> {
        self.clients.get(name).cloned()
    }

    /// Remove a client from the pool
    pub async fn remove(&mut self, name: &str) -> anyhow::Result<()> {
        if let Some(client) = self.clients.remove(name) {
            client.shutdown().await?;
        }
        Ok(())
    }

    /// Get all client names
    pub fn names(&self) -> Vec<String> {
        self.clients.keys().cloned().collect()
    }

    /// Return all cached tools across all connected servers, paired with server name.
    pub fn all_cached_tools(&self) -> Vec<(String, aiclaw_types::mcp::ToolInfo)> {
        let mut out = Vec::new();
        for (name, client) in &self.clients {
            for tool in client.cached_tools() {
                out.push((name.clone(), tool));
            }
        }
        out
    }

    /// Check health of all clients
    pub async fn health_check_all(&self) -> HashMap<String, bool> {
        let mut results = HashMap::new();
        for (name, client) in &self.clients {
            results.insert(name.clone(), client.health_check().await);
        }
        results
    }
}

impl Default for MCPClientPool {
    fn default() -> Self {
        Self::new()
    }
}

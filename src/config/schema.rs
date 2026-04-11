//! Configuration schema

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ============================================================================
// LLM Configuration
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LLMConfig {
    #[serde(default = "default_llm_enabled")]
    pub enabled: bool,
    #[serde(default = "default_llm_provider")]
    pub default_provider: String,
    #[serde(default)]
    pub routing: LLMRoutingConfig,
    #[serde(default)]
    pub providers: HashMap<String, LLMProviderConfig>,
}

fn default_llm_enabled() -> bool {
    true
}

fn default_llm_provider() -> String {
    "anthropic".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LLMRoutingConfig {
    #[serde(default = "default_routing_mode")]
    pub mode: String,
    #[serde(default)]
    pub openrouter: OpenRouterConfig,
    #[serde(default)]
    pub ollama: OllamaConfig,
}

fn default_routing_mode() -> String {
    "direct".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRouterConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OllamaConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_ollama_url")]
    pub base_url: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

fn default_ollama_url() -> String {
    "http://localhost:11434".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LLMProviderConfig {
    pub enabled: bool,
    pub provider_type: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_llm_model")]
    pub model: String,
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_llm_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
}

fn default_llm_model() -> String {
    "gpt-4o".to_string()
}

fn default_llm_max_tokens() -> u32 {
    1024
}

fn default_llm_timeout() -> u64 {
    60
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_provider: default_llm_provider(),
            routing: LLMRoutingConfig::default(),
            providers: HashMap::new(),
        }
    }
}

impl Default for LLMRoutingConfig {
    fn default() -> Self {
        Self {
            mode: default_routing_mode(),
            openrouter: OpenRouterConfig::default(),
            ollama: OllamaConfig::default(),
        }
    }
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: Some("https://openrouter.ai/api".to_string()),
            api_key: None,
        }
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: default_ollama_url(),
            model: Some("llama3".to_string()),
            timeout_secs: Some(120),
        }
    }
}

impl Default for LLMProviderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider_type: "openai".to_string(),
            api_key: None,
            base_url: None,
            model: default_llm_model(),
            max_tokens: default_llm_max_tokens(),
            timeout_secs: default_llm_timeout(),
            retry_attempts: 3,
            extra_headers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Config {
    #[serde(default)]
    pub agent: AgentConfig,

    #[serde(default)]
    pub channels: HashMap<String, ChannelConfig>,

    #[serde(default)]
    pub skills: SkillsConfig,

    #[serde(default)]
    pub mcp: MCPConfig,

    #[serde(default)]
    pub aiops: HashMap<String, AIOpsProviderConfig>,

    /// Logical cluster name → kubectl `--context` mapping (host `kubectl` only; no in-process K8s client).
    /// TOML: `[clusters.<name>]`; legacy `[kubernetes.<name>]` is accepted via `alias`.
    #[serde(default, alias = "kubernetes")]
    pub clusters: HashMap<String, ClusterConfig>,

    #[serde(default)]
    pub observability: ObservabilityConfig,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default)]
    pub logging: LoggingConfig,

    #[serde(default)]
    pub llm: LLMConfig,
}

fn channel_config_enabled(c: &ChannelConfig) -> bool {
    match c {
        ChannelConfig::Feishu(f) => f.enabled,
        ChannelConfig::WeCom(w) => w.enabled,
        ChannelConfig::Local(l) => l.enabled,
    }
}

impl Config {
    /// Bounded capacity for the agent inbound queue. When **only** `[channels.*]` is a single enabled
    /// local **stdio** channel, use `1` so an extra typed line blocks until the current turn finishes.
    pub fn agent_message_channel_capacity(&self) -> usize {
        let enabled: Vec<&ChannelConfig> = self
            .channels
            .values()
            .filter(|c| channel_config_enabled(c))
            .collect();
        if enabled.len() == 1 {
            if let ChannelConfig::Local(l) = enabled[0] {
                if l.mode == LocalChannelMode::Stdio {
                    return 1;
                }
            }
        }
        100
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfig {
    #[serde(default = "default_name")]
    pub name: String,

    #[serde(default = "default_session_timeout")]
    pub session_timeout_secs: u64,

    #[serde(default = "default_concurrent_limit")]
    pub concurrent_limit: usize,

    #[serde(default)]
    pub default_cluster: Option<String>,

    #[serde(default)]
    pub compact_prompt: bool,
}

fn default_name() -> String {
    "aiclaw".to_string()
}

fn default_session_timeout() -> u64 {
    3600
}

fn default_concurrent_limit() -> usize {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ChannelConfig {
    Feishu(FeishuConfig),
    WeCom(WeComConfig),
    Local(LocalConfig),
}

/// Local terminal or WebSocket gateway (see `LocalChannel`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LocalChannelMode {
    /// Read user lines from stdin; print assistant replies to stdout.
    #[default]
    Stdio,
    /// Bind HTTP + `/ws` WebSocket on `bind`:`port`.
    Http,
}

fn default_local_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_local_port() -> u16 {
    18789
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LocalConfig {
    pub enabled: bool,

    #[serde(default)]
    pub mode: LocalChannelMode,

    #[serde(default = "default_local_bind")]
    pub bind: String,

    #[serde(default = "default_local_port")]
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeishuConfig {
    pub enabled: bool,

    #[serde(default)]
    pub bot_name: Option<String>,

    #[serde(default)]
    pub verify_token: Option<String>,

    #[serde(default)]
    pub encrypt_key: Option<String>,

    #[serde(default)]
    pub app_id: Option<String>,

    #[serde(default)]
    pub app_secret: Option<String>,

    #[serde(default)]
    pub webhook_url: Option<String>,

    /// Local address to listen for incoming Feishu webhooks (e.g., "0.0.0.0:8089")
    #[serde(default, alias = "webhook_listen_addr")]
    pub webhook_listen_addr: Option<String>,

    #[serde(default, alias = "long_polling_timeout_secs")]
    pub polling_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WeComConfig {
    pub enabled: bool,

    #[serde(default)]
    pub corp_id: Option<String>,

    #[serde(default)]
    pub corp_secret: Option<String>,

    #[serde(default)]
    pub agent_id: Option<String>,

    #[serde(default)]
    pub webhook_url: Option<String>,

    /// Local address to listen for incoming WeCom webhooks (e.g., "0.0.0.0:8090")
    #[serde(default, alias = "webhook_listen_addr")]
    pub webhook_listen_addr: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SkillsConfig {
    #[serde(default = "default_skills_dir")]
    pub skills_dir: PathBuf,

    #[serde(default = "default_open_skills_enabled")]
    pub open_skills_enabled: bool,

    #[serde(default)]
    pub open_skills_dir: Option<PathBuf>,

    #[serde(default)]
    pub allowed_scripts: bool,

    #[serde(default)]
    pub trusted_skill_roots: Vec<PathBuf>,

    /// LLM-driven shell execution policy (OpenClaw-style `tools.exec` subset).
    #[serde(default)]
    pub exec: SkillsExecConfig,
}

/// Controls how skill-driven shell commands are validated before execution.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SkillsExecSecurity {
    /// Reject every shell command (policy kill-switch).
    Deny,
    /// Use kubectl/helm allowlists and blocked substrings (recommended for production).
    #[default]
    Allowlist,
    /// Allow any base command except blocked keywords / pipes (development only).
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SkillsExecConfig {
    /// When true, `SKILL.md` skills may run the LLM iteration + shell loop.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub security: SkillsExecSecurity,
    /// Maximum LLM iterations (each may run one shell command).
    #[serde(default = "default_skills_exec_max_steps")]
    pub max_steps: usize,
    /// Per-command timeout (seconds).
    #[serde(default = "default_skills_exec_timeout_secs")]
    pub timeout_secs: u64,
    /// Prepend `kubectl --context=<name>` for commands starting with `kubectl`.
    #[serde(default)]
    pub prepend_kubectl_context: bool,
    /// When `security = allowlist`, also allow `helm list|get|status|version` style reads.
    #[serde(default)]
    pub allow_helm: bool,
}

fn default_skills_exec_max_steps() -> usize {
    10
}

fn default_skills_exec_timeout_secs() -> u64 {
    120
}

impl Default for SkillsExecConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            security: SkillsExecSecurity::Allowlist,
            max_steps: default_skills_exec_max_steps(),
            timeout_secs: default_skills_exec_timeout_secs(),
            prepend_kubectl_context: false,
            allow_helm: false,
        }
    }
}

fn default_skills_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".aiclaw")
        .join("skills")
}

fn default_open_skills_enabled() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MCPConfig {
    #[serde(default)]
    pub servers: HashMap<String, MCPServerConfig>,

    #[serde(default)]
    pub transport: MCPTransport,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(tag = "transport_type")]
pub enum MCPTransport {
    #[default]
    Stdio,
    SSE { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MCPServerConfig {
    pub enabled: bool,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AIOpsProviderConfig {
    pub enabled: bool,
    pub provider_type: String,
    pub endpoint: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClusterConfig {
    pub enabled: bool,
    /// When set, used as `kubectl --context`; when absent, the TOML table key (logical name) is used.
    #[serde(default)]
    pub context: Option<String>,
}

impl ClusterConfig {
    /// Context string passed to `kubectl --context=...` when `prepend_kubectl_context` is enabled.
    pub fn kubectl_context_name<'a>(&'a self, logical_name: &'a str) -> &'a str {
        self.context.as_deref().unwrap_or(logical_name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObservabilityConfig {
    #[serde(default)]
    pub observability_type: ObservabilityType,

    #[serde(default)]
    pub otel_endpoint: Option<String>,

    #[serde(default)]
    pub metrics_port: Option<u16>,

    #[serde(default)]
    pub tracing_level: TracingLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub enum ObservabilityType {
    #[default]
    Log,
    Prometheus,
    OpenTelemetry,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub enum TracingLevel {
    #[default]
    Info,
    Debug,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct SecurityConfig {
    #[serde(default = "default_dangerous_commands")]
    pub dangerous_commands: Vec<String>,

    #[serde(default)]
    pub sensitive_path_suffixes: Vec<String>,

    #[serde(default)]
    pub api_key_vault: Option<String>,
}

fn default_dangerous_commands() -> Vec<String> {
    vec![
        "rm".to_string(),
        "dd".to_string(),
        "mkfs".to_string(),
        ">:".to_string(),
        "|".to_string(),
        "&".to_string(),
        ";".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,

    #[serde(default)]
    pub format: LogFormat,

    #[serde(default)]
    pub output: LogOutput,
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub enum LogFormat {
    #[default]
    Json,
    Plain,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub enum LogOutput {
    #[default]
    Stdout,
    File { path: PathBuf },
}

impl Default for Config {
    fn default() -> Self {
        Self {
            agent: AgentConfig::default(),
            channels: HashMap::new(),
            skills: SkillsConfig::default(),
            mcp: MCPConfig::default(),
            aiops: HashMap::new(),
            clusters: HashMap::new(),
            observability: ObservabilityConfig::default(),
            security: SecurityConfig::default(),
            logging: LoggingConfig::default(),
            llm: LLMConfig::default(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            session_timeout_secs: default_session_timeout(),
            concurrent_limit: default_concurrent_limit(),
            default_cluster: None,
            compact_prompt: false,
        }
    }
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_dir: default_skills_dir(),
            open_skills_enabled: default_open_skills_enabled(),
            open_skills_dir: None,
            allowed_scripts: false,
            trusted_skill_roots: Vec::new(),
            exec: SkillsExecConfig::default(),
        }
    }
}

impl Default for MCPConfig {
    fn default() -> Self {
        Self {
            servers: HashMap::new(),
            transport: MCPTransport::Stdio,
        }
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            observability_type: ObservabilityType::Log,
            otel_endpoint: None,
            metrics_port: None,
            tracing_level: TracingLevel::Info,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: LogFormat::Json,
            output: LogOutput::Stdout,
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn load_or_default(config_path: Option<&PathBuf>) -> anyhow::Result<Self> {
        match config_path {
            Some(path) if path.exists() => Self::load(path),
            _ => Ok(Config::default()),
        }
    }
}

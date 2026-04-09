//! Configuration schema

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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

    #[serde(default)]
    pub kubernetes: HashMap<String, K8sClusterConfig>,

    #[serde(default)]
    pub observability: ObservabilityConfig,

    #[serde(default)]
    pub security: SecurityConfig,

    #[serde(default)]
    pub logging: LoggingConfig,
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

    #[serde(default)]
    pub long polling_timeout_secs: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport_type")]
pub enum MCPTransport {
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
pub struct K8sClusterConfig {
    pub enabled: bool,
    #[serde(default)]
    pub context: Option<String>,
    pub kubeconfig_path: PathBuf,
    #[serde(default = "default_namespace")]
    pub default_namespace: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_namespace() -> String {
    "default".to_string()
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecurityConfig {
    #[serde(default = "default_dangerous_commands")]
    pub dangerous_commands: Vec<String>,

    #[serde(default)]
    pub sensitive_path_suffixes: Vec<String>,

    #[serde(default)]
    pub allowed_kubeconfig_paths: Vec<PathBuf>,

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
            kubernetes: HashMap::new(),
            observability: ObservabilityConfig::default(),
            security: SecurityConfig::default(),
            logging: LoggingConfig::default(),
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

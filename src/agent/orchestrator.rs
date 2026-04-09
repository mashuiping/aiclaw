//! Agent orchestrator - main agent logic

use aiclaw_types::agent::{AgentResponse, Intent, IntentType, OutgoingMessage, OutputFormat};
use aiclaw_types::channel::{ChannelMessage, SendMessage};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::intent::IntentParser;
use super::router::{Router, RouteResult, SkillRegistryAccess};
use super::session::SessionManager;
use crate::aiops::AIOpsProvider;
use crate::channels::Channel;
use crate::mcp::MCPClientPool;
use crate::observability::{Observer, ObserverEvent};
use crate::skills::SkillRegistry;

/// Agent orchestrator - coordinates all agent components
pub struct AgentOrchestrator {
    name: String,
    session_manager: Arc<SessionManager>,
    intent_parser: IntentParser,
    router: Arc<Router>,
    skill_registry: Arc<SkillRegistry>,
    mcp_pool: Arc<MCPClientPool>,
    aiops_providers: HashMap<String, Box<dyn AIOpsProvider>>,
    k8s_clients: HashMap<String, Box<dyn crate::kubernetes::K8sClient>>,
    channels: HashMap<String, Box<dyn Channel>>,
    observer: Arc<dyn Observer>,
}

impl AgentOrchestrator {
    pub fn new(
        name: impl Into<String>,
        session_manager: Arc<SessionManager>,
        skill_registry: Arc<SkillRegistry>,
        mcp_pool: Arc<MCPClientPool>,
        aiops_providers: HashMap<String, Box<dyn AIOpsProvider>>,
        k8s_clients: HashMap<String, Box<dyn crate::kubernetes::K8sClient>>,
        channels: HashMap<String, Box<dyn Channel>>,
        observer: Arc<dyn Observer>,
    ) -> Self {
        Self {
            name: name.into(),
            session_manager,
            intent_parser: IntentParser::new(),
            router: Arc::new(Router::new(skill_registry.clone())),
            skill_registry,
            mcp_pool,
            aiops_providers,
            k8s_clients,
            channels,
            observer,
        }
    }

    /// Handle an incoming message
    pub async fn handle_message(&self, message: ChannelMessage) -> anyhow::Result<AgentResponse> {
        let session = self.session_manager.get_or_create(
            &message.sender.user_id,
            &message.channel_name,
            message.thread_id.as_deref(),
        );

        self.observer.record_event(ObserverEvent::AgentStart {
            provider: "internal".to_string(),
            model: "rule-based".to_string(),
        });

        info!(
            "Handling message from {} on channel {}: {}",
            message.sender.user_id,
            message.channel_name,
            &message.content.text[..message.content.text.len().min(100)]
        );

        let intent = self.intent_parser.parse(&message.content.text);
        debug!("Parsed intent: {:?} (confidence: {:.2})", intent.intent_type, intent.confidence);

        let routes = self.router.route(&intent);
        debug!("Routed to: {:?}", routes);

        let response = if routes.is_empty() {
            self.handle_unknown_intent(&message, &intent).await
        } else {
            self.execute_routes(&message, &intent, &routes).await
        };

        let _ = self.session_manager.add_interaction(
            &session.id,
            &format!("{:?}", intent.intent_type),
            routes.first().and_then(|r| r.skill_name.as_deref()),
            response.as_ref().ok().map(|r| r.message.content.clone()),
            response.as_ref().is_ok(),
        );

        self.observer.record_event(ObserverEvent::AgentEnd {
            provider: "internal".to_string(),
            model: "rule-based".to_string(),
            duration: Default::default(),
            tokens_used: None,
            cost_usd: None,
        });

        response
    }

    /// Handle unknown intent
    async fn handle_unknown_intent(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
    ) -> anyhow::Result<AgentResponse> {
        let response_text = format!(
            "抱歉，我没有理解你的请求。你说的是 \"{}\" 吗？\n\n我可以帮你：\n- 查看日志：\"查看 pod xxx 的日志\"\n- 查询指标：\"查询 CPU 使用率\"\n- 检查健康：\"检查集群状态\"\n- 排查问题：\"排查 pod xxx\"",
            intent.raw_query
        );

        let response = AgentResponse {
            session_id: message.channel_id.clone(),
            message: OutgoingMessage {
                content: response_text,
                format: OutputFormat::Markdown,
                code_block: None,
                table: None,
            },
            success: false,
            evidence: vec![],
            error: Some("Unknown intent".to_string()),
        };

        Ok(response)
    }

    /// Execute routes
    async fn execute_routes(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
        routes: &[RouteResult],
    ) -> anyhow::Result<AgentResponse> {
        let mut all_evidence = Vec::new();
        let mut response_text = String::new();
        let mut success = true;

        for route in routes {
            if let Some(ref skill_name) = route.skill_name {
                match self.execute_skill(skill_name, message, intent).await {
                    Ok((text, evidence)) => {
                        response_text = text;
                        all_evidence.extend(evidence);
                        break;
                    }
                    Err(e) => {
                        warn!("Skill {} failed: {}", skill_name, e);
                        success = false;
                    }
                }
            } else if let (Some(ref mcp_server), Some(ref tool_name)) = (&route.mcp_server, &route.tool_name) {
                match self.execute_mcp_tool(mcp_server, tool_name, message, intent).await {
                    Ok((text, evidence)) => {
                        response_text = text;
                        all_evidence.extend(evidence);
                        break;
                    }
                    Err(e) => {
                        warn!("MCP tool {}:{} failed: {}", mcp_server, tool_name, e);
                        success = false;
                    }
                }
            }
        }

        if response_text.is_empty() {
            response_text = "抱歉，处理你的请求时遇到了问题，请稍后重试。".to_string();
            success = false;
        }

        Ok(AgentResponse {
            session_id: message.channel_id.clone(),
            message: OutgoingMessage {
                content: response_text,
                format: OutputFormat::Markdown,
                code_block: None,
                table: None,
            },
            success,
            evidence: all_evidence,
            error: if success { None } else { Some("Execution failed".to_string()) },
        })
    }

    /// Execute a skill
    async fn execute_skill(
        &self,
        skill_name: &str,
        message: &ChannelMessage,
        intent: &Intent,
    ) -> anyhow::Result<(String, Vec<aiclaw_types::agent::EvidenceRecord>)> {
        info!("Executing skill: {}", skill_name);

        self.observer.record_event(ObserverEvent::SkillExecutionStart {
            skill: skill_name.to_string(),
        });

        let skill = self.skill_registry.get(skill_name)
            .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", skill_name))?;

        let mut params = HashMap::new();
        if let Some(ref pod) = intent.entities.pod_name {
            params.insert("pod_name".to_string(), pod.clone());
        }
        if let Some(ref ns) = intent.entities.namespace {
            params.insert("namespace".to_string(), ns.clone());
        }
        if let Some(ref cluster) = intent.entities.cluster {
            params.insert("cluster".to_string(), cluster.clone());
        }

        let context = aiclaw_types::skill::SkillContext {
            skill_name: skill_name.to_string(),
            user_id: message.sender.user_id.clone(),
            channel: message.channel_name.clone(),
            thread_id: message.thread_id.clone(),
            parameters: params,
            session_id: Some(message.channel_id.clone()),
        };

        let results = self.execute_skill_tools(&skill, &context).await?;

        let mut output = String::new();
        let mut evidence = Vec::new();

        for result in results {
            if result.success {
                if let Some(ref out) = result.output {
                    output += out;
                    output += "\n";
                }
            } else {
                if let Some(ref err) = result.error {
                    output += &format!("Error: {}\n", err);
                }
            }

            evidence.push(aiclaw_types::agent::EvidenceRecord {
                timestamp: Utc::now(),
                source: "skill".to_string(),
                action: result.tool_name,
                data: serde_json::json!({
                    "success": result.success,
                    "output": result.output,
                    "error": result.error,
                }),
            });
        }

        self.observer.record_event(ObserverEvent::SkillExecutionEnd {
            skill: skill_name.to_string(),
            duration: Default::default(),
            success: !output.is_empty(),
        });

        Ok((output, evidence))
    }

    /// Execute skill tools
    async fn execute_skill_tools(
        &self,
        skill: &aiclaw_types::skill::SkillMetadata,
        context: &aiclaw_types::skill::SkillContext,
    ) -> anyhow::Result<Vec<aiclaw_types::skill::ToolResult>> {
        use crate::skills::SkillExecutor;

        let executor = SkillExecutor::new();
        let mut results = Vec::new();

        let tools = self.skill_registry.get_tools(skill.name.as_str())
            .unwrap_or_default();

        for tool in tools {
            let args: HashMap<String, String> = tool.args.iter()
                .map(|(k, v)| {
                    let mut interpolated = v.clone();
                    for (pk, pv) in &context.parameters {
                        interpolated = interpolated.replace(&format!("{{{{{}}}}}", pk), pv);
                    }
                    (k.clone(), interpolated)
                })
                .collect();

            match executor.execute_tool(&tool, &args).await {
                Ok(result) => results.push(result),
                Err(e) => {
                    results.push(aiclaw_types::skill::ToolResult {
                        tool_name: tool.name.clone(),
                        success: false,
                        output: None,
                        error: Some(e.to_string()),
                        execution_time_ms: 0,
                        evidence: vec![],
                    });
                }
            }
        }

        Ok(results)
    }

    /// Execute MCP tool
    async fn execute_mcp_tool(
        &self,
        mcp_server: &str,
        tool_name: &str,
        message: &ChannelMessage,
        intent: &Intent,
    ) -> anyhow::Result<(String, Vec<aiclaw_types::agent::EvidenceRecord>)> {
        info!("Executing MCP tool: {}:{}", mcp_server, tool_name);

        let client = self.mcp_pool.get(mcp_server)
            .ok_or_else(|| anyhow::anyhow!("MCP server not found: {}", mcp_server))?;

        let mut args = HashMap::new();
        if let Some(ref query) = intent.entities.query {
            args.insert("query".to_string(), serde_json::json!(query.clone()));
        }
        if let Some(ref pod) = intent.entities.pod_name {
            args.insert("pod".to_string(), serde_json::json!(pod.clone()));
        }
        if let Some(ref ns) = intent.entities.namespace {
            args.insert("namespace".to_string(), serde_json::json!(ns.clone()));
        }

        let start = std::time::Instant::now();
        let result = client.call_tool(tool_name, args).await;
        let duration = start.elapsed();

        let evidence = vec![aiclaw_types::agent::EvidenceRecord {
            timestamp: Utc::now(),
            source: "mcp".to_string(),
            action: tool_name.to_string(),
            data: serde_json::json!({
                "server": mcp_server,
                "duration_ms": duration.as_millis(),
            }),
        }];

        match result {
            Ok(value) => {
                self.observer.record_event(ObserverEvent::McpCall {
                    server: mcp_server.to_string(),
                    tool: tool_name.to_string(),
                    duration,
                    success: true,
                });

                let output = serde_json::to_string_pretty(&value)?;
                Ok((output, evidence))
            }
            Err(e) => {
                self.observer.record_event(ObserverEvent::McpCall {
                    server: mcp_server.to_string(),
                    tool: tool_name.to_string(),
                    duration,
                    success: false,
                });

                Err(e)
            }
        }
    }

    /// Send response back to channel
    pub async fn send_response(&self, message: &SendMessage) -> anyhow::Result<()> {
        let channel = self.channels.get(&message.recipient)
            .or_else(|| self.channels.values().next())
            .ok_or_else(|| anyhow::anyhow!("No channel available"))?;

        channel.send(message).await
    }

    /// Start message processing loop
    pub async fn start(&self, mut rx: mpsc::Receiver<ChannelMessage>) {
        info!("Agent orchestrator {} started", self.name);

        while let Some(message) = rx.recv().await {
            match self.handle_message(message).await {
                Ok(response) => {
                    let send_msg = SendMessage::markdown(
                        &response.session_id,
                        &response.message.content,
                    );

                    if let Err(e) = self.send_response(&send_msg).await {
                        error!("Failed to send response: {}", e);
                    }
                }
                Err(e) => {
                    error!("Failed to handle message: {}", e);
                }
            }
        }
    }

    /// Health check
    pub async fn health_check(&self) -> bool {
        for channel in self.channels.values() {
            if !channel.health_check().await {
                return false;
            }
        }
        true
    }
}

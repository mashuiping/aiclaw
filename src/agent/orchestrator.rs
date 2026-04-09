//! Agent orchestrator - main agent logic

use aiclaw_types::agent::{AgentResponse, Intent, IntentType, MessageRole, OutgoingMessage, OutputFormat};
use aiclaw_types::channel::{ChannelMessage, SendMessage};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::intent::IntentParser;
use super::planner::{ExecutionPlan, Planner, QueryType};
use super::router::{Router, RouteResult, SkillRegistryAccess};
use super::session::SessionManager;
use crate::aiops::AIOpsProvider;
use crate::channels::Channel;
use crate::llm::summarizer::{Summarizer, ToolOutput};
use crate::llm::traits::LLMProvider;
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
    summarizer: Option<Arc<Summarizer>>,
    planner: Option<Arc<Planner>>,
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
            summarizer: None,
            planner: None,
        }
    }

    pub fn with_llm(
        name: impl Into<String>,
        session_manager: Arc<SessionManager>,
        skill_registry: Arc<SkillRegistry>,
        mcp_pool: Arc<MCPClientPool>,
        aiops_providers: HashMap<String, Box<dyn AIOpsProvider>>,
        k8s_clients: HashMap<String, Box<dyn crate::kubernetes::K8sClient>>,
        channels: HashMap<String, Box<dyn Channel>>,
        observer: Arc<dyn Observer>,
        llm_provider: Option<Arc<dyn LLMProvider>>,
    ) -> Self {
        let summarizer = llm_provider.as_ref().map(|p| Arc::new(Summarizer::new(p.clone())));
        let planner = llm_provider.as_ref().map(|p| Arc::new(Planner::new(p.clone())));

        let intent_parser = match llm_provider {
            Some(provider) => IntentParser::with_llm(provider),
            None => IntentParser::new(),
        };

        Self {
            name: name.into(),
            session_manager,
            intent_parser,
            router: Arc::new(Router::new(skill_registry.clone())),
            skill_registry,
            mcp_pool,
            aiops_providers,
            k8s_clients,
            channels,
            observer,
            summarizer,
            planner,
        }
    }

    /// Handle an incoming message
    pub async fn handle_message(&self, message: ChannelMessage) -> anyhow::Result<AgentResponse> {
        let session = self.session_manager.get_or_create(
            &message.sender.user_id,
            &message.channel_name,
            message.thread_id.as_deref(),
        );

        // Add user message to conversation history
        self.session_manager.add_message(
            &session.id,
            MessageRole::User,
            message.content.text.clone(),
        );

        let model_used = if self.summarizer.is_some() {
            "llm-powered"
        } else {
            "rule-based"
        };

        self.observer.record_event(ObserverEvent::AgentStart {
            provider: "internal".to_string(),
            model: model_used.to_string(),
        });

        info!(
            "Handling message from {} on channel {}: {}",
            message.sender.user_id,
            message.channel_name,
            &message.content.text[..message.content.text.len().min(100)]
        );

        // Check if there's a pending clarification question
        let pending_question = session.context.pending_question.clone();

        // Check for follow-up questions or clarification requests
        let is_followup = self.is_followup_question(&message.content.text);

        let mut intent = if is_followup && self.summarizer.is_some() {
            // For follow-up questions, use LLM to interpret in context
            self.parse_followup_intent(&message.content.text, &session).await
        } else {
            self.intent_parser.parse(&message.content.text).await
        };

        // Inherit cluster/namespace from session context if not specified in message
        if intent.entities.cluster.is_none() {
            intent.entities.cluster = session.context.current_cluster.clone();
        }
        if intent.entities.namespace.is_none() {
            intent.entities.namespace = session.context.current_namespace.clone();
        }

        debug!("Parsed intent: {:?} (confidence: {:.2})", intent.intent_type, intent.confidence);

        let routes = self.router.route(&intent);
        debug!("Routed to: {:?}", routes);

        // For Debug intent with planner available, use planner-based execution
        let response = if intent.intent_type == IntentType::Debug && self.planner.is_some() {
            self.execute_with_planner(&message, &intent).await
        } else if routes.is_empty() {
            self.handle_unknown_intent(&message, &intent).await
        } else if self.is_multi_cluster_query(&intent) {
            // Execute across multiple clusters
            self.execute_multi_cluster(&message, &intent, &routes).await
        } else {
            self.execute_routes(&message, &intent, &routes).await
        };

        // Add interaction to history
        let _ = self.session_manager.add_interaction(
            &session.id,
            &format!("{:?}", intent.intent_type),
            routes.first().and_then(|r| r.skill_name.as_deref()),
            response.as_ref().ok().map(|r| r.message.content.clone()),
            response.as_ref().is_ok(),
        );

        // Add assistant response to conversation history
        if let Ok(ref resp) = response {
            self.session_manager.add_message(
                &session.id,
                MessageRole::Assistant,
                resp.message.content.clone(),
            );

            // Update context with extracted entities
            if let Some(ref cluster) = intent.entities.cluster {
                self.session_manager.set_current_cluster(&session.id, cluster.clone());
            }
            if let Some(ref ns) = intent.entities.namespace {
                self.session_manager.set_current_namespace(&session.id, ns.clone());
            }
        }

        self.observer.record_event(ObserverEvent::AgentEnd {
            provider: "internal".to_string(),
            model: model_used.to_string(),
            duration: Default::default(),
            tokens_used: None,
            cost_usd: None,
        });

        response
    }

    /// Check if message is a follow-up question
    fn is_followup_question(&self, message: &str) -> bool {
        let lower = message.to_lowercase();
        let followup_patterns = [
            "然后呢", "然后", "接下来", "还有呢",
            "详细", "具体", "解释一下", "为什么",
            "怎么", "如何", "什么意思",
            "继续", "说更多", "补充",
            "是", "不是", "对", "不对",
        ];

        // Check if message is short (likely a follow-up)
        if message.chars().count() < 20 {
            return true;
        }

        // Check for follow-up keywords
        for pattern in &followup_patterns {
            if lower.contains(pattern) {
                return true;
            }
        }

        false
    }

    /// Parse follow-up intent using conversation context
    async fn parse_followup_intent(&self, message: &str, session: &aiclaw_types::agent::Session) -> Intent {
        use crate::llm::types::ChatMessage;

        // Build context from conversation history
        let history_context = session
            .context
            .conversation_history
            .iter()
            .rev()
            .take(6) // Last 6 messages
            .map(|m| format!("{}: {}", m.role.as_str(), m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let context_hint = format!(
            "当前上下文：\n\
             Cluster: {:?}\n\
             Namespace: {:?}\n\
             最近对话：\n{}\n\n\
             当前问题：{}",
            session.context.current_cluster,
            session.context.current_namespace,
            history_context,
            message
        );

        // For now, fall back to regular parsing
        // A full implementation would call LLM to interpret the follow-up in context
        let mut intent = self.intent_parser.parse(message).await;

        // Update entities with session context if not specified in current message
        if intent.entities.cluster.is_none() {
            intent.entities.cluster = session.context.current_cluster.clone();
        }
        if intent.entities.namespace.is_none() {
            intent.entities.namespace = session.context.current_namespace.clone();
        }

        intent
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
        let mut tool_outputs: Vec<ToolOutput> = Vec::new();
        let mut raw_response_text = String::new();
        let mut success = true;

        for route in routes {
            if let Some(ref skill_name) = route.skill_name {
                match self.execute_skill(skill_name, message, intent).await {
                    Ok((text, evidence, tool_results)) => {
                        raw_response_text = text;
                        all_evidence.extend(evidence);
                        // Collect tool outputs for summarization
                        for (tool_name, tool_content, tool_success) in tool_results {
                            tool_outputs.push(ToolOutput::new(tool_name, tool_content, tool_success));
                        }
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
                        raw_response_text = text;
                        all_evidence.extend(evidence);
                        tool_outputs.push(ToolOutput::new(
                            format!("{}/{}", mcp_server, tool_name),
                            text.clone(),
                            true,
                        ));
                        break;
                    }
                    Err(e) => {
                        warn!("MCP tool {}:{} failed: {}", mcp_server, tool_name, e);
                        success = false;
                    }
                }
            }
        }

        // Try to summarize the results using LLM
        let response_text = if !raw_response_text.is_empty() {
            if let Some(ref summarizer) = self.summarizer {
                match summarizer
                    .summarize(&intent.intent_type, &tool_outputs, &intent.raw_query)
                    .await
                {
                    Ok(summary) => {
                        debug!("LLM summary generated successfully");
                        summary
                    }
                    Err(e) => {
                        warn!("LLM summarization failed, using raw output: {}", e);
                        raw_response_text
                    }
                }
            } else {
                raw_response_text
            }
        } else {
            "抱歉，处理你的请求时遇到了问题，请稍后重试。".to_string();
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
    ) -> anyhow::Result<(
        String,
        Vec<aiclaw_types::agent::EvidenceRecord>,
        Vec<(String, String, bool)>,
    )> {
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
        let mut tool_results: Vec<(String, String, bool)> = Vec::new();

        for result in results {
            let tool_name = result.tool_name.clone();
            let tool_output = if result.success {
                result.output.clone().unwrap_or_default()
            } else {
                result.error.clone().unwrap_or_default()
            };

            output += &format!("[{}]\n{}\n\n", tool_name, tool_output);
            tool_results.push((tool_name.clone(), tool_output, result.success));

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

        Ok((output, evidence, tool_results))
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

    /// Execute using planner-based approach (for Debug intents)
    async fn execute_with_planner(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
    ) -> anyhow::Result<AgentResponse> {
        let Some(ref planner) = self.planner else {
            anyhow::bail!("Planner not available");
        };

        info!("Using planner for Debug intent: {}", intent.raw_query);

        // Create execution plan
        let plan = match planner
            .plan(&intent.raw_query, &format!("{:?}", intent.intent_type))
            .await
        {
            Ok(p) => p,
            Err(e) => {
                warn!("Planner failed, falling back to route-based execution: {}", e);
                let routes = self.router.route(intent);
                return self.execute_routes(message, intent, &routes).await;
            }
        };

        debug!("Execution plan created: {:?}", plan);

        // Execute plan steps
        let mut all_tool_outputs: Vec<ToolOutput> = Vec::new();
        let mut all_evidence: Vec<aiclaw_types::agent::EvidenceRecord> = Vec::new();
        let mut success = true;

        for step in &plan.steps {
            info!("Executing plan step {}: {}", step.step_id, step.description);

            // Build parameters from plan
            let mut params: HashMap<String, String> = HashMap::new();
            for param in &step.parameters {
                // Replace placeholders with actual entity values if available
                let value = if param.value.starts_with("{{") && param.value.ends_with("}}") {
                    let key = &param.value[2..param.value.len() - 2];
                    match key {
                        "pod_name" => intent.entities.pod_name.clone(),
                        "namespace" => intent.entities.namespace.clone(),
                        "cluster" => intent.entities.cluster.clone(),
                        "service_name" => intent.entities.service_name.clone(),
                        _ => Some(param.value.clone()),
                    }
                } else {
                    Some(param.value.clone())
                };

                if let Some(v) = value {
                    params.insert(param.name.clone(), v);
                }
            }

            // Execute based on query type
            let result = self.execute_planned_step(step, &params).await;
            match result {
                Ok((tool_output, evidence)) => {
                    all_tool_outputs.push(tool_output);
                    all_evidence.extend(evidence);
                }
                Err(e) => {
                    warn!("Plan step {} failed: {}", step.step_id, e);
                    success = false;
                }
            }
        }

        // Generate final reasoning response
        let response_text = if let Some(ref summarizer) = self.summarizer {
            // Use LLM to reason over all collected data
            let reasoning_prompt = format!(
                "用户问题：{}\n\nLLM 规划分析：{}\n\n查询结果：\n{}\n\n请综合分析以上信息，给出最终的问题诊断结论和解决方案。",
                intent.raw_query,
                plan.reasoning,
                all_tool_outputs
                    .iter()
                    .map(|o| format!("- {}: {}", o.tool_name, o.content))
                    .collect::<Vec<_>>()
                    .join("\n")
            );

            match summarizer.summarize_text(&reasoning_prompt, "诊断结论和解决方案").await {
                Ok(summary) => summary,
                Err(e) => {
                    warn!("LLM reasoning failed: {}", e);
                    format!(
                        "## 诊断结论\n\n{}\n\n**查询结果**：\n{}\n\n**建议**：请人工查看上述查询结果进行分析。",
                        plan.reasoning,
                        all_tool_outputs
                            .iter()
                            .map(|o| format!("### {}\n{}\n", o.tool_name, o.content))
                            .collect::<Vec<_>>()
                            .join("\n")
                    )
                }
            }
        } else {
            format!(
                "## 诊断分析\n\n{}\n\n**查询结果**：\n{}\n\n**状态**：{}",
                plan.reasoning,
                all_tool_outputs
                    .iter()
                    .map(|o| format!("### {}\n{}\n", o.tool_name, o.content))
                    .collect::<Vec<_>>()
                    .join("\n"),
                if success { "部分成功" } else { "有问题" }
            )
        };

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
            error: if success { None } else { Some("Some steps failed".to_string()) },
        })
    }

    /// Execute a single planned step
    async fn execute_planned_step(
        &self,
        step: &PlanStep,
        params: &HashMap<String, String>,
    ) -> anyhow::Result<(ToolOutput, Vec<aiclaw_types::agent::EvidenceRecord>)> {
        // For now, we execute via skills/MCP based on query type
        // This is a simplified implementation - a full implementation would
        // directly call K8s clients or metrics APIs

        let (tool_name, query_result) = match step.query_type {
            QueryType::PodLogs => {
                let pod = params.get("pod_name").cloned().unwrap_or_default();
                let ns = params.get("namespace").cloned().unwrap_or_else(|| "default".to_string());
                (
                    format!("pod_logs/{}/{}", ns, pod),
                    format!("[模拟日志] Pod {}/{} 的最近 100 行日志", ns, pod),
                )
            }
            QueryType::PodDescribe => {
                let pod = params.get("pod_name").cloned().unwrap_or_default();
                let ns = params.get("namespace").cloned().unwrap_or_else(|| "default".to_string());
                (
                    format!("pod_describe/{}/{}", ns, pod),
                    format!("[模拟] Pod {}/{} 详细信息", ns, pod),
                )
            }
            QueryType::MetricsQuery => {
                let query = params.get("query").cloned().unwrap_or_default();
                (
                    format!("metrics_query/{}", query),
                    format!("[模拟] Metrics 查询: {}", query),
                )
            }
            QueryType::ClusterHealth => (
                "cluster_health".to_string(),
                "[模拟] 集群健康状态: 所有组件正常".to_string(),
            ),
            _ => (
                format!("{}/{:?}", step.description, step.query_type),
                format!("[模拟] 执行: {}", step.description),
            ),
        };

        Ok((
            ToolOutput::new(&tool_name, &query_result, true),
            vec![],
        ))
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

    /// Check if this is a multi-cluster query (no specific cluster specified)
    fn is_multi_cluster_query(&self, intent: &Intent) -> bool {
        // Multi-cluster if no specific cluster is mentioned and user says something like "all clusters"
        if intent.entities.cluster.is_some() {
            return false;
        }

        let query = intent.raw_query.to_lowercase();
        let multi_cluster_patterns = [
            "所有集群", "全部集群", "all cluster", "all clusters",
            "各个集群", "每个集群", "查一下全部", "集群状态",
        ];

        for pattern in &multi_cluster_patterns {
            if query.contains(&pattern.to_lowercase()) {
                return true;
            }
        }

        false
    }

    /// Get list of known clusters
    fn get_known_clusters(&self) -> Vec<String> {
        self.k8s_clients.keys().cloned().collect()
    }

    /// Execute query across multiple clusters
    async fn execute_multi_cluster(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
        routes: &[RouteResult],
    ) -> anyhow::Result<AgentResponse> {
        let clusters = self.get_known_clusters();

        if clusters.is_empty() {
            return self.execute_routes(message, intent, routes).await;
        }

        info!("Executing multi-cluster query across {} clusters: {:?}", clusters.len(), clusters);

        let mut all_tool_outputs: Vec<ToolOutput> = Vec::new();
        let mut all_evidence = Vec::new();
        let mut success = true;

        // Execute routes for each cluster in parallel
        let mut handles = Vec::new();
        for cluster in &clusters {
            let intent_clone = Intent {
                intent_type: intent.intent_type.clone(),
                confidence: intent.confidence,
                entities: {
                    let mut entities = intent.entities.clone();
                    entities.cluster = Some(cluster.clone());
                    entities
                },
                raw_query: intent.raw_query.clone(),
            };

            let routes_clone = routes.to_vec();
            let msg = message.clone();

            handles.push(tokio::spawn(async move {
                (&msg, &intent_clone, &routes_clone, cluster.clone())
            }));
        }

        // Collect results
        for handle in handles {
            if let Ok((msg, intent, routes, cluster)) = handle.await {
                match self.execute_routes(&msg, &intent, &routes).await {
                    Ok(resp) => {
                        if !resp.message.content.is_empty() {
                            all_tool_outputs.push(ToolOutput::new(
                                format!("cluster/{}", cluster),
                                resp.message.content.clone(),
                                resp.success,
                            ));
                        }
                        all_evidence.extend(resp.evidence);
                        if !resp.success {
                            success = false;
                        }
                    }
                    Err(e) => {
                        warn!("Cluster {} query failed: {}", cluster, e);
                        success = false;
                    }
                }
            }
        }

        // Generate aggregated response
        let response_text = if !all_tool_outputs.is_empty() {
            if let Some(ref summarizer) = self.summarizer {
                match summarizer.summarize(&intent.intent_type, &all_tool_outputs, &intent.raw_query).await {
                    Ok(summary) => summary,
                    Err(e) => {
                        warn!("Multi-cluster summarization failed: {}", e);
                        self.format_multi_cluster_output(&all_tool_outputs, &clusters)
                    }
                }
            } else {
                self.format_multi_cluster_output(&all_tool_outputs, &clusters)
            }
        } else {
            "抱歉，所有集群查询都失败了。".to_string();
            success = false;
        };

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
            error: if success { None } else { Some("Multi-cluster query partially failed".to_string()) },
        })
    }

    /// Format multi-cluster outputs into a readable response
    fn format_multi_cluster_output(&self, outputs: &[ToolOutput], clusters: &[String]) -> String {
        let mut result = String::from("## 多集群查询结果\n\n");
        result += &format!("| 集群 | 状态 | 摘要 |\n");
        result += &format!("|------|------|------|\n");

        for (i, output) in outputs.iter().enumerate() {
            let cluster = if i < clusters.len() {
                clusters[i].clone()
            } else {
                format!("cluster-{}", i)
            };

            let status = if output.success { "✅" } else { "❌" };
            let summary = if output.content.len() > 100 {
                format!("{}...", &output.content[..100])
            } else {
                output.content.clone()
            }.replace("\n", " ");

            result += &format!("| {} | {} | {} |\n", cluster, status, summary);
        }

        result
    }
}

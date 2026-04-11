//! Agent orchestrator - main agent logic

/// UI-facing string constants. Centralised so they can be swapped for i18n.
mod messages {
    pub const UNKNOWN_INTENT_HELP: &str = concat!(
        "抱歉，我没有理解你的请求。你说的是 \"{}\" 吗？\n\n",
        "我可以帮你：\n",
        "- 查看日志：\"查看 pod xxx 的日志\"\n",
        "- 查询指标：\"查询 CPU 使用率\"\n",
        "- 检查健康：\"检查集群状态\"\n",
        "- 排查问题：\"排查 pod xxx\"",
    );
    pub const EXEC_ERROR_FALLBACK: &str = "抱歉，处理你的请求时遇到了问题，请稍后重试。";
    pub const NO_LLM_PROVIDER: &str = "已启用 `skills.exec.enabled`，但未配置可用的 LLM provider（请使用 `AgentOrchestrator::with_llm` 并提供模型）。";
    pub const SKILL_EXEC_DISABLED: &str = "该技能包含诊断文档，但未启用自动执行。请在配置中设置 `[skills].exec.enabled = true`（并配置 LLM 与命令安全策略）。";
    pub const SKILL_NO_TOOLS: &str = "该技能未声明可执行工具（`SKILL.toml` 的 `[[tools]]`），且无可用文档内容。";
    pub const ALL_CLUSTERS_FAILED: &str = "抱歉，所有集群查询都失败了。";
    pub const MULTI_CLUSTER_TITLE: &str = "## 多集群查询结果\n\n";
    pub const MULTI_CLUSTER_HEADER: &str = "| 集群 | 状态 | 摘要 |\n|------|------|------|\n";
    pub const DIAGNOSIS_HEADER: &str = "## Diagnosis\n\n";
    pub const RESULTS_LABEL: &str = "**Results**:\n";
    pub const MANUAL_REVIEW: &str = "**Suggestion**: Please review the results above manually.";
    pub const STATUS_PARTIAL: &str = "partial success";
    pub const STATUS_ISSUES: &str = "issues found";

}

use aiclaw_types::agent::{AgentResponse, Intent, IntentType, MessageRole, OutgoingMessage, OutputFormat};
use aiclaw_types::channel::{ChannelMessage, SendMessage};
use chrono::Utc;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::context::ContextManager;
use super::intent::IntentParser;
use super::output_budget::{self, OutputBudget};
use super::planner::{PlanStep, Planner};
use super::prompt_builder::PromptBuilder;
use super::router::{Router, RouteResult};
use super::session::SessionManager;
use crate::aiops::AIOpsProvider;
use crate::channels::Channel;
use crate::config::{ClusterConfig, SkillsExecConfig};
use crate::llm::summarizer::{Summarizer, ToolOutput};
use crate::llm::traits::LLMProvider;
use crate::llm::types::Usage;
use crate::observability::{Observer, ObserverEvent};
use crate::skills::{
    apply_kubectl_context, skill_executor_for_config, LLMSkillExecutor, SkillExecutor, SkillRegistry,
};
use crate::utils::string::utf8_prefix_chars;

/// Agent orchestrator - coordinates all agent components
pub struct AgentOrchestrator {
    name: String,
    session_manager: Arc<SessionManager>,
    intent_parser: IntentParser,
    router: Arc<Router>,
    skill_registry: Arc<SkillRegistry>,
    _aiops_providers: HashMap<String, Box<dyn AIOpsProvider>>,
    clusters: HashMap<String, ClusterConfig>,
    pub channels: HashMap<String, Arc<dyn Channel>>,
    observer: Arc<dyn Observer>,
    summarizer: Option<Arc<Summarizer>>,
    planner: Option<Arc<Planner>>,
    skill_executor: Arc<SkillExecutor>,
    llm_skill_executor: Option<Arc<LLMSkillExecutor>>,
    skills_exec: SkillsExecConfig,
    /// From `AICLAW_KUBECONFIG` at startup; session `kubeconfig_path` takes precedence when present.
    kubeconfig: Option<PathBuf>,
    default_cluster: Option<String>,
    output_budget: OutputBudget,
    context_manager: Option<ContextManager>,
    /// Available for building dynamic prompts when skills context is needed.
    #[allow(dead_code)]
    prompt_builder: PromptBuilder,
}

impl AgentOrchestrator {
    pub fn new(
        name: impl Into<String>,
        session_manager: Arc<SessionManager>,
        skill_registry: Arc<SkillRegistry>,
        _aiops_providers: HashMap<String, Box<dyn AIOpsProvider>>,
        clusters: HashMap<String, ClusterConfig>,
        channels: HashMap<String, Arc<dyn Channel>>,
        observer: Arc<dyn Observer>,
        kubeconfig: Option<PathBuf>,
    ) -> Self {
        let skills_exec = SkillsExecConfig::default();
        let skill_executor = skill_executor_for_config(&skills_exec, kubeconfig.clone());
        Self {
            name: name.into(),
            session_manager,
            intent_parser: {
                let mut p = IntentParser::new();
                p.set_known_clusters(clusters.keys().cloned().collect());
                p
            },
            router: Arc::new(Router::new(skill_registry.clone())),
            skill_registry,
            _aiops_providers,
            clusters,
            channels,
            observer,
            summarizer: None,
            planner: None,
            skill_executor,
            llm_skill_executor: None,
            skills_exec,
            kubeconfig,
            default_cluster: None,
            output_budget: OutputBudget::default_budget(),
            context_manager: None,
            prompt_builder: PromptBuilder::new(),
        }
    }

    pub fn with_llm(
        name: impl Into<String>,
        session_manager: Arc<SessionManager>,
        skill_registry: Arc<SkillRegistry>,
        aiops_providers: HashMap<String, Box<dyn AIOpsProvider>>,
        clusters: HashMap<String, ClusterConfig>,
        channels: HashMap<String, Arc<dyn Channel>>,
        observer: Arc<dyn Observer>,
        llm_provider: Option<Arc<dyn LLMProvider>>,
        skills_exec: SkillsExecConfig,
        kubeconfig: Option<PathBuf>,
        default_cluster: Option<String>,
    ) -> Self {
        let summarizer = llm_provider.as_ref().map(|p| Arc::new(Summarizer::new(p.clone())));
        let planner = llm_provider.as_ref().map(|p| Arc::new(Planner::new(p.clone())));

        let intent_parser = {
            let mut p = match llm_provider.clone() {
                Some(provider) => IntentParser::with_llm(provider),
                None => IntentParser::new(),
            };
            p.set_known_clusters(clusters.keys().cloned().collect());
            p
        };

        let skill_executor = skill_executor_for_config(&skills_exec, kubeconfig.clone());
        let llm_skill_executor = match (&llm_provider, skills_exec.enabled) {
            (Some(provider), true) => Some(Arc::new(LLMSkillExecutor::with_vm_config(
                provider.clone(),
                skill_executor.clone(),
                skills_exec.max_steps,
                std::time::Duration::from_secs(skills_exec.timeout_secs.max(1)),
                skills_exec.victoriametrics.clone(),
            ))),
            _ => None,
        };

        let context_manager = llm_provider.as_ref().map(|p| ContextManager::new(p.clone()));

        Self {
            name: name.into(),
            session_manager,
            intent_parser,
            router: Arc::new(Router::new(skill_registry.clone())),
            skill_registry,
            _aiops_providers: aiops_providers,
            clusters,
            channels,
            observer,
            summarizer,
            planner,
            skill_executor,
            llm_skill_executor,
            skills_exec,
            kubeconfig,
            default_cluster,
            output_budget: OutputBudget::default_budget(),
            context_manager,
            prompt_builder: PromptBuilder::new(),
        }
    }

    /// Append turn timing / token summary for local stdio (stdout transcript).
    fn stdio_reply_markdown(resp: &AgentResponse) -> String {
        let mut body = resp.message.content.clone();
        if resp.source_channel_id != "stdio" {
            return body;
        }
        body.push_str("\n\n---\n");
        let mut parts = Vec::new();
        if let Some(ms) = resp.turn_duration_ms {
            parts.push(format!("turn {} ms", ms));
        }
        if let Some(t) = resp.turn_total_tokens {
            parts.push(format!("llm_tokens {}", t));
        }
        if parts.is_empty() {
            body.push_str("(no LLM token usage recorded for this turn)");
        } else {
            body.push_str(&parts.join(" · "));
        }
        body
    }

    fn effective_kubeconfig_path(&self, session_id: &str) -> Option<PathBuf> {
        self.session_manager
            .get(session_id)
            .and_then(|s| s.context.kubeconfig_path.clone())
            .or_else(|| self.kubeconfig.clone())
    }

    /// Handle an incoming message
    pub async fn handle_message(&self, message: ChannelMessage) -> anyhow::Result<AgentResponse> {
        let stdio_trace = message.channel_id == "stdio";
        let turn_start = Instant::now();
        let mut usage = Usage::zero();

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

        if let Some(p) = crate::skills::kubeconfig_hint::extract_from_user_text(&message.content.text) {
            self.session_manager
                .set_kubeconfig_path(&session.id, p.clone());
            info!(
                session_id = %session.id,
                path = %p.display(),
                "Recorded kubeconfig path from user message"
            );
        }
        let runtime_kube = self.effective_kubeconfig_path(&session.id);

        let model_used = if self.summarizer.is_some() {
            "llm-powered"
        } else {
            "rule-based"
        };

        self.observer.record_event(ObserverEvent::AgentStart {
            provider: "internal".to_string(),
            model: model_used.to_string(),
        });

        if stdio_trace {
            eprintln!("(aiclaw) state=intent · one line per message; reply will appear below as [aiclaw]");
        }

        info!(
            "Handling message from {} on channel {}: {}",
            message.sender.user_id,
            message.channel_name,
            utf8_prefix_chars(&message.content.text, 100)
        );

        // Check for follow-up questions or clarification requests
        let is_followup = self.is_followup_question(&message.content.text);

        let (mut intent, parse_usage) = if is_followup && self.summarizer.is_some() {
            self.parse_followup_intent(&message.content.text, &session).await
        } else {
            self.intent_parser.parse(&message.content.text).await
        };
        usage.merge_assign(&parse_usage);

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

        if stdio_trace {
            eprintln!(
                "(aiclaw) state=routes · intent={:?} route_count={}",
                intent.intent_type,
                routes.len()
            );
        }

        // Log context utilization for observability
        if let Some(ref cm) = self.context_manager {
            let history = self.session_manager.get_conversation_history(&session.id);
            let llm_msgs: Vec<crate::llm::types::ChatMessage> = history
                .iter()
                .map(|h| crate::llm::types::ChatMessage {
                    role: match h.role {
                        aiclaw_types::agent::MessageRole::User => crate::llm::types::MessageRole::User,
                        aiclaw_types::agent::MessageRole::Assistant => crate::llm::types::MessageRole::Assistant,
                        _ => crate::llm::types::MessageRole::User,
                    },
                    content: h.content.clone(),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                })
                .collect();
            cm.log_utilization(&llm_msgs);
        }

        // For Debug intent with planner available, use planner-based execution
        let response = if intent.intent_type == IntentType::Debug && self.planner.is_some() {
            self.execute_with_planner(&message, &intent, runtime_kube.clone(), &mut usage)
                .await
        } else if routes.is_empty() {
            self.handle_unknown_intent(&message, &intent).await
        } else if self.is_multi_cluster_query(&intent) {
            self.execute_multi_cluster(&message, &intent, &routes, runtime_kube.clone(), &mut usage)
                .await
        } else {
            self.execute_routes(&message, &intent, &routes, runtime_kube.clone(), &mut usage)
                .await
        };

        // Add interaction to history
        let result_str = response.as_ref().ok().map(|r| r.message.content.clone());
        let _ = self.session_manager.add_interaction(
            &session.id,
            &format!("{:?}", intent.intent_type),
            routes.first().and_then(|r| r.skill_name.as_deref()),
            result_str.as_deref(),
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

        let elapsed = turn_start.elapsed();
        let token_total = u64::from(usage.total_tokens);
        self.observer.record_event(ObserverEvent::AgentEnd {
            provider: "internal".to_string(),
            model: model_used.to_string(),
            duration: elapsed,
            tokens_used: (token_total > 0).then_some(token_total),
            cost_usd: None,
        });

        if stdio_trace {
            eprintln!(
                "(aiclaw) state=ready · elapsed={:?} llm_total_tokens={}",
                elapsed, usage.total_tokens
            );
        }

        let turn_duration_ms = elapsed.as_millis() as u64;
        let turn_total_tokens = (usage.total_tokens > 0).then_some(usage.total_tokens);

        response.map(|mut r| {
            r.session_id = session.id.clone();
            r.turn_duration_ms = Some(turn_duration_ms);
            r.turn_total_tokens = turn_total_tokens;
            r
        })
    }

    /// Check if message is a follow-up question.
    /// Short messages (< 20 chars) are treated as follow-ups heuristically.
    fn is_followup_question(&self, message: &str) -> bool {
        message.chars().count() < 20
    }

    /// Parse follow-up intent using conversation context.
    ///
    /// Enriches the user's short follow-up message with conversation history
    /// so the intent parser (LLM or rule-based) has enough context to classify.
    async fn parse_followup_intent(
        &self,
        message: &str,
        session: &aiclaw_types::agent::Session,
    ) -> (Intent, Usage) {
        let history_context = session
            .context
            .conversation_history
            .iter()
            .rev()
            .take(6)
            .map(|m| format!("{}: {}", m.role.as_str(), utf8_prefix_chars(&m.content, 200)))
            .collect::<Vec<_>>()
            .join("\n");

        let enriched_message = format!(
            "Context — cluster: {}, namespace: {}\n\
             Recent conversation:\n{}\n\n\
             Current question: {}",
            session.context.current_cluster.as_deref().unwrap_or("unknown"),
            session.context.current_namespace.as_deref().unwrap_or("default"),
            history_context,
            message
        );

        // Pass the enriched message to the intent parser so LLM has context
        let (mut intent, usage) = self.intent_parser.parse(&enriched_message).await;

        // Keep the original short message as raw_query for display
        intent.raw_query = message.to_string();

        if intent.entities.cluster.is_none() {
            intent.entities.cluster = session.context.current_cluster.clone();
        }
        if intent.entities.namespace.is_none() {
            intent.entities.namespace = session.context.current_namespace.clone();
        }

        (intent, usage)
    }

    /// Handle unknown intent
    async fn handle_unknown_intent(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
    ) -> anyhow::Result<AgentResponse> {
        let response_text = messages::UNKNOWN_INTENT_HELP.replace("{}", &intent.raw_query);

        let response = AgentResponse {
            session_id: String::new(),
            channel_name: message.channel_name.clone(),
            source_channel_id: message.channel_id.clone(),
            message: OutgoingMessage {
                content: response_text,
                format: OutputFormat::Markdown,
                code_block: None,
                table: None,
            },
            success: false,
            evidence: vec![],
            error: Some("Unknown intent".to_string()),
            turn_duration_ms: None,
            turn_total_tokens: None,
        };

        Ok(response)
    }

    /// Execute routes
    async fn execute_routes(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
        routes: &[RouteResult],
        kubeconfig: Option<PathBuf>,
        usage: &mut Usage,
    ) -> anyhow::Result<AgentResponse> {
        let mut all_evidence = Vec::new();
        let mut tool_outputs: Vec<ToolOutput> = Vec::new();
        let mut raw_response_text = String::new();
        let mut success = true;

        for route in routes {
            if let Some(ref skill_name) = route.skill_name {
                match self
                    .execute_skill(skill_name, message, intent, kubeconfig.clone(), usage)
                    .await
                {
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
            }
        }

        // Try to summarize the results using LLM
        let response_text = if !raw_response_text.is_empty() {
            if let Some(ref summarizer) = self.summarizer {
                match summarizer
                    .summarize(&intent.intent_type, &tool_outputs, &intent.raw_query)
                    .await
                {
                    Ok((summary, u)) => {
                        debug!("LLM summary generated successfully");
                        usage.merge_assign(&u);
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
            success = false;
            messages::EXEC_ERROR_FALLBACK.to_string()
        };

        Ok(AgentResponse {
            session_id: String::new(),
            channel_name: message.channel_name.clone(),
            source_channel_id: message.channel_id.clone(),
            message: OutgoingMessage {
                content: response_text,
                format: OutputFormat::Markdown,
                code_block: None,
                table: None,
            },
            success,
            evidence: all_evidence,
            error: if success { None } else { Some("Execution failed".to_string()) },
            turn_duration_ms: None,
            turn_total_tokens: None,
        })
    }

    /// Execute a skill
    async fn execute_skill(
        &self,
        skill_name: &str,
        message: &ChannelMessage,
        intent: &Intent,
        kubeconfig: Option<PathBuf>,
        usage: &mut Usage,
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
            parameters: params.clone(),
            session_id: Some(message.channel_id.clone()),
        };

        let kubectl_ctx = self.kubectl_context_for_intent(intent);

        let mut output = String::new();
        let mut evidence = Vec::new();
        let mut tool_results: Vec<(String, String, bool)> = Vec::new();

        if !skill.tools.is_empty() {
            let results = self
                .execute_skill_tools(
                    skill.as_ref(),
                    &context,
                    kubectl_ctx.clone(),
                    kubeconfig.as_deref(),
                )
                .await?;

            for result in results {
                let tool_name = result.tool_name.clone();
                let raw_output = if result.success {
                    result.output.clone().unwrap_or_default()
                } else {
                    result.error.clone().unwrap_or_default()
                };

                let truncated = output_budget::truncate_tool_output(&raw_output, &self.output_budget);
                let tool_output = truncated.content;

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
        } else if self.skills_exec.enabled && !skill.raw_content.is_empty() {
            match &self.llm_skill_executor {
                Some(lse) => {
                    let res = lse
                        .execute_skill(
                            &skill.raw_content,
                            &message.content.text,
                            &params,
                            kubectl_ctx.as_deref(),
                            kubeconfig.as_deref(),
                        )
                        .await?;

                    usage.merge_assign(&res.llm_usage);
                    output = res.output.clone();
                    for (i, rec) in res.execution_history.iter().enumerate() {
                        let label = format!("shell_step_{}", i + 1);
                        let body = format!("```text\n{}\n```\n\n{}", rec.command, rec.output);
                        tool_results.push((label.clone(), body, rec.success));
                        evidence.push(aiclaw_types::agent::EvidenceRecord {
                            timestamp: Utc::now(),
                            source: "skill".to_string(),
                            action: label,
                            data: serde_json::json!({
                                "success": rec.success,
                                "command": rec.command,
                                "output": rec.output,
                            }),
                        });
                    }
                }
                None => {
                    warn!("skills.exec.enabled but no LLM provider; cannot run LLM skill loop");
                    output = messages::NO_LLM_PROVIDER.to_string();
                }
            }
        } else if !skill.raw_content.is_empty() && !self.skills_exec.enabled {
            output = messages::SKILL_EXEC_DISABLED.to_string();
        } else {
            output = messages::SKILL_NO_TOOLS.to_string();
        }

        self.observer.record_event(ObserverEvent::SkillExecutionEnd {
            skill: skill_name.to_string(),
            duration: Default::default(),
            success: !output.is_empty(),
        });

        Ok((output, evidence, tool_results))
    }

    fn kubectl_context_for_intent(&self, intent: &Intent) -> Option<String> {
        if !self.skills_exec.prepend_kubectl_context {
            return None;
        }
        let cluster = intent
            .entities
            .cluster
            .as_deref()
            .or(self.default_cluster.as_deref())?;
        let cfg = self.clusters.get(cluster)?;
        if !cfg.enabled {
            return None;
        }
        Some(cfg.kubectl_context_name(cluster).to_string())
    }

    fn interpolate_skill_placeholders(template: &str, args: &HashMap<String, String>) -> String {
        let mut result = template.to_string();
        for (key, value) in args {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, value);
        }
        result
    }

    /// Execute skill tools
    async fn execute_skill_tools(
        &self,
        skill: &aiclaw_types::skill::SkillMetadata,
        context: &aiclaw_types::skill::SkillContext,
        kubectl_ctx: Option<String>,
        kubeconfig: Option<&std::path::Path>,
    ) -> anyhow::Result<Vec<aiclaw_types::skill::ToolResult>> {
        let mut results = Vec::new();

        for tool in skill.tools.iter() {
            let mut args: HashMap<String, String> = tool
                .args
                .iter()
                .map(|(k, v)| {
                    let mut interpolated = v.clone();
                    for (pk, pv) in &context.parameters {
                        interpolated = interpolated.replace(&format!("{{{{{}}}}}", pk), pv);
                    }
                    (k.clone(), interpolated)
                })
                .collect();

            for (pk, pv) in &context.parameters {
                args.entry(pk.clone()).or_insert_with(|| pv.clone());
            }

            let mut tool_run = tool.clone();
            if tool_run.kind == aiclaw_types::skill::ToolKind::Shell {
                let expanded =
                    Self::interpolate_skill_placeholders(&tool_run.command, &args);
                tool_run.command =
                    apply_kubectl_context(&expanded, kubectl_ctx.as_deref());
            }

            match self
                .skill_executor
                .execute_tool(&tool_run, &args, kubeconfig)
                .await
            {
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

    /// Execute using planner-based approach (for Debug intents)
    async fn execute_with_planner(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
        kubeconfig: Option<PathBuf>,
        usage: &mut Usage,
    ) -> anyhow::Result<AgentResponse> {
        let Some(ref planner) = self.planner else {
            anyhow::bail!("Planner not available");
        };

        info!("Using planner for Debug intent: {}", intent.raw_query);

        // Gather matched skills for dynamic prompt injection
        let routes = self.router.route(intent);
        let matched_skills: Vec<_> = routes
            .iter()
            .filter_map(|r| r.skill_name.as_ref())
            .filter_map(|name| self.skill_registry.get(name))
            .collect();
        let skill_refs: Vec<&aiclaw_types::skill::SkillMetadata> =
            matched_skills.iter().map(|s| s.as_ref()).collect();

        // Create execution plan with dynamic skill context
        let plan = match planner
            .plan(&intent.raw_query, &format!("{:?}", intent.intent_type), &skill_refs)
            .await
        {
            Ok((p, u)) => {
                usage.merge_assign(&u);
                p
            }
            Err(e) => {
                warn!("Planner failed, falling back to route-based execution: {}", e);
                let routes = self.router.route(intent);
                return self
                    .execute_routes(message, intent, &routes, kubeconfig.clone(), usage)
                    .await;
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
            let kubectl_ctx = self.kubectl_context_for_intent(intent);
            let result = self.execute_planned_step(
                step,
                &params,
                kubeconfig.as_deref(),
                kubectl_ctx.as_deref(),
            ).await;
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

        let tool_summary = all_tool_outputs
            .iter()
            .map(|o| format!("- {}: {}", o.tool_name, o.content))
            .collect::<Vec<_>>()
            .join("\n");

        let tool_detail = all_tool_outputs
            .iter()
            .map(|o| format!("### {}\n{}\n", o.tool_name, o.content))
            .collect::<Vec<_>>()
            .join("\n");

        let response_text = if let Some(ref summarizer) = self.summarizer {
            let reasoning_prompt = format!(
                "User question: {}\n\nLLM planner analysis: {}\n\nQuery results:\n{}\n\nSynthesize the information above and provide a final diagnosis and solution.",
                intent.raw_query,
                plan.reasoning,
                tool_summary
            );

            match summarizer.summarize_text(&reasoning_prompt, "diagnosis and solution").await {
                Ok((summary, u)) => {
                    usage.merge_assign(&u);
                    summary
                }
                Err(e) => {
                    warn!("LLM reasoning failed: {}", e);
                    format!(
                        "{}{}\n\n{}{}\n\n{}",
                        messages::DIAGNOSIS_HEADER,
                        plan.reasoning,
                        messages::RESULTS_LABEL,
                        tool_detail,
                        messages::MANUAL_REVIEW,
                    )
                }
            }
        } else {
            format!(
                "{}{}\n\n{}{}\n\n**Status**: {}",
                messages::DIAGNOSIS_HEADER,
                plan.reasoning,
                messages::RESULTS_LABEL,
                tool_detail,
                if success { messages::STATUS_PARTIAL } else { messages::STATUS_ISSUES }
            )
        };

        Ok(AgentResponse {
            session_id: String::new(),
            channel_name: message.channel_name.clone(),
            source_channel_id: message.channel_id.clone(),
            message: OutgoingMessage {
                content: response_text,
                format: OutputFormat::Markdown,
                code_block: None,
                table: None,
            },
            success,
            evidence: all_evidence,
            error: if success { None } else { Some("Some steps failed".to_string()) },
            turn_duration_ms: None,
            turn_total_tokens: None,
        })
    }

    /// Execute a single planned step by running the LLM-decided command directly.
    async fn execute_planned_step(
        &self,
        step: &PlanStep,
        params: &HashMap<String, String>,
        kubeconfig: Option<&std::path::Path>,
        kubectl_ctx: Option<&str>,
    ) -> anyhow::Result<(ToolOutput, Vec<aiclaw_types::agent::EvidenceRecord>)> {
        let mut command = step.command.clone();
        for (key, value) in params {
            let placeholder = format!("{{{{{}}}}}", key);
            command = command.replace(&placeholder, value);
        }

        let tool_name = format!("plan_step_{}/{}", step.step_id, step.description);
        let cmd_with_ctx = crate::skills::apply_kubectl_context(&command, kubectl_ctx);

        let tool = aiclaw_types::skill::SkillTool {
            name: tool_name.clone(),
            description: step.description.clone(),
            kind: aiclaw_types::skill::ToolKind::Shell,
            command: cmd_with_ctx,
            args: HashMap::new(),
            env: HashMap::new(),
            timeout_secs: Some(30),
        };

        let result = self
            .skill_executor
            .execute_tool(&tool, &HashMap::new(), kubeconfig)
            .await;

        let (output_text, success) = match result {
            Ok(r) => {
                let text = if r.success {
                    r.output.unwrap_or_default()
                } else {
                    r.error.unwrap_or_default()
                };
                (text, r.success)
            }
            Err(e) => (format!("Error: {}", e), false),
        };

        let truncated = output_budget::truncate_tool_output(&output_text, &self.output_budget);

        let evidence = vec![aiclaw_types::agent::EvidenceRecord {
            timestamp: Utc::now(),
            source: "planner".to_string(),
            action: tool_name.clone(),
            data: serde_json::json!({
                "command": command,
                "success": success,
                "step_id": step.step_id,
            }),
        }];

        Ok((
            ToolOutput::new(&tool_name, &truncated.content, success),
            evidence,
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
            if message.channel_id == "stdio" {
                eprintln!("(aiclaw) state=queued · processing your line…");
            }
            match self.handle_message(message).await {
                Ok(response) => {
                    let reply_to = if response.source_channel_id.is_empty() {
                        None
                    } else {
                        Some(response.source_channel_id.clone())
                    };
                    let body = Self::stdio_reply_markdown(&response);
                    let send_msg = SendMessage::markdown_to_channel(
                        &response.channel_name,
                        reply_to,
                        body,
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

    /// Check if this is a multi-cluster query.
    ///
    /// True when no specific cluster is targeted AND the query explicitly mentions
    /// "all clusters" (or Chinese equivalent). We keep this minimal -- LLM entity
    /// extraction sets `intent.entities.cluster` when a single cluster is meant.
    fn is_multi_cluster_query(&self, intent: &Intent) -> bool {
        if intent.entities.cluster.is_some() {
            return false;
        }
        let query = intent.raw_query.to_lowercase();
        query.contains("all cluster") || query.contains("所有集群") || query.contains("全部集群")
    }

    /// Get list of known clusters
    fn get_known_clusters(&self) -> Vec<String> {
        self.clusters
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Execute query across multiple clusters
    async fn execute_multi_cluster(
        &self,
        message: &ChannelMessage,
        intent: &Intent,
        routes: &[RouteResult],
        kubeconfig: Option<PathBuf>,
        usage: &mut Usage,
    ) -> anyhow::Result<AgentResponse> {
        let clusters = self.get_known_clusters();

        if clusters.is_empty() {
            return self
                .execute_routes(message, intent, routes, kubeconfig.clone(), usage)
                .await;
        }

        info!("Executing multi-cluster query across {} clusters: {:?}", clusters.len(), clusters);

        let mut all_tool_outputs: Vec<ToolOutput> = Vec::new();
        let mut all_evidence = Vec::new();
        let mut success = true;

        // One query per cluster (sequential). True parallelism would need scoped tasks
        // and careful ordering of aggregated outputs vs cluster labels.
        for cluster in &clusters {
            let intent_for_cluster = Intent {
                intent_type: intent.intent_type.clone(),
                confidence: intent.confidence,
                entities: {
                    let mut entities = intent.entities.clone();
                    entities.cluster = Some(cluster.clone());
                    entities
                },
                raw_query: intent.raw_query.clone(),
            };

            match self
                .execute_routes(message, &intent_for_cluster, routes, kubeconfig.clone(), usage)
                .await
            {
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

        // Generate aggregated response
        let response_text = if !all_tool_outputs.is_empty() {
            if let Some(ref summarizer) = self.summarizer {
                match summarizer.summarize(&intent.intent_type, &all_tool_outputs, &intent.raw_query).await {
                    Ok((summary, u)) => {
                        usage.merge_assign(&u);
                        summary
                    }
                    Err(e) => {
                        warn!("Multi-cluster summarization failed: {}", e);
                        self.format_multi_cluster_output(&all_tool_outputs, &clusters)
                    }
                }
            } else {
                self.format_multi_cluster_output(&all_tool_outputs, &clusters)
            }
        } else {
            let response_text = messages::ALL_CLUSTERS_FAILED.to_string();
            success = false;
            response_text
        };

        Ok(AgentResponse {
            session_id: String::new(),
            channel_name: message.channel_name.clone(),
            source_channel_id: message.channel_id.clone(),
            message: OutgoingMessage {
                content: response_text,
                format: OutputFormat::Markdown,
                code_block: None,
                table: None,
            },
            success,
            evidence: all_evidence,
            error: if success { None } else { Some("Multi-cluster query partially failed".to_string()) },
            turn_duration_ms: None,
            turn_total_tokens: None,
        })
    }

    /// Format multi-cluster outputs into a readable response
    fn format_multi_cluster_output(&self, outputs: &[ToolOutput], clusters: &[String]) -> String {
        let mut result = String::from(messages::MULTI_CLUSTER_TITLE);
        result += messages::MULTI_CLUSTER_HEADER;

        for (i, output) in outputs.iter().enumerate() {
            let cluster = if i < clusters.len() {
                clusters[i].clone()
            } else {
                format!("cluster-{}", i)
            };

            let status = if output.success { "✅" } else { "❌" };
            let summary = {
                let s = crate::utils::string::utf8_prefix_chars(&output.content, 100);
                if output.content.chars().count() > 100 {
                    format!("{}...", s)
                } else {
                    s.to_string()
                }
            }.replace('\n', " ");

            result += &format!("| {} | {} | {} |\n", cluster, status, summary);
        }

        result
    }
}

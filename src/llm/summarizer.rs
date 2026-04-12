//! LLM-based result summarizer

use std::sync::Arc;

use aiclaw_types::agent::IntentType;
use crate::llm::traits::LLMProvider;
use crate::llm::types::{ChatDelta, ChatMessage, ChatOptions, Usage};

/// Result summarizer - transforms raw tool output into structured, understandable responses
pub struct Summarizer {
    provider: Arc<dyn LLMProvider>,
}

impl Summarizer {
    pub fn new(provider: Arc<dyn LLMProvider>) -> Self {
        Self { provider }
    }

    /// Summarize tool execution results based on intent type
    pub async fn summarize(
        &self,
        intent_type: &IntentType,
        tool_outputs: &[ToolOutput],
        context: &str,
    ) -> anyhow::Result<(String, Usage)> {
        let prompt = self.build_prompt(intent_type, tool_outputs, context);

        let messages = vec![
            ChatMessage::system(SYSTEM_PROMPT),
            ChatMessage::user(&prompt),
        ];

        let options = ChatOptions::new()
            .with_temperature(0.3) // Lower temperature for more consistent output
            .with_max_tokens(2048);

        let response = self.provider.chat(messages, Some(options)).await?;
        let usage = response.usage.clone();
        Ok((response.content, usage))
    }

    /// Summarize a single piece of text
    pub async fn summarize_text(&self, text: &str, format_hint: &str) -> anyhow::Result<(String, Usage)> {
        let messages = vec![
            ChatMessage::system(SYSTEM_PROMPT),
            ChatMessage::user(format!(
                "请总结以下内容，用简洁的 Markdown 格式输出（{}）：\n\n{}",
                format_hint, text
            )),
        ];

        let options = ChatOptions::new()
            .with_temperature(0.3)
            .with_max_tokens(1024);

        let response = self.provider.chat(messages, Some(options)).await?;
        let usage = response.usage.clone();
        Ok((response.content, usage))
    }

    /// Stream-summarize tool outputs. Tokens are sent via `tx`.
    pub async fn summarize_stream(
        &self,
        intent_type: &IntentType,
        tool_outputs: &[ToolOutput],
        context: &str,
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> anyhow::Result<()> {
        let prompt = self.build_prompt(intent_type, tool_outputs, context);
        let messages = vec![
            ChatMessage::system(SYSTEM_PROMPT),
            ChatMessage::user(&prompt),
        ];
        let options = ChatOptions::new()
            .with_temperature(0.3)
            .with_max_tokens(2048);

        let (inner_tx, mut inner_rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn the streaming call
        let handle = {
            let tx = inner_tx;
            self.provider.stream_chat(messages, Some(options), tx)
        };

        // Forward ChatDelta::TextDelta as string to outer tx
        while let Some(delta) = inner_rx.recv().await {
            match delta {
                ChatDelta::TextDelta(text) => {
                    let _ = tx.send(text);
                }
                ChatDelta::Done { .. } => {
                    break;
                }
                _ => {}
            }
        }

        handle.await?;
        Ok(())
    }

    fn build_prompt(&self, intent_type: &IntentType, tool_outputs: &[ToolOutput], context: &str) -> String {
        let intent_name = format!("{:?}", intent_type);

        let tool_outputs_text = if tool_outputs.is_empty() {
            "（无工具执行结果）".to_string()
        } else {
            tool_outputs
                .iter()
                .map(|o| format!("### {} 输出\n{}\n", o.tool_name, o.content))
                .collect::<Vec<_>>()
                .join("\n")
        };

        match intent_type {
            IntentType::Logs => format!(
                r#"你是运维日志分析专家。用户正在排查问题，请分析以下日志输出。

意图类型: {}
上下文: {}

{}

请用 Markdown 格式总结：
1. **日志概览**：总行数、时间范围（如可判断）
2. **关键发现**：ERROR/WARN/异常模式
3. **可能原因**：基于日志内容的推断
4. **建议操作**：下一步排查建议（如有）

请用中文回答。"#,
                intent_name, context, tool_outputs_text
            ),

            IntentType::Metrics => format!(
                r#"你是运维指标分析专家。用户正在查看监控指标，请分析以下指标数据。

意图类型: {}
上下文: {}

{}

请用 Markdown 格式总结：
1. **指标概览**：主要指标名称和当前值
2. **趋势分析**：上升/下降/稳定
3. **异常检测**：是否超过阈值或存在异常模式
4. **根因分析**：可能的性能问题原因
5. **优化建议**：如有问题，提出解决建议

请用中文回答，尽量用表格展示数据。"#,
                intent_name, context, tool_outputs_text
            ),

            IntentType::Health => format!(
                r#"你是运维健康检查专家。请分析以下集群/服务健康状态。

意图类型: {}
上下文: {}

{}

请用 Markdown 格式总结：
1. **健康状态**：✅ 正常 / ⚠️ 警告 / ❌ 异常
2. **组件状态**：各组件的运行状态
3. **问题清单**：发现的问题（如有）
4. **处理建议**：如有问题，建议的处理步骤

请用中文回答。"#,
                intent_name, context, tool_outputs_text
            ),

            IntentType::Debug => format!(
                r#"你是运维故障排查专家。用户正在排查问题，请综合分析以下数据找出根因。

意图类型: {}
上下文: {}

{}

请用 Markdown 格式总结：
1. **问题确认**：确认的问题现象
2. **根因分析**：最可能的根本原因（给出分析过程）
3. **证据支持**：支持根因判断的具体证据
4. **修复建议**：具体的修复步骤（优先级排序）
5. **预防措施**：如何避免类似问题

请用中文回答，分析要有逻辑性。"#,
                intent_name, context, tool_outputs_text
            ),

            IntentType::Query => format!(
                r#"你是数据查询专家。请总结以下查询结果。

意图类型: {}
上下文: {}

{}

请用 Markdown 格式总结：
1. **查询结果**：关键数据
2. **结果分析**：数据含义
3. **补充说明**：如有

请用中文回答，尽量用表格展示结构化数据。"#,
                intent_name, context, tool_outputs_text
            ),

            IntentType::Scale => format!(
                r#"你是运维扩缩容专家。请分析以下扩缩容相关状态。

意图类型: {}
上下文: {}

{}

请用 Markdown 格式总结：
1. **当前状态**：当前副本数、资源使用
2. **扩缩建议**：建议的扩缩方案
3. **影响评估**：扩缩容对服务的影响

请用中文回答。"#,
                intent_name, context, tool_outputs_text
            ),

            IntentType::Deploy => format!(
                r#"你是运维部署专家。请分析以下部署状态。

意图类型: {}
上下文: {}

{}

请用 Markdown 格式总结：
1. **部署状态**：成功/失败/进行中
2. **版本信息**：当前版本（如有）
3. **问题处理**：部署中的问题及建议

请用中文回答。"#,
                intent_name, context, tool_outputs_text
            ),

            _ => format!(
                r#"请总结以下运维工具的输出结果。

意图类型: {}
上下文: {}

{}

请用简洁的 Markdown 格式总结关键信息和发现。

请用中文回答。"#,
                intent_name, context, tool_outputs_text
            ),
        }
    }
}

/// Tool output data
#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub tool_name: String,
    pub content: String,
    pub success: bool,
}

impl ToolOutput {
    pub fn new(tool_name: impl Into<String>, content: impl Into<String>, success: bool) -> Self {
        Self {
            tool_name: tool_name.into(),
            content: content.into(),
            success,
        }
    }
}

const SYSTEM_PROMPT: &str = r#"你是一个运维 AI 助手，专门帮助用户理解和分析运维数据。

你的职责：
1. 将原始、复杂的运维输出转化为易懂的总结
2. 识别问题模式和异常
3. 提供清晰的下一步行动建议
4. 使用 Markdown 格式输出，便于阅读

输出要求：
- 使用中文回答
- 结构清晰，分点阐述
- 如有数据，尽量用表格展示
- 突出关键发现和警告信息
- 建议要具体可操作"#;

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

    /// Summarize tool execution results based on intent type.
    ///
    /// Uses a unified prompt that provides the intent as context rather than
    /// prescribing fixed analysis steps per intent type. The LLM determines
    /// the appropriate analysis structure based on the actual data.
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
            .with_temperature(0.3)
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

        let intent_hint = match intent_type {
            IntentType::Logs => "用户正在分析日志数据",
            IntentType::Metrics => "用户正在分析监控指标",
            IntentType::Health => "用户正在检查系统健康状态",
            IntentType::Debug => "用户正在排查故障根因",
            IntentType::Query => "用户正在查询数据",
            IntentType::Scale => "用户正在评估扩缩容",
            IntentType::Deploy => "用户正在检查部署状态",
            IntentType::Unknown => "用户发起了一个运维请求",
        };

        format!(
            r#"场景: {intent_hint}
意图类型: {intent_name}
用户上下文: {context}

{tool_outputs_text}

请基于以上数据进行分析。根据数据内容自行决定最合适的分析结构和重点，
用 Markdown 格式输出。突出关键发现、异常和可操作的建议。请用中文回答。"#
        )
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

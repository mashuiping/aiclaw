//! Conversation runtime: manages the LLM <-> tool loop for the REPL.
//!
//! The LLM receives tool definitions, autonomously decides which tools to call,
//! and iterates until the task is complete (no more tool calls).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::agent::context::ContextManager;
use crate::agent::output_budget::{self, OutputBudget};
use crate::llm::traits::LLMProvider;
use crate::llm::types::{ChatDelta, ChatMessage, ChatOptions, ToolCall};

use super::render::TerminalRenderer;
use super::spinner::Spinner;
use super::tools::{self, ToolResult};

/// Manages a multi-turn conversation with tool use.
pub struct ConversationRuntime {
    provider: Arc<dyn LLMProvider>,
    messages: Vec<ChatMessage>,
    renderer: TerminalRenderer,
    kubeconfig: Option<PathBuf>,
    max_tool_iterations: usize,
    /// Last thinking content for /thinkback.
    last_thinking: String,
    output_budget: OutputBudget,
    context_manager: ContextManager,
    /// Track recent tool calls for loop detection.
    recent_tool_calls: Vec<(String, String)>,
}

impl ConversationRuntime {
    pub fn new(
        provider: Arc<dyn LLMProvider>,
        kubeconfig: Option<PathBuf>,
    ) -> Self {
        let messages = vec![ChatMessage::system(super::SYSTEM_PROMPT)];
        let context_manager = ContextManager::new(provider.clone());
        Self {
            provider,
            messages,
            renderer: TerminalRenderer::new(),
            kubeconfig,
            max_tool_iterations: 25,
            last_thinking: String::new(),
            output_budget: OutputBudget::default_budget(),
            context_manager,
            recent_tool_calls: Vec::new(),
        }
    }

    /// Get a reference to the conversation messages (for session persistence).
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Replace messages (for session resume).
    pub fn set_messages(&mut self, messages: Vec<ChatMessage>) {
        self.messages = messages;
    }

    /// Get the last thinking content for /thinkback.
    pub fn last_thinking(&self) -> &str {
        &self.last_thinking
    }

    /// Run a single user turn: stream LLM, execute tools, loop until done.
    pub async fn run_turn(&mut self, user_input: &str) {
        self.messages.push(ChatMessage::user(user_input));
        self.recent_tool_calls.clear();

        // Compact context if approaching limit
        if let Err(e) = self.context_manager.compact_if_needed(&mut self.messages).await {
            self.renderer.render_error(&format!("Context compaction failed: {e:#}"));
        }

        let tool_specs = tools::tool_specs();
        let options = ChatOptions {
            tools: Some(tool_specs),
            ..Default::default()
        };

        for iteration in 0..self.max_tool_iterations {
            let spinner = Spinner::new("Thinking...");
            spinner.start();

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ChatDelta>();
            let messages = self.messages.clone();
            let opts = options.clone();
            let provider = self.provider.clone();

            let stream_task = tokio::spawn(async move {
                provider.stream_chat(messages, Some(opts), tx).await
            });

            let mut full_text = String::new();
            let mut thinking_text = String::new();
            let mut spinner_stopped = false;

            // Accumulate tool calls from the stream
            let mut pending_tool_calls: HashMap<usize, (String, String, String)> = HashMap::new(); // index -> (id, name, args_json)

            while let Some(delta) = rx.recv().await {
                match delta {
                    ChatDelta::TextDelta(text) => {
                        if !spinner_stopped {
                            spinner.stop_success();
                            spinner_stopped = true;
                        }
                        self.renderer.write_stream_chunk(&text);
                        full_text.push_str(&text);
                    }
                    ChatDelta::ThinkingDelta(text) => {
                        thinking_text.push_str(&text);
                    }
                    ChatDelta::ToolCallStart { index, id, name } => {
                        if !spinner_stopped {
                            spinner.stop_success();
                            spinner_stopped = true;
                        }
                        pending_tool_calls.insert(index, (id, name, String::new()));
                    }
                    ChatDelta::ToolCallDelta { index, json_chunk } => {
                        if let Some(entry) = pending_tool_calls.get_mut(&index) {
                            entry.2.push_str(&json_chunk);
                        }
                    }
                    ChatDelta::Done { usage } => {
                        if !spinner_stopped {
                            spinner.stop_success();
                            spinner_stopped = true;
                        }
                        if !thinking_text.is_empty() {
                            self.renderer.render_thinking_summary(thinking_text.len());
                            self.last_thinking = thinking_text.clone();
                        }
                        if !full_text.is_empty() {
                            self.renderer.finish_stream();
                        }
                        self.renderer.render_usage(&usage);
                    }
                    ChatDelta::Error(msg) => {
                        if !spinner_stopped {
                            spinner.stop_error();
                        }
                        self.renderer.render_error(&msg);
                        stream_task.abort();
                        return;
                    }
                }
            }

            // Wait for the stream task
            match stream_task.await {
                Ok(Err(e)) => {
                    if !spinner_stopped { spinner.stop_error(); }
                    self.renderer.render_error(&format!("Stream error: {e:#}"));
                    return;
                }
                Err(e) if e.is_cancelled() => {
                    // Task was aborted after ChatDelta::Error, already handled
                }
                Err(e) => {
                    if !spinner_stopped { spinner.stop_error(); }
                    self.renderer.render_error(&format!("Task error: {e:#}"));
                    return;
                }
                Ok(Ok(())) => {
                    if !spinner_stopped { spinner.stop_success(); }
                }
            }

            // Build sorted tool calls
            let mut sorted_indices: Vec<usize> = pending_tool_calls.keys().copied().collect();
            sorted_indices.sort();

            let tool_calls: Vec<ToolCall> = sorted_indices
                .iter()
                .filter_map(|idx| {
                    pending_tool_calls.remove(idx).map(|(id, name, args)| {
                        let sanitized_args = if args.is_empty()
                            || serde_json::from_str::<serde_json::Value>(&args).is_err()
                        {
                            "{}".to_string()
                        } else {
                            args
                        };
                        ToolCall {
                            id,
                            name,
                            arguments: sanitized_args,
                        }
                    })
                })
                .collect();

            // Add assistant message to conversation history
            self.messages.push(ChatMessage::assistant_with_tool_calls(
                full_text.clone(),
                tool_calls.clone(),
            ));

            if tool_calls.is_empty() {
                // No tool calls — turn is complete
                self.renderer.render_done();
                return;
            }

            // Loop detection: check for repeated identical tool calls
            let mut loop_detected = false;
            for tc in &tool_calls {
                let call_key = (tc.name.clone(), tc.arguments.clone());
                let repeat_count = self.recent_tool_calls.iter()
                    .filter(|prev| prev.0 == call_key.0 && prev.1 == call_key.1)
                    .count();
                if repeat_count >= 2 {
                    self.renderer.render_error(&format!(
                        "Loop detected: tool '{}' called 3+ times with same arguments. Stopping.",
                        tc.name
                    ));
                    loop_detected = true;
                    break;
                }
                self.recent_tool_calls.push(call_key);
            }
            if loop_detected {
                return;
            }

            // Execute tool calls
            for tc in &tool_calls {
                let summary = build_tool_summary(&tc.name, &tc.arguments);
                self.renderer.render_tool_call_start(&tc.name, &summary);

                let result = if tc.arguments == "{}" {
                    ToolResult {
                        output: format!(
                            "Tool '{}' received empty arguments (LLM streaming may have been interrupted). \
                             Please retry the tool call with valid arguments.",
                            tc.name
                        ),
                        is_error: true,
                    }
                } else {
                    tools::execute_tool(
                        &tc.name,
                        &tc.arguments,
                        self.kubeconfig.as_ref(),
                    )
                    .await
                };

                // Apply output budget truncation before adding to context
                let truncated = output_budget::truncate_tool_output(&result.output, &self.output_budget);

                self.renderer
                    .render_tool_result(&tc.name, &result.output, result.is_error);

                self.messages
                    .push(ChatMessage::tool_result(&tc.id, &truncated.content));
            }

            if iteration + 1 >= self.max_tool_iterations {
                self.renderer.render_error(&format!(
                    "Reached maximum tool iterations ({}).",
                    self.max_tool_iterations
                ));
                return;
            }

            // Loop: the LLM will see tool results and decide next action
        }
    }
}

/// Build a human-readable summary for the tool call card.
fn build_tool_summary(tool_name: &str, args_json: &str) -> String {
    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
    match tool_name {
        "bash" => {
            let cmd = args["command"].as_str().unwrap_or("(no command)");
            format!("$ {cmd}")
        }
        "read_file" => {
            let path = args["path"].as_str().unwrap_or("(no path)");
            format!("📄 Reading {path}")
        }
        "list_files" => {
            let path = args["path"].as_str().unwrap_or(".");
            format!("📁 Listing {path}")
        }
        _ => args_json.to_string(),
    }
}

//! SSE (Server-Sent Events) stream parser for OpenAI-compatible and Anthropic APIs.

use crate::llm::types::{ChatDelta, Usage};

/// Parse a single SSE `data:` payload from an **OpenAI-compatible** streaming endpoint.
///
/// Returns `None` for `[DONE]` or unparseable lines; returns `Some(vec![...])` with zero or
/// more `ChatDelta` items for each recognized chunk.
pub fn parse_openai_sse_data(data: &str) -> Option<Vec<ChatDelta>> {
    let trimmed = data.trim();
    if trimmed == "[DONE]" {
        return None;
    }

    let json: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "Failed to parse OpenAI SSE data as JSON: {e} (input: {})",
                &trimmed[..trimmed.len().min(200)]
            );
            return Some(vec![]);
        }
    };

    let mut deltas = Vec::new();

    // Usage block (present in the final chunk when `stream_options.include_usage` is set,
    // or always present in some providers).
    if let Some(usage) = json.get("usage") {
        if usage.is_object() {
            let u = Usage {
                prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0) as u32,
                completion_tokens: usage["completion_tokens"].as_u64().unwrap_or(0) as u32,
                total_tokens: usage["total_tokens"].as_u64().unwrap_or(0) as u32,
            };
            deltas.push(ChatDelta::Done { usage: u });
        }
    }

    if let Some(choices) = json["choices"].as_array() {
        for choice in choices {
            let delta = &choice["delta"];

            // Text content
            if let Some(content) = delta["content"].as_str() {
                if !content.is_empty() {
                    deltas.push(ChatDelta::TextDelta(content.to_string()));
                }
            }

            // Reasoning / thinking content (DeepSeek / OpenAI o-series)
            if let Some(reasoning) = delta["reasoning_content"].as_str() {
                if !reasoning.is_empty() {
                    deltas.push(ChatDelta::ThinkingDelta(reasoning.to_string()));
                }
            }

            // Tool calls
            if let Some(tool_calls) = delta["tool_calls"].as_array() {
                for tc in tool_calls {
                    let index = tc["index"].as_u64().unwrap_or(0) as usize;

                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func["name"].as_str() {
                            let id = tc["id"].as_str().unwrap_or("").to_string();
                            deltas.push(ChatDelta::ToolCallStart {
                                index,
                                id,
                                name: name.to_string(),
                            });
                        }
                        if let Some(args) = func["arguments"].as_str() {
                            if !args.is_empty() {
                                deltas.push(ChatDelta::ToolCallDelta {
                                    index,
                                    json_chunk: args.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    Some(deltas)
}

/// Parse a single SSE `data:` payload from the **Anthropic Messages** streaming endpoint.
pub fn parse_anthropic_sse_event(event_type: &str, data: &str) -> Option<Vec<ChatDelta>> {
    let trimmed = data.trim();
    let json: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "Failed to parse Anthropic SSE event ({event_type}) as JSON: {e} (input: {})",
                &trimmed[..trimmed.len().min(200)]
            );
            return None;
        }
    };

    let mut deltas = Vec::new();

    match event_type {
        "content_block_delta" => {
            let delta = &json["delta"];
            let delta_type = delta["type"].as_str().unwrap_or("");
            match delta_type {
                "text_delta" => {
                    if let Some(text) = delta["text"].as_str() {
                        deltas.push(ChatDelta::TextDelta(text.to_string()));
                    }
                }
                "thinking_delta" => {
                    if let Some(thinking) = delta["thinking"].as_str() {
                        deltas.push(ChatDelta::ThinkingDelta(thinking.to_string()));
                    }
                }
                "input_json_delta" => {
                    if let Some(partial) = delta["partial_json"].as_str() {
                        let index = json["index"].as_u64().unwrap_or(0) as usize;
                        deltas.push(ChatDelta::ToolCallDelta {
                            index,
                            json_chunk: partial.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        "content_block_start" => {
            let block = &json["content_block"];
            let block_type = block["type"].as_str().unwrap_or("");
            if block_type == "tool_use" {
                let index = json["index"].as_u64().unwrap_or(0) as usize;
                deltas.push(ChatDelta::ToolCallStart {
                    index,
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                });
            }
        }
        "message_delta" => {
            if let Some(usage) = json.get("usage") {
                let u = Usage {
                    prompt_tokens: 0,
                    completion_tokens: usage["output_tokens"].as_u64().unwrap_or(0) as u32,
                    total_tokens: 0,
                };
                deltas.push(ChatDelta::Done { usage: u });
            }
        }
        "message_start" => {
            if let Some(usage) = json["message"].get("usage") {
                let u = Usage {
                    prompt_tokens: usage["input_tokens"].as_u64().unwrap_or(0) as u32,
                    completion_tokens: 0,
                    total_tokens: 0,
                };
                // We don't emit Done here, just record usage later
                deltas.push(ChatDelta::Done { usage: u });
            }
        }
        _ => {}
    }

    if deltas.is_empty() {
        None
    } else {
        Some(deltas)
    }
}

/// Iterate over raw SSE bytes and yield (`event_type`, `data`) pairs.
/// Standard SSE: lines starting with `event:` set the type, `data:` accumulates the payload,
/// and a blank line dispatches.
pub fn iter_sse_lines(text: &str) -> Vec<(String, String)> {
    let mut events = Vec::new();
    let mut current_event = String::new();
    let mut current_data = String::new();

    for line in text.lines() {
        if line.is_empty() {
            if !current_data.is_empty() {
                events.push((current_event.clone(), current_data.clone()));
                current_event.clear();
                current_data.clear();
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("event: ").or_else(|| line.strip_prefix("event:")) {
            current_event = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
            if !current_data.is_empty() {
                current_data.push('\n');
            }
            current_data.push_str(rest);
        }
    }
    // Flush trailing data without a blank line
    if !current_data.is_empty() {
        events.push((current_event, current_data));
    }

    events
}

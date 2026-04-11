//! Interactive REPL mode for AIClaw.
//!
//! Provides a rich terminal experience with rustyline editing,
//! slash commands, streaming LLM output, and tool-call rendering.

pub mod commands;
pub mod conversation;
pub mod input;
pub mod render;
pub mod session;
pub mod spinner;
pub mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use tracing::info;

use crate::config::Config;
use crate::llm::traits::LLMProvider;
use crate::skills::SkillRegistry;

use self::commands::SlashCommand;
use self::conversation::ConversationRuntime;
use self::input::LineEditor;
use self::render::TerminalRenderer;

/// Run the interactive REPL loop.
pub async fn run_repl(
    config: &Config,
    llm_provider: Option<Arc<dyn LLMProvider>>,
    skill_registry: Arc<SkillRegistry>,
    kubeconfig: Option<PathBuf>,
) -> anyhow::Result<()> {
    let renderer = TerminalRenderer::new();

    let model_name = config
        .llm
        .providers
        .values()
        .next()
        .map(|p| p.model.as_str())
        .unwrap_or("none");
    renderer.print_banner(model_name, &skill_registry, kubeconfig.as_deref());

    if llm_provider.is_none() {
        eprintln!("(aiclaw) Warning: no LLM provider configured. Chat will not work.");
    }

    let completions = commands::completion_candidates(&skill_registry);
    let mut editor = LineEditor::new("> ", completions);

    let session_id = session::new_session_id();
    info!("Session: {}", session_id);

    let mut conv_runtime = llm_provider
        .as_ref()
        .map(|p| ConversationRuntime::new(p.clone(), kubeconfig.clone()));

    loop {
        match editor.read_line() {
            Ok(input::ReadOutcome::Submit(input)) => {
                let trimmed = input.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                if matches!(trimmed.as_str(), "/exit" | "/quit") {
                    // Auto-save on exit
                    if let Some(ref conv) = conv_runtime {
                        if conv.messages().len() > 1 {
                            match session::save_session(&session_id, conv.messages()) {
                                Ok(path) => eprintln!("(aiclaw) Session saved to {}", path.display()),
                                Err(e) => eprintln!("(aiclaw) Failed to save session: {e}"),
                            }
                        }
                    }
                    info!("User requested exit");
                    break;
                }

                match SlashCommand::parse(&trimmed) {
                    Ok(Some(cmd)) => {
                        handle_slash_command(
                            cmd,
                            &skill_registry,
                            &renderer,
                            &mut conv_runtime,
                            &session_id,
                        );
                        continue;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("Error: {e}");
                        continue;
                    }
                }

                if let Some(ref mut conv) = conv_runtime {
                    conv.run_turn(&trimmed).await;
                } else {
                    eprintln!("(aiclaw) No LLM provider configured. Use /help for commands.");
                }
            }
            Ok(input::ReadOutcome::Cancel) => continue,
            Ok(input::ReadOutcome::Exit) => {
                // Auto-save on Ctrl-D
                if let Some(ref conv) = conv_runtime {
                    if conv.messages().len() > 1 {
                        match session::save_session(&session_id, conv.messages()) {
                            Ok(path) => eprintln!("(aiclaw) Session saved to {}", path.display()),
                            Err(e) => eprintln!("(aiclaw) Failed to save session: {e}"),
                        }
                    }
                }
                break;
            }
            Err(e) => {
                eprintln!("Input error: {e}");
                break;
            }
        }
    }

    eprintln!("(aiclaw) Session ended.");
    Ok(())
}

fn handle_slash_command(
    cmd: SlashCommand,
    skill_registry: &SkillRegistry,
    renderer: &TerminalRenderer,
    conv_runtime: &mut Option<ConversationRuntime>,
    session_id: &str,
) {
    match cmd {
        SlashCommand::Help => renderer.print_help(),
        SlashCommand::Skills => renderer.print_skills(skill_registry),
        SlashCommand::Status => eprintln!("(aiclaw) Status: running"),
        SlashCommand::Model => eprintln!("(aiclaw) Model switching not yet implemented"),
        SlashCommand::Save => {
            if let Some(ref conv) = conv_runtime {
                match session::save_session(session_id, conv.messages()) {
                    Ok(path) => eprintln!("(aiclaw) Session saved to {}", path.display()),
                    Err(e) => eprintln!("(aiclaw) Failed to save session: {e}"),
                }
            } else {
                eprintln!("(aiclaw) No active conversation to save.");
            }
        }
        SlashCommand::Resume => {
            match session::load_latest_session() {
                Ok((id, messages)) => {
                    if let Some(ref mut conv) = conv_runtime {
                        let msg_count = messages.len();
                        conv.set_messages(messages);
                        eprintln!("(aiclaw) Resumed session {id} ({msg_count} messages)");
                    } else {
                        eprintln!("(aiclaw) No LLM provider; cannot resume.");
                    }
                }
                Err(e) => eprintln!("(aiclaw) Failed to resume: {e}"),
            }
        }
        SlashCommand::Thinkback => {
            if let Some(ref conv) = conv_runtime {
                let thinking = conv.last_thinking();
                if thinking.is_empty() {
                    eprintln!("(aiclaw) No thinking content from last response.");
                } else {
                    eprintln!("\n--- Thinking (last response) ---");
                    println!("{thinking}");
                    eprintln!("--- End thinking ---\n");
                }
            } else {
                eprintln!("(aiclaw) No active conversation.");
            }
        }
    }
}

pub(crate) const SYSTEM_PROMPT: &str = "\
You are AIClaw, an AI operations assistant specializing in Kubernetes diagnostics \
and infrastructure troubleshooting. You help users analyze cluster issues, debug pods, \
inspect GPU scheduling (HAMi/vGPU), check logs, and provide actionable remediation steps.\n\
\n\
You have access to tools for running shell commands and reading files. \
Use them proactively to gather diagnostic data.\n\
\n\
When diagnosing issues:\n\
- Be systematic: gather data before concluding\n\
- Show your reasoning at each step\n\
- Run multiple commands in parallel when possible\n\
- Format output clearly with Markdown tables and code blocks\n\
- Provide actionable remediation steps";

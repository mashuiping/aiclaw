//! Terminal rendering: Markdown-to-ANSI, tool call cards, banners, streaming.

use std::io::Write;
use std::path::Path;

use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};

use crate::llm::types::Usage;
use crate::skills::SkillRegistry;

/// Maximum characters of tool output to display before truncation.
const TOOL_OUTPUT_DISPLAY_MAX: usize = 3000;

/// Rich terminal renderer for REPL output.
pub struct TerminalRenderer;

impl TerminalRenderer {
    pub fn new() -> Self {
        Self
    }

    /// Print the startup banner.
    pub fn print_banner(
        &self,
        model: &str,
        skill_registry: &SkillRegistry,
        kubeconfig: Option<&Path>,
    ) {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let skill_count = skill_registry.len();
        let kube_display = kubeconfig
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not set".to_string());

        eprintln!(
            "{fg}{bold}Model{ra}            {model}\n\
             {fg}{bold}Skills{ra}           {skill_count} loaded\n\
             {fg}{bold}Workspace{ra}        {cwd}\n\
             {fg}{bold}Kubeconfig{ra}       {kube_display}{reset}\n",
            fg = SetForegroundColor(Color::Cyan),
            bold = SetAttribute(Attribute::Bold),
            ra = SetAttribute(Attribute::Reset),
            reset = ResetColor,
        );
        eprintln!("Type /help for commands · Ctrl-D to exit\n");
    }

    /// Render assistant text as Markdown to the terminal (full block).
    pub fn render_assistant_text(&self, text: &str) {
        let skin = termimad::MadSkin::default();
        skin.print_text(text);
        println!();
    }

    /// Write a streaming text chunk to stdout (raw, no Markdown parsing -- for token-by-token output).
    pub fn write_stream_chunk(&self, chunk: &str) {
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(chunk.as_bytes());
        let _ = stdout.flush();
    }

    /// Write a newline after streaming finishes.
    pub fn finish_stream(&self) {
        println!();
    }

    /// Render token usage footer.
    pub fn render_usage(&self, usage: &Usage) {
        eprintln!(
            "{dim}tokens: {prompt} in → {completion} out (total {total}){reset}",
            dim = SetAttribute(Attribute::Dim),
            prompt = usage.prompt_tokens,
            completion = usage.completion_tokens,
            total = usage.total_tokens,
            reset = SetAttribute(Attribute::Reset),
        );
    }

    /// Render a thinking block summary (collapsed).
    pub fn render_thinking_summary(&self, char_count: usize) {
        eprintln!(
            "\n{dim}▶ Thinking ({char_count} chars hidden){reset}",
            dim = SetAttribute(Attribute::Dim),
            reset = SetAttribute(Attribute::Reset),
        );
    }

    /// Render a tool call start card.
    pub fn render_tool_call_start(&self, tool_name: &str, summary: &str) {
        let header = format!("─ {tool_name} ─");
        let bottom = "─".repeat(header.len());
        eprintln!(
            "\n{fg}╭{header}╮\n│  {summary}\n╰{bottom}╯{reset}",
            fg = SetForegroundColor(Color::DarkYellow),
            reset = ResetColor,
        );
    }

    /// Render a tool call result.
    pub fn render_tool_result(&self, tool_name: &str, output: &str, is_error: bool) {
        let icon = if is_error { "✘" } else { "✓" };
        let color = if is_error { Color::Red } else { Color::Green };
        eprintln!(
            "{fg}{icon} {tool_name}{reset}",
            fg = SetForegroundColor(color),
            reset = ResetColor,
        );
        if !output.is_empty() {
            let display = truncate_output(output, TOOL_OUTPUT_DISPLAY_MAX);
            println!("{display}");
            if output.len() > TOOL_OUTPUT_DISPLAY_MAX {
                eprintln!(
                    "{dim}… output truncated ({} chars total){reset}",
                    output.len(),
                    dim = SetAttribute(Attribute::Dim),
                    reset = SetAttribute(Attribute::Reset),
                );
            }
        }
    }

    /// Render the "done" footer after a turn completes.
    pub fn render_done(&self) {
        eprintln!(
            "{fg}✨ Done{reset}",
            fg = SetForegroundColor(Color::Green),
            reset = ResetColor,
        );
    }

    /// Render an error footer.
    pub fn render_error(&self, msg: &str) {
        eprintln!(
            "{fg}✘ {msg}{reset}",
            fg = SetForegroundColor(Color::Red),
            reset = ResetColor,
        );
    }

    /// Print help text.
    pub fn print_help(&self) {
        eprintln!(
            "\n{bold}Available commands:{ra}\n\
             \n\
             {cmd}/help{rc}       Show this help\n\
             {cmd}/skills{rc}     List loaded skills\n\
             {cmd}/status{rc}     Show agent status\n\
             {cmd}/model{rc}      Show/switch model\n\
             {cmd}/save{rc}       Save current session\n\
             {cmd}/resume{rc}     Resume the latest session\n\
             {cmd}/thinkback{rc}  Show thinking from last response\n\
             {cmd}/exit{rc}       Exit the REPL\n\
             \n\
             Type any text to chat with the AI assistant.\n",
            bold = SetAttribute(Attribute::Bold),
            ra = SetAttribute(Attribute::Reset),
            cmd = SetForegroundColor(Color::Green),
            rc = ResetColor,
        );
    }

    /// Print loaded skills.
    pub fn print_skills(&self, registry: &SkillRegistry) {
        let names = registry.list_names();
        if names.is_empty() {
            eprintln!("(aiclaw) No skills loaded.");
            return;
        }
        eprintln!(
            "\n{bold}Loaded skills ({count}):{ra}",
            bold = SetAttribute(Attribute::Bold),
            count = names.len(),
            ra = SetAttribute(Attribute::Reset),
        );
        for name in &names {
            eprintln!(
                "  {fg}{name}{reset}",
                fg = SetForegroundColor(Color::Cyan),
                reset = ResetColor,
            );
        }
        eprintln!();
    }
}

fn truncate_output(s: &str, max_chars: usize) -> &str {
    if s.len() <= max_chars {
        return s;
    }
    let boundary = s
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    &s[..boundary]
}

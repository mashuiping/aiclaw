//! Slash command parsing for the REPL.

use crate::skills::SkillRegistry;

/// Available slash commands.
#[derive(Debug, Clone)]
pub enum SlashCommand {
    Help,
    Skills,
    Status,
    Model,
    Save,
    Resume,
    Thinkback,
}

impl SlashCommand {
    /// Parse a slash command from input. Returns `Ok(None)` if the input is not a command.
    pub fn parse(input: &str) -> Result<Option<Self>, String> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return Ok(None);
        }

        let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
        let cmd = parts[0];

        match cmd {
            "/help" | "/h" | "/?" => Ok(Some(Self::Help)),
            "/skills" => Ok(Some(Self::Skills)),
            "/status" => Ok(Some(Self::Status)),
            "/model" => Ok(Some(Self::Model)),
            "/save" => Ok(Some(Self::Save)),
            "/resume" => Ok(Some(Self::Resume)),
            "/thinkback" => Ok(Some(Self::Thinkback)),
            "/exit" | "/quit" => Ok(None), // handled by caller
            _ => Err(format!("Unknown command: {cmd}. Type /help for available commands.")),
        }
    }
}

/// Build tab-completion candidates from slash commands and loaded skills.
pub fn completion_candidates(skill_registry: &SkillRegistry) -> Vec<String> {
    let mut candidates = vec![
        "/help".to_string(),
        "/skills".to_string(),
        "/status".to_string(),
        "/model".to_string(),
        "/save".to_string(),
        "/resume".to_string(),
        "/thinkback".to_string(),
        "/exit".to_string(),
        "/quit".to_string(),
    ];

    for name in skill_registry.list_names() {
        candidates.push(format!("/{name}"));
    }

    candidates
}

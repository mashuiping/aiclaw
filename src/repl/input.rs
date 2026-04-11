//! Line editor wrapper around rustyline.

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper};

/// Outcome from reading a line.
pub enum ReadOutcome {
    /// User submitted text (Enter).
    Submit(String),
    /// User pressed Ctrl-C (cancel current input).
    Cancel,
    /// User pressed Ctrl-D (exit).
    Exit,
}

/// Helper for rustyline: provides tab completion for slash commands and skill names.
struct SlashCompleter {
    candidates: Vec<String>,
}

impl Completer for SlashCompleter {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let prefix = &line[..pos];
        if !prefix.starts_with('/') && !prefix.is_empty() {
            return Ok((0, vec![]));
        }
        let matches: Vec<Pair> = self
            .candidates
            .iter()
            .filter(|c| c.starts_with(prefix))
            .map(|c| Pair {
                display: c.clone(),
                replacement: c.clone(),
            })
            .collect();
        Ok((0, matches))
    }
}

impl Hinter for SlashCompleter {
    type Hint = String;
}
impl Highlighter for SlashCompleter {}
impl Validator for SlashCompleter {}
impl Helper for SlashCompleter {}

/// Readline-based line editor with tab completion and history.
pub struct LineEditor {
    editor: Editor<SlashCompleter, rustyline::history::DefaultHistory>,
    prompt: String,
}

impl LineEditor {
    pub fn new(prompt: &str, completions: Vec<String>) -> Self {
        let config = rustyline::Config::builder()
            .auto_add_history(true)
            .build();
        let helper = SlashCompleter {
            candidates: completions,
        };
        let mut editor = Editor::with_config(config).expect("failed to create line editor");
        editor.set_helper(Some(helper));

        // Load history from ~/.aiclaw/history if it exists
        let history_path = dirs::home_dir()
            .map(|h| h.join(".aiclaw").join("history"));
        if let Some(ref path) = history_path {
            let _ = editor.load_history(path);
        }

        Self {
            editor,
            prompt: prompt.to_string(),
        }
    }

    pub fn read_line(&mut self) -> Result<ReadOutcome, String> {
        match self.editor.readline(&self.prompt) {
            Ok(line) => Ok(ReadOutcome::Submit(line)),
            Err(ReadlineError::Interrupted) => Ok(ReadOutcome::Cancel),
            Err(ReadlineError::Eof) => Ok(ReadOutcome::Exit),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Save history on drop.
    pub fn save_history(&mut self) {
        if let Some(path) = dirs::home_dir().map(|h| h.join(".aiclaw").join("history")) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = self.editor.save_history(&path);
        }
    }
}

impl Drop for LineEditor {
    fn drop(&mut self) {
        self.save_history();
    }
}

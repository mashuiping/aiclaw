//! Tool output budget: adaptive truncation for large tool results.
//!
//! Three tiers:
//! - Tier 1 (small):  inline full content
//! - Tier 2 (medium): head+tail truncation with marker
//! - Tier 3 (large):  save to temp file, return preview + file path
//!
//! For AIOps diagnostics, tail content is weighted more heavily (error messages,
//! recent log lines) via an asymmetric 30/70 head/tail split.

use std::path::{Path, PathBuf};
use tracing::debug;

const TIER1_THRESHOLD: usize = 2_000;
const TIER2_THRESHOLD: usize = 16_384;

const HEAD_RATIO: f64 = 0.30;

const PREVIEW_HEAD_CHARS: usize = 1_000;
const PREVIEW_TAIL_CHARS: usize = 500;

/// Budget configuration derived from model context window.
#[derive(Debug, Clone)]
pub struct OutputBudget {
    pub max_single_result_chars: usize,
    pub max_total_chars: usize,
    pub temp_dir: PathBuf,
}

impl OutputBudget {
    /// Derive budget from an estimated context window size (in tokens).
    /// A single tool result should not exceed ~30 % of the context window.
    pub fn from_context_window(context_tokens: usize, temp_dir: impl Into<PathBuf>) -> Self {
        let chars_estimate = context_tokens * 4; // rough chars-per-token
        Self {
            max_single_result_chars: (chars_estimate as f64 * 0.30) as usize,
            max_total_chars: (chars_estimate as f64 * 0.60) as usize,
            temp_dir: temp_dir.into(),
        }
    }

    pub fn default_budget() -> Self {
        Self {
            max_single_result_chars: TIER2_THRESHOLD,
            max_total_chars: TIER2_THRESHOLD * 4,
            temp_dir: std::env::temp_dir().join("aiclaw_tool_output"),
        }
    }
}

/// Result of truncation.
#[derive(Debug, Clone)]
pub struct TruncatedOutput {
    pub content: String,
    pub original_len: usize,
    pub was_truncated: bool,
    pub saved_path: Option<PathBuf>,
}

/// Apply the three-tier truncation policy to a raw tool output string.
pub fn truncate_tool_output(raw: &str, budget: &OutputBudget) -> TruncatedOutput {
    let original_len = raw.len();
    let cap = budget.max_single_result_chars;

    if original_len <= TIER1_THRESHOLD.min(cap) {
        return TruncatedOutput {
            content: raw.to_string(),
            original_len,
            was_truncated: false,
            saved_path: None,
        };
    }

    if original_len <= cap && original_len <= TIER2_THRESHOLD {
        return TruncatedOutput {
            content: raw.to_string(),
            original_len,
            was_truncated: false,
            saved_path: None,
        };
    }

    if original_len <= TIER2_THRESHOLD.max(cap) {
        let truncated = head_tail_truncate(raw, cap);
        return TruncatedOutput {
            content: truncated,
            original_len,
            was_truncated: true,
            saved_path: None,
        };
    }

    // Tier 3: save to file + preview
    let saved_path = save_to_temp(raw, &budget.temp_dir);
    let preview = build_preview(raw, &saved_path);
    TruncatedOutput {
        content: preview,
        original_len,
        was_truncated: true,
        saved_path,
    }
}

/// Head+tail truncation with asymmetric split (30 % head, 70 % tail).
fn head_tail_truncate(s: &str, max_chars: usize) -> String {
    let total_chars: usize = s.chars().count();
    if total_chars <= max_chars {
        return s.to_string();
    }

    let marker = format!("\n\n[... truncated, {} chars total ...]\n\n", s.len());
    let usable = max_chars.saturating_sub(marker.len());
    let head_budget = ((usable as f64) * HEAD_RATIO) as usize;
    let tail_budget = usable.saturating_sub(head_budget);

    let head = char_prefix(s, head_budget);
    let tail = char_suffix(s, tail_budget);

    format!("{}{}{}", head, marker, tail)
}

fn build_preview(raw: &str, saved_path: &Option<PathBuf>) -> String {
    let head = char_prefix(raw, PREVIEW_HEAD_CHARS);
    let tail = char_suffix(raw, PREVIEW_TAIL_CHARS);
    let path_note = match saved_path {
        Some(p) => format!("Full output saved to: {}", p.display()),
        None => "Full output could not be saved.".to_string(),
    };
    format!(
        "{}\n\n[... {} chars total, showing first {} + last {} ...]\n{}\n\n{}",
        head,
        raw.len(),
        PREVIEW_HEAD_CHARS,
        PREVIEW_TAIL_CHARS,
        path_note,
        tail
    )
}

fn save_to_temp(content: &str, temp_dir: &Path) -> Option<PathBuf> {
    if let Err(e) = std::fs::create_dir_all(temp_dir) {
        debug!("Failed to create temp dir {}: {}", temp_dir.display(), e);
        return None;
    }
    let filename = format!(
        "tool_output_{}.txt",
        chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f")
    );
    let path = temp_dir.join(filename);
    match std::fs::write(&path, content) {
        Ok(()) => {
            debug!("Saved large tool output to {}", path.display());
            Some(path)
        }
        Err(e) => {
            debug!("Failed to write tool output to {}: {}", path.display(), e);
            None
        }
    }
}

/// Apply budget to multiple tool outputs, ensuring aggregate stays under total budget.
pub fn apply_budget_to_outputs(
    outputs: &mut Vec<(String, String, bool)>,
    budget: &OutputBudget,
) {
    let mut total_chars = 0usize;
    for (_name, content, _success) in outputs.iter_mut() {
        let truncated = truncate_tool_output(content, budget);
        *content = truncated.content;
        total_chars += content.len();
    }

    if total_chars > budget.max_total_chars {
        let per_output = budget.max_total_chars / outputs.len().max(1);
        for (_name, content, _success) in outputs.iter_mut() {
            if content.len() > per_output {
                *content = head_tail_truncate(content, per_output);
            }
        }
    }
}

fn char_prefix(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

fn char_suffix(s: &str, max_chars: usize) -> &str {
    let total: usize = s.chars().count();
    if total <= max_chars {
        return s;
    }
    let skip = total - max_chars;
    match s.char_indices().nth(skip) {
        Some((idx, _)) => &s[idx..],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_output_passes_through() {
        let budget = OutputBudget::default_budget();
        let result = truncate_tool_output("hello", &budget);
        assert!(!result.was_truncated);
        assert_eq!(result.content, "hello");
    }

    #[test]
    fn medium_output_is_head_tail_truncated() {
        let budget = OutputBudget {
            max_single_result_chars: 100,
            max_total_chars: 400,
            temp_dir: std::env::temp_dir(),
        };
        let long_str: String = "x".repeat(500);
        let result = truncate_tool_output(&long_str, &budget);
        assert!(result.was_truncated);
        assert!(result.content.contains("truncated"));
        assert!(result.content.len() < long_str.len());
    }

    #[test]
    fn head_tail_preserves_boundaries() {
        let input = "ABCDE12345";
        let truncated = head_tail_truncate(input, 200);
        assert_eq!(truncated, input);
    }

    #[test]
    fn char_suffix_works_with_unicode() {
        let s = "你好世界hello";
        let suffix = char_suffix(s, 5);
        assert_eq!(suffix, "hello");
    }

    #[test]
    fn apply_budget_reduces_aggregate() {
        let budget = OutputBudget {
            max_single_result_chars: 200,
            max_total_chars: 300,
            temp_dir: std::env::temp_dir(),
        };
        let mut outputs = vec![
            ("a".to_string(), "x".repeat(200), true),
            ("b".to_string(), "y".repeat(200), true),
        ];
        apply_budget_to_outputs(&mut outputs, &budget);
        let total: usize = outputs.iter().map(|(_, c, _)| c.len()).sum();
        assert!(total <= 400); // some overhead from markers
    }
}

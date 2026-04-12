//! Feishu interactive card renderer

/// Card status for state machine: Thinking → Executing → Complete
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStatus {
    Thinking,
    Executing,
    Complete,
}

/// Progress step shown during Executing state
#[derive(Debug, Clone)]
pub struct ProgressStep {
    pub icon: &'static str,  // "✓" | "⟳" | "✗"
    pub text: String,
}

impl CardStatus {
    pub fn render_card(&self, progress_lines: &[ProgressStep]) -> serde_json::Value {
        match self {
            CardStatus::Thinking => self.render_thinking_card(),
            CardStatus::Executing => self.render_executing_card(progress_lines),
            CardStatus::Complete => self.render_complete_card(),
        }
    }

    fn render_thinking_card(&self) -> serde_json::Value {
        serde_json::json!({
            "config": { "wide_screen_mode": true },
            "elements": [
                {
                    "tag": "markdown",
                    "content": "**🤖 AIOps Bot** 正在思考..."
                },
                { "tag": "hr" },
                {
                    "tag": "markdown",
                    "content": "░░░░░░░░░░░░░░░░  思考中"
                }
            ]
        })
    }

    fn render_executing_card(&self, steps: &[ProgressStep]) -> serde_json::Value {
        let steps_md = steps.iter()
            .map(|s| format!("{} {}", s.icon, s.text))
            .collect::<Vec<_>>()
            .join("\n");

        serde_json::json!({
            "config": { "wide_screen_mode": true },
            "elements": [
                {
                    "tag": "markdown",
                    "content": "**🤖 AIOps Bot** 执行中"
                },
                { "tag": "hr" },
                {
                    "tag": "markdown",
                    "content": steps_md
                },
                { "tag": "hr" },
                {
                    "tag": "markdown",
                    "content": "░░░░░░░░░░░░░░░░  处理中"
                }
            ]
        })
    }

    fn render_complete_card(&self) -> serde_json::Value {
        serde_json::json!({
            "config": { "wide_screen_mode": true },
            "elements": [
                {
                    "tag": "markdown",
                    "content": "**🤖 AIOps Bot** ✅ 完成"
                }
            ]
        })
    }
}

/// Build a complete result card with content
pub fn build_result_card(title: &str, body: &str) -> serde_json::Value {
    serde_json::json!({
        "config": { "wide_screen_mode": true },
        "elements": [
            {
                "tag": "markdown",
                "content": format!("**🤖 AIOps Bot** {}", title)
            },
            { "tag": "hr" },
            {
                "tag": "markdown",
                "content": body
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thinking_card_has_content() {
        let card = CardStatus::Thinking.render_card(&[]);
        let s = serde_json::to_string(&card).unwrap();
        assert!(s.contains("正在思考"));
    }

    #[test]
    fn test_complete_card_has_content() {
        let card = build_result_card("✅ 完成", "问题已解决");
        let s = serde_json::to_string(&card).unwrap();
        assert!(s.contains("完成"));
        assert!(s.contains("问题已解决"));
    }
}
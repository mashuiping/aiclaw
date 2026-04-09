//! Channel types for communication adapters

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Incoming message from a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelMessage {
    pub id: String,
    pub channel_name: String,
    pub channel_id: String,
    pub sender: SenderInfo,
    pub content: MessageContent,
    pub timestamp: DateTime<Utc>,
    pub thread_id: Option<String>,
    pub mentions_bot: bool,
    pub raw: serde_json::Value,
}

/// Sender information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderInfo {
    pub user_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub is_bot: bool,
}

/// Message content with parsed elements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageContent {
    pub text: String,
    pub mentions: Vec<Mention>,
    pub attachments: Vec<Attachment>,
}

/// A mention of a user or bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mention {
    pub id: String,
    pub name: String,
    pub mention_type: MentionType,
}

/// Type of mention
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MentionType {
    User,
    Channel,
    Here,
}

/// Message attachment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub attachment_type: String,
    pub url: Option<String>,
    pub content: Option<String>,
}

/// Outgoing message to send via channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessage {
    pub id: Option<String>,
    pub recipient: String,
    pub content: OutgoingContent,
    pub thread_id: Option<String>,
    pub reply_to: Option<String>,
}

/// Outgoing message content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OutgoingContent {
    Text(String),
    Formatted(FormattedMessage),
}

/// Formatted message with rich content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormattedMessage {
    pub body: String,
    pub format: MessageFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_block: Option<CodeBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<Table>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<MessageAction>>,
}

/// Message format type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageFormat {
    Plain,
    Markdown,
    Html,
}

/// Code block in message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub content: String,
}

/// Table in message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Action button in message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAction {
    pub action_type: ActionType,
    pub text: String,
    pub url: Option<String>,
    pub command: Option<String>,
}

/// Type of message action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    Button,
    Link,
    Command,
}

/// Create a basic text message
impl SendMessage {
    pub fn text(recipient: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: Some(Uuid::new_v4().to_string()),
            recipient: recipient.into(),
            content: OutgoingContent::Text(text.into()),
            thread_id: None,
            reply_to: None,
        }
    }

    pub fn markdown(recipient: impl Into<String>, markdown: impl Into<String>) -> Self {
        Self {
            id: Some(Uuid::new_v4().to_string()),
            recipient: recipient.into(),
            content: OutgoingContent::Formatted(FormattedMessage {
                body: markdown.into(),
                format: MessageFormat::Markdown,
                code_block: None,
                table: None,
                actions: None,
            }),
            thread_id: None,
            reply_to: None,
        }
    }
}

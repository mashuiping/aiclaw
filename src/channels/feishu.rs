//! Feishu (Lark) channel implementation

use async_trait::async_trait;
use aiclaw_types::channel::{
    ChannelMessage, Mention, MentionType, MessageContent, MessageFormat, OutgoingContent, SendMessage, SenderInfo,
};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::traits::Channel;
use crate::config::FeishuConfig;

// ============================================================================
// Feishu Webhook HTTP Handlers
// ============================================================================

#[derive(Clone)]
struct FeishuWebhookState {
    tx: mpsc::Sender<ChannelMessage>,
    verify_token: Option<String>,
}

#[derive(Deserialize)]
struct FeishuWebhookVerifyQuery {
    challenge: Option<String>,
}

async fn feishu_webhook_verify(
    Query(query): Query<FeishuWebhookVerifyQuery>,
    State(_state): State<Arc<FeishuWebhookState>>,
) -> Response {
    // Feishu webhook verification challenge
    if let Some(challenge) = query.challenge {
        info!("Feishu webhook verification challenge received");
        let body = serde_json::json!({ "challenge": challenge });
        (StatusCode::OK, Json(body)).into_response()
    } else {
        (StatusCode::BAD_REQUEST, "Missing challenge").into_response()
    }
}

#[derive(Deserialize)]
struct FeishuWebhookPayload {
    schema: Option<String>,
    header: Option<FeishuEventHeader>,
    event: Option<FeishuEventContent>,
}

async fn feishu_webhook_event(
    State(state): State<Arc<FeishuWebhookState>>,
    Json(payload): Json<serde_json::Value>,
) -> Response {
    debug!("Feishu webhook event received: {:?}", payload);

    // Try to parse as FeishuEvent first
    if let Ok(event) = serde_json::from_value::<FeishuEvent>(payload.clone()) {
        match forward_feishu_event(&state.tx, event).await {
            Ok(()) => (StatusCode::OK, "ok").into_response(),
            Err(e) => {
                error!("Failed to forward Feishu webhook event: {}", e);
                (StatusCode::INTERNAL_SERVER_ERROR, "Failed to process event").into_response()
            }
        }
    } else if let Ok(incoming) = serde_json::from_value::<FeishuWebhookPayload>(payload.clone()) {
        if let (Some(header), Some(event)) = (incoming.header, incoming.event) {
            let full_event = FeishuEvent {
                schema: incoming.schema,
                header,
                event,
            };
            match forward_feishu_event(&state.tx, full_event).await {
                Ok(()) => (StatusCode::OK, "ok").into_response(),
                Err(e) => {
                    error!("Failed to forward Feishu webhook event: {}", e);
                    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to process event").into_response()
                }
            }
        } else {
            warn!("Feishu webhook: missing header or event in payload");
            (StatusCode::BAD_REQUEST, "Missing header or event").into_response()
        }
    } else {
        warn!("Feishu webhook: unhandled payload format");
        (StatusCode::BAD_REQUEST, "Unsupported payload format").into_response()
    }
}

async fn forward_feishu_event(
    tx: &mpsc::Sender<ChannelMessage>,
    event: FeishuEvent,
) -> anyhow::Result<()> {
    // Extract all needed fields from event before consuming event.event.message
    let sender = event.event.sender.clone();
    let mentions = event.event.mentions.clone();

    if let Some(message) = event.event.message {
        let content: serde_json::Value = serde_json::from_str(&message.content)?;
        let text = content.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let message_id = message.message_id.clone();
        let chat_id = message.chat_id.clone();
        let thread_id = message.parent_id.clone().or(message.root_id.clone());
        let message_raw = message.clone();

        let sender_info = sender.as_ref()
            .map(|s| SenderInfo {
                user_id: s.sender_id.open_id.clone(),
                username: s.sender_id.open_id.clone(),
                display_name: None,
                is_bot: s.sender_type == "bot",
            })
            .unwrap_or_else(|| SenderInfo {
                user_id: "unknown".to_string(),
                username: "unknown".to_string(),
                display_name: None,
                is_bot: false,
            });

        let mentions_list = mentions.as_ref()
            .map(|mentions| {
                mentions.iter().map(|m| Mention {
                    id: m.id.open_id.clone().unwrap_or_default(),
                    name: m.key.clone(),
                    mention_type: if m.mention_type == "at" {
                        MentionType::User
                    } else {
                        MentionType::User
                    },
                }).collect()
            })
            .unwrap_or_default();

        let channel_msg = ChannelMessage {
            id: message_id,
            channel_name: "feishu".to_string(),
            channel_id: chat_id,
            sender: sender_info,
            content: MessageContent {
                text: text.to_string(),
                mentions: mentions_list,
                attachments: Vec::new(),
            },
            timestamp: Utc::now(),
            thread_id,
            mentions_bot: true,
            raw: serde_json::to_value(&message_raw)?,
        };

        tx.send(channel_msg).await?;
    } else {
        debug!("Feishu event dropped: no message in event");
    }
    Ok(())
}

// ============================================================================
// Feishu Channel Implementation
// ============================================================================

/// Feishu channel implementation
pub struct FeishuChannel {
    name: String,
    config: FeishuConfig,
}

impl FeishuChannel {
    pub fn new(name: impl Into<String>, config: FeishuConfig) -> anyhow::Result<Self> {
        Ok(Self {
            name: name.into(),
            config,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuEvent {
    #[serde(rename = "schema")]
    pub schema: Option<String>,
    pub header: FeishuEventHeader,
    pub event: FeishuEventContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuEventHeader {
    pub event_id: String,
    pub event_type: String,
    pub create_time: String,
    pub token: String,
    pub app_id: String,
    pub tenant_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuEventContent {
    pub sender: Option<FeishuSender>,
    pub message: Option<FeishuMessage>,
    pub mentions: Option<Vec<FeishuMention>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuSender {
    pub sender_id: FeishuSenderId,
    pub sender_type: String,
    pub tenant_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuSenderId {
    pub open_id: String,
    pub union_id: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuMessage {
    pub message_id: String,
    pub root_id: Option<String>,
    pub parent_id: Option<String>,
    pub create_time: String,
    pub chat_id: String,
    pub chat_type: String,
    pub message_type: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuMention {
    pub key: String,
    pub id: FeishuMentionId,
    #[serde(rename = "type")]
    pub mention_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuMentionId {
    pub open_id: Option<String>,
    pub user_id: Option<String>,
    pub chat_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuOutgoingMessage {
    pub receive_id: String,
    pub receive_id_type: String,
    pub msg_type: String,
    pub content: String,
}

impl FeishuOutgoingMessage {
    pub fn new_text(recipient: &str, text: &str) -> Self {
        Self {
            receive_id: recipient.to_string(),
            receive_id_type: "open_id".to_string(),
            msg_type: "text".to_string(),
            content: serde_json::json!({ "text": text }).to_string(),
        }
    }

    pub fn new_markdown(recipient: &str, markdown: &str) -> Self {
        Self {
            receive_id: recipient.to_string(),
            receive_id_type: "open_id".to_string(),
            msg_type: "interactive".to_string(),
            content: serde_json::json!({
                "config": { "wide_screen_mode": true },
                "elements": [{
                    "tag": "markdown",
                    "content": markdown
                }]
            }).to_string(),
        }
    }
}

#[async_trait]
impl Channel for FeishuChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, msg: &SendMessage) -> anyhow::Result<()> {
        let webhook_url = self.config.webhook_url.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Feishu webhook URL not configured"))?;

        let feishu_msg = match &msg.content {
            OutgoingContent::Text(text) => FeishuOutgoingMessage::new_text(&msg.recipient, text),
            OutgoingContent::Formatted(formatted) => {
                match formatted.format {
                    MessageFormat::Markdown | MessageFormat::Plain => {
                        FeishuOutgoingMessage::new_markdown(&msg.recipient, &formatted.body)
                    }
                    MessageFormat::Html => {
                        FeishuOutgoingMessage::new_text(&msg.recipient, &formatted.body)
                    }
                }
            }
        };

        let client = reqwest::Client::new();
        let response = client
            .post(webhook_url)
            .json(&feishu_msg)
            .send()
            .await?;

        if response.status().is_success() {
            debug!("Feishu message sent successfully to {}", msg.recipient);
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Feishu API error: {} - {}", status, body);
            anyhow::bail!("Feishu API returned error: {} - {}", status, body)
        }
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("Starting Feishu listener for channel {}", self.name);
        self.listen_webhook(tx).await
    }

    async fn health_check(&self) -> bool {
        if let Some(webhook_url) = &self.config.webhook_url {
            let client = reqwest::Client::new();
            match client.get(webhook_url).send().await {
                Ok(response) => response.status().is_success(),
                Err(_) => false,
            }
        } else {
            true
        }
    }

    fn supports_typing(&self) -> bool {
        false
    }
}

impl FeishuChannel {
    async fn listen_webhook(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let bind = self.config.webhook_listen_addr.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Feishu webhook_listen_addr not configured"))?;

        info!("Feishu webhook listener starting on {}", bind);

        let state = FeishuWebhookState {
            tx,
            verify_token: self.config.verify_token.clone(),
        };

        let app = Router::new()
            .route("/webhook", get(feishu_webhook_verify))
            .route("/webhook", post(feishu_webhook_event))
            .with_state(Arc::new(state));

        let addr: std::net::SocketAddr = bind.parse()
            .map_err(|_| anyhow::anyhow!("Invalid socket address: {}", bind))?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!("Feishu webhook server listening on {}", addr);

        axum::serve(listener, app).await?;
        Ok(())
    }
}

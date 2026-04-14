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
use tokio::time::Duration;
use tracing::{debug, error, info, warn};

use super::traits::Channel;
use super::feishu_api::{FeishuAPIClient, LongPollMessage};
use super::feishu_card::build_result_card;
use super::streaming_buffer::StreamingBuffer;
use crate::config::FeishuConfig;

// ============================================================================
// Feishu Webhook HTTP Handlers
// ============================================================================

#[derive(Clone)]
struct FeishuWebhookState {
    tx: mpsc::Sender<ChannelMessage>,
    channel_name: String,
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
        match forward_feishu_event(&state.tx, &state.channel_name, event).await {
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
            match forward_feishu_event(&state.tx, &state.channel_name, full_event).await {
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
    channel_name: &str,
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
                    mention_type: match m.mention_type.as_str() {
                        "at" => MentionType::User,
                        "here" => MentionType::Here,
                        "channel" => MentionType::Channel,
                        _ => MentionType::Channel,
                    },
                }).collect()
            })
            .unwrap_or_default();

        let channel_msg = ChannelMessage {
            id: message_id,
            channel_name: channel_name.to_string(),
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
    api_client: Arc<FeishuAPIClient>,
    streaming_buffer: StreamingBuffer,
}

impl FeishuChannel {
    pub fn new(name: impl Into<String>, config: FeishuConfig) -> anyhow::Result<Self> {
        let api_client = if let (Some(app_id), Some(app_secret)) = (&config.app_id, &config.app_secret) {
            FeishuAPIClient::new(app_id.clone(), app_secret.clone())
        } else {
            FeishuAPIClient::new(String::new(), String::new())
        };
        Ok(Self {
            name: name.into(),
            config,
            api_client: Arc::new(api_client),
            streaming_buffer: StreamingBuffer::new(),
        })
    }
}

fn convert_long_poll_message(msg: LongPollMessage) -> Option<ChannelMessage> {
    let content: serde_json::Value = serde_json::from_str(&msg.content).ok()?;
    let text = content.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let sender_info = msg.sender.as_ref().map(|s| SenderInfo {
        user_id: s.sender_id.open_id.clone(),
        username: s.sender_id.open_id.clone(),
        display_name: None,
        is_bot: s.sender_type == "bot",
    }).unwrap_or_else(|| SenderInfo {
        user_id: "unknown".to_string(),
        username: "unknown".to_string(),
        display_name: None,
        is_bot: false,
    });

    let message_id = msg.message_id.clone();
    let chat_id = msg.chat_id.clone();
    let thread_id = msg.parent_id.clone().or(msg.root_id.clone());

    Some(ChannelMessage {
        id: message_id.clone(),
        channel_name: "feishu".to_string(),
        channel_id: chat_id.clone(),
        sender: sender_info,
        content: MessageContent {
            text: text.to_string(),
            mentions: Vec::new(),
            attachments: Vec::new(),
        },
        timestamp: chrono::Utc::now(),
        thread_id,
        mentions_bot: true,
        raw: serde_json::json!({
            "message_id": message_id,
            "root_id": msg.root_id,
            "parent_id": msg.parent_id,
            "create_time": msg.create_time,
            "chat_id": chat_id,
            "chat_type": msg.chat_type,
            "message_type": msg.message_type,
            "content": msg.content,
        }),
    })
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
        // If message has an ID (updatable message), use update path
        if let Some(ref msg_id) = msg.id {
            match &msg.content {
                OutgoingContent::Text(text) => {
                    self.api_client.update_text_message(msg_id, text).await?;
                }
                OutgoingContent::Formatted(formatted) => {
                    match formatted.format {
                        MessageFormat::Markdown | MessageFormat::Plain => {
                            let card = build_result_card("✅ 完成", &formatted.body);
                            self.api_client.update_interactive_card(msg_id, &card).await?;
                        }
                        MessageFormat::Html => {
                            self.api_client.update_text_message(msg_id, &formatted.body).await?;
                        }
                    }
                }
            }
            return Ok(());
        }

        // No ID = create new message
        let recipient = &msg.recipient;
        match &msg.content {
            OutgoingContent::Text(text) => {
                self.api_client.send_text_message(recipient, text).await?;
            }
            OutgoingContent::Formatted(formatted) => {
                match formatted.format {
                    MessageFormat::Markdown | MessageFormat::Plain => {
                        let card = build_result_card("✅ 完成", &formatted.body);
                        self.api_client.send_interactive_card(recipient, &card).await?;
                    }
                    MessageFormat::Html => {
                        self.api_client.send_text_message(recipient, &formatted.body).await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("Starting Feishu channel {} (webhook + long poll)", self.name);

        // Start long polling if app_id/app_secret are configured
        if self.config.app_id.is_some() && self.config.app_secret.is_some() {
            let api_client = self.api_client.clone();
            let polling_timeout = if self.config.polling_timeout_secs > 0 {
                self.config.polling_timeout_secs
            } else {
                30
            };
            let tx_clone = tx.clone();

            tokio::spawn(async move {
                loop {
                    match api_client.long_poll_messages(polling_timeout).await {
                        Ok(messages) => {
                            for msg in messages {
                                if let Some(channel_msg) = convert_long_poll_message(msg) {
                                    let _ = tx_clone.send(channel_msg).await;
                                }
                            }
                        }
                        Err(e) => {
                            warn!("Feishu long poll error: {}", e);
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
            });
        }

        // Also start webhook server
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
            channel_name: self.name.clone(),
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

    async fn listen_long_polling(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("Feishu long polling listener started");

        let app_id = self
            .config
            .app_id
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Feishu app_id not configured"))?;
        let app_secret = self
            .config
            .app_secret
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Feishu app_secret not configured"))?;

        let tenant_access_token = self.get_tenant_access_token(app_id, app_secret).await?;

        loop {
            match self.fetch_long_polling_events(&tenant_access_token).await {
                Ok(events) => {
                    for event in events {
                        if let Err(e) = self.process_event(&tx, event).await {
                            error!("Error processing Feishu event: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Feishu long polling error: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn get_tenant_access_token(&self, app_id: &str, app_secret: &str) -> anyhow::Result<String> {
        let client = reqwest::Client::new();
        let response = client
            .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
            .json(&serde_json::json!({
                "app_id": app_id,
                "app_secret": app_secret
            }))
            .send()
            .await?;

        #[derive(Deserialize)]
        struct TokenResponse {
            #[allow(dead_code)]
            code: i32,
            msg: String,
            tenant_access_token: Option<String>,
        }

        let token_resp: TokenResponse = response.json().await?;
        token_resp.tenant_access_token
            .ok_or_else(|| anyhow::anyhow!("Failed to get tenant access token: {}", token_resp.msg))
    }

    async fn fetch_long_polling_events(&self, _token: &str) -> anyhow::Result<Vec<FeishuEvent>> {
        Ok(Vec::new())
    }

    async fn process_event(
        &self,
        tx: &mpsc::Sender<ChannelMessage>,
        event: FeishuEvent,
    ) -> anyhow::Result<()> {
        forward_feishu_event(tx, &self.name, event).await
    }
}

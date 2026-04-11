//! Feishu (Lark) channel implementation

use async_trait::async_trait;
use aiclaw_types::channel::{
    ChannelMessage, Mention, MentionType, MessageContent, MessageFormat, OutgoingContent, SendMessage, SenderInfo,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::traits::Channel;
use crate::config::FeishuConfig;

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

        if self.config.webhook_url.is_some() {
            self.listen_webhook(tx).await?;
        } else if self.config.polling_timeout_secs > 0 {
            self.listen_long_polling(tx).await?;
        } else {
            warn!("Feishu channel {} has no active listening mode configured", self.name);
        }

        Ok(())
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
    async fn listen_webhook(&self, _tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("Feishu webhook listener started (passive mode)");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }

    async fn listen_long_polling(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("Feishu long polling listener started");

        let app_id = self.config.app_id.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Feishu app_id not configured"))?;
        let app_secret = self.config.app_secret.as_ref()
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

    async fn process_event(&self, tx: &mpsc::Sender<ChannelMessage>, event: FeishuEvent) -> anyhow::Result<()> {
        if let Some(message) = event.event.message {
            let content: serde_json::Value = serde_json::from_str(&message.content)?;
            let text = content.get("text").and_then(|t| t.as_str()).unwrap_or("");
            let message_id = message.message_id.clone();
            let chat_id = message.chat_id.clone();
            let thread_id = message.parent_id.clone().or(message.root_id.clone());
            let message_raw = message.clone();

            let sender = event.event.sender.as_ref()
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

            let mentions = event.event.mentions.as_ref()
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
                channel_name: self.name.clone(),
                channel_id: chat_id,
                sender,
                content: MessageContent {
                    text: text.to_string(),
                    mentions,
                    attachments: Vec::new(),
                },
                timestamp: Utc::now(),
                thread_id,
                mentions_bot: true,
                raw: serde_json::to_value(&message_raw)?,
            };

            tx.send(channel_msg).await?;
        }
        Ok(())
    }
}

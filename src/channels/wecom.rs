//! WeCom (企业微信) channel implementation

use async_trait::async_trait;
use aiclaw_types::channel::{
    ChannelMessage,
    MessageContent, MessageFormat, OutgoingContent, SendMessage, SenderInfo,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::traits::Channel;
use crate::config::WeComConfig;

/// WeCom channel implementation
pub struct WeComChannel {
    name: String,
    config: WeComConfig,
}

impl WeComChannel {
    pub fn new(name: impl Into<String>, config: WeComConfig) -> anyhow::Result<Self> {
        Ok(Self {
            name: name.into(),
            config,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeComIncomingMessage {
    pub msg_type: String,
    pub agent_id: u64,
    pub content: String,
    pub from_username: String,
    pub create_time: u64,
    pub chat_id: Option<String>,
    pub msg_id: Option<String>,
    #[serde(rename = "toUsername")]
    pub to_username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeComOutgoingMessage {
    pub msgtype: String,
    pub agentid: u64,
    pub text: Option<WeComTextContent>,
    pub markdown: Option<WeComMarkdownContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeComTextContent {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeComMarkdownContent {
    pub content: String,
}

impl WeComOutgoingMessage {
    pub fn new_text(agent_id: u64, text: &str) -> Self {
        Self {
            msgtype: "text".to_string(),
            agentid: agent_id,
            text: Some(WeComTextContent {
                content: text.to_string(),
            }),
            markdown: None,
        }
    }

    pub fn new_markdown(agent_id: u64, markdown: &str) -> Self {
        Self {
            msgtype: "markdown".to_string(),
            agentid: agent_id,
            text: None,
            markdown: Some(WeComMarkdownContent {
                content: markdown.to_string(),
            }),
        }
    }
}

#[async_trait]
impl Channel for WeComChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, msg: &SendMessage) -> anyhow::Result<()> {
        let webhook_url = self.config.webhook_url.as_ref()
            .ok_or_else(|| anyhow::anyhow!("WeCom webhook URL not configured"))?;

        let agent_id = self.config.agent_id.as_ref()
            .and_then(|id| id.parse::<u64>().ok())
            .unwrap_or(0);

        let wecom_msg = match &msg.content {
            OutgoingContent::Text(text) => WeComOutgoingMessage::new_text(agent_id, text),
            OutgoingContent::Formatted(formatted) => {
                match formatted.format {
                    MessageFormat::Markdown => {
                        WeComOutgoingMessage::new_markdown(agent_id, &formatted.body)
                    }
                    _ => {
                        WeComOutgoingMessage::new_text(agent_id, &formatted.body)
                    }
                }
            }
        };

        let client = reqwest::Client::new();
        let response = client
            .post(webhook_url)
            .json(&wecom_msg)
            .send()
            .await?;

        if response.status().is_success() {
            debug!("WeCom message sent successfully to {}", msg.recipient);
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("WeCom API error: {} - {}", status, body);
            anyhow::bail!("WeCom API returned error: {} - {}", status, body)
        }
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("Starting WeCom listener for channel {}", self.name);

        if self.config.webhook_url.is_some() {
            self.listen_webhook(tx).await?;
        } else {
            warn!("WeCom channel {} has no webhook URL configured", self.name);
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

impl WeComChannel {
    async fn listen_webhook(&self, _tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("WeCom webhook listener started (passive mode - use HTTP server for webhook endpoint)");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }

    pub async fn handle_webhook(
        &self,
        message: WeComIncomingMessage,
        tx: &mpsc::Sender<ChannelMessage>,
    ) -> anyhow::Result<()> {
        let msg_clone = message.clone();
        let channel_msg = ChannelMessage {
            id: message.msg_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            channel_name: self.name.clone(),
            channel_id: message.chat_id.clone().unwrap_or_default(),
            sender: SenderInfo {
                user_id: message.from_username.clone(),
                username: message.from_username.clone(),
                display_name: None,
                is_bot: false,
            },
            content: MessageContent {
                text: message.content.clone(),
                mentions: Vec::new(),
                attachments: Vec::new(),
            },
            timestamp: Utc::now(),
            thread_id: None,
            mentions_bot: true,
            raw: serde_json::to_value(&msg_clone)?,
        };

        tx.send(channel_msg).await?;
        Ok(())
    }
}

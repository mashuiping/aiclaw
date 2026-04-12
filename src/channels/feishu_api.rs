//! Feishu Open Platform API client

use std::sync::Arc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, error};

const FEISHU_API_BASE: &str = "https://open.feishu.cn/open-apis";

/// Feishu API client with automatic token management
#[derive(Clone)]
pub struct FeishuAPIClient {
    app_id: String,
    app_secret: String,
    client: Client,
    token: Arc<RwLock<Option<AppAccessToken>>>,
}

#[derive(Debug, Clone)]
struct AppAccessToken {
    token: String,
    expires_at: tokio::time::Instant,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    code: i32,
    msg: String,
    app_access_token: String,
    expire: i32,
}

#[derive(Debug, Serialize)]
struct SendMessageRequest {
    receive_id: String,
    receive_id_type: String,
    msg_type: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct SendMessageResponse {
    code: i32,
    msg: String,
    data: Option<MessageData>,
}

#[derive(Debug, Deserialize)]
struct MessageData {
    message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LongPollResponse {
    code: i32,
    msg: String,
    data: Option<LongPollData>,
}

#[derive(Debug, Deserialize)]
struct LongPollData {
    has_more: bool,
    sync_tokens: Option<String>,
    messages: Option<Vec<LongPollMessage>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LongPollMessage {
    pub message_id: String,
    pub root_id: Option<String>,
    pub parent_id: Option<String>,
    pub create_time: String,
    pub chat_id: String,
    pub chat_type: String,
    pub sender: Option<LongPollSender>,
    pub message_type: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LongPollSender {
    pub sender_id: LongPollSenderId,
    pub sender_type: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LongPollSenderId {
    pub open_id: String,
    pub union_id: Option<String>,
}

impl FeishuAPIClient {
    pub fn new(app_id: String, app_secret: String) -> Self {
        Self {
            app_id,
            app_secret,
            client: Client::new(),
            token: Arc::new(RwLock::new(None)),
        }
    }

    /// Get a valid app_access_token (auto-refreshes when near expiry)
    async fn get_token(&self) -> anyhow::Result<String> {
        // Check if existing token is still valid (refresh 60s early)
        {
            let token_guard = self.token.read().await;
            if let Some(ref t) = *token_guard {
                if t.expires_at > tokio::time::Instant::now() + tokio::time::Duration::from_secs(60) {
                    return Ok(t.token.clone());
                }
            }
        }

        // Fetch new token
        let url = format!("{}/auth/v3/app_access_token/internal", FEISHU_API_BASE);
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret
        });

        let resp = self.client.post(&url).json(&body).send().await?;
        let token_resp: TokenResponse = resp.json().await?;

        if token_resp.code != 0 {
            anyhow::bail!("Feishu token error: {} - {}", token_resp.code, token_resp.msg);
        }

        let token_clone = token_resp.app_access_token.clone();
        let new_token = AppAccessToken {
            token: token_clone,
            expires_at: tokio::time::Instant::now() + tokio::time::Duration::from_secs(token_resp.expire as u64),
        };

        *self.token.write().await = Some(new_token);
        Ok(token_resp.app_access_token)
    }

    /// Send a text message, returns message_id
    pub async fn send_text_message(&self, receive_id: &str, content: &str) -> anyhow::Result<String> {
        self.send_message(receive_id, "open_id", "text", &serde_json::json!({ "text": content }).to_string()).await
    }

    /// Send an interactive card, returns message_id
    pub async fn send_interactive_card(&self, receive_id: &str, card: &serde_json::Value) -> anyhow::Result<String> {
        self.send_message(receive_id, "open_id", "interactive", &card.to_string()).await
    }

    /// Update a text message's content
    pub async fn update_text_message(&self, message_id: &str, content: &str) -> anyhow::Result<()> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/messages/{}", FEISHU_API_BASE, message_id);

        let body = serde_json::json!({
            "receive_id": "",
            "content": serde_json::json!({ "text": content }).to_string(),
            "msg_type": "text"
        });

        let resp = self.client.patch(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            error!("Feishu update message error: {} - {}", status, body_text);
            anyhow::bail!("Feishu update message failed: {} - {}", status, body_text);
        }

        Ok(())
    }

    /// Update an interactive card
    pub async fn update_interactive_card(&self, message_id: &str, card: &serde_json::Value) -> anyhow::Result<()> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/messages/{}", FEISHU_API_BASE, message_id);

        let body = serde_json::json!({
            "content": card.to_string(),
            "msg_type": "interactive"
        });

        let resp = self.client.patch(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            error!("Feishu update card error: {} - {}", status, body_text);
            anyhow::bail!("Feishu update card failed: {} - {}", status, body_text);
        }

        Ok(())
    }

    /// Long-poll for new messages
    pub async fn long_poll_messages(&self, _timeout_secs: u64) -> anyhow::Result<Vec<LongPollMessage>> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/messages?receive_id_type=open_id&container_id_type=p2p", FEISHU_API_BASE);

        let resp = self.client.get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("X-Tt-Logid", "aiclaw-long-poll")
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            debug!("Feishu long poll: {} - {}", status, body_text);
            return Ok(Vec::new());
        }

        let poll_resp: LongPollResponse = resp.json().await?;

        if poll_resp.code != 0 {
            debug!("Feishu long poll empty: {}", poll_resp.msg);
            return Ok(Vec::new());
        }

        Ok(poll_resp.data.and_then(|d| d.messages).unwrap_or_default())
    }

    async fn send_message(&self, receive_id: &str, receive_id_type: &str, msg_type: &str, content: &str) -> anyhow::Result<String> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/messages?receive_id_type={}", FEISHU_API_BASE, receive_id_type);

        let body = SendMessageRequest {
            receive_id: receive_id.to_string(),
            receive_id_type: receive_id_type.to_string(),
            msg_type: msg_type.to_string(),
            content: content.to_string(),
        };

        let resp = self.client.post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            error!("Feishu send message error: {} - {}", status, body_text);
            anyhow::bail!("Feishu send message failed: {} - {}", status, body_text);
        }

        let resp_body: SendMessageResponse = resp.json().await?;

        if resp_body.code != 0 {
            anyhow::bail!("Feishu API error: {} - {}", resp_body.code, resp_body.msg);
        }

        resp_body.data
            .and_then(|d| d.message_id)
            .ok_or_else(|| anyhow::anyhow!("No message_id in response"))
    }
}

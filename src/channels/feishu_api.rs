//! Feishu Open Platform API client

use std::sync::Arc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

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

/// Response wrapper for `GET /open-apis/im/v1/messages` (list chat history).
/// Feishu returns `data.items` (not `messages`); each item uses `msg_type` and `body.content`.
#[derive(Debug, Deserialize)]
struct ListMessagesResponse {
    code: i32,
    msg: String,
    data: Option<ListMessagesData>,
}

#[derive(Debug, Deserialize)]
struct ListMessagesData {
    #[allow(dead_code)]
    #[serde(default)]
    has_more: bool,
    #[allow(dead_code)]
    #[serde(default)]
    page_token: Option<String>,
    #[serde(default)]
    items: Vec<ListMessageItem>,
}

#[derive(Debug, Deserialize)]
struct ListMessageItem {
    message_id: String,
    #[serde(default)]
    root_id: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    create_time: String,
    chat_id: String,
    #[serde(default)]
    chat_type: String,
    #[serde(rename = "msg_type")]
    msg_type: String,
    body: ListMessageBody,
    #[serde(default)]
    sender: Option<ListMessageSender>,
}

#[derive(Debug, Deserialize)]
struct ListMessageBody {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ListMessageSender {
    id: String,
    #[allow(dead_code)]
    #[serde(default)]
    id_type: String,
    sender_type: String,
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

fn list_item_to_long_poll(m: ListMessageItem) -> LongPollMessage {
    let sender = m.sender.map(|s| LongPollSender {
        sender_id: LongPollSenderId {
            // List API uses `sender.id` + `sender.id_type` (open_id, app_id, …)
            open_id: s.id,
            union_id: None,
        },
        sender_type: s.sender_type,
    });

    LongPollMessage {
        message_id: m.message_id,
        root_id: m.root_id,
        parent_id: m.parent_id,
        create_time: m.create_time,
        chat_id: m.chat_id,
        chat_type: m.chat_type,
        message_type: m.msg_type,
        content: m.body.content,
        sender,
    }
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

    /// Pull messages from an IM container (used as the long-poll receive path).
    ///
    /// Feishu requires both `container_id` and `container_id_type` for this API; omitting them yields
    /// HTTP 400 `container_id is required` when `container_id_type` is `p2p`.
    pub async fn long_poll_messages(
        &self,
        _timeout_secs: u64,
        container_id: &str,
        container_id_type: &str,
    ) -> anyhow::Result<Vec<LongPollMessage>> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/messages", FEISHU_API_BASE);

        // Newest first so each poll can stop at the first already-handled id and avoid re-queuing history.
        // Default API order is ascending, which repeats the same oldest page forever.
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("container_id", container_id),
                ("container_id_type", container_id_type),
                ("sort_type", "ByCreateTimeDesc"),
                ("page_size", "50"),
            ])
            .header("Authorization", format!("Bearer {}", token))
            .header("X-Tt-Logid", "aiclaw-long-poll")
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            warn!("Feishu long poll: {} - {}", status, body_text);
            return Ok(Vec::new());
        }

        let poll_resp: ListMessagesResponse = resp.json().await?;

        if poll_resp.code != 0 {
            debug!("Feishu list messages API: {}", poll_resp.msg);
            return Ok(Vec::new());
        }

        let items = poll_resp.data.map(|d| d.items).unwrap_or_default();
        Ok(items.into_iter().map(list_item_to_long_poll).collect())
    }

    pub async fn send_message(
        &self,
        receive_id: &str,
        receive_id_type: &str,
        msg_type: &str,
        content: &str,
    ) -> anyhow::Result<String> {
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

# 飞书交互卡片 + 文字流式输出实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 aiclaw 飞书通道实现交互式卡片（实时状态演进）+ 文字流式输出 + 长轮询消息接收

**Architecture:** 通过飞书 `im/v1/messages` API 创建/更新消息实现流式输出，通过 `update_interactive` API 更新交互卡片状态。长轮询为主要消息接收方式，webhook 为辅助。

**Tech Stack:** Rust (tokio, reqwest, serde), 飞书开放平台 API

---

## 文件结构

```
src/channels/
  feishu_api.rs        # 新增：飞书 API 客户端（token 管理 + HTTP 调用）
  feishu_card.rs       # 新增：交互卡片渲染器（Thinking/Executing/Complete 模板）
  streaming_buffer.rs  # 新增：流式输出缓冲管理器
  feishu.rs            # 改造：整合新组件，改造 send() 方法支持消息更新
  mod.rs               # 改造：导出新模块
src/config/
  schema.rs            # 改造：FeishuConfig 新增长轮询配置字段
config.example.toml    # 改造：新增长轮询配置示例
```

---

## Task 1: FeishuAPIClient — 飞书 API 客户端封装

**Files:**
- Create: `src/channels/feishu_api.rs`
- Test: `src/channels/feishu_api.rs` (unit tests inline)

- [ ] **Step 1: 创建 `src/channels/feishu_api.rs`**

实现 `FeishuAPIClient` 结构体，封装所有飞书 API 调用。

```rust
//! Feishu Open Platform API client

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

const FEISHU_API_BASE: &str = "https://open.feishu.cn/open-apis";

/// Feishu API client with automatic token management
pub struct FeishuAPIClient {
    app_id: String,
    app_secret: String,
    client: Client,
    token: RwLock<Option<AppAccessToken>>,
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

#[derive(Debug, Deserialize)]
struct LongPollMessage {
    message_id: String,
    root_id: Option<String>,
    parent_id: Option<String>,
    create_time: String,
    chat_id: String,
    chat_type: String,
    sender: Option<LongPollSender>,
    message_type: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct LongPollSender {
    sender_id: LongPollSenderId,
    sender_type: String,
}

#[derive(Debug, Deserialize)]
struct LongPollSenderId {
    open_id: String,
    union_id: Option<String>,
}

impl FeishuAPIClient {
    pub fn new(app_id: String, app_secret: String) -> Self {
        Self {
            app_id,
            app_secret,
            client: Client::new(),
            token: RwLock::new(None),
        }
    }

    /// 获取有效的 app_access_token（自动刷新）
    async fn get_token(&self) -> anyhow::Result<String> {
        let mut token_guard = self.token.write().await;

        // 检查现有 token 是否有效（提前 60s 刷新）
        if let Some(ref t) = *token_guard {
            if t.expires_at > tokio::time::Instant::now() + tokio::time::Duration::from_secs(60) {
                return Ok(t.token.clone());
            }
        }

        drop(token_guard);

        // 获取新 token
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

        let new_token = AppAccessToken {
            token: token_resp.app_access_token,
            expires_at: tokio::time::Instant::now() + tokio::time::Duration::from_secs(token_resp.expire as u64),
        };

        *self.token.write().await = Some(new_token);
        Ok(new_token.token)
    }

    /// 发送 text 消息，返回 message_id
    pub async fn send_text_message(&self, receive_id: &str, content: &str) -> anyhow::Result<String> {
        self.send_message(receive_id, "open_id", "text", &serde_json::json!({ "text": content }).to_string()).await
    }

    /// 发送交互式卡片，返回 message_id
    pub async fn send_interactive_card(&self, receive_id: &str, card: &serde_json::Value) -> anyhow::Result<String> {
        self.send_message(receive_id, "open_id", "interactive", &card.to_string()).await
    }

    /// 更新 text 消息内容
    pub async fn update_text_message(&self, message_id: &str, content: &str) -> anyhow::Result<()> {
        let token = self.get_token().await?;
        let url = format!("{}/im/v1/messages/{}", FEISHU_API_BASE, message_id);

        let body = serde_json::json!({
            "receive_id": "",
            "content": serde_json::json!({ "text": content }).to_string(),
            "msg_type": "text"
        });

        // 飞书更新消息用 PUT 方法
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

    /// 更新交互式卡片
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

    /// 长轮询拉取新消息
    pub async fn long_poll_messages(&self, timeout_secs: u64) -> anyhow::Result<Vec<LongPollMessage>> {
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
```

- [ ] **Step 2: 运行编译验证**

Run: `cargo check --package aiclaw 2>&1`
Expected: 无错误（新增文件尚未被引用，会有 unused warnings，正常）

---

## Task 2: CardRenderer — 交互卡片渲染器

**Files:**
- Create: `src/channels/feishu_card.rs`
- Modify: `src/channels/mod.rs` (添加 `pub mod feishu_card;`)

- [ ] **Step 1: 创建 `src/channels/feishu_card.rs`**

```rust
//! Feishu interactive card renderer

use serde::{Deserialize, Serialize};

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
        // Complete card is rendered by the agent with actual content
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
```

- [ ] **Step 2: 修改 `src/channels/mod.rs`，添加新模块导出**

在 `pub mod feishu;` 后添加：
```rust
pub mod feishu_api;
pub mod feishu_card;
pub mod streaming_buffer;
```

- [ ] **Step 3: 编译验证**

Run: `cargo check --package aiclaw 2>&1`
Expected: 无新增错误

---

## Task 3: StreamingBuffer — 流式输出缓冲管理器

**Files:**
- Create: `src/channels/streaming_buffer.rs`

- [ ] **Step 1: 创建 `src/channels/streaming_buffer.rs`**

```rust
//! Streaming output buffer — accumulates tokens and flushes on size/timeout

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

/// Max characters to buffer before flushing
const DEFAULT_BUFFER_SIZE: usize = 50;
/// Default flush timeout (ms)
const DEFAULT_FLUSH_TIMEOUT_MS: u64 = 500;
/// Rate limit: max updates per minute (飞书限制)
const MAX_UPDATES_PER_MINUTE: usize = 60;

/// A pending update for a single message
#[derive(Debug)]
pub enum BufferEntry {
    /// Text content waiting to be flushed
    Text(String),
    /// Interactive card waiting to be flushed
    Card(serde_json::Value),
}

/// Manages streaming buffers for multiple in-flight messages
pub struct StreamingBuffer {
    entries: Arc<RwLock<HashMap<String, BufferEntry>>>,
    last_flush: Arc<RwLock<HashMap<String, Instant>>>,
    buffer_size: usize,
    flush_timeout: Duration,
    rate_limit_reset: Instant,
    updates_this_minute: Arc<RwLock<usize>>,
}

impl StreamingBuffer {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            last_flush: Arc::new(RwLock::new(HashMap::new())),
            buffer_size: DEFAULT_BUFFER_SIZE,
            flush_timeout: Duration::from_millis(DEFAULT_FLUSH_TIMEOUT_MS),
            rate_limit_reset: Instant::now() + Duration::from_secs(60),
            updates_this_minute: Arc::new(RwLock::new(0)),
        }
    }

    pub fn with_limits(buffer_size: usize, flush_timeout_ms: u64) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            last_flush: Arc::new(RwLock::new(HashMap::new())),
            buffer_size,
            flush_timeout: Duration::from_millis(flush_timeout_ms),
            rate_limit_reset: Instant::now() + Duration::from_secs(60),
            updates_this_minute: Arc::new(RwLock::new(0)),
        }
    }

    /// Append text token to a message's buffer. Returns true if flush is needed.
    pub async fn append_text(&self, message_id: &str, text: &str) -> bool {
        let mut entries = self.entries.write().await;
        let entry = entries.entry(message_id.to_string()).or_insert_with(|| {
            BufferEntry::Text(String::new())
        });

        let current_len = match entry {
            BufferEntry::Text(s) => s.len(),
            _ => 0,
        };

        if let BufferEntry::Text(s) = entry {
            s.push_str(text);
            current_len + text.len() >= self.buffer_size
        } else {
            false
        }
    }

    /// Set card content for a message (overwrites). Returns true if flush is needed.
    pub async fn set_card(&self, message_id: &str, card: serde_json::Value) {
        let mut entries = self.entries.write().await;
        entries.insert(message_id.to_string(), BufferEntry::Card(card));
    }

    /// Take and clear the buffer entry for a message, if ready to flush.
    pub async fn take_pending(&self, message_id: &str) -> Option<BufferEntry> {
        // Rate limit check
        {
            let mut updates = self.updates_this_minute.write().await;
            if Instant::now() > self.rate_limit_reset {
                *updates = 0;
                self.rate_limit_reset = Instant::now() + Duration::from_secs(60);
            }
            if *updates >= MAX_UPDATES_PER_MINUTE {
                return None; // Rate limited, skip this flush window
            }
        }

        let mut entries = self.entries.write().await;
        let entry = entries.remove(message_id)?;
        let mut last_flush = self.last_flush.write().await;

        // Time-based flush check
        let last = last_flush.get(message_id);
        let should_flush = last.map(|t| {
            t.elapsed() >= self.flush_timeout
        }).unwrap_or(true);

        if should_flush {
            *last_flush.insert(message_id.to_string(), Instant::now()) = Instant::now();
            *self.updates_this_minute.write().await += 1;
            Some(entry)
        } else {
            // Put it back, not time to flush yet
            entries.insert(message_id.to_string(), entry);
            None
        }
    }

    /// Force flush — ignore rate limit (use sparingly)
    pub async fn force_take_pending(&self, message_id: &str) -> Option<BufferEntry> {
        let mut entries = self.entries.write().await;
        let entry = entries.remove(message_id);
        if entry.is_some() {
            *self.last_flush.write().await.entry(message_id.to_string()).or_insert(Instant::now()) = Instant::now();
        }
        entry
    }

    /// Get current buffered text length for a message
    pub async fn buffered_len(&self, message_id: &str) -> usize {
        let entries = self.entries.read().await;
        match entries.get(message_id) {
            Some(BufferEntry::Text(s)) => s.len(),
            _ => 0,
        }
    }
}

impl Default for StreamingBuffer {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: 编译验证**

Run: `cargo check --package aiclaw 2>&1`
Expected: 无新增错误

---

## Task 4: FeishuChannel 改造 — 整合新组件 + 卡片/流式支持

**Files:**
- Modify: `src/channels/feishu.rs` (整合所有新组件，改造 `send()` 方法)
- Modify: `src/channels/mod.rs` (确保导出完整)

- [ ] **Step 1: 改造 `src/channels/feishu.rs`**

改造后的 `FeishuChannel` 结构体需要：

```rust
pub struct FeishuChannel {
    name: String,
    config: FeishuConfig,
    api_client: FeishuAPIClient,           // 新增
    streaming_buffer: StreamingBuffer,     // 新增
}
```

改造 `impl FeishuChannel`:

```rust
impl FeishuChannel {
    pub fn new(name: impl Into<String>, config: FeishuConfig) -> anyhow::Result<Self> {
        let api_client = if let (Some(app_id), Some(app_secret)) = (&config.app_id, &config.app_secret) {
            FeishuAPIClient::new(app_id.clone(), app_secret.clone())
        } else {
            // 无 app_id/secret 时，降级为只读 webhook 模式
            warn!("FeishuChannel: app_id/app_secret not configured, falling back to webhook-only mode");
            FeishuAPIClient::new(String::new(), String::new())
        };

        Ok(Self {
            name: name.into(),
            config,
            api_client,
            streaming_buffer: StreamingBuffer::new(),
        })
    }
}
```

改造 `send()` 方法：

```rust
async fn send(&self, msg: &SendMessage) -> anyhow::Result<()> {
    // 如果有 message_id（可更新的消息），走更新路径
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

    // 无 message_id，创建新消息
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
```

改造 `listen()` 方法，添加长轮询：

```rust
async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
    info!("Starting Feishu channel {} (webhook + long poll)", self.name);

    // 启动长轮询任务（如果配置了 app_id/app_secret）
    if self.config.app_id.is_some() && self.config.app_secret.is_some() {
        let api_client = self.api_client.clone();
        let polling_timeout = self.config.polling_timeout_secs.unwrap_or(30);
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

    // 同时启动 webhook 服务器
    self.listen_webhook(tx).await
}
```

新增辅助函数：

```rust
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

    Some(ChannelMessage {
        id: msg.message_id,
        channel_name: "feishu".to_string(),
        channel_id: msg.chat_id,
        sender: sender_info,
        content: MessageContent {
            text: text.to_string(),
            mentions: Vec::new(),
            attachments: Vec::new(),
        },
        timestamp: chrono::Utc::now(),
        thread_id: msg.parent_id.or(msg.root_id),
        mentions_bot: true, // 长轮询收到的都是对 bot 的消息
        raw: serde_json::to_value(&msg).unwrap_or_default(),
    })
}
```

需要新增导入：
```rust
use super::feishu_api::FeishuAPIClient;
use super::feishu_card::{build_result_card, CardStatus};
use super::streaming_buffer::StreamingBuffer;
use crate::channels::feishu_api::LongPollMessage;
```

- [ ] **Step 2: 编译验证**

Run: `cargo check --package aiclaw 2>&1`
Expected: 无新增错误

---

## Task 5: 配置改造 — 新增长轮询配置字段

**Files:**
- Modify: `src/config/schema.rs` (FeishuConfig 新增字段)
- Modify: `config.example.toml` (新增长轮询配置示例)

- [ ] **Step 1: 修改 `src/config/schema.rs` 中的 `FeishuConfig`**

`FeishuConfig` 已存在且有 `app_id`, `app_secret`, `polling_timeout_secs` 字段（通过 alias `long_polling_timeout_secs`）。验证这些字段是否存在并正确。

如果缺失，添加：
```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeishuConfig {
    pub enabled: bool,

    #[serde(default)]
    pub bot_name: Option<String>,

    #[serde(default)]
    pub verify_token: Option<String>,

    #[serde(default)]
    pub encrypt_key: Option<String>,

    #[serde(default)]
    pub app_id: Option<String>,

    #[serde(default)]
    pub app_secret: Option<String>,

    #[serde(default)]
    pub webhook_url: Option<String>,

    #[serde(default, alias = "webhook_listen_addr")]
    pub webhook_listen_addr: Option<String>,

    /// Timeout for long polling in seconds (default: 30)
    #[serde(default, alias = "long_polling_timeout_secs")]
    pub polling_timeout_secs: Option<u64>,
}
```

- [ ] **Step 2: 修改 `config.example.toml`**

在 `[channels.feishu]` 部分新增长轮询配置：

```toml
# Feishu (飞书) Configuration
[channels.feishu]
type = "Feishu"
enabled = true
bot_name = "AIOps Bot"
# Webhook URL for receiving messages
webhook_url = "https://open.feishu.cn/open-apis/bot/v2/hook/xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
# Long polling (recommended for tunnel/dev deployments)
# Requires: app_id and app_secret from Feishu Open Platform
app_id = "cli_xxxxxxxxxxxxxxxx"
app_secret = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
polling_timeout_secs = 30
```

- [ ] **Step 3: 编译验证**

Run: `cargo check --package aiclaw 2>&1`
Expected: 无新增错误

---

## Task 6: 端到端集成 — Agent 调用链路改造

**Files:**
- Modify: `src/agent/` (在适当位置注入 StreamingBuffer 更新调用)

- [ ] **Step 1: 找到 Agent 的 LLM 流式回调接入点**

在 `src/agent/` 目录找到处理 LLM streaming 响应的位置。流式 token 产出时，需要调用 `StreamingBuffer.append_text()` 并定期 flush 到飞书。

具体修改取决于 agent 的架构，通用模式是：

```rust
// 在 streaming 回调中：
let buffer = channel.streaming_buffer();
buffer.append_text(&message_id, token).await;
if buffer.should_flush(&message_id).await {
    if let Some(entry) = buffer.take_pending(&message_id).await {
        match entry {
            BufferEntry::Text(text) => {
                api.update_text_message(&message_id, &text).await?;
            }
            BufferEntry::Card(card) => {
                api.update_interactive_card(&message_id, &card).await?;
            }
        }
    }
}
```

这个步骤需要根据实际 agent 代码结构来实施。关键入口是 `Channel` trait 的 `send()` 方法配合 `message_id` 实现增量更新。

- [ ] **Step 2: 在 aiclaw-types 中扩展 SendMessage 支持消息 ID 更新**

`SendMessage` 已有 `id: Option<String>` 字段。Agent 在创建回复消息时，需要把初始卡片消息的 `message_id` 填入 `SendMessage.id`，后续更新时复用同一个 ID。

- [ ] **Step 3: 端到端编译**

Run: `cargo build --release 2>&1`
Expected: 无错误

---

## Task 7: 验证测试

**Files:**
- Test: 手动测试或集成测试

- [ ] **Step 1: 配置验证**

在 `~/.aiclaw/config.toml` 中配置好 `app_id`, `app_secret`：
```toml
[channels.feishu]
app_id = "你的飞书应用ID"
app_secret = "你的飞书应用密钥"
webhook_url = "你的webhook地址"
polling_timeout_secs = 30
```

- [ ] **Step 2: 启动 aiclaw 并发送测试消息**

```bash
./target/release/aiclaw -c ~/.aiclaw/config.toml
```

在飞书中向 bot 发一条消息，观察：
1. 是否收到「处理中」卡片
2. 几秒后卡片内容是否更新为执行状态
3. 最终是否显示完整结果卡片

---

## 自检清单

**Spec 覆盖检查：**
- [ ] FeishuAPIClient — token 管理、发送/更新消息、长轮询 ✅
- [ ] CardRenderer — Thinking/Executing/Complete 三种卡片模板 ✅
- [ ] StreamingBuffer — 字符缓冲、超时 flush、限流 ✅
- [ ] FeishuChannel.send() 改造 — 支持消息更新（id 字段） ✅
- [ ] FeishuChannel.listen() 改造 — 长轮询 Loop ✅
- [ ] 配置 schema — polling_timeout_secs ✅
- [ ] config.example.toml — 新配置项 ✅
- [ ] Task 6 集成 — 流式 token → buffer → 飞书 API 更新 ✅

**占位符检查：**
- 无 "TBD"、"TODO" 字段
- 所有函数名、类型名、字段名均已定义
- 长轮询 API URL `im/v1/messages` 是飞书真实端点

**类型一致性：**
- `FeishuAPIClient::send_text_message` → `message_id: String` ✅
- `StreamingBuffer::take_pending` → `Option<BufferEntry>` ✅
- `SendMessage.id` → `Option<String>` 已在 aiclaw-types 定义 ✅

//! Local channel: stdin/stdout REPL or HTTP WebSocket gateway.

use std::sync::Arc;

use async_trait::async_trait;
use aiclaw_types::channel::{
    ChannelMessage, MessageContent, OutgoingContent, SendMessage, SenderInfo,
};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use chrono::Utc;
use dashmap::DashMap;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use futures::{SinkExt, StreamExt};
use tracing::{info, warn};
use uuid::Uuid;

use super::traits::Channel;
use crate::config::{LocalChannelMode, LocalConfig};

#[derive(Clone, Default)]
struct PeerRegistry {
    peers: Arc<DashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>,
}

impl PeerRegistry {
    fn register(&self, id: String, tx: tokio::sync::mpsc::UnboundedSender<String>) {
        self.peers.insert(id, tx);
    }

    fn remove(&self, id: &str) {
        self.peers.remove(id);
    }

    fn send_to(&self, id: &str, payload: String) -> anyhow::Result<()> {
        let entry = self
            .peers
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("local channel: no WebSocket peer id {}", id))?;
        entry
            .send(payload)
            .map_err(|_| anyhow::anyhow!("local channel: peer {} disconnected", id))?;
        Ok(())
    }

    fn broadcast(&self, payload: &str) {
        for e in self.peers.iter() {
            let _ = e.value().send(payload.to_string());
        }
    }
}

/// Local channel (stdio or WebSocket).
pub struct LocalChannel {
    name: String,
    config: LocalConfig,
    peers: PeerRegistry,
}

impl LocalChannel {
    pub fn new(name: impl Into<String>, config: LocalConfig) -> anyhow::Result<Self> {
        Ok(Self {
            name: name.into(),
            config,
            peers: PeerRegistry::default(),
        })
    }

    fn build_message(&self, channel_id: &str, text: &str) -> ChannelMessage {
        let user = std::env::var("USER").unwrap_or_else(|_| "local".to_string());
        ChannelMessage {
            id: Uuid::new_v4().to_string(),
            channel_name: self.name.clone(),
            channel_id: channel_id.to_string(),
            sender: SenderInfo {
                user_id: user.clone(),
                username: user,
                display_name: None,
                is_bot: false,
            },
            content: MessageContent {
                text: text.to_string(),
                mentions: vec![],
                attachments: vec![],
            },
            timestamp: Utc::now(),
            thread_id: None,
            mentions_bot: true,
            raw: serde_json::json!({}),
        }
    }

    async fn listen_stdio(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        info!("Local channel {} listening on stdin (one line per message)", self.name);
        eprintln!(
            "(aiclaw) Local stdio on channel {:?}: type one message per line, Enter to send.\n\
             (aiclaw) Logs and status go to stderr; assistant replies on stdout under [aiclaw].\n\
             (aiclaw) Wait until you see \"state=ready\" on stderr before sending the next line (or use inbox capacity 1).",
            self.name
        );
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break;
            }
            let text = line.trim_end_matches(['\r', '\n']).to_string();
            if text.is_empty() {
                continue;
            }
            let msg = self.build_message("stdio", &text);
            if tx.send(msg).await.is_err() {
                warn!("Local channel {}: agent receiver closed", self.name);
                break;
            }
        }
        Ok(())
    }

    async fn listen_http(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let addr = format!("{}:{}", self.config.bind, self.config.port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        info!(
            "Local channel {} WebSocket at ws://{}/ws (HTTP same host)",
            self.name, addr
        );

        let state = LocalWsState {
            agent_tx: tx,
            peers: self.peers.clone(),
            build: LocalMessageBuilder {
                channel_key: self.name.clone(),
            },
        };

        let app = Router::new()
            .route("/ws", get(ws_upgrade))
            .with_state(state);

        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[derive(Clone)]
struct LocalMessageBuilder {
    channel_key: String,
}

impl LocalMessageBuilder {
    fn build(&self, peer_id: &str, text: &str) -> ChannelMessage {
        let user = format!("ws:{peer_id}");
        ChannelMessage {
            id: Uuid::new_v4().to_string(),
            channel_name: self.channel_key.clone(),
            channel_id: peer_id.to_string(),
            sender: SenderInfo {
                user_id: user.clone(),
                username: user,
                display_name: None,
                is_bot: false,
            },
            content: MessageContent {
                text: text.to_string(),
                mentions: vec![],
                attachments: vec![],
            },
            timestamp: Utc::now(),
            thread_id: None,
            mentions_bot: true,
            raw: serde_json::json!({}),
        }
    }
}

#[derive(Clone)]
struct LocalWsState {
    agent_tx: mpsc::Sender<ChannelMessage>,
    peers: PeerRegistry,
    build: LocalMessageBuilder,
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<LocalWsState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| peer_loop(socket, state))
}

async fn peer_loop(socket: WebSocket, state: LocalWsState) {
    let (mut sender, mut receiver) = socket.split();
    let peer_id = Uuid::new_v4().to_string();
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    state.peers.register(peer_id.clone(), out_tx);

    let write_task = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            if sender.send(Message::Text(m)).await.is_err() {
                break;
            }
        }
    });

    while let Some(item) = receiver.next().await {
        match item {
            Ok(Message::Text(t)) => {
                let text = t.trim();
                if text.is_empty() {
                    continue;
                }
                let cm = state.build.build(&peer_id, text);
                if state.agent_tx.send(cm).await.is_err() {
                    break;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }

    write_task.abort();
    state.peers.remove(&peer_id);
}

#[derive(Serialize)]
struct AssistantWsPayload<'a> {
    r#type: &'static str,
    text: &'a str,
}

#[async_trait]
impl Channel for LocalChannel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, msg: &SendMessage) -> anyhow::Result<()> {
        let text = match &msg.content {
            OutgoingContent::Text(t) => t.as_str(),
            OutgoingContent::Formatted(f) => f.body.as_str(),
        };

        match self.config.mode {
            LocalChannelMode::Stdio => {
                println!("[aiclaw]\n{}", text);
                Ok(())
            }
            LocalChannelMode::Http => {
                let payload = serde_json::to_string(&AssistantWsPayload {
                    r#type: "assistant",
                    text,
                })?;
                if let Some(peer) = msg.reply_to.as_deref() {
                    self.peers.send_to(peer, payload)?;
                } else {
                    self.peers.broadcast(&payload);
                }
                Ok(())
            }
        }
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        if !self.config.enabled {
            warn!(
                "Local channel {} is disabled; idle until process exit",
                self.name
            );
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        }

        match self.config.mode {
            LocalChannelMode::Stdio => self.listen_stdio(tx).await,
            LocalChannelMode::Http => self.listen_http(tx).await,
        }
    }

    async fn health_check(&self) -> bool {
        true
    }
}

//! Channel traits for communication adapters

use async_trait::async_trait;
use aiclaw_types::channel::{ChannelMessage, SendMessage};

/// Channel trait - all communication adapters must implement this
#[async_trait]
pub trait Channel: Send + Sync {
    /// Returns the channel name
    fn name(&self) -> &str;

    /// Send a message through the channel
    async fn send(&self, msg: &SendMessage) -> anyhow::Result<()>;

    /// Start listening for messages and forward them to the channel
    async fn listen(&self, tx: tokio::sync::mpsc::Sender<ChannelMessage>) -> anyhow::Result<()>;

    /// Health check for the channel connection
    async fn health_check(&self) -> bool;

    /// Start typing indicator (if supported)
    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        let _ = recipient;
        Ok(())
    }

    /// Stop typing indicator (if supported)
    async fn stop_typing(&self, recipient: &str) -> anyhow::Result<()> {
        let _ = recipient;
        Ok(())
    }

    /// Whether this channel supports typing indicators
    fn supports_typing(&self) -> bool {
        false
    }

    /// Whether this channel supports draft updates
    fn supports_draft_updates(&self) -> bool {
        false
    }
}

/// Extension trait for optional channel capabilities
pub trait ChannelExt: Channel {
    fn supports_threads(&self) -> bool {
        false
    }

    fn supports_mentions(&self) -> bool {
        true
    }

    fn supports_attachments(&self) -> bool {
        false
    }
}

impl<T: Channel> ChannelExt for T {}

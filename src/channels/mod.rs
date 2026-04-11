//! Channels module - Communication adapters

pub mod traits;
pub mod feishu;
pub mod local;
pub mod wecom;

pub use traits::*;
pub use feishu::*;
pub use local::*;
pub use wecom::*;

use crate::config::{ChannelConfig, Config};
use std::collections::HashMap;

/// Factory for creating channel instances
pub struct ChannelFactory;

impl ChannelFactory {
    pub fn create_channel(
        name: &str,
        config: &ChannelConfig,
    ) -> anyhow::Result<Box<dyn Channel>> {
        match config {
            ChannelConfig::Feishu(cfg) => {
                let channel = FeishuChannel::new(name, cfg.clone())?;
                Ok(Box::new(channel))
            }
            ChannelConfig::WeCom(cfg) => {
                let channel = WeComChannel::new(name, cfg.clone())?;
                Ok(Box::new(channel))
            }
            ChannelConfig::Local(cfg) => {
                let channel = LocalChannel::new(name, cfg.clone())?;
                Ok(Box::new(channel))
            }
        }
    }

    pub fn create_channels(config: &Config) -> anyhow::Result<HashMap<String, Box<dyn Channel>>> {
        let mut channels = HashMap::new();
        for (name, channel_config) in &config.channels {
            let channel = Self::create_channel(name, channel_config)?;
            channels.insert(name.to_string(), channel);
        }
        Ok(channels)
    }
}

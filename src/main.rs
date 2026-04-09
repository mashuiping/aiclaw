//! AIClaw - AI Ops Agent
//!
//! Main entry point for the AI operations agent.

use aiclaw::{
    AgentOrchestrator, AIOpsProviderFactory, ChannelFactory, Config,
    K8sClientFactory, MCPClient, MCPClientPool, ObservabilityConfig, Observer,
    SkillLoader, SkillRegistry, SessionManager,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!(
        r#"
   _    ____   ____ ___ ___      _ _____ ____  
  / \  |  _ \ / ___|_ _|_ _|    / |_   _|  _ \ 
 / _ \ | |_) | |    | | | |_____| | | | | |_) |
/ ___ \|  __/| |___ | | | |_____| | | | |  _ < 
/_/   \_\_|    \____|___| |_|     |_| |_|_| \_\
                                                 
AI Ops Agent v{}
"#,
        env!("CARGO_PKG_VERSION")
    );

    init_logging()?;

    info!("Starting AIClaw AI Ops Agent");

    let config = Config::load_or_default(None)?;

    info!("Configuration loaded");

    let observer: Arc<dyn Observer> = Arc::new(aiclaw::LogObserver::new("aiclaw"));

    let skill_loader = SkillLoader::new(&config.skills.skills_dir);
    let skill_registry = Arc::new(SkillRegistry::new());

    match skill_loader.load_skills() {
        Ok(skills) => {
            for skill in skills {
                skill_registry.register(skill);
            }
            info!("Loaded {} skills", skill_registry.len());
        }
        Err(e) => {
            error!("Failed to load skills: {}", e);
        }
    }

    let session_manager = Arc::new(SessionManager::new(config.agent.session_timeout_secs));

    let mcp_pool = Arc::new(MCPClientPool::new());
    for (name, server_config) in &config.mcp.servers {
        if server_config.enabled {
            let client = Arc::new(MCPClient::new(name));
            if let Err(e) = client
                .start_stdio(&server_config.command, &server_config.args, &server_config.env)
                .await
            {
                error!("Failed to start MCP server {}: {}", name, e);
            } else {
                mcp_pool.add(name.clone(), client);
                info!("Started MCP server: {}", name);
            }
        }
    }

    let aiops_providers = AIOpsProviderFactory::create_all(&config.aiops)?;
    info!("Initialized {} AI/OPS providers", aiops_providers.len());

    let k8s_clients = K8sClientFactory::create_all(&config.kubernetes)?;
    info!("Initialized {} K8s clients", k8s_clients.len());

    let channels = ChannelFactory::create_channels(&config)?;
    info!("Initialized {} channels", channels.len());

    let (tx, rx) = mpsc::channel::<aiclaw_types::channel::ChannelMessage>(100);

    let orchestrator = Arc::new(AgentOrchestrator::new(
        &config.agent.name,
        session_manager,
        skill_registry,
        mcp_pool,
        aiops_providers,
        k8s_clients,
        channels,
        observer,
    ));

    let orchestrator_clone = orchestrator.clone();
    tokio::spawn(async move {
        orchestrator_clone.start(rx).await;
    });

    for (name, channel) in &orchestrator.channels {
        let tx_clone = tx.clone();
        let name_clone = name.clone();
        tokio::spawn(async move {
            match channel.listen(tx_clone).await {
                Ok(()) => info!("Channel {} listener started", name_clone),
                Err(e) => error!("Channel {} listener error: {}", name_clone, e),
            }
        });
    }

    info!("AIClaw is running. Press Ctrl+C to stop.");

    tokio::signal::ctrl_c().await?;

    info!("Shutting down AIClaw...");
    Ok(())
}

fn init_logging() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer().with_target(true).with_thread_ids(true))
        .with(filter)
        .init();

    Ok(())
}

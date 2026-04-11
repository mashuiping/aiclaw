//! AIClaw - AI Ops Agent
//!
//! Main entry point. Supports two runtime modes:
//! - Interactive REPL (terminal)
//! - Service mode (Feishu / WeCom / WebSocket channels)

use aiclaw::{
    channels::Channel, default_chat_provider, AgentOrchestrator, AIOpsProviderFactory,
    ChannelFactory, Config, MCPClient, MCPClientPool, Observer, SkillLoader,
    SkillRegistry, SessionManager,
};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// AIClaw - AI Ops Agent
#[derive(Parser, Debug)]
#[command(name = "aiclaw", version, about = "AI Ops Agent for Kubernetes diagnostics")]
struct Cli {
    /// Force interactive REPL mode
    #[arg(short, long)]
    interactive: bool,

    /// Path to config file (overrides AICLAW_CONFIG env)
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Override default LLM model
    #[arg(short, long)]
    model: Option<String>,

    /// Path to kubeconfig (overrides AICLAW_KUBECONFIG env)
    #[arg(short, long, value_name = "FILE")]
    kubeconfig: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    eprintln!(
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

    // --- Config ---
    let config_path = cli
        .config
        .clone()
        .filter(|p| p.exists())
        .or_else(|| {
            std::env::var_os("AICLAW_CONFIG")
                .map(PathBuf::from)
                .filter(|p| p.exists())
        })
        .or_else(|| dirs::home_dir().map(|h| h.join(".aiclaw").join("config.toml")))
        .filter(|p| p.exists());

    let mut config = match config_path {
        Some(ref p) => Config::load(p)?,
        None => Config::load_or_default(None)?,
    };
    info!("Configuration loaded (file={:?})", config_path);

    // --- Model override ---
    if let Some(ref model) = cli.model {
        if let Some((_name, provider_cfg)) = config.llm.providers.iter_mut().next() {
            provider_cfg.model = model.clone();
        }
    }

    // --- Kubeconfig ---
    let kubeconfig = cli
        .kubeconfig
        .clone()
        .or_else(aiclaw::skills::kubeconfig_from_aiclaw_env);
    if kubeconfig.is_some() {
        info!("Kubeconfig is set; skill shells will use it for cluster access");
    }

    // --- Skills ---
    let skill_loader = SkillLoader::new(&config.skills.skills_dir);
    let skill_registry = Arc::new(SkillRegistry::new());
    match skill_loader.load_skills() {
        Ok(skills) => {
            for skill in skills {
                skill_registry.register(skill);
            }
            info!("Loaded {} skills", skill_registry.len());
        }
        Err(e) => error!("Failed to load skills: {}", e),
    }

    // --- MCP ---
    let mut mcp_pool = MCPClientPool::new();
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
    let mcp_pool = Arc::new(mcp_pool);

    // --- LLM provider ---
    let llm_provider = match default_chat_provider(&config) {
        Ok(p) => p,
        Err(e) => {
            warn!("LLM not available (continuing without LLM): {}", e);
            None
        }
    };

    // --- Decide runtime mode ---
    let should_interactive = cli.interactive || is_interactive_default(&config);

    if should_interactive {
        info!("Starting in interactive REPL mode");
        aiclaw::repl::run_repl(
            &config,
            llm_provider,
            skill_registry,
            mcp_pool,
            kubeconfig,
        )
        .await
    } else {
        info!("Starting in service mode");
        run_service(config, llm_provider, skill_registry, mcp_pool, kubeconfig).await
    }
}

/// Returns true if the config implies interactive mode (no channels or only local stdio).
fn is_interactive_default(config: &Config) -> bool {
    if config.channels.is_empty() {
        return true;
    }
    let enabled: Vec<_> = config
        .channels
        .values()
        .filter(|c| match c {
            aiclaw::config::ChannelConfig::Feishu(f) => f.enabled,
            aiclaw::config::ChannelConfig::WeCom(w) => w.enabled,
            aiclaw::config::ChannelConfig::Local(l) => l.enabled,
        })
        .collect();
    if enabled.is_empty() {
        return true;
    }
    if enabled.len() == 1 {
        if let aiclaw::config::ChannelConfig::Local(l) = enabled[0] {
            return l.mode == aiclaw::config::LocalChannelMode::Stdio;
        }
    }
    false
}

/// Run aiclaw in service mode (original channel-based architecture).
async fn run_service(
    config: Config,
    llm_provider: Option<Arc<dyn aiclaw::LLMProvider>>,
    skill_registry: Arc<SkillRegistry>,
    mcp_pool: Arc<MCPClientPool>,
    kubeconfig: Option<PathBuf>,
) -> anyhow::Result<()> {
    let observer: Arc<dyn Observer> = Arc::new(aiclaw::LogObserver::new("aiclaw"));
    let session_manager = Arc::new(SessionManager::new(config.agent.session_timeout_secs));
    let aiops_providers = AIOpsProviderFactory::create_all(&config.aiops)?;
    info!("Initialized {} AI/OPS providers", aiops_providers.len());

    let clusters = config.clusters.clone();
    info!(
        "Loaded {} cluster context entries",
        clusters.len()
    );

    let channels: HashMap<String, Arc<dyn Channel>> = ChannelFactory::create_channels(&config)?
        .into_iter()
        .map(|(name, ch)| (name, Arc::from(ch)))
        .collect();
    info!("Initialized {} channels", channels.len());

    let inbox_cap = config.agent_message_channel_capacity();
    if inbox_cap == 1 {
        info!("Agent inbox capacity=1 (local stdio only); next line waits until the current reply is sent");
    }
    let (tx, rx) = mpsc::channel::<aiclaw_types::channel::ChannelMessage>(inbox_cap);

    let orchestrator = if llm_provider.is_some() {
        Arc::new(AgentOrchestrator::with_llm(
            &config.agent.name,
            session_manager.clone(),
            skill_registry.clone(),
            mcp_pool.clone(),
            aiops_providers,
            clusters,
            channels,
            observer.clone(),
            llm_provider,
            config.skills.exec.clone(),
            kubeconfig,
            config.agent.default_cluster.clone(),
        ))
    } else {
        Arc::new(AgentOrchestrator::new(
            &config.agent.name,
            session_manager,
            skill_registry,
            mcp_pool,
            aiops_providers,
            clusters,
            channels,
            observer,
            kubeconfig,
        ))
    };

    let orchestrator_clone = orchestrator.clone();
    tokio::spawn(async move {
        orchestrator_clone.start(rx).await;
    });

    for (name, channel) in &orchestrator.channels {
        let tx_clone = tx.clone();
        let name_clone = name.clone();
        let channel = channel.clone();
        tokio::spawn(async move {
            match channel.listen(tx_clone).await {
                Ok(()) => info!("Channel {} listener started", name_clone),
                Err(e) => error!("Channel {} listener error: {}", name_clone, e),
            }
        });
    }

    info!("AIClaw is running in service mode. Press Ctrl+C to stop.");
    tokio::signal::ctrl_c().await?;
    info!("Shutting down AIClaw...");
    Ok(())
}

fn init_logging() -> anyhow::Result<()> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_target(true)
                .with_thread_ids(true)
                .with_writer(std::io::stderr),
        )
        .with(filter)
        .init();

    Ok(())
}

# AIClaw

<p align="center">
  <img src="assets/mascot.png" alt="" width="280">
</p>

A Rust AI operations agent: Feishu and WeCom in front, filesystem Skills and MCP alongside, VictoriaMetrics and Kubernetes when you need to see what happened.

It loads skills from disk, integrates Model Context Protocol servers (for example VictoriaMetrics MCP), pulls metrics and logs from VictoriaMetrics or Prometheus, inspects cluster state and pod output, and ships with OpenTelemetry tracing for the agent itself.

## Goals

- **Chat-first ops**: Talk to the agent from enterprise messaging bots instead of context-switching to a dozen consoles.
- **Skills you own**: Drop-in skills under `~/.aiclaw/skills/` with a small declarative contract.
- **MCP where it fits**: Reuse observability and tooling through MCP instead of bespoke glue for every backend.
- **Observable agent**: Trace the orchestrator with OpenTelemetry, not only the workloads it watches.

## Features

- **Channels**: Feishu (飞书) and WeCom (企业微信)
- **Skill system**: Load and execute skills from the filesystem
- **MCP client**: Call MCP servers (e.g. VictoriaMetrics MCP)
- **Observability data**: Metrics and logs via VictoriaMetrics / Prometheus
- **Kubernetes**: Cluster state, pod logs, events

## Architecture

```
┌─────────────┐     ┌─────────────┐
│   Feishu    │     │   WeCom     │
└──────┬──────┘     └──────┬──────┘
       │                   │
       └─────────┬─────────┘
                 ▼
         ┌───────────────┐
         │   Channel     │
         │   Adapter     │
         └───────┬───────┘
                 ▼
         ┌───────────────┐
         │    Agent      │
         │ Orchestrator  │
         └───────┬───────┘
                 │
    ┌────────────┼────────────┐
    ▼            ▼            ▼
┌───────┐  ┌─────────┐  ┌─────────┐
│ Skill │  │   MCP   │  │ Intent  │
│Router │  │ Client  │  │ Parser  │
└───┬───┘  └────┬────┘  └─────────┘
    │           │
    ▼           ▼
┌───────────────────────────────┐
│         Integrations         │
├───────────────────────────────┤
│  K8s Client │ AIOps Provider │
│  (kubectl)  │  (Victoria)    │
└───────────────────────────────┘
```

## Quick start

### Prerequisites

- Rust 1.87+
- Kubernetes cluster (optional)
- VictoriaMetrics (optional)
- Feishu/WeCom bot token

### Installation

```bash
# Clone the repository
git clone https://github.com/mashuiping/aiclaw.git
cd aiclaw

# Build
cargo build --release

# Copy configuration
mkdir -p ~/.aiclaw
cp config.example.toml ~/.aiclaw/config.toml

# Edit configuration
vim ~/.aiclaw/config.toml
```

### Configuration

Edit `~/.aiclaw/config.toml`:

```toml
[agent]
name = "aiclaw"

[channels.feishu]
enabled = true
webhook_url = "https://..."

[aiops.victoria]
enabled = true
endpoint = "http://victoriametrics:8428"

[kubernetes.prod]
enabled = true
kubeconfig_path = "~/.kube/config"
```

### Running

```bash
# Run with default config
cargo run

# Run with custom config path
AICLAW_CONFIG=/path/to/config.toml cargo run
```

## Skills

Skills are defined in `~/.aiclaw/skills/` directory. Each skill has a `SKILL.toml` file:

```toml
name = "k8s-log-reader"
description = "Read K8s pod logs"
tags = ["kubernetes", "logs"]

[[tools]]
name = "kubectl_logs"
kind = "shell"
command = "kubectl"
args = { namespace = "{{namespace}}", pod = "{{pod_name}}" }
```

## Usage

Once running, interact with the agent via your messaging platform:

```
@AIOps ai 网关突发 50x，排查下原因
@AIOps 集群 xxx，pod xxx 一直 pending，排查下原因
@AIOps 推理服务 OOM，分析一下可能原因
```

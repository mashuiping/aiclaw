# AIClaw - AI Ops Agent

<p align="center">
  <img src="assets/mascot.png" alt="AIClaw 吉祥物：穿龙虾装的小仓鼠" width="280">
</p>
<p align="center"><em>本项目吉祥物 —— 外壳是龙虾的硬，内心是仓鼠的软；查指标撸 K8s 时一样凶萌。</em></p>

A Rust-based AI operations agent that connects to messaging platforms (Feishu, WeCom), loads Skills, queries observability data (VictoriaMetrics, Prometheus), and troubleshoots Kubernetes clusters.

## Features

- **Multi-Channel Support**: Connect to Feishu (飞书) and WeCom (企业微信)
- **Skill System**: Load and execute custom skills from filesystem
- **MCP Client**: Integrate with Model Context Protocol servers (e.g., VictoriaMetrics MCP)
- **Observability**: Query metrics and logs from VictoriaMetrics/Prometheus
- **Kubernetes Integration**: Query cluster state, pod logs, events
- **Observability**: Full OpenTelemetry tracing support

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

## Getting Started

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
@AIOps 查看 pod nginx-123 的日志
@AIOps 检查集群健康状态
@AIOps 查询 CPU 使用率
```

## License

MIT OR Apache-2.0

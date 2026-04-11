<p align="center">
  <img src="assets/mascot.png" alt="AIClaw" width="240">
</p>

<h1 align="center">AIClaw</h1>

<p align="center">
  <strong>Chat-first AI Ops agent for Kubernetes, observability, and infrastructure diagnostics.</strong>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> ·
  <a href="#features">Features</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#skills">Skills</a> ·
  <a href="#architecture">Architecture</a>
</p>

<p align="center">
  <img alt="Rust 1.87+" src="https://img.shields.io/badge/rust-1.87%2B-orange?logo=rust">
  <img alt="License" src="https://img.shields.io/badge/license-MIT%2FApache--2.0-blue">
</p>

---

AIClaw connects LLMs to your infrastructure through enterprise messaging channels (Feishu / WeCom), a local channel (stdio / WebSocket), or an interactive terminal REPL. It loads filesystem skills, talks to MCP servers, queries VictoriaMetrics or Prometheus, and runs `kubectl` on the agent host — all driven by natural-language conversation.

## Quick Start

**Prerequisites:** Rust 1.87+ · `kubectl` on PATH (optional, for cluster workflows)

```bash
git clone https://github.com/mashuiping/aiclaw.git && cd aiclaw
cargo build --release

mkdir -p ~/.aiclaw
cp config.example.toml ~/.aiclaw/config.toml
# edit LLM keys, channels, skills_dir, clusters, etc.
```

Run the REPL:

```bash
# auto-detected when no remote channels are configured
./target/release/aiclaw

# or force interactive mode
./target/release/aiclaw -i
```

Run as a service (Feishu / WeCom / WebSocket listeners):

```bash
./target/release/aiclaw -c ~/.aiclaw/config.toml
```

## Features

| | |
|---|---|
| **Channels** | Feishu (飞书), WeCom (企业微信), Local (`stdio` / `http` WebSocket gateway) |
| **Interactive REPL** | Streaming replies, slash commands (`/help`, `/skills`, `/status`, `/model`, `/save`, `/resume`, `/thinkback`), tab completion, session save & resume |
| **LLM Providers** | OpenAI, Anthropic, DeepSeek, Qwen, Zhipu, MiniMax — with routing via direct, OpenRouter, or Ollama |
| **Skill System** | Drop-in skills from the filesystem (`SKILL.toml` or `SKILL.md` with YAML frontmatter), with shell / HTTP / script tool types |
| **Exec Loop** | LLM-driven command execution — the model proposes `kubectl` / `helm` commands, validated by configurable security policy, then executed on the host |
| **MCP Client** | Stdio JSON-RPC transport to any MCP server (e.g. VictoriaMetrics MCP) |
| **Observability** | VictoriaMetrics / Prometheus integration for metrics and logs; OpenTelemetry tracing for the agent itself |
| **Cluster Routing** | Named cluster entries with optional `kubectl --context` injection; multi-cluster support |
| **Tool Use** | LLM function calling with `bash`, `read_file`, `list_files` tools in REPL mode |

## CLI

```
aiclaw [OPTIONS]

Options:
  -i, --interactive          Force interactive REPL mode
  -c, --config <FILE>        Path to config file
  -m, --model <MODEL>        Override default LLM model
  -k, --kubeconfig <FILE>    Path to kubeconfig (or set AICLAW_KUBECONFIG)
```

**Default mode:** The binary starts the REPL when there are no channel entries, no enabled channels, or only Local with `mode = "stdio"`. Otherwise it runs in service mode. Use `-i` to always get the REPL.

## Configuration

Config file resolution order:

1. `--config` / `-c` (CLI)
2. `AICLAW_CONFIG` environment variable
3. `$HOME/.aiclaw/config.toml`
4. Built-in defaults

Key sections in `config.example.toml`:

```toml
[agent]
name = "aiclaw"
default_cluster = "prod"

[llm]
enabled = true
default_provider = "openai"

[llm.providers.openai]
enabled = true
api_key = "${OPENAI_API_KEY}"
model = "gpt-4o"

[skills]
skills_dir = "/absolute/path/to/your/skills"

[skills.exec]
enabled = true
security = "allowlist"           # deny | allowlist | full
max_steps = 10
prepend_kubectl_context = true

[clusters.prod]
enabled = true
# context = "my-eks-prod"

[channels.feishu]
enabled = true
# ...

[mcp.servers.victoria]
enabled = true
command = "npx"
args = ["-y", "@anthropic/victoria-mcp"]
```

> **Kubeconfig** is not stored in config.toml. Pass it via `--kubeconfig`, `AICLAW_KUBECONFIG`, or mention a path in a user message.

## Skills

Each skill lives in its own directory under `skills_dir` with a `SKILL.toml` or `SKILL.md` manifest.

**TOML example** (`skills/k8s-log-reader/SKILL.toml`):

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

**Markdown example** (`skills/inf-k8s-hami-gpu-pod/SKILL.md`): YAML frontmatter for metadata, Markdown body for the diagnostic runbook — used by the exec loop to guide LLM-driven `kubectl` commands.

### Bundled Skills

| Skill | Format | Description |
|-------|--------|-------------|
| `k8s-log-reader` | TOML | Read Kubernetes pod logs |
| `k8s-health-check` | TOML | Kubernetes cluster health check |
| `vm-query` | TOML | VictoriaMetrics / Prometheus queries |
| `inf-k8s-hami-gpu-pod` | Markdown | HAMi GPU pod pending troubleshooting runbook |

> To use the bundled skills, set `skills_dir` to this repo's `skills/` directory or symlink them into `~/.aiclaw/skills`.

## Architecture

```
                    ┌──────────┐  ┌──────────┐  ┌──────────┐
                    │  Feishu  │  │  WeCom   │  │  Local   │
                    └────┬─────┘  └────┬─────┘  └────┬─────┘
                         │             │              │
                         └──────┬──────┘──────────────┘
                                ▼
                      ┌───────────────────┐
                      │  Agent            │
                      │  Orchestrator     │
                      │                   │
                      │  Sessions · LLM   │
                      │  Intent · Routing │
                      └────┬────┬────┬────┘
                           │    │    │
              ┌────────────┘    │    └────────────┐
              ▼                 ▼                  ▼
       ┌────────────┐   ┌────────────┐    ┌────────────┐
       │   Skills   │   │    MCP     │    │   AIOps    │
       │            │   │   Client   │    │  Provider  │
       │ TOML · MD  │   │   Pool     │    │            │
       │ Exec Loop  │   │  (stdio)   │    │ Victoria   │
       └─────┬──────┘   └────────────┘    │ Prometheus │
             │                            └────────────┘
             ▼
       ┌────────────┐
       │  kubectl   │
       │  helm      │
       │  (host)    │
       └────────────┘
```

## Repository Layout

```
├── src/
│   ├── main.rs              # Entry point, CLI parsing
│   ├── lib.rs               # Public API surface
│   ├── agent/               # Orchestrator, sessions
│   ├── channels/            # Feishu, WeCom, Local adapters
│   ├── config/              # Config schema and loading
│   ├── llm/                 # Providers, routing, streaming, intent, summarizer
│   ├── mcp/                 # MCP client (stdio JSON-RPC)
│   ├── repl/                # Interactive REPL, commands, tool use
│   ├── skills/              # Skill loader, registry, executor
│   ├── aiops/               # Victoria / Prometheus provider
│   ├── security/            # Command validation, audit
│   ├── observability/       # Log observer
│   └── utils/               # Shared helpers
├── crates/aiclaw-types/     # Shared types crate
├── skills/                  # Bundled example skills
├── config.example.toml      # Full configuration template
└── Cargo.toml               # Workspace manifest
```

## Usage Examples

Once the agent is running via channels or REPL:

```
> 网关突发 50x，排查下原因
> 集群 prod，pod xxx 一直 pending，帮忙看看
> 推理服务 OOM，分析一下可能原因
> /k8s-log-reader namespace=default pod=my-app
```

## Known Limitations

- **No in-process Kubernetes client** — all cluster access is via host `kubectl` subprocess.
- **`[skills].exec`** requires a working LLM provider, `kubectl` on PATH, and appropriate RBAC. Treat `security = "full"` as development-only.
- MCP and channels require valid credentials and network access; optional components can be disabled for a smaller dev setup.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE).

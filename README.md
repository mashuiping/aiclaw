# AIClaw

<p align="center">
  <img src="assets/mascot.png" alt="" width="280">
</p>

A Rust AI operations agent: Feishu and WeCom in front, filesystem Skills and MCP alongside, VictoriaMetrics and Kubernetes when you need to see what happened.

It loads skills from disk, integrates Model Context Protocol servers (for example VictoriaMetrics MCP), pulls metrics and logs from VictoriaMetrics or Prometheus, inspects cluster state and pod output, and ships with OpenTelemetry tracing for the agent itself.

## Repository layout

- `src/`, `Cargo.toml` — `aiclaw` binary and library (workspace root package).
- `crates/aiclaw-types/` — shared types crate.
- `skills/` — **example / bundled skills** checked into the repo (`SKILL.toml` or `SKILL.md` per directory). The runtime still loads from whatever path you set in config (usually `~/.aiclaw/skills`); symlink or point `skills_dir` here if you want to use these without copying.
- `config.example.toml` — full configuration template.
- `docs/HAMI_PENDING_RUNBOOK.md` — **HAMi GPU Pod 长期 Pending** 的 kubectl 排查指引（与 `skills/inf-k8s-hami-gpu-pod/SKILL.md` 对齐，可不依赖本程序编译）。

## Goals

- **Chat-first ops**: Talk to the agent from enterprise messaging bots instead of context-switching to a dozen consoles.
- **Skills you own**: Drop-in skills under `~/.aiclaw/skills/` (or another directory via `skills_dir` in config) with a small declarative contract (`SKILL.toml` or Markdown + frontmatter `SKILL.md`).
- **MCP where it fits**: Reuse observability and tooling through MCP instead of bespoke glue for every backend.
- **Observable agent**: Trace the orchestrator with OpenTelemetry, not only the workloads it watches.

## Features

- **Channels**: Feishu (飞书) and WeCom (企业微信)
- **Skill system**: Load and execute skills from the filesystem
- **MCP client**: Call MCP servers (e.g. VictoriaMetrics MCP)
- **Observability data**: Metrics and logs via VictoriaMetrics / Prometheus
- **Kubernetes**: Cluster state, pod logs, events (via `kube` client when enabled in config)

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
│  (kube-rs)  │  (Victoria)     │
└───────────────────────────────┘
```

## Quick start

### Prerequisites

- Rust **1.87+** (see workspace `rust-version` in `Cargo.toml`)
- Kubernetes cluster access (optional but needed for cluster workflows)
- VictoriaMetrics or Prometheus (optional)
- Feishu / WeCom bot configuration when using those channels

### Installation

```bash
git clone https://github.com/mashuiping/aiclaw.git
cd aiclaw

cargo build --release

mkdir -p ~/.aiclaw
cp config.example.toml ~/.aiclaw/config.toml
# Edit secrets, channels, clusters, skills_dir, LLM keys, etc.
vim ~/.aiclaw/config.toml
```

### Configuration highlights

- **Config file resolution**: The binary loads configuration in this order:  
  1. Path from environment variable **`AICLAW_CONFIG`** (must exist).  
  2. Otherwise **`$HOME/.aiclaw/config.toml`** if it exists.  
  3. Otherwise built-in defaults (often not enough for production; prefer a real file).
- **`[skills].skills_dir`**: Directory whose **immediate subdirectories** are scanned for skills. To use repo `skills/`, either symlink into `~/.aiclaw/skills` or set `skills_dir` to an **absolute** path to the `skills` folder (leading `~` in TOML paths may not expand everywhere in the stack—prefer absolute paths for kubeconfig and skills when in doubt).
- **`[kubernetes.<name>]`**: Set `enabled`, `context`, `kubeconfig_path`, `default_namespace`, `timeout_secs` per cluster. Keys under `[kubernetes]` in TOML are cluster names (e.g. `prod` in `config.example.toml`).

Example fragment:

```toml
[agent]
name = "aiclaw"
default_cluster = "prod"

[skills]
skills_dir = "/absolute/path/to/your/skills"

[channels.feishu]
enabled = true
# ...

[kubernetes.prod]
enabled = true
context = "prod"
kubeconfig_path = "/Users/you/.kube/config"
default_namespace = "default"
```

### Running

```bash
# Default: $AICLAW_CONFIG if set, else ~/.aiclaw/config.toml if present, else defaults
cargo run

# Explicit config file
AICLAW_CONFIG=/path/to/config.toml cargo run

# Release binary path after `cargo build --release`
./target/release/aiclaw
```

## HAMi GPU Pod pending 排查（不依赖程序是否启动）

若你的目标是 **HAMi 环境下 GPU Pod 一直 Pending**，请直接打开：

- **[docs/HAMI_PENDING_RUNBOOK.md](docs/HAMI_PENDING_RUNBOOK.md)** — 中文步骤清单与前置说明。  
- 技能全文：**[skills/inf-k8s-hami-gpu-pod/SKILL.md](skills/inf-k8s-hami-gpu-pod/SKILL.md)** — 更细的命令、表与排障逻辑。

以上流程以 **`kubectl` 与集群日志** 为主；即便 AIClaw 进程未运行，也可完整执行。

## Skills

Each skill lives in its own directory with **`SKILL.md`** (Markdown + YAML frontmatter) or **`SKILL.toml`**. Example from the repo, `skills/k8s-log-reader/SKILL.toml`:

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

## Usage (channels)

Once the agent is running with at least one channel enabled, interact via your messaging platform, for example:

```
@AIOps ai 网关突发 50x，排查下原因
@AIOps 集群 xxx，pod xxx 一直 pending，排查下原因
@AIOps 推理服务 OOM，分析一下可能原因
```

## Known limitations

- **Build health** (current state): `cargo build` still reports a large number of compiler errors in the `aiclaw` library (agent orchestrator, MCP client, channels, session store, and related modules). Until that is cleaned up, you cannot rely on `cargo run` for production. HAMi pending triage remains fully viable via `kubectl` using **[docs/HAMI_PENDING_RUNBOOK.md](docs/HAMI_PENDING_RUNBOOK.md)** and **[skills/inf-k8s-hami-gpu-pod/SKILL.md](skills/inf-k8s-hami-gpu-pod/SKILL.md)**.
- **MCP and channels** require valid credentials and network access; optional components can be disabled in `config.toml` for a smaller dev setup.

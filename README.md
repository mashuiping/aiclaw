# AIClaw

<p align="center">
  <img src="assets/mascot.png" alt="" width="280">
</p>

A Rust AI operations agent: enterprise channels (Feishu / WeCom) or a **local channel** (stdio / WebSocket gateway), an **interactive terminal REPL**, filesystem Skills and MCP alongside, VictoriaMetrics when you need metrics, and **`kubectl` on the agent host** (via **`[skills].exec`**) when you need cluster state.

It loads skills from disk, integrates Model Context Protocol servers (for example VictoriaMetrics MCP), pulls metrics and logs from VictoriaMetrics or Prometheus, and ships with OpenTelemetry tracing for the agent itself. Cluster inspection for Markdown skills is **subprocess `kubectl`**; there is **no in-process Kubernetes API client**—HAMi-style runbooks rely on host **`kubectl`** plus **`[skills].exec`** (and optional **`[clusters.*]`** for **`--context`**).

## Repository layout

- `src/`, `Cargo.toml` — `aiclaw` binary and library (workspace root package).
- `src/repl/` — interactive REPL (line editor, slash commands, streaming replies).
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

- **Channels**: Feishu (飞书), WeCom (企业微信), and **Local** (`stdio` one-line-per-message or `http` WebSocket gateway; see `config.example.toml` **`[channels.local]`**)
- **Interactive REPL**: Terminal mode with **`/help`**, **`/skills`**, skill shortcuts, session save/resume—pass **`--interactive`** or **`-i`**, or start with no remote channels enabled (see **Running** below)
- **Skill system**: Load and execute skills from the filesystem
- **MCP client**: Call MCP servers (e.g. VictoriaMetrics MCP)
- **Observability data**: Metrics and logs via VictoriaMetrics / Prometheus
- **Cluster workflows**: Host **`kubectl`** (and optional **`helm`**) via **`[skills].exec`**, plus optional **`[clusters.*]`** entries for `kubectl --context` injection and multi-cluster routing

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
│ Skills exec │ AIOps Provider │
│ (kubectl)   │  (Victoria)    │
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
  1. **`--config` / `-c`** if the path exists (CLI).  
  2. Otherwise **`AICLAW_CONFIG`** if set and the path exists.  
  3. Otherwise **`$HOME/.aiclaw/config.toml`** if it exists.  
  4. Otherwise built-in defaults (often not enough for production; prefer a real file).
- **`[skills].skills_dir`**: Directory whose **immediate subdirectories** are scanned for skills. To use repo `skills/`, either symlink into `~/.aiclaw/skills` or set `skills_dir` to an **absolute** path to the `skills` folder (leading `~` in TOML paths may not expand everywhere in the stack—prefer absolute paths when in doubt).
- **`[skills].exec`**: Optional OpenClaw-style loop: when **`enabled`**, a Markdown skill’s **`SKILL.md`** body is fed to the LLM, which proposes shell commands (typically **`kubectl`**) validated by **`security`** (`allowlist` recommended), then executed on the host running `aiclaw`. See **`config.example.toml`** and **[docs/HAMI_PENDING_RUNBOOK.md](docs/HAMI_PENDING_RUNBOOK.md)** §5.
- **`[clusters.<name>]`**: Optional registry of **logical cluster names** for routing and for **`[skills].exec.prepend_kubectl_context`**: each stanza has **`enabled`** and optional **`context`** (the value passed to `kubectl --context`; when omitted, the table key is used). **Kubeconfig file paths are not stored in `config.toml`.** For **`[skills].exec`**, pass kubeconfig as **`--kubeconfig`** / **`AICLAW_KUBECONFIG`** (absolute path recommended), or mention a path in a user message (`AICLAW_KUBECONFIG=...`, `kubeconfig: ...`, or an absolute path that looks like a config file)—session heuristics can pick it up for channel flows. Skill subprocesses do **not** inherit the parent’s **`KUBECONFIG`** environment variable. Legacy **`[kubernetes.<name>]`** TOML tables are still accepted as an alias for **`[clusters.<name>]`**.

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

[clusters.prod]
enabled = true
# context = "my-eks-prod"
```

### Running

```bash
# Config: -c/--config if set, else AICLAW_CONFIG, else ~/.aiclaw/config.toml, else defaults
cargo run

# Force interactive REPL (even when Feishu/WeCom are enabled in config)
cargo run -- --interactive
# short form
cargo run -- -i

# Explicit config file (overrides AICLAW_CONFIG)
cargo run -- -c /path/to/config.toml

# Optional: override default provider model from CLI (-m / --model)
cargo run -- -c /path/to/config.toml --model claude-3-5-sonnet-20241022

# Cluster skills (`[skills].exec`): kubeconfig via env or CLI (not stored in config.toml)
AICLAW_KUBECONFIG=/path/to/kubeconfig cargo run -- -c /path/to/config.toml
cargo run -- -c /path/to/config.toml --kubeconfig /path/to/kubeconfig

# Release binary path after `cargo build --release`
./target/release/aiclaw
```

**Default mode:** If **`--interactive`** is not passed, the binary starts the **REPL** when there are **no channel entries** in config, **no enabled channels**, or the **only** enabled channel is **Local** with **`mode = "stdio"`**. Otherwise it runs **service mode** (channel listeners + orchestrator). Use **`-i`** to always use the REPL locally.

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

- **Planner / some debug paths** may still use placeholder “simulated” cluster responses; Markdown skills with **`[skills].exec`** run real subprocess **`kubectl`** on the agent host.
- **`[skills].exec`** requires a working **`[llm]`** default provider, **`kubectl`** on `PATH`, and appropriate RBAC; treat **`security = "full"`** as development-only.
- **MCP and channels** require valid credentials and network access; optional components can be disabled in `config.toml` for a smaller dev setup.

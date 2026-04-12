# 飞书交互卡片 + 文字流式输出实现方案

## 背景

aiclaw 当前飞书通道仅实现基础的「收到消息 → 回复文字」能力，与飞书官方 openclaw-lark 相比缺少：交互式卡片（实时状态演进）、文字流式输出、消息主动拉取（长轮询）等能力。

本文档描述在单 bot 模式下，通过飞书交互式卡片 + 消息编辑 API 实现完整交互体验的设计方案。

## 目标

- 通过飞书交互式卡片实时展示 agent 处理状态（Thinking → Executing → Complete）
- 通过消息编辑 API 实现文字流式输出效果
- 以长轮询为主要消息接收方式，webhook 作为辅助（解决 tunnel 部署场景下的 webhook URL 不稳定问题）

## 架构概览

```
Feishu Platform
  用户消息 ──→ 飞书 ──→ Webhook
            ←── 长轮询拉取消息
            ←── 更新卡片（交互式状态）
            ←── 编辑文字消息（流式模拟）

aiclaw 服务
  FeishuAPIClient (新增 channels/feishu_api.rs)
  CardRenderer (新增 channels/feishu_card.rs)
  StreamingBuffer (新增 channels/streaming_buffer.rs)
  长轮询 Loop (新增)
  Feishu Channel (改造现有 feishu.rs)
```

## 新增组件

### 1. FeishuAPIClient (`channels/feishu_api.rs`)

封装飞书开放平台所有 API 调用：

- `send_text_message(recipient, content) -> message_id` — 创建 text 消息
- `send_interactive_card(recipient, card) -> message_id` — 创建卡片消息
- `update_message(message_id, content) -> ()` — 更新 text 消息内容
- `update_interactive_card(message_id, card) -> ()` — 更新交互卡片
- `long_poll_messages(timeout_secs) -> Vec<FeishuMessage>` — 长轮询拉取新消息

认证方式：`app_access_token`（通过 `app_id + app_secret` 获取），使用 `reqwest` 发送 HTTP 请求。

### 2. CardRenderer (`channels/feishu_card.rs`)

定义交互卡片模板，支持三种状态演进：

**Thinking 状态：**
```json
{
  "config": { "wide_screen_mode": true },
  "elements": [
    { "tag": "markdown", "content": "**🤖 AIOps Bot** 正在思考..." },
    { "tag": "hr" },
    { "tag": "markdown", "content": "░░░░░░░░░░░░░░░░  思考中" }
  ]
}
```

**Executing 状态：**
```json
{
  "config": { "wide_screen_mode": true },
  "elements": [
    { "tag": "markdown", "content": "**🤖 AIOps Bot** 执行中" },
    { "tag": "hr" },
    { "tag": "markdown", "content": "✓ 查询 pod 状态\n✓ 发现异常: CrashLoopBackOff\n⟳ 分析根因..." },
    { "tag": "hr" },
    { "tag": "markdown", "content": "░░░░░░░░░░░░░░░░  处理中" }
  ]
}
```

**Complete 状态：**
```json
{
  "config": { "wide_screen_mode": true },
  "elements": [
    { "tag": "markdown", "content": "**🤖 AIOps Bot** ✅ 完成" },
    { "tag": "hr" },
    { "tag": "markdown", "content": "问题：Pod xxx 处于 CrashLoop\n原因：OOMKilled，内存限制过低\n建议：调整 memory.limits=512Mi" }
  ]
}
```

### 3. StreamingBuffer (`channels/streaming_buffer.rs`)

- 维护每个 `message_id` 对应的当前文本内容
- 缓冲流式 token：默认累积 50 个字符或 500ms 超时，批量一次调用 `update_message()`
- 防止更新频率超过飞书 API 限制（每分钟 60 次消息更新）
- 超过限流时自动延长 buffer 超时到 1s，合并更新

## 消息生命周期

```
1. 用户在飞书发送消息
2. 长轮询 Loop 检测到新消息（webhook 也可能同时收到，二者按 event_id 去重）
3. 创建「处理中」交互卡片 → 得到 message_id → 发送卡片给用户
4. Agent 开始执行技能/命令
5. LLM streaming token 通过回调写入 StreamingBuffer
6. StreamingBuffer 累积字符/超时 → 调用 FeishuAPI.update_message() 更新卡片内容
7. 执行过程中卡片内容实时刷新（Executing 状态，分段展示进度）
8. 执行完成，最终更新卡片为 Complete 状态
9. 用户看到完整排查结果
```

## 消息接收策略

- **长轮询为主**：持续调用 `im/v1/messages?receive_id_type=open_id`，超时设置 30s
- **Webhook 为辅**：接收飞书事件推送，用于 token 验证和突发事件通知
- **去重**：长轮询和 webhook 通过 `event_id` 做去重，避免同一消息被处理两次

## 错误处理

| 场景 | 处理方式 |
|---|---|
| 飞书 API 调用失败（网络抖动） | 重试 3 次，指数退避 |
| 卡片更新超限（>60次/分） | 合并更新，延长 buffer 超时到 1s |
| 消息已过期（5分钟窗口） | 卡片更新失败时降级为发送新消息 |
| 长轮询超时 | 正常空轮询，重新拉取 |
| Webhook + 长轮询收到同一消息 | 按 `event_id` 去重 |

## 权限需求

飞书开放平台应用后台需开通以下权限：

| 权限名 | 用途 |
|---|---|
| `im:message:send_as_bot` | 发送消息 |
| `im:message` | 读取/发送/更新消息 |
| `im:message.p2p_msg:readonly` | 读取私聊消息 |
| `im:message.group_msg:readonly` | 读取群消息 |
| `im:message.interactive:update` | 更新交互卡片 |
| `im:chat:readonly` | 读取群聊信息 |

## 文件变更清单

| 文件 | 操作 | 说明 |
|---|---|---|
| `src/channels/feishu_api.rs` | 新增 | Feishu API 客户端封装 |
| `src/channels/feishu_card.rs` | 新增 | 交互卡片渲染器 |
| `src/channels/streaming_buffer.rs` | 新增 | 流式输出缓冲区 |
| `src/channels/feishu.rs` | 改造 | 整合新组件，改造消息发送逻辑 |
| `src/channels/mod.rs` | 改造 | 导出新模块 |
| `src/config/schema.rs` | 改造 | 新增长轮询配置项 |
| `config.example.toml` | 改造 | 新增长轮询配置示例 |

## 实现顺序

1. `FeishuAPIClient` — 基础 API 调用封装
2. `CardRenderer` — 卡片模板定义
3. `StreamingBuffer` — 流式缓冲管理
4. 长轮询 Loop — 消息接收改造
5. 卡片发送/更新集成 — 改造 `feishu.rs` 的 `send()` 方法
6. 端到端测试验证

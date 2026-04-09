# AIClaw 演进路线图

本文档定义 AIClaw 从「命令路由器」到「AI 运维专家」的演进计划。

---

## Phase 1: LLM 能力集成（核心 AI 能力）

**目标**：赋予 Agent 真正的语义理解和结果分析能力

### 1.1 LLM 意图理解层 ✅ 已完成

**问题**：当前正则匹配无法理解自然语言

**方案**：
- 新增 `src/llm/` 模块
- 集成 Claude/OpenAI API 做意图识别
- 输入：用户原始消息；输出：结构化 Intent + 置信度

**实现状态**：
```
✅ 新增: src/llm/mod.rs              # 模块入口
✅ 新增: src/llm/types.rs            # 运行时类型 (ChatMessage, ChatResponse)
✅ 新增: src/llm/traits.rs           # Provider trait, IntentClassifier trait
✅ 新增: src/llm/factory.rs          # ProviderFactory, RouterFactory
✅ 新增: src/llm/providers/mod.rs    # Provider 模块
✅ 新增: src/llm/providers/openai.rs # OpenAI Provider
✅ 新增: src/llm/providers/anthropic.rs # Anthropic/Claude Provider
✅ 新增: src/llm/providers/deepseek.rs   # DeepSeek Provider
✅ 新增: src/llm/providers/zhipu.rs    # 智谱 GLM Provider
✅ 新增: src/llm/providers/minimax.rs  # MiniMax Provider
✅ 新增: src/llm/providers/qwen.rs     # Qwen 通义 Provider
✅ 新增: src/llm/routing/mod.rs      # 路由模块
✅ 新增: src/llm/routing/direct.rs   # 直接 API 调用
✅ 新增: src/llm/routing/openrouter.rs # OpenRouter 聚合
✅ 新增: src/llm/routing/ollama.rs   # Ollama 本地
✅ 新增: src/llm/intent/mod.rs       # 意图模块
✅ 新增: src/llm/intent/classifier.rs # LLM 意图分类器
✅ 修改: src/config/schema.rs        # 添加 LLMConfig
✅ 修改: src/agent/intent.rs         # 保留正则作为 fallback
✅ 修改: src/agent/orchestrator.rs   # 集成 LLM 层
✅ 修改: config.example.toml          # 添加 LLM 配置示例
```

**支持的 Provider**：OpenAI, Anthropic (Claude), DeepSeek, Zhipu (智谱), MiniMax, Qwen (通义)

**支持的路由模式**：Direct API, OpenRouter, Ollama

**验收标准**：
- "为什么我的支付服务响应很慢" 能正确识别为 Metrics/Debug 意图
- 实体（服务名、集群）能正确抽取

---

### 1.2 LLM 结果总结层 ✅ 已完成

**问题**：原始命令输出用户看不懂

**方案**：
- 执行完工具后，结果过 LLM 总结
- 输出结构化 Markdown（表格、要点总结）
- 根据意图类型定制输出格式

**实现状态**：
```
✅ 新增: src/llm/summarizer.rs      # 结果总结器
✅ 修改: src/agent/orchestrator.rs  # 执行后调用 LLM 总结
```

**总结器支持的意图类型**：
- Logs: 日志概览、关键发现、可能原因、建议操作
- Metrics: 指标概览、趋势分析、异常检测、根因分析、优化建议
- Health: 健康状态、组件状态、问题清单、处理建议
- Debug: 问题确认、根因分析、证据支持、修复建议、预防措施
- Query: 查询结果、结果分析、补充说明
- Scale: 当前状态、扩缩建议、影响评估
- Deploy: 部署状态、版本信息、问题处理

**验收标准**：
- `kubectl describe pod xxx` 输出转为「Pod 状态、原因、建议」格式
- Metrics 查询结果转为「指标解读 + 图表描述」

---

### 1.3 Plan-Execute-Reason 执行模式 ✅ 已完成

**问题**：当前是单命令执行，无法处理复杂问题

**方案**：
- 用户问题 → LLM 规划需要哪些查询 → 并行执行 → LLM 综合分析 → 结论

**实现状态**：
```
✅ 新增: src/agent/planner.rs         # 执行规划器
✅ 修改: src/agent/orchestrator.rs   # 支持 P-E-R 模式
```

**工作流程**：
1. Debug 意图触发 planner 模式
2. LLM 分析问题，规划查询步骤
3. 执行每个计划步骤
4. LLM 综合所有结果给出诊断结论

**注意**：当前实现中计划步骤的执行是简化的模拟版本，真正的 K8s/Metrics API 调用需要后续与现有 skill/MCP 系统集成。

**示例流程**：
```
用户: "网关突发 502，帮我排查"

Plan:
  1. 查询网关 Pod 状态
  2. 查询后端服务日志
  3. 查询网关 access log 错误率
  4. 查询相关 Metrics

Execute: 执行以上查询

Reason: LLM 综合分析，给出根因
```

---

## Phase 2: 对话与上下文 ✅ 已完成

**目标**：支持多轮对话，让排查像与专家对话一样自然

### 2.1 对话上下文管理 ✅

**问题**：每次查询独立，无法追问

**方案**：
- 扩展 Session，增加 conversation_history 列表
- 维护最近 20 轮对话历史
- LLM 调用时带入上下文

**实现状态**：
```
✅ 修改: aiclaw-types/src/agent.rs   # Session 增加 conversation_history 字段
✅ 修改: src/agent/session.rs         # 对话历史管理 + add_message 方法
✅ 修改: src/agent/orchestrator.rs    # 带入历史上下文
```

### 2.2 主动澄清与追问 ✅

**问题**：意图不明确时直接返回错误

**方案**：
- 支持追问识别（短消息、follow-up 关键词）
- 追问时继承会话上下文（cluster、namespace）
- 追问时带入最近对话历史

**实现状态**：
```
✅ 修改: src/agent/orchestrator.rs    # is_followup_question + parse_followup_intent
```

**支持的追问模式**：
- 简短问题："然后呢"、"详细说说"、"为什么"
- 确认类："是"、"不对"
- 继续类："继续"、"补充"、"./details"

---

## Phase 3: 安全与合规 ✅ 进行中

**目标**：满足生产环境安全要求

### 3.1 白名单命令机制 ✅ 已完成

**问题**：黑名单不安全

**方案**：
- 只允许预定义的 kubectl 命令模式
- 命令执行前校验
- 敏感操作（如 delete）需要额外确认

**实现状态**：
```
✅ 新增: src/security/mod.rs
✅ 新增: src/security/command_validator.rs
✅ 新增: src/security/audit_logger.rs
✅ 修改: src/skills/executor.rs  # 执行前校验
```

**默认白名单规则**：
- 允许：get, describe, logs, top, events, api-resources, cluster-info, namespace, config, explain
- 敏感操作：delete, scale, stop（需要确认）
- 始终阻止：rm, dd, mkfs, eval, exec, ssh, curl, wget 等

---

### 3.2 审计日志增强 ✅ 已完成

**问题**：当前只有基础 interaction 记录

**方案**：
- 记录完整操作上下文（用户、意图、执行的命令、结果）
- 持久化到日志系统/ES
- 支持查询和导出

**实现状态**：
```
✅ AuditEvent 结构完整记录：timestamp, user_id, channel, session_id, command, intent, skill, success, risk_level
✅ 事件类型：CommandExecution, BlockedCommand, ConfirmationRequested, SkillExecution, IntentClassification
✅ 异步 channel 传输，不阻塞主流程
```

---

## Phase 4: 可靠性与容错

**目标**：生产级别稳定性

### 4.1 超时、重试、熔断

**方案**：
- 每个工具执行设置超时
- 失败时自动重试（指数退避）
- MCP 服务器不可用时熔断

**实现状态**：
```
✅ 新增: src/utils/retry.rs       # RetryConfig, with_retry, CircuitBreaker
✅ 修改: src/skills/executor.rs  # 加超时和重试
```

---

### 4.2 降级策略

**问题**：某组件不可用时完全失败

**方案**：
- 部分成功时返回降级响应（"VM 不可用，仅返回 K8s 状态"）
- 健康检查 + 自动恢复

---

## Phase 5: 多集群支持

**目标**：满足多集群运维场景

### 5.1 集群上下文管理

**方案**：
- 用户可指定集群（"查一下 prod 集群的..."）
- 维护用户当前集群上下文
- 集群名称自动补全

**实现状态**：
```
✅ 修改: src/agent/intent.rs          # 增加 cluster 上下文提取 (prod/staging/test/dev)
✅ 修改: src/agent/session.rs         # 修复重复代码
✅ 修改: src/agent/orchestrator.rs    # 继承 session cluster/namespace 上下文
```

---

### 5.2 跨集群聚合

**方案**：
- 支持同时查询多个集群
- 聚合结果统一展示

**实现状态**：
```
✅ 修改: src/agent/orchestrator.rs  # is_multi_cluster_query, execute_multi_cluster
```

---

## Phase 6: 企业级能力

**目标**：满足大型企业需求

### 6.1 RBAC 集成

- 企业 SSO/LDAP 认证
- 基于角色的操作权限控制
- 敏感操作审批流

**实现状态**：
```
✅ 新增: src/security/rbac.rs  # Role, Permission, RBACValidator, RBACMiddleware
✅ 修改: src/security/mod.rs  # 导出 RBAC 模块
```

### 6.2 多租户隔离

- 数据隔离
- 租户独立配置

**实现状态**：
```
✅ 新增: src/security/tenant.rs  # TenantManager, TenantContext, RateLimiter
```

### 6.3 高可用部署

- 多副本部署
- 负载均衡
- 故障自动切换

**说明**：HA 部署主要依赖于部署架构（Kubernetes StatefulSet/Deployment + Service + 负载均衡器），而非应用代码。当前代码已支持多实例部署，状态存储建议使用外部 Redis/PostgreSQL。

---

## Phase 7: 反馈驱动优化

**目标**：越用越智能

### 7.1 用户反馈收集

**方案**：
- 对每个回答显示 👍👎
- 用户可补充正确结果
- 反馈数据用于优化

**实现状态**：
```
✅ 新增: src/feedback/mod.rs  # FeedbackCollector, FeedbackRecord, SkillMetrics
```

### 7.2 技能质量体系

- 技能使用率统计
- 用户评分
- 自动标记低质量技能

**实现状态**：
```
✅ SkillMetrics: total_uses, success_rate, satisfaction_rate, quality_score
✅ get_low_quality_skills() - 自动标记低质量技能
```

---

## 实施优先级

```
Phase 1 (LLM 能力)  ← 当前最关键
Phase 2 (对话上下文)
Phase 3 (安全合规)
Phase 4 (可靠性)
Phase 5 (多集群)
Phase 6 (企业级)
Phase 7 (反馈优化)
```

---

## 里程碑

| 里程碑 | 内容 | 对应 Phase |
|--------|------|-----------|
| M1 | LLM 意图识别上线 | Phase 1.1 |
| M2 | LLM 结果总结上线 | Phase 1.2 |
| M3 | Plan-Execute-Reason 完成 | Phase 1.3 |
| M4 | 多轮对话支持 | Phase 2 |
| M5 | 白名单安全机制 | Phase 3 |
| M6 | 可靠性保障 | Phase 4 |

---

## 技术债务

以下问题在演进过程中需逐步解决：

1. **测试覆盖**：当前缺少单元测试和集成测试
2. **错误处理**：部分错误是 `anyhow::anyhow!`，不够精确
3. **配置管理**：配置分散在 schema.rs 和 config.example.toml，需统一
4. **日志规范**：tracing 使用不够规范，级别混乱
5. **文档**：API 文档缺失，SKILL.toml 格式无完整文档

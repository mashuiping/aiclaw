# HAMi 集群 GPU Pod 长期 Pending 排查指引

本文对应仓库内技能说明 [`skills/inf-k8s-hami-gpu-pod/SKILL.md`](../skills/inf-k8s-hami-gpu-pod/SKILL.md)，面向在真实集群上用 `kubectl`（及可选的日志链路）做人工或半自动排查。**不依赖本仓库编译是否通过**；只要在当前环境安装 `kubectl`、具备目标集群的 `kubeconfig` 权限即可执行。

## 1. 前置条件

- `kubectl` 已安装，且 `kubectl cluster-info`、`kubectl get nodes` 正常。
- 目标集群上已部署 HAMi（名称可能因 Helm Release 不同略有差异）。
- 已知 **待排查 Pod 的 namespace 与名称**；若只有 workload（如 Deployment），先 `kubectl get pods -n <ns> -o wide` 定位具体 Pod。

可选：安装 `jq`、`helm`（用于检索 Helm Release 与解析 JSON）。

## 2. 技能文档中的主流程（摘要）

按 [`SKILL.md`](../skills/inf-k8s-hami-gpu-pod/SKILL.md) 推荐的顺序执行即可。

1. **Step 0：发现 HAMi 安装 namespace**  
   - `kubectl get pods -A | grep -i hami`  
   - 或按 label：`kubectl get pods -A -l app=hami-scheduler`  
   将得到的 namespace 记为 `$HAMI_NS`，后续命令替换占位符。

2. **Step 1：HAMi 组件健康**  
   - scheduler、device-plugin、vgpu-monitor、MutatingWebhookConfiguration（如 `hami-webhook`）。

3. **Step 2：GPU 资源容量与分配**  
   - 节点上 `nvidia.com/gpu`、`gpumem`、`gpucores` 等扩展资源。  
   - 注意：`kubectl describe node` 的汇总分配 **不等于** 单卡 UUID 上仍有余量；pending 时要对照技能中的 **`hami.io/vgpu-devices-allocated`** 与 **`hami.io/node-nvidia-register`**（见 SKILL 中 **Step 2 / D**）。

4. **Step 3：Pending Pod 根因**  
   - `kubectl describe pod <pod> -n <ns>` 查看 Events（`Insufficient nvidia.com/*`、亲和性、调度器过滤失败等）。  
   - 结合 `hami-scheduler` 日志中的 `FilteringFailed`、`CardInsufficientMemory` 等关键字。

5. **Step 4 及以后：Webhook 注入、调度器 profile、设备插件、网络策略等**  
   详见 `SKILL.md` 全链路表与命令。

## 3. 把仓库内的技能交给 AIClaw 使用（可选）

AIClaw 从配置项 **`[skills].skills_dir`** 指向的目录**一级子目录**加载技能（每个子目录需含 `SKILL.md` 或 `SKILL.toml`）。

要让本仓库自带的 `inf-k8s-hami-gpu-pod` 被加载，可采用任一方式：

```bash
mkdir -p ~/.aiclaw/skills
ln -s "$(pwd)/skills/inf-k8s-hami-gpu-pod" ~/.aiclaw/skills/inf-k8s-hami-gpu-pod
```

或在 `~/.aiclaw/config.toml` 中把 `skills_dir` 设为克隆仓库内的 `skills` 目录的**绝对路径**（注意：TOML 里的路径若含 `~`，当前实现可能不会展开，建议使用绝对路径）。

同时需启用飞书/企业微信等通道，并配置好对应的集群名、`[kubernetes.*]` kubeconfig 与上下文，参见根目录 `config.example.toml`。

## 4. 与「主程序跑起来」的关系（重要）

能否通过 `cargo run` 启动 AIClaw 本体，取决于当前 **`aiclaw` 二进制是否能在工作区成功编译**。若编译失败，你仍可按 **第 2 节** 完全在集群侧完成 HAMi pending 排查；与技能文件的思路一致，只是不经过编排器。

仓库根目录 `README.md` 的 **「已知限制」** 一节会同步说明当前构建状态与配置注意事项。

---
name: k8s-health-check
description: 检查 Kubernetes 集群和 Pod 的健康状态
version: "1.0.0"
author: platform-team
tags: ["kubernetes", "health", "status", "k8s"]
---

# Kubernetes 健康检查

你是 K8s 健康检查助手：综合节点、Pod、事件与资源使用，给出整体评估。

## 命令参考

按需选用（注意当前 kubeconfig / context）：

```bash
kubectl get nodes -o wide
kubectl get pods -n <namespace> -o wide
kubectl get events -A --sort-by='.lastTimestamp'
kubectl top nodes
```

结合输出判断 NotReady、CrashLoopBackOff、Pending、异常事件等。

## 参数占位

可能带有：`namespace` — 未指定时可先全集群或默认命名空间再收窄。

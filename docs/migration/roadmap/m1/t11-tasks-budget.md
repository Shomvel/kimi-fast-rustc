# T11 有界任务、预算和取消 — 细化

`M1 · E · 依赖：T07, T08, T10` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t11) · 设计见 [build-pipeline.md](../../architecture/build-pipeline.md)

## 可复用的现有实现

| 文件 | 可复用 |
|---|---|
| [scheduler.rs](../../../../src/scheduler.rs) | 固定 DAG、有界并发、失败后停止派发、按登记顺序返回 |
| [pipeline.rs](../../../../src/pipeline.rs) | typed 有界同步通道、背压、取消 |
| [parallel_native.rs](../../../../src/parallel_native.rs) | 原子计数器领取函数下标；结果按声明顺序 |

## M1 范围

任务粒度：每个函数一个 `CompileInstance` 任务（son::build → optimize → lower → encode）。前端（parse/resolve/check/body）在 M1 仍串行。

```rust
enum TaskState { Pending, Ready, Running, Validated, Published, Failed, Cancelled }
struct Budget { cpu_tokens: usize, resident_bytes: usize }
```

- worker 数 = `min(--jobs, 可用 CPU 配额)`；从 Cargo jobserver 继承留到 T28。
- 内存：每个任务估算 `node_count × 常数` 作为驻留字节，超过 `resident_bytes` 时暂停领取新任务。
- 产物在 `Validated` 后按 `InstanceKey` 排序写入对象，不按完成顺序。
- fail-fast：第一个错误设置取消标志，所有 worker 在任务边界退出；返回第一个按 `InstanceKey` 排序的错误（确定性）。

## 测试

- 结果确定性：T03 全部用例在 `--jobs 1/2/4/8` 下对象字节一致。
- 故障注入（测试专用开关）：某个任务失败、某个任务延迟 1 s、队列容量 1、`resident_bytes` 很小 → 都在有限时间内结束，无半成品对象。
- 大图：生成一个 5 万节点的函数 + 100 个小函数，检查峰值内存受预算限制。

## 完成标志

以上全部通过；串行参考路径（`--jobs 1`）仍可单独选择。

# T14 M1 出口与迁移估算 — 细化

`M1 · I · 依赖：T09, T10, T11, T12, T13` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t14)

## 出口检查表

| 项 | 证据 |
|---|---|
| 新链支持矩阵 | `capabilities.md` 中 M1 范围内的每项 `supported` 至少一个 run 用例 + 一个 reject 用例 |
| if/match 状态合并 | T01 形状 + T09 组合生成用例全部真实执行一致 |
| 旧测试迁移 | 现有 integration test 中 M1 范围内的用例改写为 `.rs` fixture 并通过 T03 runner；不在范围内的列入 `planned` |
| verifier | 新链所有图通过 full verifier；`KFRC_VERIFY=full` 在 CI 默认开启 |
| 确定性 | jobs 1/2/4/8 对象一致 |
| 无回退 | 新链日志中无 legacy 前端或 rustc 调用；来源清单只出现 kfrc |
| 实验 | T12/T13 报告，含依赖闭包与成本估算 |

## 估算输出

- M2/M3 每个任务拆成 `Txx.a/b/…` 子项，每项给出：依赖、预计规模（新增行数量级）、需要的 rustc 模块（引用 [rustc/](../../rustc/README.md) 笔记）、首批测试。
- 记录 M1 实测的缺陷密度（每千行新代码的差分失败数）作为后续估算依据，不按人数/agent 数换算。

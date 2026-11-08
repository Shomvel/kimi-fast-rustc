# PoC：用真实 crate 证明 Rust 语义 SoN

状态：**计划**，2026-11-08。完整计划（M0–M6）的工作量主要在自有前端和原生 sysroot（光 sysroot 的 hello world 一档就估计 32–48 人周，见 [research/sysroot](../../research/sysroot/README.md)），这两块与“在带 Rust 语义的 IR 上优化”这一理念无关。PoC 只验证理念：前端和标准库借用 rustc，中后端完整实现，在真实 crate 上与 rustc+LLVM 对比。

## 一句话

做一个 rustc 的外部 codegen backend（`-Zcodegen-backend`，形态同 cg_clif），把 rustc 的多态 MIR 转成 kfrc Body，经 R/S/L 三级 SoN 和自写 x86 后端生成目标文件，与 rustc 编译的 std 混链，用 regex-syntax 和 semver 的测试与负载证明正确性、编译工作量、运行时间、体积和增量。

## 要证明的六件事

| # | 主张 | 门槛 |
|---|---|---|
| 1 | 正确 | 所选 crate 的测试全部通过；W01–W10 输出与 rustc 一致 |
| 2 | 编译工作少 | 中后端指令数比 rustc 的 LLVM 部分少 ≥ 3× |
| 3 | 运行不差 | 解析负载 runtime 比值 ≤ 1.10（单侧置信上界；PoC 不做向量化） |
| 4 | 体积不差 | strip 后 `.text` ≤ 1.0 |
| 5 | 泛型只优化一次 | 每个定义 R 级优化恰好 1 次；去重比不低于 `-Zmono-dedup` |
| 6 | 增量 | 行号移动时机器码片段 100% 命中；注释修改 0 重编 |

细则与测量协议见 [metrics.md](metrics.md)。门槛在开工前冻结，可以调整，但不在看到数据后放宽。

## 文件

| 文件 | 内容 |
|---|---|
| [scope.md](scope.md) | 做什么、不做什么 |
| [architecture.md](architecture.md) | 外壳与三条接缝：`TypeOracle`、MIR → Body 转换器、std 的未优化 MIR |
| [phases.md](phases.md) | P0–P4：交付物、出口、止损点 |
| [metrics.md](metrics.md) | 指标、门槛、测量协议 |
| [relation.md](relation.md) | 与 M0–M6 的关系：哪些代码进入产品，哪些是一次性的；契约缺口清单 |
| [p0-status.md](p0-status.md) | P0 进度：已有部分、支持的输入、验证结果、离出口还差什么 |

## 已定的决策

- **前端与标准库借用 rustc。** 这是总览固定决策“不做 `CodegenBackend` 插件”在 PoC 中的例外：插件只是外壳，SoN 与后端 crate 不依赖任何 `rustc_*` crate（[architecture.md](architecture.md)）。
- **运行时间门槛 ≤ 1.10，工作负载为 regex-syntax + semver**（默认值，可在 P0 结束前调整）。
- **混链按总览红线 4 标记**：所有 PoC 结果都标为“rustc 前端 + kfrc 中后端 + rustc std”，不计入 full-native 覆盖。

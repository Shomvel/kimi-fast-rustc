# T12 实验：trait 与关联类型 — 细化

`M1 · B · 依赖：T02, T03, T06` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t12) · rustc 笔记：[traits](../../rustc/traits/README.md)

## 先要知道的事实

**rustc 1.94.1 默认的接受/拒绝结果来自旧求解器。** 类型检查、WF、借用检查、关联类型规范化、codegen 的 impl 选择都用旧求解器；新求解器默认只用于 coherence 的 overlap 检查（[which-solver.md](../../rustc/traits/which-solver.md)）。用本地 rustc 1.94.1 实测的 15 个程序中有 3 个在两种模式下结果不同（[oracle-cases/](../../rustc/traits/oracle-cases/)）。

因此：

- **算法参考用新求解器**：它通过 `Interner` 与 rustc 内部解耦，每个查询是纯函数，缓存与顺序无关，rust-analyzer 也在复用。
- **oracle 用两种模式**：默认模式和 `RUSTC_BOOTSTRAP=1 -Znext-solver=globally`。比较“接受/拒绝 + 错误码集合”；两种模式不一致的用例记为 `solver-divergence`，单独统计，不算 kfrc 的失败。

## 移植范围

| 部分 | 说明 |
|---|---|
| canonical goal 与响应 | 首次出现编号、`Param → placeholder`；region 全部 erased 但保留槽 |
| EvalCtxt 嵌套 goal 循环 | 至多 8 轮，probe 回滚 |
| 候选：impl、param env、`Sized` builtin | 简化类型索引 + 快速拒绝；合并优先级（trivial > 非全局 env > impl） |
| 规范化链 | `Projection → AliasRelate → NormalizesTo` |
| search graph（无缓存） | 栈上环检测、`PathKind`、fixpoint ≤ 8、深度 128、overflow 后预算除以 4 |
| param env | `predicates_of`（默认 `Sized`、`Self: Trait`）+ supertrait 展开 |
| fulfillment 外层 | 反复求值到无进展；剩余歧义报 E0282/E0283 |

递归 goal 的三种结果：路径上至少经过一个 auto trait 或 `Sized` 的 where 子句 → 成功；未知 → 歧义（报 overflow）；只有类型关联或规范化 → 失败。

## 输入闭包

[oracle-cases/t12_base.rs](../../rustc/traits/oracle-cases/t12_base.rs)：一个 trait、一个关联类型、两个候选 impl、带 `where` 约束的泛型函数、跨函数调用。以及 c01–c15 中的有效/缺 impl/投影不匹配/歧义/环。

`Copy`/`Clone` 对基本类型的实现来自 core 的 impl，不是求解器内建。M4 之前的用例只用用户自定义 trait，或附带一个声明 lang item 的 mini core（oracle 侧 `#![no_core]` + `RUSTC_BOOTSTRAP=1`，可行性在本任务中实测）。

## 复用 rustc crate 的评估

`rustc_next_trait_solver` + `rustc_type_ir` 可在 stable 上编译。直接复用需要实现约 70 个关联类型 + 96 个方法的主接口、30 个辅助 trait（159 个方法）、48 个方法的推导上下文接口，kfrc 的类型表示也要采用 rustc 的泛型枚举。

做法：主线写最小自有求解器（推断 2–3k 行）；另用 1–2 天在同一组用例上试复用路线；T14 决定。

## 可推迟

alias bound、auto trait、闭包/`Fn*`、dyn/unsize、coherence、opaque 与 TypingMode、高阶约束与 leak check、GAT、特化、const trait、负 impl、全局缓存。全部列入未覆盖清单。

## 完成标志

c01–c15 与 `t12_base.rs` 的结果与 oracle 一致（`solver-divergence` 用例与任一模式一致即可，并记录跟哪一个一致）；复用评估报告；M3 的 T21 子项拆分与估算。

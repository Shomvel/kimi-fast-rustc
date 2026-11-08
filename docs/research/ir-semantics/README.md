# R/S/L 三级 IR 的可执行语义

状态：2026-11-08 调研，供 [design/ir/](../../design/ir/levels.md) draft-0 修订用。来源已核实的结论和推断分开标注；找不到原始来源的写“未找到原始来源”。

## 结论

1. **写语义，但只写可执行的那种。** 每级一个 Rust 写的参考解释器（CakeML、Graal Isabelle 语义、Wasm 规范解释器的路线），不做机器检查证明（CompCert 约 3 人年，CakeML 约 10 万行 HOL4，代价主要在证明）。按总览红线 3，解释器只用于定位，正确性仍以真实执行对拍为准。
2. **执行模型取 Graal IR 的形式化：** 固定节点沿控制骨架执行，浮动节点按需求值，phi 同时更新。它要求浮动节点是全函数，所以必须引入 **poison**（不要 undef，freeze 按需）：`Add.nsw` 溢出得 poison，而不是现设计写的 UB。这是本调研最重要的修改（[S1](recommendations-soundness.md#s1-addnsw-的-ub-与浮动冲突)）。
3. **内存三级共用 MiniRust 式字节内存**（抽象字节 + provenance），只有 SSA 值分级。所以 R 的语义以答案表为参数：⟦R⟧_A。R→S 是相等（代入定理），S→L 的值关系就是 MiniRust 的 `encode`。
4. **分区分两类：** `Local`/`Heap`/`Const` 是分析结果，违反即编译器 bug；`Frozen`/`Unique`/`Reborrow` 是 UB 条件，证明义务是“kfrc 的分区 UB ⊆ Miri 在 SB 与 TB 下都报的 UB”。别名模型不进 kfrc 解释器，由 Miri 判定源程序。
5. **检查模式**把 verifier 的静态规则变成动态断言（动态分区标签、字节归属、token 新鲜度、`Refine` 来源、答案读取）。[findings](../walkthroughs/findings.md) 的 14 条健全性问题里 11 条会在写语义时被问到，或被检查模式直接抓到。
6. **CTFE 不解释 R 图**（优化会删掉必须报告的 UB），维持解释 Body；但与 IR 解释器共享值、内存、`encode/decode` 核心，如 rustc 的 CTFE 与 Miri 共享 `InterpCx` + `Machine`。
7. **代价：** 全量约 1.05 万行、4–5 人月；首版（与 M2 的 T17 同期）约 4–5 千行、7–9 人周（推断）。

## 三级语义总表

| | Body | R | S | L |
|---|---|---|---|---|
| 值 | MiniRust `Value` + Place | MiniRust `Value` + 标量 `Poison` | 同 R | `MachineTy` 位串 + `Poison` |
| 内存 | 抽象字节 + provenance | 同 | 同 | 同（S→L 内存不变） |
| 参数 | 答案表 | 答案表 A（布局、impl、`NeedsDrop`、`PanicStrategy`） | 无（已代入） | 布局表、`TargetSpec` |
| UB | Rust 的立即 UB（MiniRust/Miri） | UB 点 + 立即 UB 固定节点 + `Refine{Assumed}` + UB 分区 | 同 R | 同 R，另加机器陷入 |
| 与上一级 | 对 Miri 对拍 | ⟦R⟧_A ⊑ ⟦Body⟧_A | ⟦S⟧ = ⟦R⟧_A | 值 = `encode(S 值)`，⊑ |

## 先例总表

| 先例 | 文件 | 结论 |
|---|---|---|
| MiniRust：规范即解释器，`Value`/`AbstractByte`/`encode`/`decode`，内存可插拔 | [prior-art-rust.md](prior-art-rust.md) | **采纳结构**，不直接运行（无浮点、unwind 不全、输入是 rustc MIR） |
| Miri：与 CTFE 共享引擎，找出全部 de-facto UB | 同上 | **采纳**为源程序 oracle 与共享核心的范例 |
| LLVM LangRef poison/freeze、Taming UB、Alive2 refinement | [prior-art-llvm.md](prior-art-llvm.md) | **采纳** poison 与 UB 点、refinement 定义；不做整图 SMT |
| Vellvm：抽取的可执行解释器 refine 命题模型 | 同上 | **改编**：只要可执行一层 |
| CompCert：每级语义 + 逐级 simulation；CakeML：带时钟的函数式大步语义 | [prior-art-verified.md](prior-art-verified.md) | **采纳**每级一套可执行语义；**不采纳**证明 |
| Graal IR 的 Isabelle 语义（ATVA '21）、Demange 等 CC '18、TurboTV（ICSE '24） | [prior-art-graph-irs.md](prior-art-graph-irs.md) | **采纳** Graal 的三层执行模型；TurboTV 的“所有调度等价”作良构条件 |
| Swift SIL：只有文字 UB 说明与 verifier；MLIR 语义 dialect、Lean-MLIR | 同上 | SIL 为反面参照；MLIR 的 poison/UB 效果分层**采纳**，DSL 不采纳 |
| Crocus/Arrival、CLIF 解释器 + fuzzgen、Wasm 规范解释器与 SpecTec、Lean IR 解释器 | [prior-art-rules-specs.md](prior-art-rules-specs.md) | 标量语义写一次、同时生成解释器与 SMT；解释器作 fuzz oracle |
| “Formalizing the Sea of Nodes” 这一标题的论文 | — | 未找到原始来源；相近工作是上面的 Graal 与 Demange 等 |

## 设计文件

| 文件 | 内容 |
|---|---|
| [values.md](values.md) | 值域：聚合、enum、宽指针、union、poison、`noundef`、浮点 NaN |
| [memory.md](memory.md) | 内存模型、provenance、分区的两类语义地位、检查模式的影子状态 |
| [execution.md](execution.md) | 控制骨架 + 按需求值、节点四类、`Refine`、`Select`、投机 load、fence |
| [refinement.md](refinement.md) | Body→R→S→L 的关系、⟦R⟧_A、MiniRust 对齐、CTFE |
| [uses-and-cost.md](uses-and-cost.md) | 用途、规则验证、工作量、首版范围 |
| [verifier-mapping.md](verifier-mapping.md) | verifier 检查与语义规则的对应、缺少的检查 |
| [issue-origins.md](issue-origins.md) | walkthrough 发现与语义空白的对应 |
| [recommendations-soundness.md](recommendations-soundness.md) | 健全性建议 S1–S12 |
| [recommendations-gaps.md](recommendations-gaps.md) | 语义空白与流程建议 G1–G9 |

## 建议速览

| # | 建议 | 目标文件 |
|---|---|---|
| S1 | `nsw/nuw` 溢出得 poison；列 UB 点；节点按四类登记 | `design/ir/values-and-types.md`、`nodes.md`、`levels.md` |
| S2 | `noundef` 不覆盖 union/`MaybeUninit` | `design/ir/values-and-types.md` |
| S3 | 固定 `Load` 与 `Load.spec`：投机读解码失败得 poison | `design/ir/nodes.md`、`memory-partitions.md` |
| S4 | 可浮动 load 的三处写法统一；`Local` load 受槽活跃区间约束 | `design/ir/levels.md`、`nodes.md`、`verifier.md` |
| S5 | 浮动 load 的反依赖 | `design/ir/graph.md`、`verifier.md` |
| S6 | `Refine{Proven \| Assumed}` | `design/ir/values-and-types.md`、`nodes.md` |
| S7 | `Select` 的 poison 语义 | `design/ir/nodes.md`、`optimization-pipeline.md` |
| S8 | `Transmute` 节点与布局孔位 | `design/ir/nodes.md`、`generics-specialization.md` |
| S9 | 禁止按指针相等替换 | `design/ir/optimization-pipeline.md`、`memory-partitions.md` |
| S10 | 分区分“分析”与“UB 条件”两类 | `design/ir/memory-partitions.md` |
| S11 | NaN 折叠与硬件一致，禁止复制后分别折叠 | `design/ir/values-and-types.md`、`optimization-pipeline.md` |
| S12 | placed 级 `ReleaseOrder` 检查 + litmus | `design/ir/verifier.md` |
| G1–G9 | 新建 `design/ir/semantics.md`；分配不可观察；⟦R⟧_A；规则表语义标志；移位/除法模式；`VariantField`；unwind 值；解释器进计划；CTFE 共享核心 | 见 [recommendations-gaps.md](recommendations-gaps.md) |

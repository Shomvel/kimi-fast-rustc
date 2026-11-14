# 渐进降低：MLIR、Flang HLFIR、Lean 4、Koka

## 1. 是什么

| 系统 | 来源 | 年份 | 级别 |
|---|---|---|---|
| MLIR | Lattner 等, [MLIR: Scaling Compiler Infrastructure for Domain Specific Computation](https://research.google/pubs/mlir-scaling-compiler-infrastructure-for-domain-specific-computation/)（CGO '21）；[Toy 第 5 章：部分降低](https://mlir.llvm.org/docs/Tutorials/Toy/Ch-5/) | 2021 | 多个 dialect 可在一个模块中混合；逐步改写到 LLVM dialect |
| Flang HLFIR | [High-Level Fortran IR 设计文档](https://flang.llvm.org/docs/HighLevelFIR.html)；[RFC: 默认启用 HLFIR](https://discourse.llvm.org/t/rfc-enabling-the-hlfir-lowering-by-default/72778) | 2022–2023 | HLFIR → FIR → LLVM dialect |
| Lean 4 新编译器（LCNF） | [4.22.0 发布说明](https://lean-lang.org/doc/reference/latest/releases/v4.22.0/)（2025-08 成为默认）；[4.30.0](https://lean-lang.org/doc/reference/latest/releases/v4.30.0/)（2026-05 C 发射移到 LCNF）；[PassManager API](https://lean-lang.org/doc/api/Lean/Compiler/LCNF/PassManager.html) | 2025–2026 | base → mono → impure |
| Koka Perceus | Reinking, Xie, de Moura, Leijen, [Perceus](https://www.microsoft.com/en-us/research/uploads/prod/2021/06/perceus-pldi21.pdf)（PLDI '21） | 2021 | 显式控制流核心语言 → 插入精确 RC → 复用 → C |

## 2. 关键证据

**MLIR：** 设计原则之一是 “Progressivity”，原文 “Premature lowering is the root of all evil”；每个 op/type/attribute 属于一个 dialect，但允许混合以支持渐进降低。dialect 转换框架允许“部分合法”的中间状态。

**Flang HLFIR（与 kfrc 的论点同构）：** FIR 刻意只保留内存分配、数组寻址、描述符等最小操作。直接从前端降到 FIR 时，“所有数组表达式和标量字符/派生类型表达式在 lowering 中被缓冲”，即直接给结果分配内存当变量处理；lowering 代码因此“复杂度显著增长”，F95 的字符数组表达式和 FORALL 仍有缺口；部分变量信息（非常量数组边界）丢失。HLFIR 的核心动机是“避免在 lowering 时把表达式物化成临时量”。默认启用的 RFC 报告：lowering 简化、难支持的特性变得可支持、临时数组小得多。

**Lean 4 LCNF：**

| 事实 | 来源 |
|---|---|
| 三个阶段：base（大部分类型信息仍在，依赖部分已擦除）、mono（多态已消除）、impure（RC、IO 变换）；pass 分为 basePasses / monoPasses / impurePasses | PassManager、Phase 文档 |
| `saveBase`/`saveMono` 是流水线中的保存点，阶段结果进入 `.olean`（[issue #8439](https://github.com/leanprover/lean4/issues/8439)：olean 中出现非最终的 mono 声明）；下游如何使用这些声明未找到原始设计文档 | 4.22.0 #8764；issue #8439 |
| 检查不再每个 pass 后都跑，只在 `init`、`saveBase`、`toMono`、`saveMono` 后跑：“这是编译时间改进”；在决定“不再尝试在整个编译中保持类型”后，检查的价值下降，开发中“不是发现问题的重要途径” | 4.22.0 #8764 |
| reset/reuse pass 移到 LCNF：二进制约小 15%，各处略快 | 4.30.0 #12665 |

**Perceus：** 在显式控制流的核心语言上插入精确的 dup/drop，使无环程序“garbage free”；复用分析让同大小的释放-分配成对复用（FBIP）。论文说 Koka 除 RC 相关外优化很少，性能仍与主流 GC 竞争；复用会延长内存持有，后续工作（[Frame Limited Reuse](https://www.microsoft.com/en-us/research/wp-content/uploads/2021/11/flreuse-tr.pdf)）对此加了限制。

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| 高层表示保留值语义，推迟物化 | **采纳（已一致）** | Flang 先降到低层 FIR 后不得不补一个 HLFIR，问题正是“表达式被迫进内存、信息丢失”；与 evidence.md 第 1 节对 LLVM 的观察一致 |
| 发布多态阶段的图，供下游内联/特化 | **采纳（已一致）** | Lean 的 `saveBase` 把多态阶段结果存入模块产物，对应 kfrc 的“R 图进 metadata” |
| MLIR 式多 dialect 混合 | **不采纳** | kfrc 三级共用一种图、单向降低；混合状态只在 `lower_layout` 内部短暂存在 |
| verifier 只在级别边界全量运行 | **采纳** | Lean 的数据：逐 pass 检查花编译时间且很少发现问题。kfrc 在 R 发布、S 结束、L 结束做全量检查；逐 pass 检查只在调试构建 |
| 只在 L 级丢 Rust 类型 | **采纳（已一致）** | Lean 决定不在整个编译中保持类型；kfrc 的红线 3 已规定 L 级不出现 Rust 类型推理 |
| 在高层 IR 上做表示相关优化 | **采纳** | Lean 把 reuse 从低层 IR 移到 LCNF，二进制小 15% |
| Perceus 式 RC 插入 | **不采纳** | Rust 的 drop 由语义决定，不需要推断 |
| FBIP 式“drop 后同布局分配复用” | **不采纳（首版）** | 依赖分配器调用可删除/合并的语义；rustc 依赖 LLVM 的 allockind 属性，kfrc 若要做需单独论证 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [levels.md](../../design/ir/levels.md) | 开头或“为什么不是三种 IR” | 增加“检查点”：verifier 全量检查只在三处运行——R 图发布前、S 级优化结束、L 级进入 Machine IR 前；每个 pass 后只做 O(改动) 的局部检查，全量逐 pass 检查只在调试构建（依据 Lean 4.22 #8764） |
| [levels.md](../../design/ir/levels.md) | 为什么不是三种 IR | 先例补两条：Flang HLFIR（过早降低迫使表达式进内存，后来补高层 dialect）；Lean 4 LCNF 的 base/mono/impure 与 R/S/L 一一对应，base 阶段同样存入模块产物 |
| [levels.md](../../design/ir/levels.md) | L 级 | 写明 `lower_layout` 内部允许“部分降低”的中间状态（MLIR 部分合法），但对外只有 S 与 L 两个合法状态，verifier 不检查中间状态 |

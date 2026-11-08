# 先例五：图 IR 与高层 IR 的语义——Graal、SoN、TurboFan、Swift SIL、MLIR

核实日期 2026-11-08。“只见摘要”表示原文 PDF 无法获取。

## Sea of Nodes 与 Graal IR

| 工作 | 要点 | 状态 |
|---|---|---|
| Click & Paleczny, A Simple Graph-Based IR（IR '95，[DOI](https://doi.org/10.1145/202530.202534)） | 据检索摘要：没有给形式语义，各运算的语义局部定义，数据子图不用 token 语义，形式处理指向 John Field 的图重写工作 | 只见检索摘要 |
| Webb, Utting, Hayes, [A Formal Semantics of the GraalVM IR](https://arxiv.org/abs/2107.01815)（ATVA '21，[理论](https://github.com/uqcyber/veriopt-releases/tree/atva2021)） | Isabelle/HOL，三层：无副作用数据节点的**大步**语义、方法内控制流的小步语义、跨方法（调用、异常）的小步语义。浮动节点**按需求值**，读“方法状态” `m`（节点 → 值）；有副作用的节点（Invoke、NewInstance、LoadField）在控制路径上，控制到达时把值存入 `m[nid]`，数据求值之后只读它。合并点从第 i 个 End 进入时，在旧 `m` 中求全部 phi 的第 i 个输入，再**同时**更新；循环中 phi 引用返回上一轮的值。经 Isabelle 代码生成可执行，翻译了 1400+ 个 Graal 单元测试，修两处后全部通过 | 已核实 |
| Utting, Webb, Hayes, [Differential Testing of a Verification Framework…](https://arxiv.org/abs/2212.01748)（FormaliSE '23） | 语义通过 2544 个翻译测试中的 2538 个（6 个失败都涉及动态调用）；把 Graal 的 Java 优化与 Isabelle 版优化差分比较 | 已核实 |
| Webb, Hayes, Utting, [Verifying Term Graph Optimizations using Isabelle/HOL](https://arxiv.org/abs/2212.06956)（CPP '23） | 优化先在项上证明，再经数据精化提升到项图；项 refinement：`e1` 能求出的每个值 `e2` 也能求出；重写规则语言自动生成终止性与 refinement 证明义务，证明了 45 条规则 | 已核实 |
| Demange, Fernández de Retana, Pichardie, Semantic Reasoning about the Sea of Nodes（CC '18，[HAL](https://inria.hal.science/hal-01723236)） | 语义分两部分：数据计算用指称式、控制流用操作式；证明基于支配的性质（节点的值在图的哪些区域保持不变），并用它验证冗余除零检查消除；全部在 Coq 中机械验证。是否含 SSA→SoN 的验证翻译：未找到原始来源 | 只见摘要 |
| Kwon 等, [TurboTV: Translation Validation for JIT Compiler in V8](https://kihongheo.kaist.ac.kr/publications/icse24.pdf)（ICSE '24） | 把 TurboFan IR 编码成 SMT，Alive2 式检查 refinement；**语义定义在已调度的 CFG 上**，不直接定义在 SoN 上；SoN 的“有效”= 所有合法调度等价，V8 默认假设成立，TurboTV 去检查它 | 已核实 |

## Swift SIL

| 文档 | 要点 | 状态 |
|---|---|---|
| [SIL.md](https://github.com/swiftlang/swift/blob/main/docs/SIL/SIL.md)“Undefined Behavior” | “有 UB 的 SIL 程序没有意义……不能在 SIL 层总被诊断或验证”；未绑定到该类型的内存做类型化访问是 UB（`pointer_to_address` 例外）；“Runtime Failure”：trap 可以重排但有限制。没有找到“SIL 未形式化”之类的原文 | 已核实 |
| [Instructions.md](https://github.com/swiftlang/swift/blob/main/docs/SIL/Instructions.md) | `alloc_stack` 分配未初始化内存，栈式配对 `dealloc_stack`；`load` 读未初始化或已释放内存是 UB，`[take]` 移出后位置未初始化；`begin_access` 每条路径恰好结束一次，“进入基本块的每条边上活跃访问集合相同” | 已核实 |
| [Ownership.md](https://github.com/swiftlang/swift/blob/main/docs/SIL/Ownership.md) | OSSA：结束生命周期的使用联合后支配其他使用，每条路径恰好一次；由 SILVerifier 检查 | 已核实 |
| 解释器 | 没有找到 SIL 解释器存在或被删除的来源；最接近的是常量求值器 `ConstExprEvaluator`（`lib/SILOptimizer/Utils/ConstExpr.cpp`） | 未找到原始来源 |

## MLIR

| 工作 | 要点 | 状态 |
|---|---|---|
| Fehr 等, [First-Class Verification Dialects for MLIR](https://users.cs.utah.edu/~regehr/papers/pldi25.pdf)（PLDI '25，[xdsl-smt](https://github.com/opencompl/xdsl-smt)） | 每个 dialect 的语义写成到“语义 dialect”的 lowering（SMT-LIB 之上加 `poison`、`effect`、`ub_effect`、`mem_effect`，Alive2 式），再降到 SMT；覆盖 `arith`/`func`/`memref` 等，只含无控制流的运算、无浮点；用于翻译验证、`pdl` 重写的事前验证、数据流传递函数验证；找到 5 个上游误编译 | 已核实 |
| Bhat 等, [Verifying Peephole Rewriting in SSA Compiler IRs](https://drops.dagstuhl.de/entities/document/10.4230/LIPIcs.ITP.2024.9)（ITP '24） | Lean 中内在良类型的带 region SSA 演算，对 IR 泛型；用户经 `TyDenote`/`OpDenote` 给出指称；定理：窥孔重写后指称不变；只支持结构化控制流 | 已核实 |

## 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| Graal 式三层语义：浮动节点按需求值 + 控制骨架小步 + phi 同时更新 | **采纳为 R/S/L 的执行模型**（[execution.md](execution.md)） | 唯一既形式化又**可执行且对拍过真实编译器**的 SoN 语义；与 kfrc 的固定/浮动划分同构 |
| 有副作用节点的值存入状态、之后只读 | **采纳** | 对应 kfrc 的固定 load：控制到达时求值；浮动 load 才按需读内存，因此需要 token 新鲜度断言 |
| TurboTV 的“所有合法调度等价” | **采纳为良构条件** | kfrc 要求浮动节点全函数（poison 而非 UB），正是为了让这个条件成立 |
| Demange 等的“值在哪些区域保持” | **对应 `Refine` 与 `RefineEscapes`** | 控制依赖事实只在被支配区域内有效 |
| Swift SIL 只有文字 UB 说明、没有解释器 | **反面参照** | OSSA 靠 verifier 而非语义；Swift 6.2 的生命周期回归（[ownership-ssa.md](../prior-art/ownership-ssa.md)）说明“verifier 通过 ≠ 语义保持” |
| `begin_access` 每条入边活跃集合相同 | **对应 `EscapedPartitionUse` 的汇合规则** | 都是“状态在汇合处一致”的静态检查 |
| MLIR 语义 dialect | **不采纳 DSL，采纳 poison/UB 效果的分层** | kfrc 的规则表直接生成 SMT；poison 与 UB 作为两个独立的标志位，与 xdsl-smt、Alive2 一致 |

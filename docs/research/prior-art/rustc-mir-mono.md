# rustc 的 MIR 优化与单态化

## 1. 是什么

| 项 | 来源 | 年份 |
|---|---|---|
| MIR GVN（基于 `SsaLocals` 的受限 CSE） | [PR #109597](https://github.com/rust-lang/rust/pull/109597)（1.74）；`SsaLocals` 见 [#106908](https://github.com/rust-lang/rust/pull/106908)；借用局部仍视为 SSA 见 [#123602](https://github.com/rust-lang/rust/pull/123602)（1.80） | 2023–2024 |
| DestinationPropagation 默认开启 | [PR #142915](https://github.com/rust-lang/rust/pull/142915)（1.92，2025-09-17 合并） | 2025 |
| MIR move 消除需要改 `move` 语义 | [项目目标 2025h2](https://rust-lang.github.io/rust-project-goals/2025h2/mir-move-elimination.html) | 2025 |
| post-mono MIR 优化 | Aaron1011, [issue #66969](https://github.com/rust-lang/rust/issues/66969)（2019）；saethlin, [PR #131650](https://github.com/rust-lang/rust/pull/131650)（2026-09 因不活跃关闭；后继 [#156858](https://github.com/rust-lang/rust/pull/156858) 未合并） | 2019–2026 |
| polymorphization 移除 | [MCP #810](https://github.com/rust-lang/compiler-team/issues/810)、[PR #133883](https://github.com/rust-lang/rust/pull/133883)（2024-12，1.85）；[tracking #124962](https://github.com/rust-lang/rust/issues/124962)；[issue #75325](https://github.com/rust-lang/rust/issues/75325) | 2020–2025 |
| `-Zshare-generics` | [PR #48779](https://github.com/rust-lang/rust/pull/48779)（2018）；[Cargo Book: Profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)；[issue #142164](https://github.com/rust-lang/rust/issues/142164) | 2018– |
| `rustc_codegen_cranelift` | [LWN 2024-03](https://lwn.net/Articles/964735/)；bjorn3 [2023-07 进度报告](https://bjorn3.github.io/2023/07/29/progress-report-july-2023.html) | 2020– |

## 2. 关键证据

**MIR 优化的增量：** GVN 合并后 perf 结论“no relevant changes”。DestProp 默认开启时，作者说明“完整活跃性数据流约占该 pass 一半时间”；合并后指令数总体 0.0%，二进制 −0.1%，评审结论“编译时间基本持平，编译器自身二进制变小”。与 [evidence.md](../evidence.md) 第 2 节一致：逐个关 MIR pass，未优化 IR 变化在 ±1.6% 内。

**post-mono MIR 的代价（PR #131650 的 perf）：**

| 时间 | 指令数（primary 均值） | max RSS | bootstrap |
|---|---|---|---|
| 2024-10 | +11.7%（最多 +93.7%） | +14.5% | +3.28% |
| 2025-06 | +0.7% | +7.0%（最多 +31.1%） | +1.48% |

作者原话：代价“基本就是克隆、单态化、再缓存 MIR”；查询系统只能按实例化模式决定缓存，于是“所有 LocalCopy 函数都缓存”；“编译时内存开销无法修复”。评审还指出参数属性推导（`deduced_param_attrs`）在 post-mono 变换后不再成立。issue #66969 列出单态化前优化漏掉的三类事实：布局（如 ZST）、额外 impl（`Copy`、`!Drop`）、因此多出来的 landing pad。

**polymorphization 的教训：**

| 原因 | 原文要点 |
|---|---|
| 维护负担 | “被 polymorphize 过的东西不能再被 monomorphize”，而每个 crate 都有自己的 `fn monomorphize`，“出奇地难以维持”；单态化之后也不能用 `TypingEnv::fully_monomorphized` |
| 收益小、无用户 | 只处理“未使用的类型参数”；“实现非常有限，并不真正有用”；GitHub 上找不到 `-Zpolymorphize` 用户；4 年无人维护 |
| 正确性 | “仍有错误地 polymorphize 的 bug”；`type_name`/`TypeId` 使“未使用”参数可观察；闭包继承父函数的参数，要知道闭包怎么被用才能决定，会碰到查询环 |
| 重设计方向（lcnr，2023-08） | 浅分析：参数只在“布局计算、常量求值、`type_of`/`type_name`”直接用到时算“用到”，忽略嵌套调用；在**收集完单态实例之后**记住实际用到了多少具体参数，再决定共享 |

**`-Zshare-generics`：** opt-level 0/1/s/z 默认开，2/3 关；不能在兄弟 crate 之间共享；issue #142164 报告在 `opt-level=z` 下有时变大，因为降低了 LLVM 的内联能力。

**cg_clif：** Cranelift 自身 debug 构建 wall 37.5 s → 29.6 s（−20%，LWN）；同文的 [HN 讨论](https://news.ycombinator.com/item?id=39743447)中 CPU 时间 211 s → 125 s。2023-07 报告：关掉 debug assertions 后，某基准的编译时间相对 cg_llvm 的改进从约 13% 变为约 39%；cg_clif 的 release 模式产物约快 20%，在 simple-raytracer 上快于 LLVM 的 debug 产物。定位是 debug 构建，release 仍用 LLVM；与 LLVM release 的运行时间对比未找到原始来源。

## 3. 对 kfrc 的结论

| 问题 | 结论 | 理由 |
|---|---|---|
| “R 级优化一次 + 答案键去重”会重蹈 polymorphization 吗 | **不会，但要守住三条** | kfrc 的答案键正是 lcnr 的重设计：浅孔位（只问图用到的事）、`TypeName`/`TypeIdOf` 是孔位、共享在收集之后按答案决定。polymorphization 的坑是“共享后的实例又被单态化”和多个 `monomorphize` 入口；kfrc 要保证：只有一个 `specialize` 入口；S 图不含 `Param`（verifier 检查）；`InstanceKey` 管符号与可达性，答案键只管代码 |
| S 级特化的成本 | **采纳 rustc 的教训** | post-mono MIR 的成本来自“克隆 + 单态化 + 缓存每个实例”。kfrc 已规定 S 图不缓存；还要规定 S 图是 L 降低任务内的临时对象，先按答案键去重再特化，同时存活的 S 图数受内存预算限制 |
| 摘要依赖特化后的图 | **采纳** | rustc 的 `deduced_param_attrs` 在 post-mono 变换后失效；kfrc 的调用摘要只从最终 S 图计算 |
| “去掉的 IR 不等于省的时间” | **采纳（已一致）** | GVN、DestProp 都是 IR 变好、编译时间持平；evidence.md 第 5 节同结论 |
| 共享与内联分开 | **采纳** | `-Zshare-generics` 的体积退化来自共享实例不能内联。kfrc 的内联从 S 图取，与机器码在哪个 crate 生成无关 |
| 单靠换后端提速 | **记录为基线** | cg_clif 只换后端，debug wall −20%；kfrc 的编译时间目标不能只靠后端 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 特化 | 新增“不变量”：`specialize` 是唯一的单态化入口；S 图不含 `Param`（verifier 检查）；S 图之后的所有查询使用完全单态的环境；共享代码的别名实例不再被特化 |
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 特化 | 写明 S 图生命周期：在 L 降低任务内创建、降低后释放；先查答案键再特化；同时存活的 S 图数受字节预算限制。引用 PR #131650 的 max RSS +7%（最多 +31%） |
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 孔位 | 加一段“与 polymorphization 的区别”：列 lcnr 重设计的三点，并说明 `TypeName`/`TypeIdOf` 作为孔位让可观察差异进入答案 |
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 与 metadata 的关系 | 补一句：机器码共享位置与可内联性无关，下游内联用 R/S 图；避免 `-Zshare-generics` 的内联损失 |
| [evidence.md](../evidence.md) | 2. MIR 层 | 补外部数据：DestProp 1.92 默认开启后编译时间持平，活跃性分析占 pass 一半时间 |

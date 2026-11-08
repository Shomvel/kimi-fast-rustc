# 运行速度：怎么保证

这是四个目标里最难的一个：LLVM 有几十年的循环优化、向量化和指令选择积累。kfrc 的优势是别名和值结构是精确的，劣势是机器级优化要自己写。

## Rust 语义直接带来的部分

| 优化 | LLVM 怎么得到 | kfrc 怎么得到 |
|---|---|---|
| 标量替换 | SROA，需要先证明 alloca 不逃逸 | 聚合天生是值 |
| move 消除 | MemCpyOpt/DSE/GVN 证明源已死 | move 是值转发 |
| 冗余 load 消除、循环外提 | noalias 元数据 + MemorySSA + LICM | 分区：`Frozen` load 是纯的，`Unique` 链上没有别人 |
| enum 判断折叠 | niche 解码后 InstCombine/CVP/JumpThreading | `VariantOf(MakeVariant)` 直接折叠 |
| 越界检查消除 | 从 icmp 和 assume 推区间 | `Refine` + 宽指针 len + 类型范围 |
| 闭包/迭代器 | 内联 + SROA 多轮 | S 级内联已优化的图，接缝种子化简 |
| nounwind | FunctionAttrs 推断 | 调用摘要 |

## 必须自己做的部分

| 优化 | 计划 | 风险 |
|---|---|---|
| 指令选择：地址模式、`lea`、`cmov`、`setcc`、`test` | L 级窥孔 + Machine IR 选择表 | 低，规则有限 |
| 常量除法/取模（乘法逆元、`x % c == 0`） | L 级 | 低；W02 的 filter 依赖它 |
| switch 降低：区间比较、位测试、跳转表 | L 级；niche enum 按 tag 值直接分支 | 低；W03 中 rustc 用 `bt $35` |
| 堆分配消除 | S 级 `Alloc`/`Dealloc` 配对且未逃逸 | 中；W07 中 O3 删掉了 `Box` 分配，不做就是运行时退步 |
| 过程间常量（IPSCCP）、全局变量化简（GlobalOpt） | 调用摘要加 `const_return`；先用“按 pass 名跳过”的 LLVM 插件量出运行时价值再决定 | 未知：LLVM 时间中过程间族占 6–8%，运行时价值没有数据 |
| 后端补全：栈槽着色、shrink wrapping、热路径块布局与尾复制、x86 修补（false deps、vzeroupper） | 列为“必须有 / 可以晚做 / 不做”清单（[codegen-backend](../../research/llvm-mir/codegen-backend.md)） | 中 |
| 循环内的 load 转发与 PRE | 穿过分区 Phi | 中；W05 中 LLVM 也每次重读 cap/ptr，这里是追平而非超越 |
| 寄存器分配 | SSA 线性扫描（Wimmer & Franz 2010）+ 区间拆分 + 远者优先 spill；约束接口（Early/Late × Fixed/Reuse、并行 move）和分配后的符号 checker 学 regalloc2，算法不照搬 | 中：热循环的 spill |
| 指令选择 | 与窥孔共用自写规则 DSL（[规则表](../ir/optimization-pipeline.md#共同机制)）：有优先级、拒绝重叠，不另设选择 IR | 低 |
| 归纳变量与强度削减 | L 级 | 低 |
| 循环展开 | 小循环，体积预算内 | 中 |
| 向量化 | 晚期，T38 之后单独验收 | **高**：ripgrep/Cargo 中依赖自动向量化的热点要先定位 |
| 显式 SIMD（`core::arch`） | intrinsic 1:1 映射为 `Vector` 值节点；**提前做**，M0 的 profile 列出 ripgrep/Cargo 必需的 intrinsic | 中：工作量大但机械。memchr 按 AVX2→SSE2→SWAR 分派，比较时双方必须走同一条分派路径，否则这组比较无效 |

ripgrep 的热路径主要在 memchr 和 regex-automata，前者大量使用显式 SIMD（按 target feature 分派），后者是状态机循环。先用 profile 确认这一点（M0 的 rustc 基线 profile），再决定向量化是否在关键路径上。

## 风险先量化

“运行时间 ≤ 1.0”在业界没有先例：Cranelift 仍比 LLVM 慢约 14%，自写后端的 Zig、cg_clif 都只用于 debug 构建（[prior-art/ct-backend.md](../../research/prior-art/ct-backend.md)、[prior-art/rust-specific-irs.md](../../research/prior-art/rust-specific-irs.md)）。M0 先做归因实验：rustc O3 分别换用 basic 寄存器分配、关掉向量化、关掉循环展开，跑 ripgrep/Cargo 的 workload，量出每一项缺口的上界，据此排 T38 的顺序。

## 验证

- 先保证正确，再比较：每个 pass 可单独关闭，关闭和开启都通过真实执行对拍（[T38](../../plan/m6-quality-performance.md#t38)）。
- 每个 walkthrough 写出 kfrc 期望生成的机器码和 rustc O3 的机器码；M5 起用真实 workload 采样验证。
- 每个 pass 可 bisect：全局 pass 执行计数上限（LLVM `-opt-bisect-limit` 的做法）；bisect 设置进缓存键或绕过缓存（rustc 因增量不跟踪而删掉了 `-Zfuel`）。
- 回归时按热点逐个对比 kfrc 与 rustc 的汇编，把差异归类到上表的某一行；不能归类的差异记为设计缺口。

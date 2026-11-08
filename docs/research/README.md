# 研究材料

支撑 [design/](../design/README.md) 的依据。不读也能理解计划；要追问“为什么这样设计”时看这里。

| 目录/文件 | 内容 | 一句话结论 |
|---|---|---|
| [evidence.md](evidence.md) | 前期 rustc/LLVM 剖析数据（cargo、ripgrep，E2B） | LLVM 主要在重建结构（alloca/memcpy/lifetime 占未优化 IR 23.3%），不是缺事实；重复实例和导入体重复优化是最大的编译时间池 |
| [llvm-mir/](llvm-mir/README.md) | rustc 实际使用的 LLVM pass 流水线与 MIR 优化，按流水线编译的时间轴整理 | rustc 默认不加自己的 LLVM pass；release 对每个函数“化简两遍”（ThinLTO pre/post-link），时间分布平坦（最大的 InstCombine 类 13.6%）；rmeta 之前的 MIR 优化、单态收集、CGU 划分都串行在关键路径上；kfrc 尚缺过程间常量、改签名、循环规范化、switch 降低、调试变量位置 |
| [walkthroughs/](walkthroughs/README.md) | 10 个程序逐级模拟 kfrc 新链，对照 rustc+LLVM 实测 | 60 条发现（含 6 条分区健全性、4 类缓存错误命中），全部已改回设计（[findings](walkthroughs/findings.md)） |
| [prior-art/](prior-art/README.md) | 编译器研究与工业实践：SoN、e-graph、OSSA、MLIR、Tree Borrows、泛型共享、快速后端、增量缓存、测试 | 没有人做过“多态 Rust 语义 SoN + 按答案特化 + 自有后端”；自写后端在 release 运行时间上没有先例 |
| [rustc/](rustc/README.md) | rustc 1.94.1 精读笔记：借用、所有权、类型推导、trait、宏/解析、布局/ABI/单态化，以及模块映射（[rustc-map](rustc/rustc-map.md)） | 每个主题有“kfrc 最小闭包”与代表性 `tests/ui` 用例 |
| [frontend-perf/](frontend-perf/README.md) | 前端优化：rustc 前端时间分布、为什么慢、快速前端先例（Zig、Carbon、Go、RA、tsgo） | rustc 前端占 check 约 90%、release 约 25%；自写前端靠“同样的事做得更快”拿不到大加速，收益来自 MIR 优化移出 rmeta 前、削减每 crate 固定开销（metadata 惰性解码）、crate 内按 typeck root 并行；单 crate 前端估计快 2.7–3.9×，整个 `cargo check` 不到 2.2×；next-solver 指令数是默认模式的 1.15 倍 |
| [ir-semantics/](ir-semantics/README.md) | R/S/L 三级 IR 的可执行语义：MiniRust、Alive2、CompCert 等先例，用途与首版范围 | 三级各写一个可执行参考解释器，不做证明；引入 poison（`Add.nsw` 溢出得 poison 而非 UB），分区分“分析结果”和“UB 条件”两类；首版 7–9 人周。已并入 [semantics.md](../design/ir/semantics.md) |
| [rust-analyzer/](rust-analyzer/README.md) | rust-analyzer 与可独立使用的 rustc 组件怎么参考（不照搬） | RA 的语义有意偏离 rustc（不报歧义、不做借用检查、trait 一律 next-solver），任何规则都不以它为准；值得借的是增量友好的数据切分（item tree、BodySourceMap、durability）；`ra-ap-*` 0.148.0 与 rustc 1.94.1 源码逐字节相同，叶子算法 crate 是否直接依赖待决策 |
| [sysroot/](sysroot/README.md) | core/alloc/std 与 ripgrep/Cargo 依赖需要的 intrinsic、lang item、unstable feature、SIMD、C 依赖 | 全原生 sysroot 的 hello world 一档就要 32–48 人周（编 sysroot 时前端要处理全部 std 源码：const trait、CTFE 软浮点、asm、unwind、TLS），ripgrep +9–13，Cargo（0.95.0）+9–14，不需要 async 和 AVX-512；自有前端读不了 `.rmeta`，所以完整计划里混链官方 std 省不下前端工作；任务缺口已并入 M3/M4 |
| [runtime-profile/](runtime-profile/README.md) | ripgrep/Cargo 运行时热点与对 LLVM 优化的依赖，前期 profile 汇总 | ripgrep 的 ≤1.0 几乎全看显式 SIMD 对齐和 intrinsic 永不成为调用（去掉 SIMD 慢 3.4–10.1×）；Cargo 无单一热点，看跨 crate 内联覆盖面（1.21×）；不需要通用自动向量化；现有两个验收 workload 判定不了目标；目标 Cargo 因此改为 0.95.0（0.99.0 要求 Rust 1.96） |
| [data-structures/](data-structures/README.md) | 编译器自身数据结构：节点布局、驻留、arena、序列化、并行 | 节点做到 16 字节记录加两列、不维护 use-list、按创建顺序压实；驻留分片加每任务缓存；现状的主要问题是 O(K·N) 替换和 BTreeMap，不是 SoN 本身。已并入 design |

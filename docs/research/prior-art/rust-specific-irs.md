# Rust 专用 IR 与相近语言的语义级 IR

问题：有没有人在 MIR 之后、LLVM 之前（或代替 LLVM）做过带 Rust 语义的优化 IR？泛型共享另见 [generic-sharing.md](generic-sharing.md)。

## 1. 是什么，做到哪一步

**rustc 内部**

| 项 | 来源 | 做到哪一步 |
|---|---|---|
| MIR 本身不是 SSA | [RFC 1211](https://rust-lang.github.io/rfcs/1211-mir.html)：临时量“可以被借用，更像 alloca 而不是 SSA 值” | 设计决定 |
| SSA 事实以旁路分析给出 | `SsaLocals`（[#106908](https://github.com/rust-lang/rust/pull/106908)，2023）、GVN（[#109597](https://github.com/rust-lang/rust/pull/109597)） | MIR 仍是基于 place 的，SSA 只是分析结果 |
| “MIR → 带所有权的 SSA”提案 | 未找到原始来源（没有找到对应的 RFC 或 MCP） | — |
| 聚合在 codegen 里当 SSA 值 | scottmcm 的 [#123886](https://github.com/rust-lang/rust/pull/123886)、[#156116](https://github.com/rust-lang/rust/pull/156116)（2026） | 在 `rustc_codegen_ssa` 中减少 alloca，不是新 IR |
| move 消除 | [项目目标 2025h2](https://rust-lang.github.io/rust-project-goals/2025h2/mir-move-elimination.html)：需要改变 MIR 的 `move` 语义 | 进行中 |
| cg_clif / cg_gcc | [LWN](https://lwn.net/Articles/964735/)、[cg_gcc 进度报告](https://blog.antoyo.xyz/rustc_codegen_gcc-progress-report-36) | 直接消费 MIR，未见自己的 Rust 级优化层 |

**学术工作（都面向验证）**

| 项 | 来源 | 能否用于优化 |
|---|---|---|
| Aeneas / LLBC | Ho & Protzenko, [ICFP '22](https://dl.acm.org/doi/10.1145/3547647)；借用检查后续 [ICFP '24](https://dl.acm.org/doi/10.1145/3674640) | 否：把安全 Rust 翻译成纯函数代码供证明 |
| RustHorn | [TOPLAS 2021](https://dl.acm.org/doi/10.1145/3462205)：MIR → CHC | 否 |
| Kani | [ICSE-SEIP '22](https://cs.wellesley.edu/~avh/dyn-trait-icse-seip-2022-preprint.pdf)：MIR → Goto-C → CBMC | 否 |
| Prusti / Creusot | [Prusti 项目](https://www.researchgate.net/publication/360716882_The_Prusti_Project_Formal_Verification_for_Rust)：分别到 Viper、WhyML | 否 |
| 基于 SB 近似的 MIR 优化 | ETH 硕士论文（2023-07，[PDF](https://ethz.ch/content/dam/ethz/special-interest/infk/chair-program-method/pm/documents/Education/Theses/Janis_Peyer_MS_Thesis.pdf)） | 做了优化但“未考虑运行时改进”，没有数字 |

**其他 Rust 编译器**

| 项 | 来源 | 结构 |
|---|---|---|
| gccrs | [Rust 博客 2024-11](https://blog.rust-lang.org/2024/11/07/gccrs-an-alternative-compiler-for-rust/) | AST → HIR → TyTy → GENERIC，优化交给 GCC；自有 BIR 只用于生成 Polonius 事实 |
| mrustc | [README](https://github.com/thepowersgang/mrustc/blob/master/README.md) | HIR → MIR → C；“MIR 优化（减轻 C 编译器负担）”；优化收益数据未找到原始来源 |
| rustc_codegen_clr | [2025-04 文章](https://fractalfir.github.io/generated_html/cg_clr_odd_platforms.html) | MIR → CIL 或 C；core 测试通过率 95.9%（C 后端）；无 Rust 级优化 IR |
| Ferrocene | [ferrocene/ferrocene](https://github.com/ferrocene/ferrocene) | 上游 rustc + 资质认证，不是新 IR |
| Rust → Wasm/JVM | 未找到原始来源 | — |

**相近语言的语义级 IR**

| 项 | 来源 | 与后端的分工 / 数据 |
|---|---|---|
| Swift SIL | [WMO 博客 2016](https://www.swift.org/blog/whole-module-optimizations/) | SIL 优化器做泛型特化、内联、ARC 优化，“约占编译时间三分之一”，其余主要是 LLVM；WMO 性能“最多好 5 倍” |
| Carbon SemIR | [Euro-LLVM 2024 闪电演讲](https://llvm.org/devmtg/2024-04/slides/LightningTalks/Smith-Carbons-high-level-semanticIR.pdf) | SSA，定长指令 16 字节，降到 LLVM IR；泛型去重见 generic-sharing.md |
| Hylo IR | [lowering 文档](https://docs.hylo-lang.org/hylo-ir/concepts/lowering) | SSA，在 IR 上做所有权/借用检查，后端转 LLVM IR |
| Mojo KGEN | [Mojo vision](https://mojolang.org/docs/vision) | 基于 MLIR Core，“在实例化之前表示显式参数化代码”，带来“更快编译、更清楚的错误、同一源码编到多种设备”；所有权方言未找到公开资料 |
| Zig ZIR/AIR + 自有 x86 后端 | [0.15.1 发布说明](https://ziglang.org/download/0.15.1/release-notes.html)；[Sema 说明](https://mitchellh.com/zig/sema) | 每个实例一份 AIR（不做多态优化）；debug 默认自有后端，编译“约 5 倍快”，但“生成的机器码比 LLVM 慢”；行为测试 1984/2008 vs LLVM 1977/2008 |
| Go SSA 后端 | [Go 1.7 发布说明](https://go.dev/doc/go1.7) | 不用 LLVM；新 SSA 后端使基准 CPU 时间降 5–35% |

## 2. 关键结论

- 没有找到“多态 Rust 语义 SoN + 按答案特化”的先例。最接近的分散在：rustc MIR（多态体优化一次，但不是 SSA、不做单态后优化）、Swift SIL（语义 IR 上做特化，但后面仍是 LLVM）、Mojo KGEN（参数化代码在实例化前表示）、Carbon（前端按指纹合并实例）。
- 所有 Rust 的替代编译器/后端都直接消费 MIR 或更早的 IR，把优化交给 GCC/LLVM/C 编译器；没有一个有 Rust 级优化收益的公开数据。
- 自有后端的已知代价：Zig、cg_clif 都只在 debug 场景替代 LLVM，并明说代码更慢；Go 是唯一长期不用 LLVM 且面向 release 的例子，其运行时间与 LLVM 系的对比未找到权威原始来源。

## 3. 对 kfrc 的结论

| 借鉴 / 避开 | 结论 | 理由 |
|---|---|---|
| 语义 IR 承担特化和内联，后端只做机器级 | **采纳（已一致）** | Swift SIL 的分工；SIL 占约 1/3 编译时间可作为 kfrc R+S 级的参照预算 |
| 参数化代码在实例化前表示 | **采纳（已一致）** | Mojo KGEN 的公开定位与 R 级相同；无公开数据 |
| 紧凑定长节点 | **改编** | Carbon SemIR 16 字节/指令；kfrc 的 arena 节点可设大小目标，给 V8 的局部性问题留余量 |
| 只为 debug 替代 LLVM | **避开这条路的假设** | kfrc 的 M6 要求运行时间不退化，没有可借的先例；runtime.md 的“必须自己做的部分”是主要风险，不能靠 Zig/cg_clif 的经验估计 |
| 验证工具的 IR（LLBC 等） | **不采纳** | 面向证明，不保留优化需要的成本模型；可作为 M3 借用/drop 语义对照 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [levels.md](../../design/ir/levels.md) | 为什么不是三种 IR | 先例补 Swift SIL（语义 IR 上做特化/内联，约 1/3 编译时间）与 Mojo KGEN（实例化前表示参数化代码）；注明“未发现与 R/S/L 相同的公开实现” |
| [runtime.md](../../design/goals/runtime.md) | 必须自己做的部分 | 加一行说明：已知的自有后端（Zig x86、cg_clif）只在 debug 场景替代 LLVM，均承认代码更慢；没有 release 级先例，T38 的对拍结果是唯一依据 |
| [compile-time.md](../../design/goals/compile-time.md) | 指标与闸门 | 记录 R+S 级占总编译时间的比例，以 Swift SIL 的约 1/3 为参照 |

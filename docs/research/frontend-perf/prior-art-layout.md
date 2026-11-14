# 快速前端先例（一）：数据布局与按文件的早期阶段

Zig、Carbon、Kotlin K2、Sorbet。来源都是一手资料，除非标“二手”；“推算”是对一手数字做的除法。调研日期 2026-11-08。

## 1. 机制与数字

| 项目 | 做法 | 数字 | 来源 |
|---|---|---|---|
| Zig 内存布局（0.8，2021） | token = 1 字节 tag + 4 字节偏移，结束位置重新词法分析得到；AST 节点 = tag + main_token + 两个 u32 子节点共 13 字节，少见字段放 `extra`；全部 struct-of-arrays | 解析器 wall −22%、指令 −28%；ZIR 重做 0.93 → 0.57 s（−39%），峰值内存 645 → 386 MiB | [0.8.0 notes “Reworked Memory Layout”](https://ziglang.org/download/0.8.0/release-notes.html#Reworked-Memory-Layout) |
| 同上（演讲版） | — | token 64 → 5 B；AST 节点平均 120 → 15.6 B；ZIR 指令平均 54.0 → 20.3 B | Kelley, “Practical Data-Oriented Design”，Handmade Seattle 2021（[视频](https://vimeo.com/649009599)；据转录，二手） |
| Zig AstGen / ZIR | 整个文件 → 无类型 IR，只依赖本文件 AST；按文件并行；ZIR 按文件内容缓存，未变文件跳过词法、语法、AstGen；ZIR 是 4 个数组，读写近乎拷贝 | std 292,458 行：release 冷 33 ms、热 18 ms（8 核 i9），即约 8.9M 行/s | 同上 “Whole-File AST Lowering” |
| Zig 自托管 + InternPool | 0.10 换掉 C++ 实现；0.11 类型与值统一驻留为索引，现按线程分片 | 自举 43 → 40 s，峰值内存 9.6 → 2.8 GiB | [0.10.0](https://ziglang.org/download/0.10.0/release-notes.html#Self-Hosted-Compiler)、[0.11.0](https://ziglang.org/download/0.11.0/release-notes.html) notes |
| Zig 惰性 Sema | 只分析被引用的顶层声明；未引用函数的**类型错误不报**；AstGen 级错误（未用局部等）对所有文件都报 | 语义层面的取舍，无数字 | [langref](https://ziglang.org/documentation/master/)（C translation 节） |
| Zig 增量 | 分析单元 = 类型布局/声明类型/const 值/函数体 | 50 万行首次 14 s、改动后 63 ms；一次 37 ms 更新中约 30 ms 在重算引用图 | 已收入 [prior-art/ct-incremental-cache.md](../prior-art/ct-incremental-cache.md) |
| Carbon 工具链 | 各阶段只用向量存储 + 32 位 id；解析树按**后序**平铺；check 按顺序读一遍后序数组，像解释字节码；解析器非递归；token 8 B、解析节点 8 B、SemIR 指令 16 B（源码 `static_assert`） | 262,144 行声明密集文件、单线程：lex 65.0 ms、lex+parse 109 ms、到 check 335 ms（PR #4307，2024），推算约 4.0M / 2.4M / 0.78M 行/s；2026-10 lex 46.54 ms（PR #8031） | [toolchain/docs](https://github.com/carbon-language/carbon-lang/blob/trunk/toolchain/docs/README.md)、[#4307](https://github.com/carbon-language/carbon-lang/pull/4307)、[#8031](https://github.com/carbon-language/carbon-lang/pull/8031) |
| Carbon 目标 | 每行预算 | lex+parse 10M 行/s，check 1M 行/s，lowering 0.1M 行/s | Carruth CppNow 2023（据转录，已收入 [ct-fast-compile.md](../prior-art/ct-fast-compile.md)） |
| Carbon 微优化 | 写入时用对写方便的布局，读前一次性转成 SoA | token 种类/标志 SoA 重排后 parse −6.5…−9.3%（#8044）；parser 冷热拆分 −5.8…−7.3%（#8053） | PR 正文 |
| Kotlin K2（FIR） | K1 = PSI 语法树 + `BindingContext` 旁表；K2 = 一棵带语义的树；按固定阶段解析（签名阶段全部完成后才 BODY_RESOLVE），编译器里顺序执行、IDE 里按声明惰性执行 | Anki-Android 冷构建 57.7 → 29.7 s；分析阶段 +156%（Exposed）、+194%（Anki）的速度提升；增量分析 0.581 → 0.122 s | [JetBrains 2024-04](https://blog.jetbrains.com/kotlin/2024/04/k2-compiler-performance-benchmarks-and-how-to-measure-them-on-your-projects/)、[fir-basics.md](https://github.com/JetBrains/kotlin/blob/master/docs/fir/fir-basics.md) |
| Sorbet（Ruby） | 32 位驻留 id、平铺符号表；全局表冻结后，各方法只做前向局部推导，在只读全局状态上任意并行 | 约 10 万行/s/核；推导约占一半时间 | [Elhage 2020](https://blog.nelhage.com/post/why-sorbet-is-fast/) |
| Jai | 自写 x64 后端 + 编译期执行 | 8 万行约 1.5 s（笔记本，2018 演示）；1M 行/s 是目标；无分阶段数字 | 全部二手（[D 论坛转述](https://forum.dlang.org/thread/qswymtvohqphbvzcgmwx@forum.dlang.org)） |

## 2. 共性

1. **紧凑、按索引、struct-of-arrays 的存储单独就值 20–40%/阶段**（Zig 解析 −22%、ZIR −39% 与内存 −40%；Carbon 每个布局 PR 5–10%）。K2 把“树 + 旁表”合成一棵语义树，分析阶段约 2.5–3 倍。
2. **不依赖语义的早期阶段按文件并行、按内容缓存**（Zig AstGen，Carbon lex/parse）。Rust 的障碍是宏和 `cfg` 必须在知道条目之前运行，可缓存单位只能是**展开前**的文件。
3. **签名阶段先于函数体全部完成**（K2 阶段不变量、Sorbet 冻结全局表）是函数体并行检查的前提。rustc 的查询模型允许 typeck 中途按需拉签名，所以并行时要靠锁和等待。
4. **惰性语义分析会改变报错语义**（Zig）。Rust 必须检查所有函数体（`cargo check` 要报死代码里的错），惰性只适用于依赖的 metadata 和 codegen。

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| token/AST/Body 用 u32 索引 + SoA，少见字段外置 | **采纳** | Zig/Carbon 的实测量级；kfrc 自写数据模型，没有兼容包袱 |
| 位置信息放旁表，节点只存 token 下标 | **采纳** | Zig 由 token 偏移重算结束位置；与 [design/build/cache.md](../../design/build/cache.md) 的“代码身份不含源位置”一致 |
| 每阶段吞吐预算（ns/行、ns/节点） | **改编**：作内部回归指标，不作承诺 | Carbon 预算来自演讲；Rust 的展开/推导成本与代码形态强相关 |
| 一遍后序 check（Carbon） | **不采纳** | Rust 条目无序、glob 与宏需要不动点解析、推导双向 |
| 签名阶段完成后冻结全局表，函数体只读并行 | **采纳** | K2、Sorbet；对应 kfrc 的“语义快照不可变”（[design/ids.md](../../design/ids.md)） |
| 未引用函数不做语义检查（Zig） | **不采纳** | 改变接受/拒绝结果，违反对拍红线 |
| 展开前文件级缓存（token tree + 条目骨架） | **改编**：只用于增量 | 冷构建每个文件只解析一次，缓存无收益 |

## 4. 未找到原始来源

- Jai 的任何官方速度数字（只有论坛转述与推文图片）。
- Carbon NDC Toronto 2026 演讲中“在 Apple M4 上达到 10M 行/s lex、1M 行/s check”的说法，只见摘要，未核对视频。
- Kelley 2021 演讲中的字节数只来自第三方转录；正文数字改用 0.8.0 release notes。
- Roslyn、javac 把前端提速归因于数据布局或惰性的一手数字。

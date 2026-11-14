# 编译时间：并行与流水线

对照 [compile-time.md](../../design/goals/compile-time.md#关键路径) 的关键路径和 [build-pipeline.md](../../design/build/pipeline.md)。增量与缓存键见 [ct-incremental-cache.md](ct-incremental-cache.md)，不在本文展开。

## 1. 是什么

| 项目 | 年份 | 来源 | 要点 |
|---|---|---|---|
| cargo 流水线（rmeta 先发） | 2019（1.38） | Crichton, [Evaluating pipelined rustc compilation](https://internals.rust-lang.org/t/evaluating-pipelined-rustc-compilation/10199)；[#62766](https://github.com/rust-lang/rust/pull/62766) | 依赖 crate 写出 `.rmeta` 后下游即可开始；rmeta 在整个 crate 的类型/借用检查之后才写，仍是整 crate 屏障；二进制 crate 不能流水 |
| 更早写 metadata（#64112） | 2019 | Nethercote, [blog 2019-10](https://blog.mozilla.org/nnethercote/2019/10/11/how-to-speed-up-the-rust-compiler-some-more-in-2019/) | 把 metadata 写到类型/借用检查之前；“wasn't much of a win”，且推迟报错，放弃 |
| rustc 并行前端 `-Zthreads` | 2023 nightly；目标 2024h2–2026 | [Inside Rust 2023-11](https://blog.rust-lang.org/2023/11/09/parallel-rustc/)；goals [2024h2](https://goals.rust-lang.org/2024h2/parallel-front-end.html)、[2025h1](https://goals.rust-lang.org/2025h1/parallel-front-end.html)、[2025h2](https://goals.rust-lang.org/2025h2/parallel-front-end.html)、[2026 Fast Builds](https://goals.rust-lang.org/2026/roadmap-fast-builds.html)；[dev guide](https://rustc-dev-guide.rust-lang.org/parallel-rustc.html) | rustc 自带的 rayon fork + 查询缓存表加锁，同一查询由别的线程计算时阻塞等待；typeck/borrowck/MIR 优化并行，词法解析、HIR lowering、宏展开仍串行；环由专门的 deadlock handler 线程检测。2025h2 结束时未稳定，2026 继续 |
| CGU 并行 codegen | 现状 | [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) | 非增量 16 个、增量 256 个 CGU；CGU 越多越并行，“may produce slower code” |
| headstart（early-rmeta 原型） | 2026-09 | [PowderworksCode/headstart](https://github.com/PowderworksCode/headstart)（README、docs/design.md） | 新查询把分析拆成“接口”和“函数体”；接口检查完写 `.early-rmeta`，下游据此分析，codegen 前换成完整 metadata；等待时归还 job slot。作者在 docs/readiness.md 中自评“可以开设计讨论和 draft PR，尚不能合入”（需要 MCP，16 核数字需重测） |
| 惰性 rustc / MIR-only rlibs | 2024 / 2016–2025 | Lattimore, [Speeding up rustc by being lazy](https://davidlattimore.github.io/posts/2024/06/05/speeding-up-rustc-by-being-lazy.html)；[#38913](https://github.com/rust-lang/rust/issues/38913)；[#119017](https://github.com/rust-lang/rust/pull/119017) | 库只发检查所需的 rmeta 和 MIR，codegen 推迟到二进制 crate、只生成可达代码。#38913 已关闭，实验 PR #119017 于 2025-01 关闭 |
| Swift batch mode | 2018（4.2） | Hoare, [Forums 2018-03](https://forums.swift.org/t/compilation-speed-help-test-batch-mode/10964) | 文件动态分到“每核一批”，一批一个前端进程；保留增量，同时减少 single-file 模式的重复工作 |
| Swift emit-module-separately | 2022（5.6/5.7） | Laferrière, [Forums 2022-06](https://forums.swift.org/t/faster-compilation-through-parallel-module-build/58043) | 单独 job 生成 `.swiftmodule`（惰性解析函数体、跳过非 inlinable 体），下游不等目标代码 |
| Swift/Clang explicit modules | 2024（Xcode 16） | [WWDC24 10171](https://developer.apple.com/videos/play/wwdc2024/10171/) | 扫描 → 构建模块 → 编译三阶段，隐式模块构建变成构建系统的显式任务，调度器不再被 import 阻塞 |
| Clang C++20 模块 | 两阶段 Clang 16 起；Reduced BMI 19 实验、22 默认 | [StandardCPlusPlusModules](https://clang.llvm.org/docs/StandardCPlusPlusModules.html) | 两阶段（`--precompile`）比一阶段并行度高：B 依赖 A 时，A.pcm→A.o 与 B 同时编译；Reduced BMI 不含非 inline 函数定义；`clang-scan-deps` 先扫出 P1689 依赖 |
| Go export data | unified IR 1.20 默认；函数级并行后端 1.9 | [cmd/compile README](https://go.dev/src/cmd/compile/README)；[Go 1.9 notes](https://go.dev/doc/go1.9)；Conrod, [Export data](https://jayconrod.com/posts/112/export-data--the-secret-of-go-s-fast-builds)（2020） | 下游只读直接依赖的 export data：导出类型、可内联体、泛型体、参数逃逸摘要，带索引惰性解码；`-linkobj` 可把 export data 与目标代码拆成两个文件 |
| Zig | 0.14（2025）/ 0.15（2025-08） | [0.14 notes](https://ziglang.org/download/0.14.0/release-notes.html)；Lugg, [devlog 2025-06-14](https://ziglang.org/devlog/2025/)；[#21165](https://github.com/ziglang/zig/issues/21165)；[0.7.1 docs](https://ziglang.org/documentation/0.7.1/) | 1 个 Sema 线程 + N 个 codegen worker（AIR→MIR，按函数）+ 1 个链接线程；LLVM 后端仍单线程；`-fincremental` 实验中；顶层声明惰性分析，不被引用就不分析 |
| Java/Kotlin 头编译 | 现状 | [Bazel JavaInfo](https://bazel.build/rules/lib/providers/JavaInfo)；[turbine](https://github.com/google/turbine)；[rules_kotlin CompileAvoidance](https://github.com/bazel-contrib/rules_kotlin/blob/master/CompileAvoidance.md) | 无 API 注解处理器时直接从源码生成 header jar（只有签名），“reducing critical path”；Kotlin 用 jvm-abi-gen 生成 ABI jar |
| Buck2 Rust | 现状（只有源码） | [prelude/rust/build_params.bzl](https://github.com/facebook/buck2/blob/main/prelude/rust/build_params.bzl) | `metadata-fast`（`--emit metadata`，更快，不能用于流水线构建）与 `metadata-full`（`--emit link,metadata`）分开 |

## 2. 关键证据

| 比较 | 数字 | 来源 |
|---|---|---|
| cargo 流水线，wasm-bindgen 宏 crate | debug 10.28→7.88 s（−23%），release 19.25→10.49 s（−46%） | Crichton 2019，帖 #25 |
| cargo 流水线总体 | “speedups of up to 1.8x” | [Nethercote 2023-08](https://nnethercote.github.io/2023/08/25/how-to-speed-up-the-rust-compiler-in-august-2023.html) |
| #64112：单一 metadata 提前写 | 最多 1.07×（**二手来源**：headstart design.md；原文只写“not much of a win”）。原因：该文件要带下游 codegen 所需的 MIR，提前写仍要做大部分分析 | Nethercote 2019；headstart |
| 并行前端，8 线程 | 最多 −50%；单线程模式慢 0–2%；内存最多 +35%；cargo 末端 crate 前端 10.2→5.9 s，后端 6.2→5.3 s | Inside Rust 2023-11 |
| 并行前端，2025h2 | 整体 −20~30+% | goals 2025h2 |
| 并行前端的锁与死锁 | 超过 4 线程出现锁争用；≥16 线程数据争用；“rayon … difficult to fundamentally eliminate the deadlock problem”；2025h1 修复 deadlock handler 自身引起的死锁并改 work-stealing 调度 | dev guide；goals 2024h2/2025h1/2025h2 |
| headstart，16 核，13 个真实项目 | `cargo check` 最多 −54%，`cargo build` 最多 −42%，无变慢；在 `-Zthreads=8` 之上再最多 −25%；4 核时 rust-analyzer check −24%、build −13–15%，宽依赖图持平 | headstart README |
| headstart 的代价 | 报错延迟中位 4 ms、p99 249 ms；峰值内存上升；暂停的编译数无上限；恢复的编译不等 job slot，饱和时可能拖慢关键路径 | headstart design.md#Risks |
| 链接丢弃的已编译代码 | ripgrep 17%，evcxr 35% | Lattimore 2024 |
| MIR-only rlibs 失败原因 | proc-macro 依赖与 libstd dylib 共享单态；用户代码依赖地址稳定；“always be a second-class approach” | #119017 最终状态 |
| Zig 并行 codegen（x86 自写后端） | wall −5%~−50%；自举 13.8→10.0 s，峰值 RSS +25.7% | devlog 2025-06-14 |
| Zig 0.14 codegen 分线程 | 自举 12.8→8.57 s | 0.14 notes |
| Swift emit-module-separately | CPU 比 merge-module 多但更可并行；WMO 下“doubles some work but unblocks dependents earlier”，因而 WMO 默认关闭 | Forums 2022 |
| Bazel header jar（Snowflake） | 9,000 文件：header jar 13 s，再分约 100 片并行编译，>10 min → 约 2 min | [Snowflake 2025-03](https://www.snowflake.com/en/blog/engineering/fast-reliable-builds-snowflake-bazel/) |
| header jar 的反面（Gradle） | header jar 生成“much slower than compilation, resulting in a net penalty”；ABI 改动时 Bazel “much slower”。无具体数字，且为竞品方 | [Gradle 2023-08](https://blog.gradle.org/our-approach-to-faster-compilation) |

## 3. 对 kfrc 的结论

| 设计点 | 结论 | 理由 |
|---|---|---|
| C4 接口先于函数体优化发布，接口/R 图/机器码分开就绪 | **采纳**（维持） | 成败分界很清楚：#64112 的单一 metadata 带 MIR，几乎无收益；headstart 把只含分析所需的接口单独发出，build 最多 −42%；Clang Reduced BMI、Swift `.swiftmodule`、header jar 都是“小接口先发”。kfrc 的 −28.5% 模型与 headstart 量级一致 |
| 接口由同一编译过程产出，不另跑一个头编译器 | **采纳** | turbine 式独立工具要重复解析（Gradle 称净损失）；Swift 在 WMO 下 doubles some work |
| “接口体”先于接口发布 | **改编** | headstart：opaque 类型、const/const fn、async 状态机、static 的体必须在 early 写之前检查。build-pipeline.md 只写了 `impl Trait`，缺 const 和 async 的 auto trait 泄漏 |
| 跨 crate 逐体/逐实例等待 | **改编**：同进程内逐体；跨进程首版两道屏障（接口、R 图集合） | 跨进程逐体等待**未找到先例**；headstart 也只做“early → full”两道屏障。rustc 进程内按查询阻塞的经验是死锁要靠专门线程检测、>4 线程出现锁争用 |
| 等待集合的无环性 | **采纳**：静态分层 + SCC 一起发布（已有），不做运行时死锁检测 | rustc 并行前端多年卡在死锁，且 rayon 难根除 |
| 图之间并行、单图串行 | **采纳**（维持） | Go 1.9 函数级并行后端、Zig 按函数的 codegen worker 都是这一粒度；LLVM 共享状态多，Zig 的 LLVM 路径因此仍单线程 |
| 不要有串行的“喂料”阶段 | **采纳** | rustc 中 MIR→LLVM IR 串行造成后端“staircase”；Zig 0.14 把链接输入解析前移以消除末端瓶颈 |
| C6 二进制 crate 生成 + 库 crate 预生成 | **改编** | Lattimore 提案与 C6 同向，并指出新增的等待点会有 straggler；MIR-only rlibs 因 proc-macro/dylib 边界失败。kfrc 需对 proc-macro、dylib、cdylib/staticlib 边界整体生成，不推迟 |
| 只分析可达体（Zig 式惰性分析） | **不采纳** | 只推迟 codegen；语义检查覆盖所有体，与 rustc 一致（Zig 因此对未用函数不报错） |
| 等待时归还 jobserver token | **采纳并补规则** | headstart 证明有效，也暴露缺口：恢复不等 token、暂停数不设上限 |
| 叶子/二进制 crate 不单独提前发布接口 | **采纳** | 无下游时只增加工作（Swift WMO 默认关闭 emit-module-separately 的同一理由） |
| 确定性：线程数不改变输出 | **采纳**（维持） | 并行前端设计公理“Consistent”，输出不同即 bug；headstart 跨 crate 的诊断/JSON 顺序会变 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [compile-time.md](../../design/goals/compile-time.md) | 关键路径 | 补一段先例：rmeta 流水线最多 1.8×；#64112 失败于“早期产物带 MIR”；headstart 接口单独先发 −42%（16 核）。写明 C4 的前提是接口产物不含任何函数体图。逐体等待改写为“同进程内逐体；跨 crate 首版两道屏障（接口就绪、所需 R 图就绪），逐体跨进程等待在 M5 实测关键路径后再决定” |
| [compile-time.md](../../design/goals/compile-time.md) | 关键路径（C6 段） | 加“必须整体生成的边界”：proc-macro、dylib、cdylib/staticlib；预生成在二进制 crate 前完成的比例作为指标，防止 straggler |
| [build-pipeline.md](../../design/build/pipeline.md) | 产物依赖（两条依赖规则之后） | 新增第三条：“接口体”——返回 opaque 类型的函数、`async fn`/async 块（auto trait 泄漏）、const/const fn/数组长度、static——在接口发布前完成检查；其余体不阻塞接口 |
| [build-pipeline.md](../../design/build/pipeline.md) | CPU 与内存预算 | 补三条：恢复的任务重新取 token，或在有界额度内优先；暂停任务数和其占用字节计入内存预算并设上限；无下游的 crate 不单独提前发布接口 |
| [build-pipeline.md](../../design/build/pipeline.md) | 确定性 | 补“提前发布不改变诊断”：本 crate 的错误按定义顺序排序后输出；用 headstart 式三场景（干净、依赖出错、bin 出错）对比提前发布开/关的诊断、退出码和产物 |
| [evidence.md](../evidence.md) | §5 | 在“签名优先的流水线模型 −28.5%”后加外部旁证：headstart 13 个项目 build 最多 −42%、check 最多 −54%（16 核，4 核时收益减小）；#64112 的反例 |
| [plan/README.md](../../plan/README.md#验收) | 编译时间 | 记录核数（4/8/16 各测）和依赖图形状（深/宽）；增加参考臂 rustc `-Zthreads=8`（只作解释，不作基线）；报告峰值内存，与并行前端的 +35% 对照 |
| [compile-time.md](../../design/goals/compile-time.md) | 指标与闸门 | 关键路径构成增加“等待时间”分项：等接口 / 等 R 图 / 等 token / 暂停中；记录暂停任务的峰值数量 |

**未找到原始来源：** `go build` 是否在 export data 写出后、目标代码完成前启动下游编译；任何编译器在进程间按单个函数体等待上游的实现；Swift explicit modules、Clang 两阶段编译、Kotlin ABI jar 的端到端构建时间数字；turbine 的发布年份；Buck2 的 Rust 流水线官方文档（只有 prelude 源码注释）。

# 编译时间：不经 LLVM 的快速编译实践

对照 [compile-time.md](../../design/goals/compile-time.md) 和 [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#l-级) 的 L 级 → 机器码一段。寄存器分配算法与质量见 [ct-backend.md](ct-backend.md)，并行与流水线、Go export data 见 [ct-parallel-pipeline.md](ct-parallel-pipeline.md)，增量缓存键见 [ct-incremental-cache.md](ct-incremental-cache.md)，这里不展开。

## 1. 是什么

| 项目 | 年份 | 来源 | 要点 |
|---|---|---|---|
| Zig 自写 x86_64 后端 | 2025（0.15 起 Debug 默认） | [devlog 2025-06-08](https://ziglang.org/devlog/2025/#2025-06-08)；[0.15.1 notes](https://ziglang.org/download/0.15.1/release-notes.html) | AIR → MIR（与机器指令近 1:1）→ 编码，不经 LLVM；x86_64 Debug 默认（Windows、NetBSD、OpenBSD 因链接器仍用 LLVM）；官方说明生成码慢于 LLVM；Release 仍走 LLVM |
| Zig 增量编译与原地写入 | 0.14（2025，实验）→ 2026 | [0.14.0 notes](https://ziglang.org/download/0.14.0/release-notes.html)；Lugg, [Inside Zig's Incremental Compilation](https://mlugg.co.uk/posts/incremental-compilation-internals/)（2026-07-28） | 分析单元：类型布局、声明类型、const 值、函数体；codegen 只重跑变了的函数，不缓存 AIR/MIR；链接器与编译器一体，`MappedFile` 把输出文件分成节点，函数节点按指数增长留余量，原地写入新代码，移动的节点标 dirty 后重放重定位 |
| Go gc 编译器 | 2012 设计；2016 SSA 后端（1.7） | Pike, [Go at Google](https://go.dev/talks/2012/splash.article)（2012）；[Go 1.7 notes](https://go.dev/doc/go1.7)；Cox, [golang-dev 2016-01-28](https://groups.google.com/g/golang-dev/c/TerfinvPffw/m/wvrEefWvBwAJ) | 慢构建是 Go 立项的首要痛点；单一后端，debug 只是 `-N -l` 关优化和内联（[cmd/compile](https://go.dev/src/cmd/compile/doc.go)）；SSA 后端以编译时间换运行时间，合并前先追回编译时间 |
| Carbon | 2023 | [goals](https://docs.carbon-lang.dev/docs/project/goals.html)；[toolchain README](https://github.com/carbon-language/carbon-lang/blob/trunk/toolchain/docs/README.md)；Carruth, [CppNow 2023 演讲](https://www.youtube.com/watch?v=ZI198eFghJk) | 语法有界前瞻、解析不用语义信息、支持分离编译；工具链按“每行耗时预算”设计；debug 构建优先诊断和构建速度 |
| Jai | 2018– | 无官方文档；社区书 [The Way to Jai](https://github.com/Ivo-Balbaert/The_Way_to_Jai/blob/main/book/04A_More_info_about_the_compiler.md)（二手来源） | 自写 x64 后端快而不优化，LLVM 后端做优化构建 |
| rustc_codegen_cranelift | 2020– | [Inside Rust 2020-11-15](https://blog.rust-lang.org/inside-rust/2020/11/15/Using-rustc_codegen_cranelift/)；[2025h2 goal](https://rust-lang.github.io/rust-project-goals/2025h2/production-ready-cranelift.html)；[2025-12 更新](https://blog.rust-lang.org/2026/01/05/project-goals-2025-december-update/) | rustc 的 Cranelift 后端，面向 debug 构建；2025h2 旗舰目标因无资金“Will not complete” |
| Cranelift 后端效率 | 2021 | Fallin, [Cranelift, Part 2](https://cfallin.org/blog/2021/01/22/cranelift-isel-2/) | `MachBuffer` 在发射时线性时间做分支化简与跳转缩短；块降低顺序一次确定，不另跑 CFG 清理 pass |
| TPDE | arXiv 2025-05；CGO 2026（pp. 427–439） | Schwarz, Kamm, Engelke, [arXiv 2505.22610](https://arxiv.org/abs/2505.22610)；[CGO 版 PDF](https://home.cit.tum.de/~engelke/pubs/2602-cgo1.pdf)；[源码](https://github.com/tpde2/tpde) | 一遍分析（循环树 + 粗粒度活跃性，循环内的值整段活跃）后，**单遍**完成指令选择、寄存器分配、编码；指令模板用高级语言写、经 LLVM MIR 提取；通过 IR 适配器接 LLVM IR、CLIF、Umbra IR |
| Copy-and-patch | 2021 | Xu & Kjolstad, [OOPSLA 2021](https://doi.org/10.1145/3485513)（[摘要页](https://2021.splashcon.org/details/splash-2021-oopsla/40/Copy-and-Patch-Compilation-A-Fast-Compilation-Algorithm-for-High-Level-Languages-and)） | 预先由 clang/LLVM 编译出带“孔”的二进制模板（stencil），编译时拷贝并修补常量、跳转目标、地址 |
| CPython JIT | 2024（3.13）→ 2026 | [PEP 744](https://peps.python.org/pep-0744/)；[3.13 What's New](https://docs.python.org/3.13/whatsnew/3.13.html)；[3.15 What's New](https://github.com/python/cpython/blob/3.15/Doc/whatsnew/3.15.rst)；[SC 公告 2026-06-05](https://discuss.python.org/t/an-announcement-from-the-steering-council-regarding-the-jit-project/107638)；[PEP 836](https://peps.python.org/pep-0836/) | 微操作的 copy-and-patch；LLVM 只是构建期依赖；2026-06 指导委员会暂停新开发，PEP 836 于 2026-10-19 撤回，JIT 预计移出 main（移除报道见 [LWN](https://lwn.net/Articles/1102136/)，二手来源） |

## 2. 关键证据

| 证据 | 数字 | 来源 |
|---|---|---|
| Zig 自写后端 vs LLVM（Debug） | hello world wall 918 → 275 ms（−70.1%），峰值内存 214 → 137 MB；编译 Zig 编译器 75 → 20 s | devlog 2025-06-08 |
| Zig 0.15 发布口径 | Debug 编译约 5× 快；behavior 测试自写后端 1984/2008，LLVM 后端 1977/2008 | 0.15.1 notes |
| Zig 增量（不出二进制） | 50 万行代码首次检查 14 s，改动后重分析 63 ms | 0.14.0 notes |
| Zig 增量（出二进制，x86_64-linux） | Fizzy 首次约 5 s，每次改动 50–70 ms；一次 37 ms 的更新中 sema+codegen+link 约 1.6 ms，约 31 ms 在每次全量遍历引用图 | Lugg 2026-07 |
| Go SSA 后端引入 | 合并时整体编译慢约 2×，“不是根本性的”；1.7 发布时基准程序 CPU 时间 −5–35%，编译器“比 1.6 显著快”（无百分比） | Cox 2016-01；Go 1.7 notes |
| Go 的立项数据 | 2007 年一个 Google C++ 二进制：4.2 MB 源码展开成 8 GB 编译器输入（约 2000×）；Go 的扇出约 40×，比 C++ 好 50 倍 | Pike 2012 |
| Carbon 目标 | 词法+语法 10M 行/s（约 100 ns/行），语义检查 1M 行/s（约 1 µs/行），降到二进制 0.1M 行/s；官方 goals 文档无数字；工具链优先级：正确性 > 生成码质量 > 编译性能 | 演讲 24:28–30:08（据[转录](https://readray.ai/notes/1048442)）；toolchain README |
| Jai | 目标 1M 行/s；beta 0.0.045 CHANGELOG 称 x64 后端约 250k 行/s | 二手来源（社区书） |
| cg_clif vs LLVM（Cargo，dev） | 全量 49.93 vs 54.64 s；增量 7.98 vs 5.48 s（LLVM 反而快，因 proc-macro 在 cg_clif 下更慢） | Inside Rust 2020-11 |
| cg_clif 现状 | “真实改动中 codegen 的时间比预期少”，因此 cranelift 对用户的收益更小，不承诺数字 | goals 2025-12 更新 |
| TPDE vs LLVM -O0 后端（SPEC CPU2017，未优化 IR） | 后端编译快 8–26×（几何平均 x86-64 13.88×、AArch64 18.29×）；运行时间 ±9%；代码大 22%/16%；LLVM 后端实现 7.7 kLOC | TPDE CGO 版 §V（arXiv v1 摘要为 8–24×） |
| TPDE 优化 IR 输入 | 比 LLVM -O1 后端快 106×/107×；生成码比 -O1 后端慢 1.50×/1.71×（寄存器分配和指令选择差） | 同上 |
| TPDE 端到端 | 含前端后后端只占 1%，整体编译只快 9–24% | 同上 |
| 同论文的 copy-and-patch LLVM 后端 | 比 -O0 快 19.56×（比 TPDE 快约 1.4×），但运行慢 2.32×、代码大 4.27×：模板固定寄存器带来大量 move/spill，且没有活跃性 | 同上 |
| TPDE 用于 CLIF | 比 Cranelift 快 4.94×（比其单遍分配器配置快 3.10×），比 Winch 慢 1.53×；生成码比 Cranelift 优化分配器慢 1.58× | 同上 §VI |
| Copy-and-patch 原论文 | TPC-H：编译比 LLVM -O0 快两个数量级，代码比 -O0 快 14%；Wasm：编译比 Liftoff 快 4.9–6.5×，运行快 39–63% | OOPSLA 2021 摘要 |
| CPython 3.13 JIT | 与专化解释器“about as fast”；内存 +10–20%；脱离实验的门槛约 5% | PEP 744 |
| CPython 3.15 JIT | x86-64 Linux +7–8%，AArch64 macOS +11–12%；单项 −15% 到 +100%；靠新 tracing 前端和“基本寄存器分配”取得 | 3.15 What's New；PEP 836 附录 4.7–12.6% |

## 3. 对 kfrc 的结论

三条共性：（1）所有快速后端都是 **-O0 级**的替代，发布构建仍由优化后端负责（Zig、Jai 用 LLVM；cg_clif 只针对 debug）。kfrc 没有 LLVM，debug 与 release 共用一条链，快路径只能是同一 Machine IR 上的另一种分配/编码方式。（2）后端快到一定程度后瓶颈转到前端与增量（TPDE 后端仅占 1%；Rust 项目发现 codegen 占比小于预期；Zig 一次增量更新约 84% 花在引用图遍历）。（3）copy-and-patch 用代码质量和体积换编译速度，CPython 纯模板阶段与解释器持平，加了寄存器分配和 tracing 才有 7–12%。

| 做法 | 结论 | 理由 |
|---|---|---|
| copy-and-patch 生成冷区域代码 | **不采纳** | 冷区域是函数内的块，与热块共享寄存器状态，模板的固定寄存器在边界产生大量 move；体积 4.27× 与体积目标直接冲突（冷代码照样计入 `__text`）；stencil 要由另一个编译器预编译，kfrc 无 LLVM，只能自举，收益不抵复杂度 |
| copy-and-patch 作 debug 档后端 | **不采纳** | 只比 TPDE 式单遍快约 1.4×，代码慢 2.32×；debug 构建的主要成本不在后端 |
| TPDE 式单遍分配+编码作 **debug 档** | **改编** | 输入仍是同一 Machine IR；debug 档跳过 L 级机器优化，浮动节点也钉在原 Body 块（在[固定与浮动](../../design/ir/levels.md#固定与浮动)的控制骨架之外再固定纯节点，elaboration 退化为按块顺序输出，也利于调试信息），寄存器分配用一遍完成的方案（循环树 + 粗粒度活跃性）。release 档维持线性扫描，单遍不进 release（运行慢，见 [ct-backend.md](ct-backend.md)） |
| 冷区域单独换分配器 | **不采纳**（维持现设计） | 冷区域已只跑窥孔和 DCE、放函数末尾（[ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md#冷区域)）；函数内混用两种分配器要在冷/热边界做寄存器状态对接，收益未测出前不做 |
| 发射时线性完成分支化简与跳转缩短（Cranelift `MachBuffer`） | **采纳** | 与 L 级“冷块放末尾、panic 尾块合并”在同一处完成，省一个 CFG pass |
| 先用对拍证明自写后端不弱于参照（Zig：behavior 测试通过数超过 LLVM 后端才切默认） | **采纳**（已一致） | 对应 [T38](../../plan/m6-quality-performance.md#t38) 的逐 pass 开关对拍 |
| 新优化先定编译时间预算（Go SSA：2× 不可接受，合并前追回） | **采纳** | 补到“新优化的采纳规则”：每个 pass 带编译时间预算与 A/D/B 数据 |
| 每阶段吞吐预算（Carbon：ns/行） | **改编** | 只作内部诊断指标（每阶段 ns/行、ns/节点），不作产品承诺；[performance.md](../../plan/README.md#验收) 规定不预先承诺倍数 |
| 增量时可达性/引用图全量重算 | **不采纳** | Zig 实测它占增量更新的 31/37 ms；C6 的实例图可达性必须按变化增量更新 |
| Zig 式原地修补输出文件 | **改编**（T39 之后） | 前提是对象片段按函数粒度、重定位可重放；只服务开发循环，首版不做 |

## 4. 具体建议（按影响排序）

| # | 目标文件#节 | 改法 |
|---|---|---|
| 1 | [compile-time.md](../../design/goals/compile-time.md#构建模式) | 现有一段只讲增量/dev 构建的跨 crate 内联，扩成按 opt-level 的两档。debug 档（opt-level=0，Cargo dev 默认）：R 级只做构建期窥孔与 DCE；S 级只填孔位、不内联（`#[inline(always)]` 除外）；L 级只做布局降低；纯节点也钉在 Body 块，placement 按块顺序；寄存器分配用单遍。release 档即现有流程。现有“只内联 `#[inline]` 和小函数”改为适用于 opt-level ≥ 1 的增量构建。档位进入 R 图、对象片段的缓存键 |
| 2 | [plan/README.md](../../plan/README.md#验收) | 场景加“debug 档冷构建、叶子修改”，参照臂为 rustc 默认 dev profile（LLVM）和 cg_clif；记录 L 级之后（Machine IR、分配、编码、链接）占整链比例，用于判断是否继续投资后端 |
| 3 | [compile-time.md](../../design/goals/compile-time.md#六条结构性性质) C6 行 + “指标与闸门” | C6 加“可达性在增量构建中按变化更新，不每次全量遍历实例图”；指标表加“增量构建中可达性计算耗时” |
| 4 | [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#l-级) L 级表 | “冷区域布局”行补“分支化简和跳转缩短在编码缓冲区发射时一次完成”；表后加“debug 档跳过本表除布局降低和 placement 外的所有步骤，分配用单遍” |
| 5 | [compile-time.md](../../design/goals/compile-time.md#指标与闸门) 末段 + [参照](../../design/goals/compile-time.md#参照) | 采纳规则补：新 pass 先登记编译时间预算（占所在级别耗时的上限），超出预算不合入，与 A/D/B 一起报告；参照节加 Carbon 式每阶段吞吐（ns/行、ns/节点），只作诊断 |
| 6 | [levels.md](../../design/ir/levels.md#固定与浮动) + 风险表“placement 成本”行 | 固定与浮动节补一句“debug 档全部节点固定在 Body 块”；风险表加对照：debug 档的固定放置给出调度成本下界，release 档 placement 耗时与之比较 |
| 7 | [build-pipeline.md](../../design/build/pipeline.md#产物依赖) | 记一条后置项：开发循环的原地修补输出（Zig `MappedFile`：节点余量、dirty 重放重定位），列入 T39 之后的候选，首版不做 |

**未找到原始来源：** Jai 的任何官方速度数字（只有社区书和论坛转述）；Andrew Kelley 关于原地二进制修补的官方演讲或带数字的 devlog（改用核心成员 Lugg 2026 年博客）；Carbon 每行预算只来自演讲转录，未逐字核对视频；Go 1.7 相对 1.6 的编译时间百分比（官方只写“significantly faster”，Cheney 的约 30% 是二手）；Keith Randall 的“SSA 只多花 20% 编译时间”计划（只见二手摘要）；cg_clif 相对 LLVM 的官方 debug 编译时间总数（LWN 2024 的 −20% wall/−40% CPU 是记者自测，未采用）；CPython JIT 从 main 实际移除的 PR。

# 后端质量：寄存器分配、指令选择与机器级优化

对照 [runtime.md](../../design/goals/runtime.md#必须自己做的部分) 的“必须自己做的部分”。ISLE 规则的形式化验证见 [ct-testing.md](ct-testing.md)，IR 设计与 e-graph 见 [egraph-rewrite.md](egraph-rewrite.md)。

## 1. 是什么

| 项目 | 年份 | 来源 | 要点 |
|---|---|---|---|
| 线性扫描 | 1999 | Poletto & Sarkar, [Linear Scan Register Allocation](https://dl.acm.org/doi/10.1145/330249.330250)（TOPLAS 21(5)）；[全文](https://web.cs.ucla.edu/~palsberg/course/cs132/linearscan.pdf) | 按区间起点扫描一遍；没有空闲寄存器时 spill 结束最晚的区间；区间不拆分 |
| 区间拆分线性扫描 | 2005 | Wimmer & Mössenböck, [Optimized Interval Splitting in a Linear Scan Register Allocator](https://ssw.jku.at/Research/Papers/Wimmer05/)（VEE '05） | 生命期空洞；use position 标明“必须在寄存器”；固定区间表达机器约束；拆分点移出循环、寄存器提示消 move、删多余 spill store；不保留 scratch 寄存器。HotSpot C1 自 Java 6 起使用（[JKU 项目页](https://ssw.jku.at/Research/Projects/JVM/SSA.html)） |
| SSA 上的线性扫描 | 2010 | Wimmer & Franz, [Linear Scan Register Allocation on SSA Form](https://c9x.me/compile/bib/Wimmer10a.pdf)（CGO '10） | 每个 vreg 一个区间，不需要数据流迭代；SSA 保证的不相交可跳过相交测试；Phi 消解并入 resolution 的并行复制；块序要求支配者在前 |
| V8 | 2026 当前 | [`LinearScanAllocator`](https://github.com/v8/v8/blob/main/src/compiler/backend/register-allocator.h)（源码） | TurboFan/Turboshaft 后端仍用线性扫描 |
| Go | 2015 起 | [`ssa/regalloc.go`](https://github.com/golang/go/blob/master/src/cmd/compile/internal/ssa/regalloc.go) 头注释 | “线性扫描的一个版本”：把函数当一个长基本块贪心分配，使用前才装入寄存器，逐出下次使用最远的值；spill 放在所有 restore 的支配者处并沿支配树上移；合并边上 shuffle；Phi 与其输入同寄存器或同栈槽 |
| 图着色 | 1982–1996 | Chaitin [1982](https://doi.org/10.1145/800230.806984)；Briggs, Cooper, Torczon [1994](https://doi.org/10.1145/177492.177575)（乐观着色）；George & Appel [1996](https://doi.org/10.1145/229542.229546)（IRC） | 干涉图上 simplify / coalesce / spill 迭代 |
| SSA 着色与 spill | 2006 / 2009 | Hack, Grund, Goos, [Register Allocation for Programs in SSA-Form](https://compilers.cs.uni-saarland.de/papers/ssara.pdf)（CC '06）；Braun & Hack, [Register Spilling and Live-Range Splitting for SSA-Form Programs](https://pp.ipd.kit.edu/publication.php?id=braun09cc)（CC '09） | SSA 干涉图是弦图，色数等于最大团：先降压再着色，之后合并不增加 spill；Braun–Hack 把 furthest-first（Belady）推广到 CFG |
| LLVM greedy | 2011 | Olesen, [Greedy Register Allocation in LLVM 3.0](https://blog.llvm.org/2011/09/greedy-register-allocation-in-llvm-30.html) | 按 spill 权重的优先队列，大区间先分；每个物理寄存器一棵 B+ 树；逐出低权重区间或拆分（切出热循环），最后才 spill；取代 2004 年起的线性扫描 |
| regalloc2 | 2022 | Fallin, [Cranelift, Part 4: A New Register Allocator](https://cfallin.org/blog/2022/06/09/cranelift-regalloc2/) | 源自 IonMonkey。操作数约束：use/def、Early/Late、Any/Fixed/Reuse/Stack；输入可为 SSA（块参数）；不相交区间合成 bundle；优先队列 + 逐出 + 首冲突处拆分；所有移动统一成指令间并行 move，环用空闲寄存器 → 栈槽 → 紧急 spill 打破；附符号化 [checker](https://github.com/bytecodealliance/regalloc2/blob/main/src/checker.rs) |
| fastalloc | 2024 | Sonuga, [regalloc III](https://d-sonuga.netlify.app/gsoc/regalloc-iii/)（GSoC）；[FASTALLOC.md](https://github.com/bytecodealliance/regalloc2/blob/main/doc/FASTALLOC.md) | 反向单遍（末块到首块、末指令到首指令）；跨块的值在块边界都放在专属栈槽；Wasmtime 中名为 `SinglePass` |
| ISLE | 2023 | Fallin, [Cranelift's Instruction Selector DSL, ISLE](https://cfallin.org/blog/2023/01/20/cranelift-isle/) | 类型化项重写，编译成 Rust `match`；extractor/constructor 是 Rust 函数；`AMode` 枚举表达地址模式；`ProducesFlags/ConsumesFlags` 保证 flags 生产者和消费者配对；显式优先级，重叠检查拒绝同优先级重叠和被遮蔽的规则 |
| BURG | 1992 | Fraser, Henry, Proebsting, BURG（SIGPLAN Notices 27(4):68–76，[书目](https://experts.arizona.edu/en/publications/burg-fast-optimal-instruction-selection-and-tree-parsing/)） | BURS 最优树覆盖；动态规划在生成选择器时完成（要点为二手来源，未读原文） |
| Go rules | 2015 起 | [`_gen/rulegen.go`](https://github.com/golang/go/blob/master/src/cmd/compile/internal/ssa/_gen/rulegen.go) 头注释；[ssa/README.md](https://github.com/golang/go/blob/master/src/cmd/compile/internal/ssa/README.md) | `sexpr [&& 条件] => sexpr`，条件是任意 Go 代码；多条匹配时取文件中的第一条；`lower` pass 把通用 op 换成架构 op |
| GlobalISel | 当前文档 | [LLVM GlobalISel](https://llvm.org/docs/GlobalISel/index.html) | 动机：SelectionDAG 有专用 IR，带编译时间成本，而且只看单个基本块；GlobalISel 直接在 MIR 上做、看整个函数 |
| OSR | 2001 | Cooper, Simpson, Vick, [Operator Strength Reduction](https://doi.org/10.1145/504709.504710)（TOPLAS 23(5)） | 在 SSA 图上按 SCC 识别归纳变量，效果与 Allen–Cocke–Kennedy 相同，但更快更简单；配合线性函数测试替换（LFTR） |
| SLP | 2000 | Larsen & Amarasinghe, [Exploiting Superword Level Parallelism](https://groups.csail.mit.edu/cag/slp/SLP-PLDI-2000.pdf)（PLDI '00） | 从相邻内存访问开始，把同构、独立的标量语句打包成向量指令 |

## 2. 关键证据

| 比较 | 数字 | 来源 |
|---|---|---|
| 线性扫描 vs IRC 图着色：代码速度 | 除两个基准外都在 12% 以内；结论写“约 10%” | Poletto 1999 |
| 线性扫描 vs 图着色：编译时间 | 合成测试中同时活跃 512 个变量时快 600 倍以上（计时含整个代码生成） | Poletto 1999 |
| 区间拆分线性扫描 vs JDK 5.0 局部分配器 | 编译速度相同，整数基准快约 15% | Wimmer 2005 摘要 |
| SSA 线性扫描 vs 先消 SSA 的 C1 产品版 | 后端时间 −13%…−19%，总编译时间 −4%…−5%；reg→stack move −5%…−17%；代码相同或略好（SPECjvm2008、SPECjbb2005、DaCapo、SciMark） | Wimmer 2010 表 |
| Braun–Hack spill vs 线性扫描 / 图着色 | 实际执行的 load（store）分别减少 54%（61.5%）和 58.2%（41.9%），CINT2000/x86；运行时间与线性扫描相当 | Braun 2009 摘要 |
| LLVM greedy vs 线性扫描 | 代码小 1–2%，最多快 10%；旧 rewriter 约占线性扫描编译时间的一半 | Olesen 2011 |
| regalloc2 vs regalloc.rs | 编译整体快约 20%；寄存器压力大的 Wasmtime 基准生成代码快 10–20%；cg_clif 运行最多快 7% | Fallin 2022 |
| fastalloc vs Ion（backtracking） | rustc+Cranelift 指令数 −0.3%…−18%；Sightglass 编译快 1.07–5×，执行慢 1.06–7.50×；分配本身约快 6× | Sonuga 2024 |
| Cranelift 编译时间分布（Umbra 查询） | 寄存器分配是最大一项，LLVM 的分配器快 42%；其中约 37% 在建区间和 bundle，约 6% 在 B 树；isel 准备（唯一使用、副作用分组）8%、isel 本身 12% | Engelke & Schwarz, [CGO '24](https://home.cit.tum.de/~engelke/pubs/2403-cgo.pdf) |
| Go 1.17 寄存器 ABI | 约快 5%，体积约小 2% | [Go 1.17 release notes](https://go.dev/doc/go1.17) |
| ISLE 规模 | 4 个目标共 27k 行 DSL，迁移用 12 个月；重叠检查找出若干完全不可达的规则 | Fallin 2023 |
| Cranelift vs LLVM：Wasm | 比 V8 TurboFan 慢约 2%，比 WAVM（LLVM）慢约 14%，编译快约一个数量级（2020 年数据，早于 regalloc2 和 ISLE） | [Cranelift README](https://github.com/bytecodealliance/wasmtime/blob/main/cranelift/README.md) 引 Xu & Kjolstad, [copy-and-patch](https://arxiv.org/abs/2011.13127) 图 22 |
| Cranelift vs LLVM：查询编译 | 运行性能与未优化的 LLVM 相近，编译只快 20–35% | Engelke 2024 |
| cg_clif 运行时 | “几乎和关闭优化的 LLVM 一样快”（2020）；与 LLVM release 的运行时对比**未找到原始来源** | [bjorn3 2020-12](https://bjorn3.github.io/2021/01/07/progress-report-dec-2020.html) |
| cg_clif 显式 SIMD | `std::arch` 至今“部分支持”；2023 年以前让 `is_x86_feature_detected!` 只报告 SSE/SSE2，regex 才能运行 | [README](https://github.com/rust-lang/rustc_codegen_cranelift)、[2023-07 报告](https://bjorn3.github.io/2023/07/29/progress-report-july-2023.html) |
| memchr 的分派 | 开启 std 时运行时检测 AVX2，否则用 SSE2，再退到 SWAR | [memchr README](https://github.com/BurntSushi/memchr) |

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| Wimmer–Franz SSA 线性扫描，加 2005 年的空洞、use position、固定区间、拆分点移出循环、寄存器提示 | **采纳**（主算法） | 现有分配器已是线性扫描并有 `fixed` 区间；placement 产出的 Machine IR 是 SSA；C1 和 V8 已在生产中使用；编译时间成本低 |
| regalloc2 的操作数约束、统一并行 move 和符号 checker | **改编**（只取接口和校验，不取算法） | 分配算法可以替换而契约不变；checker 直接覆盖 T18 的“高寄存器压力、循环中的并行复制” |
| spill 选择：下次使用最远者（Go、Braun–Hack），距离按循环深度加权 | **采纳** | 线性扫描与更强分配器的差距主要在执行的 spill/reload 数（Braun 2009）；这一步不改变线性时间 |
| backtracking / greedy | **暂不采纳**，T38 后备 | 质量收益明确（LLVM：1–2% 体积、最多 10% 速度；Cranelift：压力大的基准 10–20%），但它是 Cranelift 编译时间的最大一项。如果启用，按结构判据（循环内最大压力超过可分配数）逐函数选择，不依赖 profile |
| Chaitin–Briggs / IRC、SSA 弦图着色 | **不采纳** | 干涉图构建是 O(n²)；SSA 着色要求先全局降压，与“拆分”路线重复。只吸收 Braun–Hack 的 spill 部分 |
| fastalloc 式单遍 | **不采纳** | 执行慢 1.06–7.5×，与 runtime ≤ 1.0 的目标冲突 |
| ISLE 式类型化规则 DSL：生成 Rust `match`，`AMode` 和 flags 是类型，优先级加重叠/遮蔽检查 | **改编** | x86 选择规则有限，DSL 让规则可枚举、可检查。自写还是把 `cranelift-isle` 作为构建期生成器，要看总览“不用 Cranelift”红线的范围，由项目决定 |
| 合并判据：唯一使用、中间没有副作用 | **改编** | Cranelift 为此单独扫描整个 IR（占 8%）；kfrc 可直接查分区链，判断 load 能否并入内存操作数 |
| BURG 最优树覆盖 | **不采纳** | SoN 中值有多个使用，不满足树的前提；Cranelift 和 Go 都按优先级贪心匹配 |
| 单独的选择 IR（SelectionDAG 式） | **不采纳** | GlobalISel 的动机就是这层 IR 的成本和按块的作用域；kfrc 从 L 级图按 placement 顺序直接生成 Machine IR，看整个函数 |
| Go 式条件（任意宿主代码） | **不采纳** | ISLE 博客认为谓词难以推理；条件只能用类型化的 extractor 表达 |
| 强度削减：OSR + LFTR | **采纳** | 直接在 SSA/SoN 上按 SCC 运行，不需要循环规范化 |
| 循环展开 | **改编**，T38 后，在体积预算内 | 收益数据**未找到原始来源**；先用下面的 LLVM 归因实验量出上界 |
| 向量化：若做，先做 SLP | **改编** | 块内打包与 SoN 的值图契合；循环向量化还要依赖分析和尾循环，排在后面 |
| 显式 SIMD 提前到 M5 之前 | **采纳** | ripgrep 的 memchr 按 CPU 特性运行时分派。cg_clif 的教训是：intrinsic 不全时只能隐藏 CPU 特性，热路径会静默退到慢实现 |
| 运行时 ≤ 1.0 | **风险高** | Cranelift 投入多年后，在 Wasm 上仍慢约 14%（旧数据），在查询编译上接近 LLVM -O0。kfrc 在中层的别名和值优势，补不上后端每一项缺口，必须把后端差距和中层差距分开量 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [runtime.md](../../design/goals/runtime.md) | 必须自己做的部分 | “计划”列写具体算法：指令选择写“类型化规则 DSL（ISLE 式）+ 优先级/重叠检查”；寄存器分配写“SSA 线性扫描（Wimmer–Franz）+ 拆分点移出循环 + 远者优先 spill + 提示合并”；强度削减写“OSR + LFTR”；向量化写“SLP 优先”。寄存器分配的风险改为“中–高”，附 Braun 2009 和 Olesen 2011 的数字 |
| [runtime.md](../../design/goals/runtime.md) | 显式 SIMD 一行与其后的段落 | 前移到 M5 之前；在 M0 的 profile 中列出 memchr、regex-automata 实际用到的 intrinsic 清单作为必做集合；写明 `is_x86_feature_detected!` 只能报告后端已实现的特性 |
| [runtime.md](../../design/goals/runtime.md) | 验证 | 增加 M0 归因实验：rustc O3 分别加 `-regalloc=basic`（Olesen 称其结果接近线性扫描）、关闭循环/SLP 向量化、关闭展开，跑 ripgrep/Cargo 各 workload，得到每项后端缺口的上界（具体选项名在 M0 核对） |
| [plan/README.md](../../plan/README.md#验收) | 运行时间 | 记录双方实际走的 SIMD 分派路径（AVX2/SSE2/SWAR），不一致时比较无效；后端诊断指标加静态与动态 spill/reload 数 |
| [design/backend.md](../../design/backend.md) | Machine IR 扩展 | 新增“分配器输入”一行：操作数约束 Early/Late × Any/Fixed/Reuse/Stack；所有移动表示为指令间并行 move，环先用空闲寄存器、再用栈槽打破，从而去掉 `r10/r11` 固定 scratch；分配后可运行符号 checker（测试和 fuzz 时始终开启，见 [ct-testing.md](ct-testing.md)） |
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) | L 级 | “强度削减与地址模式”注明 OSR + LFTR；“循环”注明向量化先做 SLP；新增一行“指令选择”：合并内存操作数的条件是唯一使用且分区链上没有中间写 |
| [size.md](../../design/goals/size.md) | 和 LLVM 相比可能更大的地方 | “线性扫描的 spill 代码”的对策补充：spill store 放在支配所有 restore 的位置（Go）；删除多余的 spill store（Wimmer 2005） |
| [compile-time.md](../../design/goals/compile-time.md) | 指标与闸门 | 单独记录寄存器分配占后端时间的比例（Cranelift 中它是最大一项）；分配器的每寄存器结构用有序数组或区间表，不用每寄存器一棵 B 树 |

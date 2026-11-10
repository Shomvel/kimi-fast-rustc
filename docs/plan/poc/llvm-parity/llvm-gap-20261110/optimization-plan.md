# KFRC 优化执行计划（2026-11-10）

以 `b254941179c6f052bc2b732814c61ed299e39a3b` 和 final1 验收源码为固定基线，LLVM 固定为 21.1.8。当前四库 `.text` 几何平均为 LLVM 的 2.965 倍，13 负载耗时为 3.443 倍，整进程编译指令为 1.035 倍。计划按可观测差异拆成独立候选；本文件冻结于实施前，不表示所有工作包均已实现或已获得预期收益。[地址与移位实验](implementation.md) 已因跨负载回退撤回；按后续确定的优先级，当前交付 [内建成本模型](builtin-cost-model.md)，新验收结果单独记录。

详细数据及测量限制见 [分析报告](report.md)。旧 LP 文档的现状描述不能直接使用：当前已有自然循环分析、按循环深度的调度、整数范围分析、字段需求分析、直接 tag、载荷 lane 和统一寄存器分配。

**工作包与依赖**

| ID | 工作包 | 前置条件 | 主要交付物 |
|---|---|---|---|
| O0 | 重建并保存最终版本 LLVM/KFRC 对照 | 固定基线 | 热点、IR/汇编对照、链接产物和阶段统计 |
| O1 | base64 范围事实、切片检查与字节读取 | O0 找到残留指令 | 范围传递或局部指令选择候选 |
| O2 | humantime Result 与 enum 传输 | O0 定位首次膨胀阶段 | 按需求降低聚合值的候选 |
| O3 | 内联收益模型与优化后可达性 | O0，结合 O1/O2 的残留调用 | 独立内联策略候选与符号可达性候选 |
| O4 | 函数/数据分节及匿名常量合并 | O0 的链接基线 | 分节候选、常量合并候选，分别验收 |
| O5 | 残留机器码、spill 与控制流 | O1/O2/O3 集成后重新采样 | 由热点证据选择的单项机器优化 |
| O6 | 编译阶段成本与 CGU 并行 | 聚合表示及代码质量相对稳定 | 分阶段算法改进与并行候选 |
| C1 | 完整语言路径与上游测试 | 可独立推进 | TLS、atomic_xsub、u128、SIMD/union 支持逐项验收 |

推荐执行顺序为 O0 → O1/O2 → O3 → O4 → O5 → O6。O1/O2 的实现可分目录开展，集成时分别测试单项及组合；O4 与运行性能优化独立评价。C1 不等待所有性能优化结束。

**O0：把 LLVM 对照落实到具体函数和指令**

先恢复两种后端的正式库对象、最终驱动、带重定位反汇编、符号表和链接 map。原归档排除了库对象与正式驱动，不能用旧 c12 的汇编替代 final1。重建后核对源码、工具链、sysroot、参数以及输出一致性；若对象不能复现旧指纹，先解释差异，并将新测量标为新的基线。

第一批只分析五条执行路径：

1. base64 `internal_encode`，固定长输入和短输入。
2. base64 `decode_helper → decode_chunk_8/4 → decode_suffix`，合法输入与错误输入分别记录。
3. humantime `parse_duration → Parser::parse → parse_unit`，整数、小数、溢出与未知单位分别记录。
4. semver version/requirement 解析，覆盖正常返回和错误返回。
5. Unicode `prev_boundary` 与 UTF-8 反向解码，ASCII、非 ASCII 和跨边界输入分别记录。

每条路径输出一张对照表：LLVM 优化后 IR、KFRC 的 Specialized/Layout 图、热循环机器指令、实际调用、边界检查、Phi 边复制、栈读写、动态指令、cycles、分支和分支失误。栈读写区分语义局部变量、ABI 临时区和分配器 spill。记录采样比例及不可用计数器，不用静态 mov 数量替代动态 spill。

保留原 O3/CGU16/MIR0 的正式配置。另设默认 MIR 优化配置，检查结论是否适用于普通 release；两套结果分别报告。CGU1、禁用某项 LLVM 优化仅用于归因，不替换正式基线。

O0 完成条件：五条路径均定位到具体热点函数和基本块，并至少为 O1/O2 各选出一个可以独立验证的差异。没有残留差异的检查项直接关闭，不为补齐 pass 列表而实现变换。

**O1：先减少 base64 标量热循环的额外工作**

依据：编码慢 9.84 倍、解码慢 8.15 倍，动态指令均约 6.3 倍；源程序已经手工展开。当前范围分析已处理 BitAnd 和逻辑右移，重点是检查事实是否正确传给消费者。

拆成三个候选：

- O1a 范围事实。检查 `((x >> k) & 63) as usize` 的范围是否始终保留为 `[0,63]`，以及 64 项表访问是否还保留不可触发的失败分支。若事实已有而分支未消除，修复查询、支配范围或替换传播；不新增重复的范围分析器。
- O1b 切片长度。外层切片检查成功后，传播子切片长度到固定索引，消除同一区域内重复检查。输入侧读取 26 B、每轮消耗 24 B，输出侧处理 32 B；证明必须区分读取范围与实际消耗范围，保留长度不足和索引溢出的原行为。
- O1c 字节读取。将 LLVM 的 `read_u64` 和 KFRC 产物逐条比较。仅在观察到数组复制、逐字节组装或重复转换时，补充相应局部消除或 load/bswap 选择；不扩大读取范围，不假设输入对齐。

涉及文件：[analysis/ranges.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/analysis/ranges.rs)、[transforms/scalar/ranges.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/transforms/scalar/ranges.rs)、[conditions.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/transforms/control/conditions.rs)、[seam.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/transforms/scalar/seam.rs)，以及 x86 的 arithmetic/intrinsics/address_mode 降低。

LLVM 对照重点：ValueTracking/KnownBits、InstCombine 和约束消除后的实际 IR，随后核对 x86 指令选择。是否由某个 LLVM pass 消除需要 pass 前后证据，不能仅凭最终汇编命名原因。

结构验收：有有效范围证明的表访问不再保留失败分支；有长度证明的固定索引不重复检查；负例仍保留检查。行为覆盖 0–64 B 的全部长度、长输入、合法/非法字节、各种 padding、输出缓冲区不足和非对齐地址。性能分别比较 instructions/byte、cycles/byte 与短输入延迟。

**O2：降低 humantime 的聚合值膨胀**

依据：`parse_unit` 为 23,799 B，LLVM 为 1,093 B；该函数占 humantime 已计数编译阶段指令的 42.6%。源码包含多次 checked multiply/divide、`Result`、`?`，以及单位分支和可选小数部分。

先记录原始 Specialized、内联后 Specialized、Layout、虚拟指令及分配后的节点/指令数。按类别统计 tag、payload、Phi、局部槽、memmove、边复制和 spill，确定首次大量增长发生在哪一层。

根据结果分开实施：

- O2a 消费需求传递。对只检查 tag 的消费链，不提前构造完整错误 payload；成功路径按需求使用数值。仍保留错误分支的完整错误对象、字段和副作用。
- O2b 同变体合流。仅对可证明同变体、受同一控制条件约束的构造器合流共享字段。优先消除活跃路径上的重复 materialization，不整体启用旧 Return-only 补丁。
- O2c ABI 边界。对于仅传递的聚合对象，检查能否复用拥有所有权的快照或明确的返回目的地，减少中间槽和逐边复制。源内存在读取后可能被修改时，仍在原读取点捕获值。

涉及文件：[demand.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/lower_layout/demand.rs)、[owned_transport.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/lower_layout/owned_transport.rs)、[boundary_transport.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/lower_layout/boundary_transport.rs)、[variant_lanes.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/lower_layout/variant_lanes.rs)、[variant_encoding.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/lower_layout/variant_encoding.rs)。若增长已发生在 S 层，则优先改控制/聚合简化，避免把问题推给分配器。

LLVM 对照重点：SROA 后的实际字节分区、mem2reg/Phi、SimplifyCFG 中的返回与错误路径、死 store 和不使用字段消除。允许 KFRC 保留自己的语义 IR，只采用能保持 Rust 有效性、所有权和观测语义的简化。

行为覆盖每一种单位、整数/小数、各种 overflow、UnknownUnit 的字符串和位置、嵌套 Result/Option、同变体/异变体 Phi、未选中 poison、MaybeUninit、部分初始化及输入内存别名。旧 Return-only 的 semver identifier 和 humantime 格式化退步作为固定回归负载。

结构验收要求减少已定位的 payload/Phi/搬运来源；最终是否采用同时看 duration 解析、其他 12 个负载、函数体积及编译指令。缩小函数本身不能替代运行验收。

**O3：有收益依据的内联与优化后符号可达性**

当前预算依据图节点数，跨 CGU 私有符号依赖还会限制内联。首先为每个热调用点记录 callee 大小、拒绝原因、调用位置是否在循环、内联后删除的分支/字段/调用及净增长。

分开做两项改动：

- 对能暴露常量、切片长度、固定 tag 或消除小 wrapper 的调用，按简化后的净成本评分。先处理 O1/O2 已证明需要跨调用传播的事实，再覆盖 semver 解析和 Unicode 反向 UTF-8 解码。保留递归、深度、总增长和编译工作预算；不统一提高 max_callee_nodes。
- 在优化完成后，从对外符号、地址被取用的函数、静态对象重定位、运行时入口遍历真实引用。已经不可达的内部函数不再降低和写出。与当前 CGU 内部 needed/pending 机制衔接，避免删除外部可调用或其他 CGU 引用的定义。

涉及 [ipo](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/transforms/ipo/mod.rs)、[plan.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-son/src/transforms/ipo/plan.rs)、[CguGraphs 和编译调度](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/rustc_codegen_kfrc/src/lib.rs)。LLVM 参考 InlineCost、内联后的函数简化及 GlobalDCE；具体收益通过实际调用图和最终机器码验证。

验收保留跨 CGU 调用、函数指针、静态重定位、公共 API、递归和 noinline 用例；比较热调用减少量、函数符号与 text、全部运行负载和编译工作量。新增符号数量下降只是结构证据。

**O4：改进最终链接产物**

O4a 每个函数独立 `.text.<symbol>` 节，逐节维护符号、调用和地址重定位；先只改代码分节。O4b 对需要独立回收的数据分节；以外部可见性、地址使用和数据引用为约束。O4c 仅合并允许地址身份不重要的只读匿名常量；重定位目标、addend、对齐、内容均纳入等价条件，禁止合并可写、TLS 或身份可观察的对象。

主要文件为 [object_file.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/b254941179c6f052bc2b732814c61ed299e39a3b/crates/kfrc-x86/src/object_file.rs)。对照 LLVM ELF 分节与常量合并属性，不假定所有字符串或全局对象都可合并。

每个库增加两个链接消费者：只调用一个 API 的小程序、现有完整驱动。使用相同 linker 参数，保存 GC 日志、map、strip 前后 ELF 大小、`.text/.rodata/.data.rel.ro/.eh_frame`、重定位数量和链接时间。合并后对象文件可能增大，应将此成本与最终程序的收益同时记录。不得把 metadata 导致的 rlib 比例变化当作代码优化收益。

**O5：由残留热点选择机器优化**

O1–O3 集成后重新采样，避免给即将被删除的中间表示增加复杂机器优化。只启动满足触发条件的候选：

| 观察到的残留 | 候选 | 正确性边界 |
|---|---|---|
| 热循环边上大量复制或 spill | 分配偏好、分裂位置、低成本常量重物化 | 精确处理 fixed registers、call clobber、自 Phi 和并行复制环 |
| 纯标量条件选择仍需多指令掩码 | Select/cmov 或局部选择优化 | 不投机执行可能 trap 的运算/非法 load；仅选中值传播 poison |
| 多 case 热分发仍为长比较链 | 比较树、位测试或跳转表 | 有符号范围、默认分支、不可达条件和 ELF 重定位正确 |
| 同一地址反复计算或无用扩展 | 地址模式、已知位宽传播 | 保留溢出与截断语义，不能越界读取 |

不预先承诺 cmov、跳转表或向量化一定更快。比较 LLVM 实际选择，并按热路径延迟与代码体积决定。

**O6：编译速度分两步改进**

先将 x86 大阶段拆为调度、约束选择、活跃性、linear scan、边复制和 emission。检查大图是否仍反复排序、全量构造集合、扫描存活值或重建未失效分析；当前已有增量字段化简及相邻边复制缓存，不能重复实现。优先处理 O2 后仍存在的最大函数，记录处理的节点、边、区间和复制数以验证复杂度。

第二步才设计 CGU 并行：确认 rustc 查询上下文和图所有权的线程约束，区分串行查询/冻结数据与可并行纯后端计算，提供确定性合并和错误传播。先用 1/2/4/8 worker 比较，固定全部其他参数，控制每个 worker 的图峰值内存。涉及 tcx 的阶段不能未经约束核验就直接放入线程池。

CPU 指令、CPU 时间、墙钟和 RSS 分开验收。并行候选应保持正常构建的对象与语义结果稳定，并检查峰值 RSS 与不同 crate 大小。保留“后端指令减少三倍”和“整进程减少三倍”的不同口径。

**C1：支持范围单独验收**

从实际阻断依次建立最小复现和原上游测试入口：base64 的标准库 TLS 路径，Unicode 的 atomic_xsub，humantime 测试依赖的 u128，regex-syntax 的 SIMD/union。TLS 需贯通语义路径、生命周期和存储模型；原子操作需保留内存顺序；u128 需覆盖 ABI、算术和 helper 调用；SIMD/union 需先定义类型及值语义再降低。修复一个阻断只代表该阻断通过，必须继续原始构建才能确认没有下一项缺口。

不修改 benchmark 源码来规避缺口，不用 LLVM fallback 计作 KFRC 支持。新增支持后的第五个 crate 和上游单测单独报告，保留原四库历史序列。

**共同验收与里程碑**

每个候选都比较 LLVM、固定 final1、单项候选；组合候选另做一次验收。编译和测评在独立 E2B 环境执行，核对工具链、模板、输入哈希和实际资源；同一配置各方案串行。profile 与正式计时分开，禁止跨宿主机混合绝对耗时。

保留 11 轮交错计时、独立计数器采样和正式成本重复构建；重要收益用第二个独立测量会话确认。采用候选要求：正确性差分通过、目标机制的结构变化与实测收益一致、其他负载没有稳定退步。小幅差异不能分辨时记录为结论待定；不挑选最好一轮。首次暴露一类错误后暂停同类动态验证，静态排查全部相关实例，集中修复后恢复验证。

阶段成果按以下顺序验收，均不表示可保证的加速幅度：

- M0：当前源码可复现，五条热点路径完成 LLVM/KFRC 对照，最终 ELF 数据齐全。
- M1：O1/O2 至少各形成一项通过真实差分的有效候选，解释减少了哪些热路径工作。
- M2：组合后的运行耗时和 text 均改善，全部原负载无稳定退步，形成新的冻结基线。
- M3：链接产物、编译成本和支持范围分别完成可量化改进。

最终原门槛保持：`.text ≤ LLVM`、运行耗时 `≤ 1.10 × LLVM`、编译指令 `≤ 0.33 × LLVM`，并完成规定的正确性验证。阶段性改善不得记为最终门槛已完成；本计划不提前承诺从 3.44 倍降到某个具体倍数。

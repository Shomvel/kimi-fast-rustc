# 通用 LLVM 优化机制核查

> **历史快照说明（2026-11-11）：** 本文保留当时冻结输入的结论，不描述当前工作树。固定索引 promotion、DSE/PRE/有限循环优化、seam 冗余消除、准备图可见性、SIB、DemandedBits 和有限重物化此后已有变化。当前缺口与源码证据见 [2026-11-11 通用 pass 核查](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/docs/plan/poc/llvm-parity/llvm-gap-20261110/generic-pass-gaps-20261111.md)。

**结论：KFRC 的通用优化尚未做到位。** 已有局部存储提升、整数化简、SCCP、纯表达式 GVN、内存转发、纯值全局代码移动和片段式寄存器分配；主要缺口同时存在于规则覆盖、阶段衔接和信息可见性。当前应先补这些基础能力，再判断具体负载还需要什么专门优化。

本次按用户要求暂停新增优化与测量，对照 [llvm-study](/Users/shom/Downloads/llvm-study/README.md) 的 22 篇专题，追踪实际生产调用路径。代码基线为 `7fb99c5b057e7ce7f8481f8ce9cd2e05f2d599e6`，加当前冻结的 W3 调用点准备、W4 私有整数访问及对应测试等 13 个文件；这些文件与实验 `core-source.json` 全部一致。W5 指针直接分支已撤回，函数对齐仍为 16 字节。

[输入指纹](generic-optimization-audit-inputs.json) 固定了 42 个核查源码、README 与 22 篇专题，以及引用的既有诊断证据。研究资料基于 LLVM **21.1.0** / `3623fe661ae35c6c80ac221f14d85be76aa870f1`；先前 Rust 1.94.1 对照工具链使用 LLVM **21.1.8**。本次没有运行新的 LLVM pass 对比、编译、测试或性能实验；“尚未实现/未接入”来自源码路径核查，收益大小仍需后续差分验证。

## 实际管线

```text
R 定义准备（每份定义复用）
  promotion → aggregate/InstCombine/division/GVN/forward/aggregate
  → variant 规范化 → CFG 清理与受影响值重访 → compact
  （division 在 R 上不执行；通用 SCCP 不在这条准备路径中）

S 实例准备
  从已准备的 R 特化：optimize_specialized_seams
  从具体 Body 构图：optimize_specialized_graph

crate 小 wrapper 准备
  ≤128 节点、最多两轮，收缩后替换缓存源图

每个函数
  identity summary → 有预算的 inline → identity summary
  → 若调用发生变化，optimize_specialized_seams
  → verify → layout lowering → L cleanup
  → x86 Schedule/GCM → 指令选择 → private_slots/known_width
  → constrained split linear scan → 发射/边拷贝
  → zero idiom/fall-through → 每函数 section 与循环对齐
```

入口见 [R 准备][r-prepare]、[S 准备][s-prepare]、[函数编译][compile]、[pipeline][pipeline]、[L cleanup][l-cleanup]、[x86 lowering][x86-lower]。两条 S 路径的能力不同，不能从完整 S pipeline 有某个 pass，推断每个特化实例或内联后都会运行它。

## 覆盖矩阵

“已接入”只说明生产路径有相应能力，不代表达到 LLVM 的全部覆盖范围。按 pass 名称计数或给出一个完成百分比，会掩盖这些差异。

| llvm-study 专题 | 当前能力 | 尚缺的通用部分 | 判断 |
|---|---|---|---|
| 00/01 架构、SSA、合法性 | R/S/L 分层；Region/value/effect Phi；控制锚点、poison 观察与 witness 验证 | KFRC 与 LLVM 的 refinement 合同不同，规则需重新证明 | 基础已接入，不能直接复制 LLVM 规则 |
| 02/16 分析复用、顺序、预算 | R 定义缓存；seam 工作队列；局部事实快照；预算、访问统计、验证边界 | CFG/Uses/MemoryFacts 在多个 pass 中重建；缺少统一的变更种类与可保留分析契约；内联后清理能力不齐 | **部分具备，衔接优先补** |
| 03 SROA/mem2reg | 不逃逸逻辑槽、字段路径、整值 enum、value Phi；内联后重试；L 私有标量/整数 byte range 转发 | promotion 不接受 IndexAddr、raw 访问和部分 enum 更新；L 私有消除要求整个槽的所有读取可证明，不能替代访问分区 SROA | **局部实现** |
| 04/15 InstCombine/规范化 | 整数常量、保留 poison 的恒等式、同操作嵌套常量、常量右置、bit slice 重组；全图/seeded 共用 rewrite | 通用 trunc/extend/cast 链、较广的表达式重组、浮点规则；全图仍最多 4 次扫描 | **局部实现** |
| 05 SCCP/CFG/DCE | 整数/布尔 ≤64 位单调值格、可执行控制传播、Phi；范围/路径条件、enum threading、菱形折叠、不可达与无用纯值删除 | SCCP 没有范围/enum/内存格；无通用控制依赖活性 ADCE；普通清理把可达控制和固定操作作为根 | **核心子集已接入** |
| 06 EarlyCSE/GVN/PRE | 无控制锚点的确定性纯值 exact numbering；整数交换律键；witness 按支配复用 | 非纯值由其他机制处理；无通用 PRE；seam 后未重跑 GVN | **算法子集 + 接入缺口** |
| 06 AA/MemorySSA/内存转发 | effect 链/Phi；allocation provenance/escape；Frozen 参数与调用权限；同一 typed snapshot 有界转发 | 普通非 Frozen 读取遇不相关 store 仍停止；缺少统一对象/范围 clobber 与调用 Mod/Ref 查询 | **局部实现，不必另造 MemorySSA** |
| 06 DSE | promotion、L private_memory、机器 private_slots 可删除已消除私有对象的写入 | 未见针对仍需保留对象的一般覆盖写 DSE：证明旧写必被覆盖且无人观察后只删旧写 | **私有对象能力已有，一般 DSE 缺失** |
| 07 LICM/循环/归纳变量 | 自然循环/不可约结构分析；后端 GCM 将纯值放在合法支配路径上较浅的循环层，并尽量靠近使用 | 无通用 LoopSimplify/LCSSA/Rotate 变换链；无 SCEV/IV/LSR；无一般 load/store LICM、循环内存提升与出口下沉 | **纯值移动已有，循环基础仍缺** |
| 08/17 展开、向量化、VPlan | 向量 ABI/传输及 intrinsic；固定大小内存搬运可展开 | 无通用循环完整/部分展开、LoopVectorize、SLP 和候选计划；缺依赖/尾部/运行时别名检查等前置 | **未实现，排在循环基础之后** |
| 09 内联/IPO/LTO | 同 crate 图缓存、identity summary、小 wrapper 准备、有预算 inline、有限实参常量准备、sole-call 特例 | CGU 可见性过早挡住部分准备；没有 CGSCC 后序简化闭环、通用 IPSCCP/属性推导、跨 crate IR 导入/LTO | **部分实现，存在真实阻断** |
| 10 指令选择/合法化 | 标量/intrinsic 降低、常量除法、checked 值共享、比较直接分支、常量 displacement、局部 LEA/立即数选择 | 通用 `base + index*scale + disp` 访存、一般 load folding、更多 use-site combine；类型与 ISA 支持也不等于 LLVM 后端 | **局部实现** |
| 11 寄存器分配 | 生产 `lower/allocated` 有活跃片段拆分、下一次使用驱逐、fixed/tied 约束、copy/回边偏好、spill 槽复用及 checker | 无通用廉价表达式重物化和按执行频率估价的热冷分配；不能假设它会补偿所有 GCM 外提 | **已有实质能力，补偿范围有限** |
| 12 机器调度 | 控制/依赖顺序、纯值 GCM 与尽量缩短活跃范围 | 没有按 CPU 延迟/端口/吞吐/关键路径/寄存器压力选择 ready 指令的机器调度器 | **依赖调度已有，机器资源调度缺失** |
| 13/20 布局、PGO、BOLT | RPO、分支反转/边拷贝处理、fall-through 清理；独立函数 section；函数及自然循环 16B 对齐 | 无 profile 导入、频率驱动块/函数放置与冷热拆分；无后链接优化流程 | **基础布局已有；PGO/BOLT 后续** |
| 14/18 证据、翻译验证 | graph/machine dump、阶段 profile、inline 决策、解释器、语义/原生差分与真实 E2B 证据 | 不等于每个变换都有翻译验证；Verifier/witness 不证明任意源/目标语义等价；部分拒绝原因仍无细粒度统计 | **验证设施已有，证据按范围解释** |
| 19 MLGO/成本策略 | 只读 GenericCostModel、Unknown 成本、有预算观察与可选 size policy | 默认仍为节点策略；无经训练策略，基础模型未涵盖完整降低、RA 和 CPU | **不是当前优先项** |
| 21 KnownBits/DemandedBits | 有界正向 KnownBits、范围分析、冗余 mask；机器层 known_width 有少量 use 方向的 mask 绕过 | 无通用逐 use 的需求位回传、多 use mask 合并与由此驱动的计算缩窄；字段需求和位需求不同 | **KnownBits 已有，通用 DemandedBits 缺失** |

主要源码：[promotion][promotion]、[private_memory][private-memory]、[InstCombine][instcombine]、[KnownBits][known-bits]、[SCCP][sccp]、[SimplifyCFG][simplify]、[GVN][gvn]、[forward][forward]、[MemoryFacts][memory-facts]、[LoopInfo][loop-info]、[GCM][gcm]、[地址选择][address-mode]、[生产分配器][allocator]、[known_width][known-width]、[成本模型][cost-model]、[函数 section][sections]。

## 三个最重要的衔接问题

### 1. 内联后没有闭合已有的 GVN/内存优化链

`optimize_graph` 调用 aggregate、InstCombine、division、GVN、forward；`optimize_specialized_graph` 在 CFG 清理后还会再次调用它。但生产内联完成后调用的是 `optimize_specialized_seams`：其中有 promotion、标量/字段重写、范围/条件、SCCP 和 CFG 清理，**没有 GVN、forward 或 division**。[seam 规则入口][seam] 也只分派 aggregate、InstCombine 和字段规则。

从 R 特化的 S 实例准备同样使用 seam 路径。因此，参数替换或内联令两个表达式变成同一表达式、让两个地址/读取暴露出可复用关系后，并不保证现有 GVN/forward 会再次看到它们。L cleanup 虽有 InstCombine、地址 CSE 和私有存储消除，也不等于对一般 S 读取和纯表达式补做了这些分析。

这是从调用链确认的能力缺口。本次没有为它测出新的负载加速比；不能因此宣称 semver 或 base64 的剩余差距全部来自这里。

修复方向是把适用于变化区域的去重/内存查询接到替换和内联之后，按 dirty value、dirty slot、effect/CFG 变更分派，批量提交。先以有预算的闭环验证机会，再决定是否需要增量索引；不应在每一次 splice 后全图重复所有 pass。GVN 也可能延长活跃范围，最终需要检查 spill 与运行时间。

### 2. callee 来源可见性早于特化后的可链接性检查

[CguGraphs::graph][provider] 在返回源图前检查其原始引用：只要包含目标 CGU 外的私有符号，就返回 `None`。内联器只有拿到该图后，才会进入 W3 实参事实提取与常量准备。该保护有实际链接语义依据；问题是源图中稍后可能被剪除的私有引用，也会提前挡住整个准备过程。

既有 [semver S 图诊断][semver-evidence] 已提供真实例子：req 路径的 25 个 trim/strip wrapper 决策均为 `Missing`；空格、点、逗号及 wildcard 字符仍是直接 typed Const。最早拒绝时 emitted growth 与 costed calls 均为 0，**不是 64 节点阈值或 2000 节点总预算耗尽**。此诊断来自 allv2 的既有 S 图；当前 provider 与该语义路径相同，撤回的 W5 位于后端。

另一个 Ord::cmp 路径可以进入 W3，却由 55 节点准备成 150 节点而拒绝 Size；成本摘要还有 34 个 unknown 节点。这与 req 的 Missing 是不同原因，不能用提高统一阈值或将已知 32 units 当完整成本一起处理。

可研究“有界读取来源 → 常量准备/收缩 → 检查所有存活引用与数据身份 → 确认可链接后提交”的 provider 契约，或先加强允许范围内的 callee-first wrapper 清理。不能直接取消私有符号检查，也不能凭符号名混合不同 CGU 的物理产物。期望剪枝后能否真正变小且可链接，尚待实现与验证。

### 3. 有局部内存消除，但一般 alias/clobber/DSE 尚未打通

[forward::preserves][preserves] 在普通非 Frozen 情况下可以跨读取，满足权限时可以跨调用；遇到另一条 store，并不会查询它是否属于已证明独立的对象/字节范围，而是停止。`MemoryFacts` 主要提供根来源和保守逃逸事实，[partition][partition] 主要贴 Local/Const 标签；标签本身不会自动形成按地址跳过不相关写入的 walker。

另一方面，L `private_memory` 和机器 `private_slots` 已能在严格私有条件下转发读取并删除相应存储。本轮 W4 还扩大了整数 byte-range 覆盖。**不能再将这些已有能力描述为“完全没有 DSE/内存优化”。** 真正未覆盖的是：对象仍需保留时，通过完整覆盖、执行关系和无观察证明，仅删除其中旧写入；以及在普通读取查询中利用已证不别名写入。

应先复用 effect 链与现有权限/来源事实，建立小范围 clobber 查询。先做直线、同对象、完整覆盖；跨分支必覆盖、部分覆盖与未知调用以后扩。删除 store 时还必须保留原有合法性/poison 观察，不能把固定操作直接当成无用 SSA 值删除。

## 建议实施次序与验收

| 次序 | 工作 | 可验收结果 |
|---|---|---|
| 1 | 补齐 S 特化/内联后已有 GVN、forward、division 的受控重访；区分来源准备与最终链接检查；有限 callee-first 清理 | 构造“替换后才同值/同地址”的用例，整条编译路径消除重复；req 的 Missing 能被准确拆解并在确实可链接时进入准备；存活外部私有引用仍拒绝；记录规则访问与编译成本 |
| 2 | 通用 cast 链和有限 DemandedBits；固定索引/访问分区 SROA；局部完整覆盖 DSE 与不相关写入穿越 | 正常、溢出、符号扩展、poison、多 use、生命周期及别名反例差分一致；明确哪些 load/store/cast 消失，而非只统计 IR 节点 |
| 3 | x86 SIB 比例索引访存、use-site combine；仅对已有 spill 证据需要的表达式补重物化 | `base+index*{1,2,4,8}+disp` 的真实一次访存使用 SIB；base/index 活跃值进入分配约束；检查分配前后 copy/spill 与最终执行 |
| 4 | 循环规范化 → 简单 IV/SCEV → IndVar/地址递推 → 小固定循环展开 → 更一般的内存 LICM/向量化 | 零次迭代、wrap、提前退出、多出口/不支持形态、调用与别名边界正确；随后清理/SROA 真能兑现机会；CPU 机器调度按独立证据推进 |

第一项有已确认的真实阻断，也存在无需新增算法就能补的管线覆盖。第二、三项是普通标量代码的基础；完整向量器、MLGO、PGO/BOLT 不能替代这些基础工作。具体独立子项是否并行实施，应由源码影响范围和验证资源决定。

编译时间目标仍需单独约束：增加 pass 不等于编译加速。已有 `KFRC_PROFILE`、访问/预算计数可先定位重复构建 Uses、CFG、dominators 和 MemoryFacts 的成本，再决定轻量缓存与失效维护的范围。本次只确认了重复构建入口，未测其当前占比，不为统一 Pass Manager 预先承诺收益。

每项都需要规则语义、完整管线、真实机器执行三层证据；已撤回的布局/分支试验不计作通用机制补齐。保留四库正常/错误语料和未参与选型的工作负载，分别报告运行时间、编译墙钟/指令、机器体积及 spill，才能判断通用改动是否成立。

## 当前冻结状态

远端 fn32/fn64 screen 在检查时已自然完成，本任务计时/编译/driver/perf 进程均为 0，未启动新实验。原始输出与 [冻结回执][freeze] 已下载并校验 SHA；未将其作为生产对齐选型。沙箱仍保留未全部下载的历史证据。

本次只新增核查文档、输入指纹和旧计划的导航说明，没有修改编译器实现。当前 core 候选尚未完成最终验收；此前 901 项通过属于含 W5 的 allv2，不能写成当前 core 已通过完整 workspace。现阶段结论是优先补通用优化链，保持新增调优冻结。

[r-prepare]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/specialize/mod.rs:106
[s-prepare]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen.rs:366
[compile]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen.rs:187
[pipeline]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/pipeline.rs:71
[l-cleanup]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/cleanup.rs:69
[x86-lower]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower.rs:90
[promotion]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/promotion.rs:163
[private-memory]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/private_memory.rs:206
[instcombine]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/instcombine.rs:19
[known-bits]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/known_bits.rs:40
[sccp]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/control/sccp.rs:48
[simplify]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/control/simplify.rs:97
[gvn]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/gvn.rs:33
[forward]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/forward.rs:48
[memory-facts]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/memory.rs:23
[loop-info]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/loops.rs:66
[gcm]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/schedule.rs:399
[address-mode]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/address_mode.rs:9
[allocator]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/allocated/scan.rs:274
[known-width]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/known_width.rs:95
[cost-model]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/cost.rs:82
[sections]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-x86/src/object_file.rs:99
[seam]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/seam.rs:62
[provider]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen.rs:148
[preserves]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/forward.rs:327
[partition]: /Users/shom/.codex/worktrees/kfrc-base64-opt/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/partition.rs:18
[semver-evidence]: /Users/shom/Downloads/kfrc-callsite-opt-20261110/semver-char-chain-allv2.md
[freeze]: /Users/shom/Downloads/kfrc-callsite-opt-20261110/frozen-generic-screen/freeze-status.json

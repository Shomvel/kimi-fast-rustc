# 当前通用 LLVM pass 缺口核查（2026-11-11）

后续：[程序 profile 与独立 pass 修复](program-profile-20261112.md)固定了 13 个真实负载的 PMU、采样和最终 S/L 图，并实测本报告后的工作。下文保留审计时点的能力和实验状态。

**当前距离 LLVM 的主要差距，是通用数据流、循环和机器优化的覆盖，以及这些能力在阶段之间的衔接。** 最近围绕某个调用能否压到内联节点阈值、某种临时数组能否消掉选择方案，已经偏向现有评测样本。后续应按程序类别、可复用分析和合法性条件定义优化；现有 5 个 crate 用于回归。

本报告由主 agent 核查循环、标量、机器后端，两个 subagent 分别核查内存及 IPO/阶段衔接。输入是当前工作树：HEAD `7fb99c5b057e7ce7f8481f8ce9cd2e05f2d599e6` 加继承的未提交改动，**不是单独的 HEAD，也不是某份历史候选包**。[输入指纹][inputs] 固定引用文件及三个编译器模块的源码清单；清单用于固定审计范围，不表示逐行审查了全部文件。

LLVM 算法与顺序依据是 [llvm-study][study] 固定的 LLVM **21.1.0**、commit `3623fe661ae35c6c80ac221f14d85be76aa870f1`。既有 Rust 1.94.1 性能对照使用 LLVM **21.1.8**，两者分开记录。本轮没有新增 pass、编译、测试或性能实验。结论确认源码能力边界，不预测任何一项必然带来 50% 加速或达到 1×。

## LLVM 提供的通用组合

固定版本的 [function simplification pipeline][llvm-function] 展示了以下主线，省略了选项及扩展回调：

```text
SROA / EarlyCSE
  → JumpThreading / CVP / InstCombine / Reassociate
  → LICM / Rotate / LICM / Unswitch
  → LoopIdiom / IndVar / Deletion / FullUnroll
  → SROA / GVN / SCCP / BDCE / InstCombine
  → JumpThreading / CVP / ADCE / MemCpyOpt / DSE
  → late LICM / CFG cleanup / InstCombine
```

另有 [调用图中的函数简化与属性推导][llvm-ipo]、[IPSCCP][llvm-ipsccp]、[LoopVectorize][llvm-vector] 和受开关控制的 [SLP][llvm-slp]，最后还需目标指令选择、寄存器分配与机器调度。优势来自这些通用算法相互创造并消费机会。不能把名称相同或局部样例通过视为覆盖相同，也不应照抄 pass 次数。

这里以普通优化流水线为参照，不将 PGO/BOLT 作为解释当前差距的前提。LLVM 21.1 的 Attributor 默认 `NONE`，ArgumentPromotion 在所核查的普通 CGSCC 管线中仅 O3 接入；它们不属于本报告声称的 O2 必跑项。

## 当前能力与真实边界

分类含义：**算法缺失**表示未发现通用实现；**窄子集**表示已有实质能力但覆盖受限；**阶段消费不足**表示算法已有、某条生产路径未必执行；**成本不足**表示合法变换缺少足够的盈利性判断。

| 能力 | 当前已有 | 真实缺口 | 分类 |
|---|---|---|---|
| SROA / mem2reg | 局部槽、逻辑字段、固定数组索引、whole-value enum、循环/合流值 Phi | 一个 unsupported use 拒绝整槽；不能按实际访问分区，只提升独立部分并保留剩余存储 | 窄子集 |
| AA / effect / forwarding / DSE | 精确根及常量范围；穿过不相交 store；权限支持的跨调用；同内存版本 effect Phi；all-path 和小标量字节覆盖 DSE | 统一 clobber/Call ModRef、动态地址关系、不同入边 snapshot 合成、聚合覆盖 DSE | 窄子集 + 算法缺失 |
| MemCpyOpt | L 私有整数范围消除；初始化整数源、1–8 字节、单小 memcpy 消费者 | 通用 copy-chain forwarding、copy 合并、store-of-load 合并、call destination forwarding | 算法缺失，已有局部特例 |
| GVN / PRE | 确定性浮动纯值 GVN；带执行证据的标量 Phi-translation PRE | memory GVN、不同入边值构造 value Phi、load PRE | 窄子集 |
| 循环规范化 / IV | 有限 dedicated entry、常量 modular AddRec、常数乘法递推、小循环完整展开 | dedicated exits/LCSSA/rotation；符号 trip count、nowrap/终止证明、一般地址递推；memory LICM、内存提升、出口下沉 | 窄子集 + 算法缺失 |
| 展开 / SIMD | 非 nested、单路径、常量次数、默认最多 8 次的完整展开；已有 SIMD/intrinsic 降低 | 普通有分支循环的展开、partial/runtime unroll、LoopVectorize、SLP、依赖分析及向量成本计划 | 算法缺失，已有有限展开 |
| 标量 / 控制 | InstCombine、KnownBits、DemandedBits、区间查询、SCCP、variant/条件 threading | 一般表达式树重关联、较广的位/算术传播、符号关系及循环范围求解、控制依赖 ADCE、安全 if-conversion | 窄子集 + 算法缺失 |
| IPO | 源图缓存、有限 wrapper 准备、实参常量准备、identity summary、有预算内联 | 普通 callee-first/CGSCC 简化闭环、函数效果/返回值摘要、IPSCCP；scratch 清理和阶段事件消费不足 | 算法缺失 + 阶段消费不足 |
| x86 选择 / RA | SIB、立即数、比较直接分支；分片线性扫描、下一次使用驱逐、copy/回边偏好、spill 槽复用；常量/frame/symbol 重物化 | 通用 load/reload folding、频率与 spill 成本、带活跃输入的廉价表达式重物化、较完整机器 copy/flags 清理 | 窄子集 + 成本不足 |
| 机器调度 / 布局 | 保持固定依赖链、纯值 GCM、循环深度选位、RPO/fall-through | 延迟/吞吐/资源/寄存器压力驱动的机器调度；基于合法别名信息重排独立内存操作；静态分支概率与块频率布局 | 算法缺失 + 成本不足 |

### 内存：从整槽规则走向访问分区与共享查询

[promotion][promotion] 已支持固定 IndexAddr，并在 [SSA 构造][promotion-ssa] 中处理控制合流。当前问题是整槽收集必须接受所有 use，无法让 `struct { counter, data }` 的动态 `data` 访问保留在内存，同时独立提升 `counter`。这是 partial SROA 的通用问题，继续添加某个数组长度或特定 cast 形态不能补齐它。

[MemoryFacts][memory-facts] 与 [MemoryLocations][locations] 已提供 provenance、逃逸、精确对象和常量范围。[forward][forward-preserve] 已会跨不相交写入；[effect Phi 查询][forward-phi] 要求全部入边追溯到同一内存版本，不生成不同入边的 value Phi。[DSE][dse-values] 已有 [分支必覆盖证明][dse-branches]，但旧值主要限完整 ≤64-bit 标量/Unit，不覆盖一般聚合。

应复用现有 effect 图和位置查询，统一回答一次访问对指定范围的 read/write、生命周期、捕获和观察影响，再让 forwarding、DSE、copy、memory GVN、LICM 共享。参数 `Frozen/Unique` 权限与函数整体 read/write/capture 摘要是不同事实；[CallTarget][call-target] 尚无完整函数效果摘要。为对齐名称另建平行 MemorySSA 并非前置要求。

[Issue #11](https://github.com/Shomvel/kimi-fast-rustc/issues/11) 提供的简化方向仍适用：利用 Rust 逻辑字段、enum witness、借用权限和 slice metadata，减少从原始字节重新推导事实的工作。但内容不变、地址可访问、初始化/值有效、算术边界和盈利性要分别证明。Rust 类型或 Move 本身不能跳过别名、生命周期、快照时序与观察顺序；访问分区应由这些通用事实驱动。

[L private_memory][private-memory] 仍要求整槽 closed-use、每个读取都可证明，才删除整槽；合流接受同一个 Stored 值，循环失败。[private_memory_copy][private-copy] 的小整数单消费者消除不能替代通用 MemCpyOpt。[CopyNonOverlapping 分区][partition] 仍为 Unknown，随后通常 [降低成 memcpy][copy-lowering]。

通用 copy 优化需要维护**源快照发生的时刻**。例如 `copy(a ← b); write(b); copy(c ← a)` 中，`c` 应取得旧快照。copy→copy 转发不能让它改读新 `b`，也不能让提前写最终目标改变其他读取或调用观察。LLVM 的 [MemCpyOptimizer][llvm-memcpy] 单独处理这些依赖，而非仅识别 memcpy 名字。

可实施的依赖次序是：共享 location-aware clobber/效果查询 → 访问分区 SROA 与聚合 DSE、copy graph → 合流可用值 Phi → 会补读取的 load PRE。后两项需分别验收；已有入边值合成不应与新增读取的执行安全混为一项。

### 循环：现有小展开和纯值 GCM 覆盖不了普通循环

[LoopSimplify][loop-simplify] 仅合并可约、单 latch、非 nested 循环的多个外部 header 前驱，不维护 dedicated exits，不做 LCSSA 或 rotation。[Induction][induction] 只识别两输入 header Phi 和常量 modular Add/Sub 更新。[trip-count][loop-count] 用常量 start/bound 有界模拟；[展开计划][loop-plan] 还要求单 entry/latch/exit、header test、body 单路径。[strength reduction][strength] 只将常数乘以整数 AddRec 改为 modular recurrence，没有一般 pointer/address LSR。

因此循环能力缺口分成两个方向：

- 建立可维护的 preheader/latch/exit/value 出口结构，在 effect/alias 证明上实现 memory LICM、循环内存提升和出口下沉。零次迭代、条件读取、可能 trap、提前退出和生命周期是实际限制。
- 扩展可证明的符号归纳变量、边界与地址递推，为一般 IndVar/LSR、partial/runtime unroll 和 LoopVectorize 提供事实。wrap、nowrap、终止性、对象边界各自证明。

[后端 GCM][gcm] 已能把纯值放在合法支配路径上更浅的循环层，不能再称完全没有循环不变量移动；它也不能代替对 load/store 的内存与执行安全分析。

**自动向量化是完整缺失的大能力。** SIMD ABI、向量传输和 intrinsic 支持并不产生自动 LoopVectorize/SLP。它需要依赖与别名证明、reduction、尾部和零长度处理、必要的运行时检查，以及目标成本与相应机器支持。其对数据并行程序的价值应单独验证；本轮没有证据将现有 5 个 crate 的差距量化归因于它。

### 后端：IR 更短不保证机器码更快

[生产 lower 路径][x86-lower] 是 Schedule/GCM → selection → private_slots/known_width → constrained allocation → emission；[allocation 收尾][allocated] 有有限重物化和 zero idiom。

当前 [Binary/Compare 发射接口][emit] 接收寄存器输入，普通二元 rhs 发成 `MachineOperand::Reg`。底层已有 Stack operand 或可编码内存操作，并不表示生产路径能选择 `add reg, [spill]`。一般 spill reload folding 与 load folding 需要把 memory operand 的选择、位置约束及检查接到生产路径；不能只新增一种编码。

[分配器][scan] 已有分片、约束和偏好；驱逐核心仍为 furthest next use，没有 block-frequency/spill-cost 热冷权衡或一般重着色。[重物化][remat] 的 recipe 限 Immediate/Frame/Symbol，不支持带活跃输入的廉价表达式。新外提或 GVN 可能延长活跃区间，这些补偿能力会影响最终收益。

[Schedule][schedule] 沿控制链保持 load/store/call 顺序，对纯值做 earliest/latest GCM；晚放置依据循环深度，不是 CPU latency/resources/pressure ready-list。RPO 和 fall-through 已有，但没有一般静态 BPI/BFI 驱动布局。[GenericCostModel][cost-model] 自身也明确是定性权重，尚非校准的 CPU/寄存器压力模型。

### IPO 与阶段：修正已有 pass 的覆盖，避免围绕阈值选规则

已确认三个生产衔接问题，均应与新算法分开：

1. **进入 S 所获合同不是显式消费事件。** [division][division] 在 R no-op；[R 准备][r-prepare] 后，[实例入口][s-prepare] 使用 seam，而 [批量 value/effect 清理][seam-exposed] 由 seeds/slots/变化计数触发。没有这些变化、但进入 S 后规则才合法的实例，未必执行该算法。应补阶段事件覆盖，同时保留 R/S 不同 defined/poison 合同。这是中端覆盖结论，部分结果可能已有后端补偿，本轮没有测其机器影响。
2. **候选定价前的 scratch 清理能力少于提交后的 pipeline。** [cleanup][scratch] 仅 seam/SCCP/observation/CFG，[driver][driver-cost] 随后定价或拒绝；没有已有 promotion/forward/DSE/GVN/division/PRE 的同等覆盖。[递归准备][arguments-nested] 还未消费 `_nested` 的 argument_slots。扩展清理必须维护队列 NodeId、深度与不可退款工作预算；每个 splice 后全图重跑不是默认方案。
3. **CFG 剪枝未普遍触发 promotion 重试。** [完整 S 路径][s-pipeline] 及 [seam][seam-promotion] 先 promotion、后剪枝，后续没有通用 repromotion；删掉逃逸 use 后可能留下新机会。相比之下，[unroll][unroll-promotion] 已明确回访 exposed_slots，可复用其变化种类思路。

更大的 IPO 缺口包括：

- [wrapper 准备][wrapper] 只有两轮且源图 ≤128；没有一般调用 DAG/递归 SCC 的 callee-first 简化、版本化发布和重定价闭环。
- [identity summary][identity] 只处理无额外活节点、单直线无环路径、直接返回入口参数。缺通用常量/returned-value 摘要与跨调用点 IPSCCP。替换返回值仍须保留调用的副作用和不返回行为；删除调用需要另外证明。
- [实参事实][argument-facts] 主要是 singleton scalar 和深度 2 字段，不是范围、关系或一般 enum-variant 事实；[类型权限][permissions] 不是图驱动函数效果推导。
- [driver 调度][driver-schedule] 的 branch-driver 优先、splice 后清理与重排主要属于 Preparation；Emission 完整 seam 在整轮后，没有带变更原因的拒绝重访。扩大统一 inline 阈值不能补齐上述机制。
- [默认 inline policy][inline-policy] 仍是 Legacy；可选 [size policy][inline-size] 主要消费静态代码单位，尚无频率、净收益或完整降低/RA 成本。

### 标量与控制：不能把已有名字视为 LLVM 覆盖

[GVN][gvn] 明确排除 memory、地址、call、Phi 与 anchored value；[PRE][pre] 只翻译纯标量。[SCCP][sccp] 是 `Unknown/Constant(u64)/Overdefined`，没有范围/enum/memory lattice。[Ranges][ranges] 有有限局部区间推理，不是一般符号关系和循环区间求解。

[KnownBits][known-bits] 已有 budget 32 的局部查询；[DemandedBits][demanded-bits] 已合并多 use，但受 depth 8/64 use edges 限制，对 arithmetic/Phi/memory/aggregate 大多视为全位需求。[InstCombine][instcombine] 有 exact/demanded 规则与最多 4 轮扫描；重关联主要是同运算嵌套常量，不是一般表达式树 Reassociate。

[CFG cleanup][simplify] 会将可达控制/固定/anchored 操作作为根，尚无 postdom/control-dependence ADCE。已有 variant/条件 threading 与 diamond 简化，不能再写成完全没有 JumpThreading/CVP 类能力。ADCE、安全 if-conversion、循环删除都需单独处理终止性、trap/poison 与观察顺序。

## 实施优先级与验收

优先级根据跨程序覆盖和依赖关系确定，**不代表已测收益排序**。

| 顺序 / 工作线 | 首个可验收结果 | 后续解锁 |
|---|---|---|
| 0：已有能力的阶段覆盖 | 显式消费 R→S、CFG/effect/slot 变化；scratch 定价前有预算清理；记录未运行/不合法/预算拒绝原因 | 让已有算法可靠兑现，不再靠某个函数恰好越过阈值 |
| 1：共享内存查询 + partial SROA + MemCpyOpt | 一部分字段保留存储、一部分 SSA；copy 链保留 snapshot 时序且消除中转；聚合覆盖 DSE | memory GVN、合流值 Phi、load PRE、memory LICM |
| 2：循环结构 + memory LICM / 符号 IV | 普通循环可规范化；不被写且执行安全的读取外提；可证明边界与地址递推 | 更一般展开、LoopIdiom/Deletion/Unswitch，随后 LoopVectorize/SLP |
| 3：机器访存折叠 + RA/调度成本 | 生产 constrained path 可合法选择内存 operand；压力用例减少多余 reload/copy；调度兼顾关键路径与活跃范围 | 让中端优化的收益保留到机器码；静态频率布局和更广重物化 |
| 4：IPO 摘要闭环 + 标量/控制扩面 | 返回值/函数效果摘要、callee-first 简化；ADCE/重关联/更广事实传播各有独立验收 | 较深 helper 组合和控制程序，不依赖指定符号 |

第 1、2、3 条是独立的主要工作线。函数效果摘要需要与内存查询共同设计；不必先完成完整 IPSCCP 才提供保守效果查询。向量化是明确的较大投资方向，不能由有限宽读重组或小循环展开宣称替代。

每项实现前先固定独立程序家族，并让未参与选型的真实负载保持留出：

| 家族 | 覆盖与必须拒绝/保留的情况 |
|---|---|
| 聚合/分区 | 嵌套产品、不同长度数组、独立字段与动态索引共存、whole snapshot；padding、部分初始化、重叠、union/enum 活跃字段、lifetime 复活 |
| 拷贝/调用 | copy 链、多消费者、返回槽、局部/外部混合对象；源后改、目标观察、新重叠、捕获、调用经另一别名写入 |
| 合流/冗余 | 两入边不同 store 后 load、单边已有 load、重复读取夹独立写；错误 Phi 槽、无效地址上新增读取、poison/trap 顺序 |
| 循环 | 零/一/多次、符号边界、嵌套、body 分支、多出口、条件读取；wrap、提前退出、同址更新、未知 call、SlotDead |
| 标量/控制/IPO | 不同宽度和多 use、重关联树、相同结果的分支、不同深度 helper DAG 与递归 SCC；不返回调用、外部入口和地址逃逸 |
| 机器 | 不同整数宽度、tied/fixed operands、call clobber、flags 活跃、spill 与 copy cycle、不同热冷分布；对比最终机器操作及执行 |

验收分三层：规则语义/负例与随机差分；完整生产路径的触发、证明与机器产物；独立负载的运行时间、编译墙钟/指令、代码体积和 spill。随机输入只使用有定义执行；变换拒绝和 poison/未初始化等边界使用 IR 语义与验证器检查，不能把 UB 的一次原生输出当正确性依据。

改变常量、字段排列、数组长度、wrapper 深度、内联/CGU 形式后，应仍由同一证明机制决定是否应用。预算阈值可以限制编译工作，但不能以让指定函数进入某个阈值作为优化验收目标。与 LLVM 对比时固定 target/features、优化级别与链接条件，并分别报告每个程序家族，保留回退项。

## 已过时结论与暂停状态

旧 [generic-optimization-audit.md][historical] 是历史输入的快照。以下说法不能用于当前树：promotion 不支持固定索引；没有 DSE/PRE/IV/小展开；seam 没有 GVN/division/forward；CGU 在准备前一律挡住图；没有 x86 SIB；没有 DemandedBits 或任何重物化。当前 [seam][seam-exposed] 已批量接入冗余消除，[preparation_graph][preparation-graph] 已区分准备来源和最终存活引用检查。

最近围绕窄数组和透明指针调用阈值的两个实验已保存并撤回；继承的通用机制保留。本轮只新增本报告、输入指纹，并给旧核查添加历史说明，没有实施新的编译器优化。最新冻结候选仅完成 correctness/structure 验收，[正式计时未启动][pause]，不能赋予新的性能分数。

[inputs]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/docs/plan/poc/llvm-parity/llvm-gap-20261110/generic-pass-gaps-20261111-inputs.json
[study]: /Users/shom/Downloads/llvm-study/README.md
[llvm-function]: /Users/shom/Downloads/llvm-study/references/cache/llvm/lib/Passes/PassBuilderPipelines.cpp:566
[llvm-ipo]: /Users/shom/Downloads/llvm-study/references/cache/llvm/lib/Passes/PassBuilderPipelines.cpp:955
[llvm-ipsccp]: /Users/shom/Downloads/llvm-study/references/cache/llvm/lib/Passes/PassBuilderPipelines.cpp:1169
[llvm-vector]: /Users/shom/Downloads/llvm-study/references/cache/llvm/lib/Passes/PassBuilderPipelines.cpp:1299
[llvm-slp]: /Users/shom/Downloads/llvm-study/references/cache/llvm/lib/Passes/PassBuilderPipelines.cpp:1386
[llvm-memcpy]: /Users/shom/Downloads/kfrc-llvm-one-x-20261111/s-retyped-array-paused-20261111/llvm-reference/MemCpyOptimizer.cpp:1107
[promotion]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/promotion.rs:205
[promotion-ssa]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/promotion.rs:407
[memory-facts]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/memory.rs:59
[locations]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/memory_locations.rs:49
[forward-preserve]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/forward.rs:349
[forward-phi]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/forward.rs:239
[dse-values]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/dse_values.rs:19
[dse-branches]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/dse_branches.rs:25
[call-target]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/graph.rs:505
[private-memory]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/private_memory.rs:163
[private-copy]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/private_memory_copy.rs:65
[partition]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/memory/partition.rs:47
[copy-lowering]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/semantic_memory.rs:229
[loop-simplify]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/control/loop_simplify.rs:1
[induction]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/induction.rs:1
[loop-count]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/loops/count.rs:47
[loop-plan]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/loops/plan.rs:76
[strength]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/loops/strength_reduce.rs:1
[gcm]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/schedule.rs:399
[x86-lower]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower.rs:90
[allocated]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/allocated.rs:22
[emit]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/allocated/emit.rs:440
[scan]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/allocated/scan.rs:312
[remat]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/allocated/rematerialize.rs:13
[schedule]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/schedule.rs:27
[cost-model]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/cost.rs:82
[division]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/division.rs:21
[r-prepare]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/specialize/mod.rs:120
[s-prepare]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen.rs:383
[seam-exposed]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/pipeline.rs:340
[scratch]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/ipo/arguments/cleanup.rs:26
[driver-cost]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/ipo/driver.rs:235
[arguments-nested]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/ipo/arguments.rs:486
[s-pipeline]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/pipeline.rs:195
[seam-promotion]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/pipeline.rs:271
[unroll-promotion]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/pipeline.rs:425
[wrapper]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen.rs:65
[identity]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/ipo/identity.rs:95
[argument-facts]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/ipo/arguments.rs:577
[permissions]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/permissions.rs:55
[driver-schedule]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/ipo/driver.rs:363
[inline-policy]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/inline_cost.rs:7
[inline-size]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/ipo/cost.rs:169
[gvn]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/gvn.rs:1
[pre]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/pre.rs:1
[sccp]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/control/sccp.rs:13
[ranges]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/ranges.rs:81
[known-bits]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/known_bits.rs:40
[demanded-bits]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/demanded_bits.rs:27
[instcombine]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/instcombine.rs:38
[simplify]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/control/simplify.rs:97
[historical]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/docs/plan/poc/llvm-parity/llvm-gap-20261110/generic-optimization-audit.md
[preparation-graph]: /Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen.rs:167
[pause]: /Users/shom/Downloads/kfrc-llvm-one-x-20261111/validation/validation-pause-status.json

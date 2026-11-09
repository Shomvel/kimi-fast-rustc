# SoN verifier 契约（draft-0）

这是目标 verifier 契约，不表示所有检查已经实现。当前入口在
[verify.rs](../../../crates/kfrc-son/src/verify.rs)。历史 T01 中的跨分支使用说明只检查
元数不够；现在还须区分语义要求、事实凭据、内存版本和策略提示，见
[requirements-and-facts.md](requirements-and-facts.md)、[constraints-and-effects.md](constraints-and-effects.md)。

## 检查清单

按成本从低到高排列，可分级启用（`structural` 总是开；`full` 在测试和 `--verify` 下开）。

| 级别 | 检查 | 错误 |
|---|---|---|
| structural | 输入存在、未被删除 | `DanglingInput` |
| structural | 元数：每种节点的 control/值/effect 输入个数 | `WrongArity` |
| structural | Phi 的 `values.len() == region.preds.len()` | `PhiArity` |
| structural | Phi 的 region 确实是 Region；Projection 的输入是多结果节点且下标合法 | `BadProjection` |
| typed | 每个值输入的 `ValueTy` 符合 opcode 要求（如 `Add` 两侧同宽整数、`If` 条件为 `Int{bits:1}`） | `TypeMismatch` |
| typed | Phi 的所有输入类型与自身相同；效果 Phi 只接效果 | `TypeMismatch` |
| typed | 效果输入的类型是 `Effect`；有副作用的节点都有效果输入 | `MissingEffect` |
| full | 控制可达：每个有 control 的节点从 Start 可达 | `Unreachable` |
| full | **支配**：值 `v` 被节点 `u` 使用时，`v` 的放置块支配 `u` 的块；`u` 是 Phi 时，`v` 支配对应前驱的末尾 | `DominanceViolation { def, use }` |
| full | 循环：每个循环 header Region 恰有一个入口前驱；回边来自循环体 | `BadLoop` |
| full | 纯值节点之间无环（环只能经过 Phi） | `DataCycle` |
| full | 效果链在**每条可执行路径上**线性：同一效果 token 可以被多个写效果消费，但这些消费者的控制必须两两互斥（位于同一分支的不同后继，彼此不可达）；互斥路径在合并点经效果 Phi 汇合。`if c { store A } else { store B }` 两臂消费同一 token 是合法的 | `EffectFork` |
| full | may-trap 节点（除法、Check）的 control 不早于其守卫条件 | `TrapHoisted` |
| structural | 节点种类在当前 `GraphLevel` 合法（R 级无布局常量，L 级无 Rust 聚合/孔位） | `IllegalAtLevel` |
| typed | 内存访问取了指针根集合中每个分区的 token（根集合穿过 Phi/Select/指针 Cast/聚合字段，[memory-partitions.md](memory-partitions.md#规则)） | `PartitionMismatch` |
| full | 分区在已逃逸的控制点之后不再单独使用；汇合处已合并进 `Unknown` | `EscapedPartitionUse` |
| full | 固定 `Load`/`Store` 带控制输入；`Load.spec` 满足 [semantics.md](semantics.md#load固定与投机) 的条件；Release 原子与 fence 取了全部 `Unique`/`Local`/`Heap` token | `UnpinnedAccess` / `BadSpeculation` / `FenceMissingPartition` |
| placed | 放置后浮动 load 不晚于同分区后续的写 | `AntiDependence` |
| placed | 放置后 Release 原子与 fence 之前的 `Unique`/`Local` 写没有被移到它之后（另需多线程 litmus 测试，顺序语义观察不到这类错误） | `ReleaseOrder` |
| typed | 节点种类属于 [semantics.md](semantics.md#执行模型) 四类中的哪一类已登记；立即 UB 节点不浮动 | `UnclassifiedNode` |
| full | 利用变体匹配的消费者具有同值、同变体、同动态实例的凭据；无凭据的纯 VariantField 保持不匹配时 poison，不得据此生成无保护物理 load | `UnprovenVariant` |
| full | 凭据来源有效且 scope 支配消费点；Phi 按前驱边，循环不能用下一轮或自身证明；内存依赖未失效 | `FactEscapes` / `StaleWitness` |
| typed | Require 无新业务值和内存读写；Assume 的布尔条件与 Check 的真实失败行为不混同 | `BadRequirement` |
| full | Body 指定的所有权和 drop 义务正确转移、执行或合法解除；允许 forget、ManuallyDrop、Copy 和合法泄漏 | `OwnershipObligation` |
| full | EndBorrow 覆盖 scope 的有限正常及 unwind 出口；之后不使用其凭据，不把结束 scope 等同地址死亡 | `BorrowScopeClosed` / `ExpiredBorrowWitness` |
| typed/full | MemMerge 是同点分区视图，效果 Phi 选择同一分区的前驱版本；逃逸并区后依赖覆盖完整 | `BadMemoryView` / `PartitionMismatch` |
| full | SlotLive／Dead 的动态寿命与初始化分离；move 不自行结束槽寿命；跨新寿命不复用旧访问凭据 | `StorageLifetimeMismatch` |
| placed | 证明消费者未被误当执行需求；擦除要求及 scope 前，实际访问、may-trap、原子和 fence 依赖已落实 | `LostPlacementDependency` |

全量（full）检查只在级别边界运行：R 图发布前、S 级结束、L 级结束；debug 构建可以逐 pass 打开。

支配检查可用 SoN 的控制依赖计算最早／最晚块，复用
[analysis](../../../crates/kfrc-son/src/analysis) 中的 CFG 和 dominators。
GraphState::Verified 仅针对已检查的图版本；图、类型事实、凭据或 scope 改变要使相关
验证和缓存失效。结构检查不应以“事实为 Assumed”为由跳过来源与作用域验证。

## 输出

- 错误带 `NodeId` 和一段文本 dump（节点及其输入两层），测试可断言错误种类。
- `dump(graph) -> String` 输出稳定：按 NodeId 排序，格式与现在 `n12 Phi ctl=[…] data=[…]` 类似，加上类型列。

## 必须有的负例测试

T01 的每个形状构造成图后都要被 `DominanceViolation` 拒绝；另外手工构造：Phi 元数不对、跨图 NodeId、`If` 条件为 64 位整数、Store 缺效果输入、除法被提到守卫之前。

新增契约测试：父 enum 凭据用于子 enum、循环动态值错配、nonnull 充当可访问证明、
只读但越界的投机 load、布局引入调用后 Local 失效、不同分区直接拼效果 Phi、
SlotLive 重用旧寿命凭据、EndBorrow 后用过期 scope。合法对照包括部分 move 后访问
其他字段、forget、真正无限循环和外部不返回调用。Proven 失败、解释器不支持和
执行超预算都不得归类为源 UB；相等与 refinement 测试按声明的边界分别运行。

## 其他层的 verifier


SoN 之外各层也有 verifier。verifier 通过不等于算法正确，最终以真实执行对拍为准。

| 层 | 检查 |
|---|---|
| Typed Body | 每个 Place 的类型和投影/布局、局部初始化、终结指令、退出路径；所有权结果和 drop 能对应回源位置 |
| SoN | NodeId 有效、数据类型、控制支配、Phi 与前驱对应、循环入口/回边、内存状态汇合、调用/检查/异常效果连续。拒绝：内存节点缺效果边、may-trap 节点移出其控制条件、一个分支的定义被另一个分支使用。区分合法循环和误构造的数据环 |
| 放置后的块 | 每条控制路径保留效果；节点移动满足支配和效果约束 |
| Machine IR | 操作数宽度/类别、固定寄存器、调用 clobber、控制和 flags 依赖、定义/活跃区间、spill 地址、分支目标 |
| 分配后 | 所有虚拟值有位置、栈对齐、callee-save、边上并行复制正确 |
| 对象 | section/symbol/relocation 一致、目标特征匹配、链接并真实执行成功 |

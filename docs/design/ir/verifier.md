# SoN verifier 契约（draft-0）

现在的 `Graph::validate`（[son.rs:432](../../../src/son.rs#L432)）只检查输入节点存在和各 `NodeKind` 的输入元数。T01 的探测显示错编译图能通过它（合并点后使用分支内定义的值），所以新 verifier 必须包含支配检查。

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
| full | `VariantField` 被同一变体的 `VariantSwitch` 出口或 `Refine` 支配 | `UnprovenVariant` |
| full | `Refine` 的使用者都被它的控制支配 | `RefineEscapes` |
| full | 需要 drop 的值在每条路径上恰好被消费一次（以 `Unreachable`/`Terminate` 结束的路径除外） | `OwnershipLinear` |
| full | `EndBorrow` 联合后支配对应 `Reborrow`；之后不再使用子分区地址 | `BorrowScopeClosed` / `UseAfterEndBorrow` |

全量（full）检查只在级别边界运行：R 图发布前、S 级结束、L 级结束；debug 构建可以逐 pass 打开。

支配检查需要先放置（或用 SoN 的 control 依赖计算最早/最晚块）。可以复用 [placement.rs](../../../src/placement.rs) 的结果和 [cfg.rs](../../../src/cfg.rs) 的支配计算。

## 输出

- 错误带 `NodeId` 和一段文本 dump（节点及其输入两层），测试可断言错误种类。
- `dump(graph) -> String` 输出稳定：按 NodeId 排序，格式与现在 `n12 Phi ctl=[…] data=[…]` 类似，加上类型列。

## 必须有的负例测试

T01 的每个形状构造成图后都要被 `DominanceViolation` 拒绝；另外手工构造：Phi 元数不对、跨图 NodeId、`If` 条件为 64 位整数、Store 缺效果输入、除法被提到守卫之前。

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

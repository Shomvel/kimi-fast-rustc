# SoN verifier 契约（draft-0）

现在的 `Graph::validate`（[son.rs:432](../../../../src/son.rs#L432)）只检查输入节点存在和各 `NodeKind` 的输入元数。T01 的探测显示错编译图能通过它（合并点后使用分支内定义的值），所以新 verifier 必须包含支配检查。

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
| full | 效果链线性：同一效果 token 至多被一个写效果消费（首版保守链） | `EffectFork` |
| full | may-trap 节点（除法、Check）的 control 不早于其守卫条件 | `TrapHoisted` |

支配检查需要先放置（或用 SoN 的 control 依赖计算最早/最晚块）。可以复用 [placement.rs](../../../../src/placement.rs) 的结果和 [cfg.rs](../../../../src/cfg.rs) 的支配计算。

## 输出

- 错误带 `NodeId` 和一段文本 dump（节点及其输入两层），测试可断言错误种类。
- `dump(graph) -> String` 输出稳定：按 NodeId 排序，格式与现在 `n12 Phi ctl=[…] data=[…]` 类似，加上类型列。

## 必须有的负例测试

T01 的每个形状构造成图后都要被 `DominanceViolation` 拒绝；另外手工构造：Phi 元数不对、跨图 NodeId、`If` 条件为 64 位整数、Store 缺效果输入、除法被提到守卫之前。

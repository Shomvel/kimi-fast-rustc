# use-list：要不要持久维护

## 别人付了多少（【来源】）

| 系统 | 结构 | 每条边的代价 | 来源 |
|---|---|---|---|
| LLVM `Use` | `Val`、`Next`、`Prev`、`Parent` 四个指针 | 32 B（按成员推算）；2020 年加 `Parent` 时 CTMark 内存 +1.8%、编译时间 −1.1%；deal.II 峰值 273 万个 `Use`，多出的指针约 21 MB | [Use.h](https://github.com/llvm/llvm-project/blob/main/llvm/include/llvm/IR/Use.h)；[RFC](https://discourse.llvm.org/t/rfc-removing-waymarking-from-use/54935)、提交 ff9379f（2020） |
| LLVM 常量 | 2025 年起 `ConstantData` 不再有 use-list（PR #137313/#138961），也不再计数（#137314：“维护计数不是免费的”） | 没有公布性能数字；2026 年 #223303 继续去掉元数据对它的使用跟踪，`PoisonValue` 曾有“最大的 use-list” | [#137313](https://github.com/llvm/llvm-project/pull/137313)、[#223303](https://github.com/llvm/llvm-project/pull/223303)（2025–2026） |
| LLVM use-list 顺序 | 2015 年提议默认保存 use-list 顺序 | bitcode +5–23%，`llvm-as` +3.5–39%；不保存时 33 个 SPEC 可执行文件在 bitcode 往返后变化 | [llvm-dev 2015-04](https://lists.llvm.org/pipermail/llvm-dev/2015-April/084113.html) |
| MLIR `OpOperand` | 侵入式链表 `nextUse`、`back`、`owner` + `value` | 约 32 B（推算），无成本文档 | `mlir/IR/UseDefLists.h` |
| Swift SIL `Operand` | `TheValue`、`NextUse`、`Back`、`Owner`（注释：“可以压缩”） | 约 32 B（推算） | `include/swift/SIL/SILValue.h` |
| TurboFan | `Use` 链表放在节点前 | V8：节点大“主要因为同时指向输入和使用” | [V8 博客 2025](https://v8.dev/blog/leaving-the-sea-of-nodes) |
| C2 / Graal / Simple | `_out` 数组 / 2 个内联 + 数组 / `_outputs` 数组 | 数组式，每条边 1 个指针（8 B）加增长余量 | 见 [prior-art-son-nodes.md](prior-art-son-nodes.md) |
| Turboshaft | 只有饱和的 1 B 使用计数 | 1 B/操作 | `turboshaft/operations.h` |
| Cranelift | 无；lowering 时按需算 {Unused, Once, Multiple} | 0 | `machinst/lower.rs` |

Swift 的相邻经验：把“块/指令集合”从哈希表换成节点里的位域和按块下标的数组（2021–2022，PR #35448、#35521、#60035），每个 SILNode 多 1 个字，作者“没看到内存或编译时间的相关变化”。这说明旁表从哈希表换成下标数组几乎是免费的。

## kfrc 的需求：谁需要“谁用了我”

| 用途 | 需要精确 use-list？ | 替代 |
|---|---|---|
| `replace_uses(old, new)` | 否 | 转发表，读输入时解析，pass 结束一次扫描压实 |
| worklist：节点改了，把使用者入队 | 需要“候选使用者” | 近似 users 索引（下文） |
| DCE | 否 | 从根标记（现状就是这样） |
| “只有一个使用”（内联、融合、地址模式） | 只需计数 | 饱和计数 1 B，或 placement 前一次计算 |
| placement：最晚放置要看所有使用 | 需要，但只在 L 级末尾一次 | 进入 placement 时建 CSR 反向索引 |
| verifier | 否 | 正向扫描 |

## 实测（【实测】，见 [bench-layout.md](bench-layout.md#t2-use-list-与-replace_uses)）

| 方案 | 额外内存 | 构建 | 替换 N/20 个节点（64k / 1M） |
|---|---|---|---|
| 现状：每次全图扫描 | 0 | 0 | 1 s / 未测（O(K·N)） |
| 维护 use-list `SmallVec<[u32;2]>` | 30 B/节点 | 41–57 ns/节点 | 0.41 ms / 17.2 ms |
| CSR 快照 | 12 B/节点 | 6–19 ns/节点 | — |
| 转发表 + 一次扫描 | 4 B/节点 | 0 | 0.46 ms / 11.2 ms |

维护 use-list 的额外内存（30 B/节点）比 16 B 的节点本身还大，正是 V8 说的“节点大”的来源。

## 建议（【推断】）

**不持久维护 use-list。** 用三样东西替代：

1. **转发表**（`forward: SecondaryMap<NodeId, NodeId>`，4 B/节点，pass 内分配）。`replace(old, new)` 只写 `forward[old] = new`，并给 `old` 打 `DEAD`；读输入经 `resolve()`（带路径压缩）。pass 结束时一次顺序扫描把所有输入改成终点，然后丢弃转发表。
2. **近似 users 索引**：只增不删，读时校验。在**复制或降低图时**顺便建 CSR（R→S 的 `specialize`、S→L 的降低本来就要遍历全图，额外成本约一次顺序扫描）；pass 中新增的边追加到一个小的 `(def, user)` 溢出向量。删除边时不更新索引；读 `users(x)` 时过滤掉已经不再以 `x` 为输入或已 `DEAD` 的候选。worklist 只把它当“可能受影响的节点”的来源，多入队一个节点是安全的。
3. **饱和使用计数**（节点记录 `flags` 里的 2 位：0、1、多），在建索引时算出，供“是否单一使用”判断。

这与“变化驱动、工作量与变化量成正比”（[compile-time.md](../../design/goals/compile-time.md)）一致：索引的 O(N) 建造只发生在级别边界，那里本来就有 O(N) 的复制；级别内每次改动只碰改动节点的输入和候选使用者。

不采纳的方案：

| 方案 | 理由 |
|---|---|
| C2/Graal 式持久 `_out` 数组 | 每节点 +30 B 以上；每次改边要双向维护；V8 的教训 |
| LLVM 式侵入链表 | 每条边 32 B，链表遍历不连续 |
| 每个 pass 开头建精确 CSR | 对只有少量种子的 S 级特化，O(N) 成为下限 |

## 风险

| 风险 | 缓解 |
|---|---|
| 漏用转发解析，读到死节点 | debug 构建中读 `DEAD` 节点即 ICE；verifier 检查无输入指向 `DEAD` |
| users 溢出向量在长 pass 中变大 | 超过节点数的 1/4 时重建 CSR（摊还） |
| 外部代码直接持有 `NodeId` 跨 pass | 契约规定 `NodeId` 只在图内有效（[ids.md](../../design/ids.md)），跨 pass 保存的 id 要经转发解析 |

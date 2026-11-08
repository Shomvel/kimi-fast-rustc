# 优化流水线

## 共同机制

- **构建期窥孔：** 每建一个节点就做常量折叠、代数化简和 GVN 哈希查找（Click 的做法）。大部分“清理临时值”的工作在这里完成。规则准入条件：按固定代价**单向化简**（不加交换律、结合律这类非化简恒等式）；右侧递归重写有层数和次数上限；可标记 `subsume`（结果严格更好，直接替换）。Cranelift 的同类一遍重写在约 400 万值节点中只错过 2 次等价（[prior-art/egraph-rewrite.md](../../research/prior-art/egraph-rewrite.md)）。
- **语义约束：** 不由 `p == q` 把 `q` 换成 `p`（provenance 可能不同）；浮点常量折叠选与 x86 硬件相同的 NaN；规则表每条带 `poison_producing`/`introduces_ub_point`/`speculates`/`float_bits` 标志（[semantics.md](semantics.md)）。
- **声明式规则表：** 窥孔规则写成一张表（ISLE 风格），每条规则一份记录，生成匹配代码和 SMT 查询，并做 overlap 检查。整数规则用 SMT 证明或 ≤8 位穷举检查，CI 读缓存的证明结果，缺失即失败（Alive 在 334 条 LLVM 规则里找出 8 条错误；Arrival 已进 Cranelift CI）。不做整图翻译验证。
- **GVN 表：** hashbrown `HashTable<(NodeId, u32)>`，表项存 32 位哈希，Fx 哈希（SipHash 慢 2.4–3.8 倍）；在表中的节点改输入前先移出。
- **worklist 顺序：** 初始按 `NodeId` 升序（即创建顺序，局部性好），之后 LIFO；记录访问数与改动数。
- **变化驱动：** 每个 pass 由 worklist 驱动，入口给出种子节点集合；没有种子就不运行。pipeline 不按固定序列重复跑整图。
- **乐观分析：** SoN 上可以把 SCCP 与 GVN 组合成一次乐观分析（Click 的组合分析），穿过循环 Phi 传播常量，对应 LLVM 的 SCCP/IPSCCP 在函数内的部分。
- **燃料与预算：** 每个 pass 有确定性的步数上限，内联有节点增长上限；用完就停，结果仍是合法图。
- **可观测：** 每个 pass 记录访问节点数、改动数、耗时；“访问了但没改”的比例是要压低的指标（LLVM 的 InstCombine 是 83%）。

## R 级

对每个定义跑一次，结果缓存并发布。

| 规则/pass | 用到的 Rust 语义 |
|---|---|
| 聚合折叠：`Field(Aggregate)`、`Field(InsertField)`、`VariantOf(MakeVariant)`、`VariantField(MakeVariant)` | 聚合是值 |
| **按前驱拆分：** `VariantSwitch`/`If` 的输入是 `Phi`，且某些前驱的值是 `MakeVariant`/常量时，把这些前驱直接接到对应出口（`?` 内联后 `ControlFlow` 就是这种 Phi，W01；rustc 因 GVN 排在 JumpThreading 之前而错过） | 变体是一等操作 |
| 比较链合并：同一 `VariantOf` 上的常量比较链 → 一个 `VariantSwitch` | |
| `Discriminant` 规范化（[nodes.md](nodes.md#纯值)），derive 的 `eq` 折叠成变体相等 | 判别值互异 |
| 常量 Phi 变条件：`Phi(If c; false, true) = Not(c)`。drop flag 折叠就是这条（W04 中 O3 的结果是 `xor i1 %flag, true`），不需要专用节点 | |
| 内联（见下文“内联策略”） | |
| 槽位提升：引用全部消失的 `StackSlot` 退回 SSA | 借用在 Body 上显式 |
| 分区内 load/store 转发、死 store 删除；写回已知当前值的 store 删除（借助 `Refine`，`RefCell` 的计数恢复靠它，W10） | 分区 |
| `Const` 分区上常量偏移的 load 折叠 | 常量内存 |
| 检查消除：`Refine` 已证明的 `Check`、len 相关区间推理 | 宽指针 len 是值，类型有范围事实 |
| DCE（含不可达控制） | |

**R 级能内联谁。** 非泛型函数没有孔位，R 图就是 S 图，它调用的全是已解析的实例（包括 `Vec<u8>::push` 这种具体类型的泛型实例），内联的是被调者的 S 图。泛型函数只能内联实参全由本函数 `Param` 原样转发和具体类型组成的被调者：拼接时 `Param` 换名，被调者的孔位表和 mentioned 集合并入调用者。

## S 级

只处理特化替换过的节点及其传播范围（[generics-specialization.md](generics-specialization.md#特化)）。规则与 R 级相同，额外：

- 新解析出的直接调用进入内联决策；`CallVirtual{slot}(WidePtr(_, Const(VTable K)))` 查答案表变成 `Call(K.entries[slot])`。
- `Alloc` 之后整块置零 → `Alloc{zeroed}`；配对的 `Alloc`/`Dealloc` 且未逃逸 → 删除。

## 调用摘要

```rust
pub struct CallSummary {
    may_unwind: bool, may_diverge: bool,
    memory: MemoryEffect,              // None | ReadArgs | WriteArgs | Unknown（按指针实参细分）
    captures: SmallBitSet,             // 哪些指针实参可能被捕获
    writes_indirect_args: SmallBitSet, // 哪些 Indirect 传入的聚合参数被改写（决定调用者能否不拷贝）
    size_estimate: u32,                // S 图节点数估计；未优化图会高估，按实测化简系数折算（A/D/B 校准）
    const_return: Option<ConstId>,     // 返回值恒定时折叠调用者（IPSCCP 的过程间部分，先测运行时价值）
}
```

- **在未优化的 S 图上用独立的廉价遍历计算**，不等被调者的优化和 L 级。否则调用图深度直接变成串行等待：W04 中 `work` 的调用图高度是 8，高层每层只有一个函数。精度损失用 A/D/B 测量。
- 调用图的 SCC 一起计算：从 **⊥**（不 unwind、不写、不捕获）开始迭代到最小不动点，迭代顺序固定。LLVM 能证出互递归的 `even`/`odd` nounwind（W04），从保守值开始则证不出。
- `CallVirtual`、`CallIndirect` 的摘要取顶。二进制 crate 已知全部 vtable 时取同槽摘要的并，列为后续扩展。
- 摘要单独发布，进读它的调用者的机器码键（[cache.md](../build/cache.md#依赖清单键要包含的读)）。

## 内联策略

确定性函数，输入：调用点是否在冷区域、循环深度、被调者 `size_estimate`、被调者是否只有这一个调用点且不导出、`#[inline]` 属性、**构建模式**。

- 冷区域不内联（`#[inline(always)]` 除外）。
- 被调者的图已经优化好：内联是拼接图，接缝处（参数、返回、分区 `Reborrow/EndBorrow`）作为 worklist 种子，不重新跑被调者内部的优化。
- 体积准则：`size_estimate` 小于调用开销，或内联后被调者的外部副本可以删除。
- **构建模式：** 增量/dev 构建跨 crate 只内联 `#[inline]` 和小于阈值的函数，否则改动上游的普通函数会让下游全部失效（rustc 增量模式也关闭了跨 crate 内联推断，W08）。release 构建按上面的准则。

## L 级

| 步骤 | 说明 |
|---|---|
| 布局降低 | 聚合按 PassMode 拆分或进槽、enum 按 tag 值分支、`FieldAddr` → 偏移、`DropValue` 展开、`Check` → `If`、`Repeat` 按阈值展开或调用 memset |
| 机器级窥孔与 GVN | 降低产生的地址/比较/扩展；分区仍在，load 消除继续用 |
| 常量除法/取模 | 乘法逆元；`x % c == 0` 判定（W02） |
| switch 降低 | 跳转表、位测试、查找表、比较树按阈值选择；查找表进只读数据并按内容去重（LLVM 的 `switch-to-lookup` 与 SelectionDAG 承担这部分） |
| 内部调用约定 | 被调者不导出、不取地址时，按摘要改参数传递（小 `&T` 按值、删除死参数，对应 ArgPromotion/DeadArgElim）；内部 ABI 答案进答案键 |
| 调试变量位置 | 变量位置作为旁路信息随降低传递，生成 DWARF 位置表，不进图哈希 |
| if-conversion | `Select` → `cmov` |
| 强度削减与地址模式 | 下标乘法 → 指针递增；`base + idx*scale + disp` |
| 循环 | loop-rotate（给 LICM/PRE 造前置块）、loop-deletion（需要终止性事实）、loop-idiom（识别 memset/memcpy）、unswitch 逐项验收；归纳变量化简；穿过分区 Phi 的 load 转发与 PRE（W05：只要有一条分支调用，`cap`/`ptr` 的 token 就随循环变化）；store 下沉可选。展开、向量化在 T38 之后单独验收 |
| 冷区域布局 | 冷块放末尾，相同 panic 尾块合并 |
| elaboration 式 placement → Machine IR | 只在这里做一次：从控制骨架出发按需放置浮动节点，同时做作用域 GVN、LICM 和常量/地址再物化，DCE 随之免费 |

## 和 LLVM 流水线的对照

rustc release 默认对每个函数“化简两遍”（ThinLTO pre-link + post-link，InstCombine 静态出现 13 次），时间分布平坦，最大的 InstCombine 类也只占 LLVM 时间 13.6%。逐 pass 族的承接关系与缺口见 [llvm-mir](../../research/llvm-mir/README.md) 和 [mapping-to-design](../../research/llvm-mir/mapping-to-design.md)。

## 不做

- equality saturation、超优化：扩大搜索空间，与编译时间目标冲突（eggcc 编译时间约为 LLVM -O3 的 11 倍）。
- aegraph 的多版本 e-class（union）：Cranelift 2026 的数据显示收益约 0.1%，平均 e-class 只有 1.13 个节点。
- 静态 ILP 指令调度（总览的固定决策）。
- 依赖 profile 的分层优化（E1 数据否定）。
- Machine outliner（见 [size.md](../goals/size.md#不做)）。

# 三个级别

一个 `FunctionGraph`（[design/ir/graph.md](graph.md)）在生命周期里经过三个级别。级别是图的状态字段，每个节点种类声明自己在哪些级别合法；verifier 按当前级别检查。

```rust
pub enum GraphLevel { Rust, Specialized, Layout }
```

## R 级：Rust 语义，多态

- **来源：** 每个 typeck root 的 CheckedBody（清理后），在**单态化之前**构建。非泛型函数也从 R 级开始，只是没有孔位。
- **值类型：** `TyId`，可以含 `Param`。聚合、enum、宽指针、引用都是值。
- **合法节点：** 全部 Rust 级节点 + 孔位（`SizeOf(T)`、`TraitCall`、`DropValue` 于 `T` 等，见 [nodes.md](nodes.md)）。不能出现布局常量（字段偏移、tag 值、niche 范围）。
- **优化：** 与 `T` 无关的那部分（[optimization-pipeline.md](optimization-pipeline.md#r-级)）。结果按内容哈希，**每个定义每次构建只做一次**，并作为函数体放进 metadata。

## S 级：Rust 语义，单态

- **来源：** `specialize(R 图, 答案)`。答案是孔位查询的结果（布局形状、所选 impl、`NeedsDrop` 等），见 [generics-specialization.md](generics-specialization.md)。
- **值类型：** 不含 `Param` 的 `TyId`。孔位全部消失：`TraitCall` 变成指向实例的 `Call`，`SizeOf` 变成常量。
- **优化：** 与 R 级同一套规则，但 worklist 只从被填答案的节点开始；主要工作是内联刚解析出来的被调者（闭包、trait 方法）。调用摘要在优化**之前**用一次廉价遍历算出（[optimization-pipeline.md](optimization-pipeline.md#调用摘要)），调用者不必等被调者优化完。
- **类型事实只经答案表读取：** S/L 级不能直接查 `TyId` 的布局、有效范围、`Freeze` 等事实，只能读答案表；这保证答案键覆盖了代码依赖的全部事实（debug 构建越权即 ICE）。
- **身份：** S 图由 (R 图哈希, 答案向量) 唯一决定，所以它本身不需要缓存，可以随时重算。

## L 级：布局语义

- **来源：** `lower_layout(S 图, 布局表, TargetSpec)`，原地替换节点。
- **值类型：** `MachineTy`：`Int{bits}`、`Float{bits}`、`Ptr`、`Vector{lanes, elem}`，以及给大聚合用的 `ValueSlot{size, align}`。
- **降低内容：** 聚合按 `backend_repr` 和 PassMode 拆成 Scalar/ScalarPair，≤8 字节的 `Memory` 聚合是一个整数，更大的进 value slot；enum 操作变成 tag/niche 算术；`FieldAddr` 变成常量偏移；`DropValue` 变成 drop glue 调用或内联体；`Check` 变成比较 + 冷分支。
- **保留：** 内存分区和值事实**一直保留到 Machine IR 之前**，所以机器级 GVN、LICM、load 消除仍然能用 Rust 的别名信息。这是和“降到 LLVM IR”的根本区别：降低只换表示，不丢语义。
- **优化：** 机器级窥孔、强度削减、地址模式、循环（[optimization-pipeline.md](optimization-pipeline.md#l-级)），然后 placement → Machine IR。

## 固定与浮动

有副作用的节点（调用、store、非 `Frozen`/`Const` 的 load、检查、drop）固定在控制上，构成**控制骨架**；只有纯值节点、`Frozen`/`Const` load 和未逃逸 `Local` 的 load 浮动。这与 Graal 的 FixedNode、Cranelift 的副作用骨架一致，也回应了 V8 的经验：内存操作本来就很少浮动，让它们全部自由反而增加调度成本（[prior-art/sea-of-nodes.md](../../research/prior-art/sea-of-nodes.md)）。

## verifier 何时全量运行

structural 检查每个 pass 后都跑；full 检查（支配、分区、所有权）只在级别边界跑：R 图发布前、S 级结束、L 级结束。逐 pass 的 full 检查只在 debug 构建打开（Lean 4.22 的经验：逐 pass 检查花时间，发现的问题很少）。

## 为什么不是三种 IR

- 共享一套 verifier、GVN 哈希、worklist、dump 格式和 NodeId，降低是原地节点替换，不需要整图翻译。
- S 级就是 R 级规则在单态图上再跑一遍；分成两种 IR 就要写两遍。
- 先例：V8 TurboFan 在一个 SoN 里放 JS、Simplified、Machine 三层算子，逐级 lowering；Graal 的 high/mid/low tier 同理。

## 风险：V8 离开 SoN 的教训

V8 从约 2022 年起把优化编译器从 SoN（TurboFan）逐步换成 CFG（Turboshaft，2025 年博客总结），主要理由：JS 的大多数操作有副作用，节点几乎都挂在效果链上，SoN 的自由度用不上，反而带来缓存局部性差、调度成本和推理难度。

kfrc 的应对和要测的量：

| 风险 | 应对 | 指标（M2 起每次跑 walkthrough 和真实 crate 都记录） |
|---|---|---|
| 效果节点占比高 | 聚合是值、借用分区，大部分 Rust 代码的内存访问不在同一条链上 | 效果链上节点数 / 总节点数；每条分区链的长度 |
| 局部性 | arena 按创建顺序分配；R 级图按 Body 块顺序构建 | 每节点平均访问时间；placement 耗时占比 |
| placement 成本 | 只在 L 级末尾做一次；R/S 级不放置 | placement 指令数 / 函数节点数 |
| 无效访问 | 变化驱动的 worklist | 每次改动对应的节点访问数（V8 测得约 20 次访问才改 1 次） |
| 缓存不命中 | 同上 | L1d miss（V8 测得 SoN 约为 CFG 的 3 倍） |

如果这些指标在 M5 的真实 crate 上显示 SoN 的优势被抵消，记录数据并在 T38 内评估“R/S 级 SoN + L 级 CFG”的混合方案；不提前改变固定决策。

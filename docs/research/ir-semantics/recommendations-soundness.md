# 对设计的建议（一）：健全性

按严重程度排序。“目标文件”是重组后的路径；`design/ir/semantics.md` 是建议新建的语义文件（见 [recommendations-gaps.md](recommendations-gaps.md) G1）。

## S1 `Add.nsw` 的 UB 与浮动冲突

- **问题：** `Add.nsw/nuw` 定为“溢出即 UB”，又是可浮动的纯值节点：UB 是否发生取决于 placement，语义不确定；投机执行（`Select` 臂、外提出循环/条件）会引入 UB
- **证据：** [values-and-types.md](../../design/ir/values-and-types.md#整数运算模式) 表中“UB”；[levels.md](../../design/ir/levels.md#固定与浮动) 纯值节点浮动；rustc 把 MIR 的立即 UB `AddUnchecked`（[CTFE operator.rs#L101](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src/interpret/operator.rs#L101)）降成 LLVM `add nsw`（poison，[builder.rs#L480](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/builder.rs#L480)）；TurboTV：SoN 有效 = 所有调度等价
- **改法：** 引入 poison：`nsw/nuw`、`Shl.unchecked` 溢出得 poison；列出 UB 点（分支条件、地址、除数、调用实参与返回值、固定 load 解码、`Refine`）；节点按“纯全函数 / 产生 poison / 立即 UB 固定 / 效果”四类登记（[execution.md](execution.md#节点的四类语义)）；`Div`/`Rem` 明确为立即 UB 固定节点（x86 `div` 陷入）
- **目标文件：** `design/ir/values-and-types.md`#整数运算模式、`design/ir/nodes.md`#纯值、`design/ir/levels.md`#固定与浮动、`design/ir/semantics.md`

## S2 `noundef` 范围过宽

- **问题：** “全部类型的值 `noundef`”对 `MaybeUninit<T>`、union 不成立；把它当事实会让含未初始化字节的值被当作合法整数折叠
- **证据：** rustc 只在标量 `!is_uninit_valid()` 时发 `!noundef`（[builder.rs#L699](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/builder.rs#L699)、[lib.rs#L1594](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1594)）；Reference：读未初始化内存只在 union 与 padding 中允许
- **改法：** 改为“允许未初始化的类型（union、`MaybeUninit`）以外的标量叶子 `noundef`”；union/`MaybeUninit` 的值是字节列表（MiniRust `Value::Union`）；verifier 加 `NoundefFact`
- **目标文件：** `design/ir/values-and-types.md`#值事实、`design/ir/verifier.md`

## S3 投机 load 的解码

- **问题：** 投机 load（规则 2 允许上提出条件）读出的字节若不是该类型的合法值（`bool` 为 2、引用为空），按“读出无效值即 UB”就是新引入的 UB；`&T` 指向的内容是否必须有效在 Rust 中未定
- **证据：** Reference：“指向有效值”一条“仍有争议”；MiniRust 不总把它当 UB；LLVM `dereferenceable` 只保证“投机读不陷入”
- **改法：** 区分固定 `Load`（解码失败即 UB，带 `noundef`/范围事实）与 `Load.spec`（解码失败得 poison，不带事实）；上提就是“固定 → spec”，事实以 `Refine{Assumed}` 留在原控制点
- **目标文件：** `design/ir/nodes.md`#地址与内存、`design/ir/memory-partitions.md`#规则 规则 2

## S4 可浮动 load 三处不一致

- **问题：** 哪些 load 能浮动，三处写法不一致；`Local` load 浮动还缺活跃区间约束
- **证据：** [levels.md](../../design/ir/levels.md#固定与浮动)：`Frozen`/`Const` 与未逃逸 `Local` 浮动，未提 `Unique`；[nodes.md](../../design/ir/nodes.md#地址与内存)：`Frozen`/`Unique`/`Const` 可解除固定，未提 `Local`；[verifier.md](../../design/ir/verifier.md) `UnpinnedAccess`：非 `Frozen`/`Unique`/`Const` 必须带 `ctl`（即 `Local` 必须固定）
- **改法：** 在 `semantics.md` 写唯一的一张表：`Frozen`/`Const` 可浮动；`Unique` 可浮动但受同分区写的 token 约束；`Local(s)` 只在 `SlotLive(s)`–`SlotDead(s)` 内浮动；其他固定。verifier 加 `LocalLoadOutsideSlotLife`、`SpeculativeLoadFootprint`
- **目标文件：** `design/ir/levels.md`、`design/ir/nodes.md`、`design/ir/verifier.md`

## S5 浮动 load 的反依赖

- **问题：** 浮动 load 的**反依赖**没有写：load 读 token `t`，placement 可能把它放到同分区之后的写后面
- **证据：** [graph.md](../../design/ir/graph.md#效果链) 只规定效果输入；[ct-testing.md](../prior-art/ct-testing.md) 的负例只有“load 提到 store 之前”方向
- **改法：** 写明 token 语义：load 读到的是 `t` 时刻的内存，等价于“`t` 之后第一个写该分区的节点必须在 load 之后”；placed 级检查 `AntiDependence`；解释器做 token 新鲜度断言
- **目标文件：** `design/ir/graph.md`#效果链、`design/ir/verifier.md`

## S6 `Refine` 失败时是什么

- **问题：** `Refine` 只说“控制依赖事实”，没说事实不成立时是什么：既用于已证明的事实（`Check` 出口），又用于“写回已知当前值的 store 删除”；若事实来自 UB 规则，复制/移动它会改变 UB 集合
- **证据：** [values-and-types.md](../../design/ir/values-and-types.md#值事实)；[optimization-pipeline.md](../../design/ir/optimization-pipeline.md#r-级) 的 store 删除借助 `Refine`
- **改法：** `Refine{fact, origin: Proven \| Assumed}`：`Proven` 失败是编译器 bug，可自由保留；`Assumed` 失败是程序 UB，不能移动到新控制点；verifier 加 `RefineOrigin`
- **目标文件：** `design/ir/values-and-types.md`、`design/ir/nodes.md`

## S7 `Select` 的语义

- **问题：** `Select` 没有语义：条件是 poison 时怎样、能否转回分支
- **证据：** [nodes.md](../../design/ir/nodes.md#纯值) 只列名字；L 级 if-conversion（W02）依赖它
- **改法：** 照 LangRef：条件 poison → poison，未选臂不影响；分支 → `Select` 合法；`Select` → 分支需要条件 `noundef`，否则要 `freeze`（首版不做这个方向）
- **目标文件：** `design/ir/nodes.md`#纯值、`design/ir/optimization-pipeline.md`#l-级

## S8 transmute 与 union 读

- **问题：** 没有 transmute 与 union 读的节点，但 Body 有 `Cast(Transmute)`；值形式下它是 `decode(encode(v))`，依赖布局，R 级没有对应孔位
- **证据：** [body.md](../../design/frontend/body.md) 的 `Rvalue::Cast`；MIR `CastKind::Transmute`：大小不同时运行即 UB（[syntax.rs#L1509](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/syntax.rs#L1509)）
- **改法：** 新增 `Transmute{from, to}`（RS）：语义 `decode_to(encode_from(v))`，解码失败的固定版 UB、spec 版 poison；R 级登记 `SizeAlign`、`FieldOffsets`、`VariantEncoding`、`ValidRange` 孔位；L 级为位重解释
- **目标文件：** `design/ir/nodes.md`、`design/ir/generics-specialization.md`#孔位

## S9 指针相等与 provenance

- **问题：** 地址相等不蕴含指针可互换：按 `p == q` 把 `q` 换成 `p` 会改变 provenance，导致误编译；字段级不相交也隐含“同一基址值”
- **证据：** Lee 等 OOPSLA '18 给出 GVN 按指针相等传播引起的安全 Rust 误编译
- **改法：** 规则表禁止按指针相等替换；指针 `Compare` 只比较地址；字段级不相交写明前提是**同一 SSA 基址节点**
- **目标文件：** `design/ir/optimization-pipeline.md`#共同机制、`design/ir/memory-partitions.md`#字段级不相交

## S10 分区的语义地位

- **问题：** 分区表把“数据流事实”和“UB 条件”混在一起，没有写各自的证明义务
- **证据：** [memory-partitions.md](../../design/ir/memory-partitions.md#分区)
- **改法：** 分两类（[memory.md](memory.md#分区两类地位不同)）：`Local`/`Heap`/`Const` 违反 = 编译器 bug；`Frozen`/`Unique`/`Reborrow` 违反 = 程序 UB，义务是 kfrc 的分区 UB ⊆ Miri SB 与 TB 都报的 UB
- **目标文件：** `design/ir/memory-partitions.md`#分区

## S11 浮点 NaN 与复制

- **问题：** 浮点 NaN 位串是非确定的：常量折叠与硬件选不同 NaN、再加上复制运算，`x.to_bits() == x.to_bits()` 可以不成立
- **证据：** `f32` 文档 NaN bit patterns：执行时选定、位串稳定、CTFE 可与运行时不同
- **改法：** 常量折叠器按 x86_64 硬件的 NaN 规则产出（在允许集合内）；不做 `x + 0.0 → x`、重结合；规则表标记 `float_bits`
- **目标文件：** `design/ir/values-and-types.md`、`design/ir/optimization-pipeline.md`#共同机制

## S12 fence 的放置检查

- **问题：** fence 排序（F11）在顺序语义里不可观察，现只有 verifier 的输入检查，placement 后没有检查
- **证据：** [memory-partitions.md](../../design/ir/memory-partitions.md#规则) 规则 7；F11
- **改法：** placed 级 `ReleaseOrder`：Body 顺序中 Release 之前的非 `Frozen`/`Const` 写在放置后仍在其前；多线程 litmus 程序进差分集
- **目标文件：** `design/ir/verifier.md`、`design/ir/memory-partitions.md` 规则 7

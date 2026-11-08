# 语义

R/S/L 三级的可执行语义。每级有一个参考解释器，作为定位用的 oracle；正确性仍以真实执行对拍为准（总览红线 3）。依据与先例见 [research/ir-semantics](../../research/ir-semantics/README.md)。

## 值与内存

| | R / S | L |
|---|---|---|
| 值 | MiniRust 式 `Value`（聚合、enum 变体、宽指针、union 按字节）+ 标量 `Poison` | `MachineTy` 位串 + `Poison` |
| 内存 | 三级共用：抽象字节（未初始化 / 字节 + 可选 provenance）+ 分配 | 同左；S→L 内存不变 |
| 参数 | R 的语义以答案表为参数：⟦R⟧_A | 布局表、`TargetSpec` |

- **只有 poison，没有 undef。** `freeze` 等真正需要时再加。
- **`noundef`** 只适用于 `!is_uninit_valid()` 的类型；union、`MaybeUninit` 的值可以含未初始化字节（rustc 同样只在这种条件下发 `!noundef`）。
- **分配不是可观察事件**（`GlobalAlloc` 的文档允许删除和合并分配），所以 `Alloc`/`Dealloc` 配对且未逃逸时可以删掉。
- **NaN 位串**允许在规定集合内非确定，但常量折叠必须与 x86 硬件选同一个 NaN；否则一个运算被复制后分别折叠，会出现 `x.to_bits() != x.to_bits()`。
- **可观察行为**：外部调用及其实参、进程终止方式（正常返回、panic 消息与退出码、abort）。不包括分配器调用、地址值、允许集合内的 NaN 位串。

## 执行模型

沿用 Graal IR 的形式化：**固定节点**沿控制骨架按顺序执行；**浮动节点**在被需要时求值；同一 `Region` 的全部 `Phi` 同时取值。前提是**浮动节点是全函数**——求值时机不同，结果不能从“正常”变成“UB”。节点因此分四类：

| 类 | 例 | 出错时 | 可投机 |
|---|---|---|---|
| 纯、全函数 | `Add.wrap`、`Compare`、`Field`、`Aggregate`、`MakeVariant`、`Select`、`Cast`、`FieldAddr` | 不出错；poison 逐叶传播 | 是 |
| 纯、产生 poison | `Add.nsw/nuw`、`Shl.unchecked`、`Load.spec`、`Transmute.spec`、`VariantField`（变体不符） | 返回 `Poison` | 是 |
| 立即 UB，固定 | `Div`/`Rem`（除数为 0 或 `MIN / -1`）、固定 `Load`/`Store`、`Refine`、固定 `Transmute` | UB | 否 |
| 效果 | `Call`、`Store`、原子、`Fence`、`Alloc`、drop、`SlotLive/SlotDead` | 依节点 | 否 |

**UB 点**：poison 只在以下位置变成 UB——分支条件、内存地址、除数、调用实参与返回值、固定 load 的解码。

## Load：固定与投机

| | 固定 `Load` | `Load.spec` |
|---|---|---|
| 位置 | 固定在产生它的控制下 | 可浮动、可上提出条件 |
| 读到非法内容 | UB | 得 poison |
| 什么时候允许 | 总是 | 分区是 `Frozen`、`Const`（protector 期间可解引用），或 `Unique`/`Local` 且在其有效区间内（`Local` 要在 `SlotLive`–`SlotDead` 之间）；读取不超出 sized pointee 的静态大小 |

浮动 load 有反依赖：不能被放到同分区后续的写之后（verifier 的 `AntiDependence`）。

## Refine 的两种来源

| 来源 | 例 | 事实不成立时 | 能否投机移动 |
|---|---|---|---|
| `Proven` | `Check.Bounds` 的 ok 出口后 `i < len`；`VariantSwitch` 出口后 `variant = v`；非空分支后 nonnull | 不可能，失败即编译器 bug | 可以，事实可保留 |
| `Assumed` | `assert_unchecked`、固定 load 的有效范围、`&T` 参数的对齐 | 程序 UB | 不可以；投机移动时丢弃事实 |

## Select 与指针相等

- `Select(c, a, b)`：`c` 是 poison 时结果为 poison，否则取一臂；另一臂是 poison 不影响结果。分支转 `Select`（if-conversion）合法；反方向只在 `c` 不可能是 poison 时合法。
- 禁止由 `p == q` 推出“把 `q` 换成 `p`”：两者的 provenance 可能不同，GVN 按指针相等传播曾在安全 Rust 中造成误编译（OOPSLA '18）。

## 分区的两类地位

| 分区 | 地位 | 违反时 |
|---|---|---|
| `Local`、`Heap`、`Const` | 分析结果 | 编译器 bug；检查模式下解释器给字节标记归属并断言 |
| `Frozen`、`Unique`、`Reborrow` 出的分区 | UB 条件 | 程序 UB。证明义务：kfrc 认定的分区 UB ⊆ Miri 在 Stacked Borrows 和 Tree Borrows 下都报的 UB |

别名模型本身不进 kfrc 的解释器；源程序是否违规由 Miri 判定。

## 级与级

- **Body → R**：⟦R⟧_A ⊑ ⟦Body⟧_A（refinement：目标的可观察行为是源的子集；源在某点 UB 后目标任意）。Body 层结构照 MiniRust，与 Miri 对拍。
- **R → S**：`specialize` 是代入，⟦S⟧ = ⟦R⟧_A，**相等**而不是 refinement，因此可以要求逐字节一致的差分。
- **S → L**：内存不变；值的对应就是 MiniRust 的 `encode`。
- **unwind**：`Exception` 值不可观察其内容，只有“以 panic 终止”这一事实和 panic 消息可观察。

## 解释器的用途

- 逐 pass 差分 + bisect：每个 pass 前后的图在同一输入上运行，结果不同就定位到该 pass（及其规则序号）。
- **检查模式**：把 verifier 的静态规则变成动态断言——分区标签、字节归属、token 新鲜度、`Refine` 来源、答案表读取。
- 规则验证：标量语义写一次，同时生成解释器和 SMT 查询；规则表每条带 `poison_producing`/`introduces_ub_point`/`speculates`/`float_bits` 标志。
- **CTFE 不解释 R 图**（优化会删掉必须报告的 UB），仍解释 Body；但与 IR 解释器共享值、内存和 `encode/decode` 核心（rustc 的 CTFE 与 Miri 共享 `InterpCx` + `Machine` 是同样做法）。

## 首版范围

与 T17 同期：整数、聚合、enum、union 字节值；`Unknown`/`Const` 分区；固定 `Load` 与 `Load.spec`；`nsw` 的 poison；R/S/L 三级；R→S 相等检查；逐 pass 差分加 bisect。浮点 NaN 集合、原子、unwind 和 `Frozen`/`Unique` 后加。估计首版 4–5 千行（7–9 人周），全量约 1 万行（推断）。

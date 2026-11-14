# 语义

R/S/L 三级的可执行语义。每级有一个参考解释器，作为定位用的 oracle；正确性仍以真实执行对拍为准（总览红线 3）。依据与先例见 [research/ir-semantics](../../research/ir-semantics/README.md)。

本文描述目标契约。Observe / Refine 的拆分、迁移状态及诊断与发布代码的边界见
[语义要求、事实与放置约束](requirements-and-facts.md)，不表示这些机制已经全部实现。

## 值与内存

| | R / S | L |
|---|---|---|
| 值 | MiniRust 式 `Value`（聚合、enum 变体、宽指针、union 按字节）+ 标量 `Poison` | `MachineTy` 位串 + `Poison` |
| 内存 | 三级共用：抽象字节（未初始化 / 字节 + 可选 provenance）+ 分配 | 同左；S→L 内存不变 |
| 参数 | R 的语义以答案表为参数：⟦R⟧_A | 布局表、`TargetSpec` |

- **只有 poison，没有 undef。** `freeze` 等真正需要时再加。
- 初始化状态、类型有效性和 poison 分开：已初始化的位仍可能不是合法 `T`；MaybeUninit 的内部字节与 IR poison 不等同。要求的源操作边界由 producer 登记，不能只在调用处补一次观察。
- **definedness 与表示：** 语义组成无 poison 不代表全部存储字节已初始化。ABI noundef 按实际运输的表示和 producer 契约决定，不能递归要求 padding、非活动 payload 或 MaybeUninit 内部字节已初始化。
- **分配不是可观察事件**（`GlobalAlloc` 的文档允许删除和合并分配），所以 `Alloc`/`Dealloc` 配对且未逃逸时可以删掉。
- **NaN 位串**允许在规定集合内非确定，但常量折叠必须与 x86 硬件选同一个 NaN；否则一个运算被复制后分别折叠，会出现 `x.to_bits() != x.to_bits()`。
- **可观察行为**：外部调用及其实参、进程终止方式（正常返回、panic 消息与退出码、abort）。不包括分配器调用、地址值、允许集合内的 NaN 位串。

## 执行模型

沿用 Graal IR 的形式化：**固定节点**沿控制骨架按顺序执行；**浮动节点**在被需要时求值；同一 `Region` 的全部 `Phi` 同时取值。前提是**浮动节点是全函数**——求值时机不同，结果不能从“正常”变成“UB”。节点因此分四类：

| 类 | 例 | 出错时 | 可投机 |
|---|---|---|---|
| 纯、全函数 | `Add.wrap`、`Compare`、`Field`、`Aggregate`、`MakeVariant`、`Select`、`Cast`、`FieldAddr` | 不出错；poison 逐叶传播 | 是 |
| 纯、产生 poison | `Add.nsw/nuw`、`Shl.unchecked`、`Load.spec`、`Transmute.spec`、`VariantField`（变体不符） | 返回 `Poison` | 是 |
| 立即 UB，固定 | `Div`/`Rem`（除数为 0 或 `MIN / -1`）、固定 `Load`/`Store`、固定 `Transmute`、`RequireDefined` / `RequireValid` / `Assume` | UB；语义要求不等于运行时检查 | 不能无证明地移到原本有定义的路径 |
| 效果 | `Call`、`Store`、原子、`Fence`、`Alloc`、drop、`SlotLive/SlotDead` | 依节点 | 否 |

**UB 点**：分支条件、内存地址、除法／取模操作数、调用实参与返回值、固定类型化解码以及显式要求，按各自契约消费 poison。类型有效性还可能在源语言的值产生点被要求；不能把它等同于这张 poison 消费表。纯值中 poison 的传播不等于立即 UB。

## Load：固定与投机

| | 固定 `Load` | `Load.spec` |
|---|---|---|
| 位置 | 固定在产生它的控制下 | 可浮动、可上提出条件 |
| 读到非法内容 | UB | 得 poison |
| 什么时候允许 | 固定位置执行，非法地址或解码按源契约 UB | 目标位置上地址 defined、分配存活、范围和对齐有效，且分区／访问凭据允许该读取；Local 在相应动态寿命内，Frozen 在 protector 内。只读或 sized 本身不是充分证明 |

浮动 load 有反依赖：不能被放到同分区后续的写之后（verifier 的 `AntiDependence`）。

## 事实来源与作用域

| 来源 | 例 | 事实不成立时 | 使用范围 |
|---|---|---|---|
| `Proven` | 构造器、Check 成功出口、VariantSwitch 分支等所证明的谓词 | 在其有效范围内失败是编译器 bug | 只在凭据有效的控制范围及同一动态值实例内 |
| `Assumed` | 独立 Assume 或明确的源操作契约 | 要求违反是程序 UB；错误引用凭据仍是编译器 bug | 同样受作用域约束，不能带到要求未执行的路径 |

来源不是移动许可。移出证明范围需要独立证明，或让操作不再依赖该事实且仍然安全。
enum 变体用 `VariantWitness`，标量性质用谓词记录，内存访问条件关联相关状态和寿命；
`ControlOnly` 的放置职责由实际操作承接。要求不返回新值，凭据不生成机器指令。
详细规则、循环和失效维护见 [requirements-and-facts.md](requirements-and-facts.md)。

## Select 与指针相等

- `Select(c, a, b)`：`c` 是 poison 时结果为 poison，否则取一臂；另一臂是 poison 不影响结果。分支转 `Select`（if-conversion）合法；反方向只在 `c` 不可能是 poison 时合法。
- 禁止由 `p == q` 推出“把 `q` 换成 `p`”：两者的 provenance 可能不同，GVN 按指针相等传播曾在安全 Rust 中造成误编译（OOPSLA '18）。

## 分区的两类地位

| 分区 | 地位 | 违反时 |
|---|---|---|
| `Local`、`Heap`、`Const` | 分析结果 | 编译器 bug；检查模式下解释器给字节标记归属并断言 |
| `Frozen`、`Unique` 的源 protector 契约 | 有源依据的 UB 条件 | 源程序违反契约才是源 UB；错误创建或越 scope 使用分区是编译器错误。证明义务：kfrc 认定的源分区 UB ⊆ Miri 在 Stacked Borrows 和 Tree Borrows 下都报的 UB |

别名模型本身不进 kfrc 的解释器；源程序是否违规由 Miri 判定。

Effect 是版本依赖，MemMerge 是同点视图打包，Escape 是分析保守化；它们不能仅因
不发指令就被忽略，也不能充当实际 fence。Slot 寿命与初始化／move／drop 分开。
Unreachable 表示到达即 UB，循环发散、panic 和 abort 各保留原行为；Opaque 与冷标记
是优化策略，不是 UB 证明。完整契约见 [constraints-and-effects.md](constraints-and-effects.md)。

## 级与级

- **Body → R**：⟦R⟧_A ⊑ ⟦Body⟧_A（refinement：目标的可观察行为是源的子集；源在某点 UB 后目标任意）。Body 层结构照 MiniRust，与 Miri 对拍。
- **R → S**：specialize 的纯代入要求 ⟦S⟧ = ⟦R⟧_A。若入口同时执行优化，优化子步骤必须分别标明相等或 refinement，不能把整个结果无条件称为纯代入相等。
- **S → L**：有定义执行保持内存和可观察行为，值按 encode 对应。显式的要求擦除步骤按 refinement 验证：先将仍需的事实和放置依赖交给真实消费者，再去掉不产生运行时行为的要求。证明消费者不得迫使完整 enum 展开。诊断相等模式与发布 lowering 分开。
- **unwind**：`Exception` 值不可观察其内容，只有“以 panic 终止”这一事实和 panic 消息可观察。

## 解释器的用途

- 逐 pass 差分 + bisect：每条规则声明相等或 refinement。源有定义而目标引入 UB、错误结果或不允许的效果必报错；源 UB 而目标正常不能单独判错。不得用不支持、超预算或 Proven 断言失败冒充源 UB。保持诊断的变换仍可要求相等，不能全局放宽断言。
- **检查模式**：把 verifier 的静态规则变成动态断言——分区标签、字节归属、token 新鲜度、凭据来源／作用域／动态值实例、答案表读取。可保留要求诊断，但不将诊断引用转成发布机器码的数据需求。
- 规则验证：标量语义写一次，同时生成解释器和 SMT 查询；规则表每条带 `poison_producing`/`introduces_ub_point`/`speculates`/`float_bits` 标志。
- **CTFE 不解释 R 图**（优化会删掉必须报告的 UB），仍解释 Body；但与 IR 解释器共享值、内存和 `encode/decode` 核心（rustc 的 CTFE 与 Miri 共享 `InterpCx` + `Machine` 是同样做法）。

## 首版范围

与 T17 同期：整数、聚合、enum、union 字节值；`Unknown`/`Const` 分区；固定 `Load` 与 `Load.spec`；`nsw` 的 poison；R/S/L 三级；R→S 相等检查；逐 pass 差分加 bisect。浮点 NaN 集合、原子、unwind 和 `Frozen`/`Unique` 后加。估计首版 4–5 千行（7–9 人周），全量约 1 万行（推断）。

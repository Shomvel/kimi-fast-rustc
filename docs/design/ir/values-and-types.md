# 值、类型与事实

本文描述目标设计；语义要求、事实凭据及抽象状态的职责分别见
[requirements-and-facts.md](requirements-and-facts.md) 和
[constraints-and-effects.md](constraints-and-effects.md)。现有通用 Refine 正按职责设计迁移，
不是所有类型性质都能从 TyId 无条件赋予一个可能含 poison 的中间值。

## 值类型

```rust
pub enum ValueTy {
    Rust(TyId),                       // R/S 级：任意 Rust 类型的值，含聚合、enum、引用、宽指针
    Machine(MachineTy),               // L 级
    Control, Effect(PartitionId),     // 控制 token；某个内存分区的状态 token
    Tuple(TupleTyId),                 // 多结果节点，由 Projection 取
}
pub enum MachineTy { Int { bits: u8 }, Float { bits: u8 }, Ptr, Vector { lanes: u16, elem: ScalarKind }, ValueSlot { size: u64, align: u8 } }
```

- R 级的 `TyId` 可以含 `Param`；S 级起不含。`Infer`、`Error` 永远不进图。
- region 一律擦除。借用信息不靠类型里的 region 表达，而靠[内存分区](memory-partitions.md)。

## 什么是值，什么在内存里

R/S 级的规则：**一个局部只有在地址被取走时才进内存**。

| 局部的用法 | 表示 |
|---|---|
| 只被读写、move、copy（含部分字段赋值） | SSA 值；部分赋值是 `InsertField` 产生新值 |
| 被 `&`/`&mut` 借用，但引用不逃逸出本函数（只用于读写和传给有摘要的调用） | `StackSlot` + 自己的 `Local` 分区 |
| 被裸指针借用、引用存进内存、传给未知调用 | `StackSlot`，逃逸后访问并入 `Unknown`（[规则 4](memory-partitions.md#规则)） |
| 数组值被非常量下标访问 | `StackSlot`（L 级需要地址） |

“地址是否被取走”在 Body → R 图构建时由 Body 上的 `Ref`/`RawPtr` 右值直接决定，不需要事后分析。method 调用的 autoref（`v.len()`）也是借用——但被调者内联后引用消失，所以 R 级优化后要做一次**槽位提升**：引用全部消失的 `StackSlot` 退回 SSA 值（相当于 mem2reg，但只处理确实被借用过的少数局部）。

大聚合也是值。值多大都不影响 R/S 级优化；L 级按 PassMode 决定它是一个整数（≤8 字节）还是进 value slot，由槽位分配器合并 move 链上的槽，见 [ownership-drop-unwind.md](ownership-drop-unwind.md#大值与原地构造)。

**place 地址与 Rust 指针值分开。** 结构化地址使用内部 `PlaceAddress<T>` 类型，
表示一个薄数据地址；即使 T 是 DST，也保留 T 的身份，供 `FieldAddr` 确定所属字段。
构图中的 place 另带 metadata。Rust `&T`/`*mut T` 在 T 需要 metadata 时仍是完整宽指针，
不能通过给同一个 Rust 指针类型登记 8 字节布局来伪装成薄地址。extern type 及以它
结尾的 struct 虽然 unsized，但 metadata 为 unit，实际指针仍薄；这也由 producer
显式标记，不能从 unsized 单独推导指针宽度。宽指针解引用先拆数据和
metadata；借用时按目标 Rust 指针类型重组。

slice/str 及递归以它们结尾的 struct，其尾字段偏移和对齐可由 producer 静态证明，
字段投影沿用长度 metadata；投影到 sized 前缀字段则清除 metadata。dyn 尾字段
可能需要运行时对齐，不能套用 slice 尾的静态偏移规则。具体支持范围由 producer
的显式类型事实约束，不通过缺少布局答案或类型显示名猜测。

## 整数运算模式

模式写在 opcode 上，不靠后续推断：

| Rust 来源 | 节点 | 溢出时 |
|---|---|---|
| `a + b`，overflow-checks 关 | `Add.wrap` | 回绕 |
| `a + b`，overflow-checks 开 / `checked_add` | `AddWithOverflow` → `(value, overflowed)`，接 `Check.Overflow` | 进入 panic 或返回 None |
| `unchecked_add` 等 intrinsic、std 内部已证明的算术 | `Add.nuw` / `Add.nsw` | 得 **poison**（不是立即 UB：纯值节点会浮动，UB 不能依赖放置位置；poison 在 UB 点才变成 UB，见 [semantics.md](semantics.md)）。rustc 把 MIR 的 `AddUnchecked` 也降成 LLVM 的 `add nsw` |
| `a << b` | `Shl.mask`（b 按位宽取模，Rust `<<` 在 overflow-checks 关时的语义）/ `Shl.unchecked`（越界得 poison） | |
| `a / b`、`a % b` | `Div`/`Rem`，固定节点 | 除数为 0、`MIN / -1` 是立即 UB 或接 `Check`；x86 `div` 会陷入，不能变成 poison |
| `transmute` | `Transmute`（固定，非法值 UB）/ `Transmute.spec`（非法值得 poison）；需要两边的布局孔位 | |
| `as` 浮点 → 整数 | `FloatToInt.saturating` | 饱和（Rust 语义，不是 LLVM 的 poison） |

比较的模式同样在 opcode 上：`a < b` 是 `Compare{int_signed | int_unsigned | float_ordered | float_unordered, lt}`，由类型决定；在泛型体里它是 `PartialOrd` 的 `Resolve` 孔位，所以有符号/无符号/浮点的差别进入答案键（W06）。

## 值事实

Rust 类型给出有效值必须满足的规则；在对应源操作契约或已有证明成立的位置，
这些规则才能作为值事实使用。规则查询可缓存，具体值上的凭据仍有作用域：

| 类型 | 事实 |
|---|---|
| 有效的 `&T`、`&mut T`、`Box<T>` | 由各自源契约给出非空和必要对齐；对齐答案不可得时不猜测 |
| `NonNull<T>` | 非空；不单独保证对齐、存活或可解引用，可能是悬空指针 |
| `Frozen`/`Unique` 分区的根（`&T: Freeze`、`&mut T: Unpin` 参数） | 调用期间 `dereferenceable(size_of(T))`，只对 sized `T`。`&!Freeze`、`&mut !Unpin`、`Box`、切片和 `dyn` 的内容没有（rustc 同样给 0） |
| `bool`、`char`、enum 判别值、`NonZero*`、`rustc_layout_scalar_valid_range` | 合法值范围 |
| 宽指针 `&[T]` 的 len | `len * size_of(T) <= isize::MAX` |
| 已满足源 definedness 契约的值 | 语义组成无 poison；不能由类型名直接推得一个部分初始化中间值有效。ABI 的 noundef 要求按实际表示与 producer 规则决定，不递归要求 padding、非活动 payload 或 MaybeUninit 内部字节已初始化 |

**类型事实只经答案表读取。** R 级的事实查询（包括具体类型的）都登记为孔位，S/L 级只能读答案表（[generics-specialization.md](generics-specialization.md#孔位)）。这样“`char` 的有效范围”这类事实进入答案键，不会被两个共享代码的实例悄悄用错。

**路径事实与语义要求分开。** enum 访问引用 VariantWitness；范围、相等和非空是
带 predicate、witness、scope 的事实记录；内存访问凭据还关联状态和寿命。
Assume／Require 节点承载源语义要求，事实记录表达其后可用的信息，不返回另一份值。
Proven 和 Assumed 都不能越过其有效范围使用；循环还需匹配同一动态定义。
旧 ControlOnly 的职责放到实际操作的控制／效果依赖上，不递归复制聚合的叶子。
具体契约见 [requirements-and-facts.md](requirements-and-facts.md)。

证明引用不制造机器数据需求；影响优化的事实仍须重映射、验证并记录缓存依赖，
不能与仅供诊断的源位置旁表混同。Effect、MemMerge、生命周期和借用作用域不属于
Rust 值有效性，分别按 [constraints-and-effects.md](constraints-and-effects.md) 处理。

## 不进图的东西

- **源位置：** span 存在 `NodeId → SpanId` 的旁表里，只给调试信息和诊断用；图的内容哈希不含它。panic 的 `Location` 常量是数据表里的条目，代码只引用它的符号，各级优化器**不能读它的内容**（[cache.md](../build/cache.md#源位置隔离)）。
- **调试变量：** `NodeId → (LocalId, 作用域)` 旁表；变换维护它，但不影响变换决策。

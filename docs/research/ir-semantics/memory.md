# 内存模型与分区的语义地位

## 内存

三级共用一个内存模型，仿 MiniRust 的 `Memory` 接口（[prior-art-rust.md](prior-art-rust.md)）：

| 组成 | 推荐 | 首版 |
|---|---|---|
| 分配 | `AllocId → {bytes: List<AbstractByte>, align, mutability, kind, live}`；`kind ∈ {Stack(slot), Heap, Const, Static, VTable}` | 全部 |
| 地址 | 每个分配取一个具体基址（解释器确定性分配；语义上非确定） | 全部 |
| provenance | 指针带 `(AllocId, 分区标签)`；访问越出 `AllocId` 范围即 UB | 全部 |
| ptr→int / int→ptr | exposed provenance：ptr→int 把分配标为已暴露（同时是分区逃逸事件）；int→ptr 得到“任一已暴露分配”，分区标签为 `Unknown` | 全部 |
| 借用模型（SB/TB） | **不进 kfrc 解释器**。源程序的别名 UB 交给 Miri 判定；kfrc 解释器只检查 kfrc 自己声称的分区事实（下文） | 不做 |
| 并发 | 不建模；原子按顺序语义执行，fence 为空操作（排序约束见 [execution.md](execution.md#fence-与原子)） | 不做 |

不把 SB/TB 放进 kfrc 解释器的理由（推断）：kfrc 只在 [rust-aliasing.md](../prior-art/rust-aliasing.md) 允许的范围内使用别名事实，判定“源程序是否违反别名规则”是 Miri 的职责；kfrc 解释器的职责是判定“编译器是否违背了它自己声称的事实”。两者分开后，kfrc 解释器不需要追随 Rust 别名模型的演进。

## 分区：两类，地位不同

[memory-partitions.md](../../design/ir/memory-partitions.md) 把六种分区写在一张表里，但它们的正确性来源不同，语义上应分开：

| 分区 | 依据 | 语义地位 | 解释器检查（检查模式） | 违反时说明什么 |
|---|---|---|---|---|
| `Unknown` | 保守 | 无约束 | — | — |
| `Local(s)` | 数据流：地址未逃逸 | **分析结果**，不是 UB 条件 | `s` 的字节在逃逸事件之前只被列出 `Local(s)` 的访问触及 | 编译器 bug（分析或变换错误） |
| `Heap(a)` | 数据流：新分配未逃逸 | 分析结果 | 同上 | 编译器 bug |
| `Const` | 分配不可变 | 程序 UB 条件（写不可变分配是 UB）+ 分析结果 | 写 `Const` 分配 → 程序 UB；带 `Const` 标签读到可变分配 → 编译器 bug | 二者之一 |
| `Frozen(p)` | `&T: Freeze` 参数的 protector | **UB 条件** | 调用期间 `[p, p+size_of T)` 被任何访问写 → 程序 UB | 程序 UB（必须是 SB 与 TB 都判 UB 的情形） |
| `Unique(p)` | `&mut T: Unpin` 参数的 protector | UB 条件 | 调用期间该区间被分区标签不是 `Unique(p)` 及其派生的指针访问 → 程序 UB | 同上 |
| 内联 `Reborrow` 子分区 | 被调者的 protector | UB 条件 | `[Reborrow, EndBorrow]` 期间同上 | 同上 |

两类的后果：

1. **分析分区不扩大 UB 集合。** `Local`/`Heap` 的事实对所有程序成立；解释器发现违反就是编译器 bug，不需要 Miri 判定。M3 前的全部分区都属于这一类。
2. **UB 分区有一条证明义务：** kfrc 声称的 UB ⊆ Miri 在 SB 和 TB 两种模式下都报告的 UB。这正是 [README 设计红线 1](../../design/README.md) 与 [rust-aliasing.md](../prior-art/rust-aliasing.md) 的“两种模式对拍”的形式化表述。可执行检查：生成程序先过 Miri（两种模式），无 UB 的程序在 kfrc 检查模式下也不能触发 UB 分区违反。

## 检查模式：把静态分区规则变成动态断言

解释器维护三张影子表，每条对应 [verifier.md](../../design/ir/verifier.md) 的一个静态检查：

| 影子状态 | 维护方式 | 断言 | 对应静态检查 / findings |
|---|---|---|---|
| 指针的动态分区标签 | `StackSlot`、`Alloc`、`Start`（参数）、`Reborrow` 赋标签；`FieldAddr`/`AddrAdd`/cast/`Phi`/`Select`/聚合字段原样传递；load 得到的指针、int→ptr 为 `Unknown` | 每次访问的 `mem=` 列表包含该指针的动态标签 | `PartitionMismatch`；F49、F50 |
| 字节归属 | 分配时字节归属创建它的分区；逃逸事件（指针存入内存、传给无 no-capture 摘要的调用、ptr→int）后归属 `Unknown` | 访问的字节归属在 `mem=` 列表中 | `EscapedPartitionUse`；F46、F47 |
| 每字节最后写入时刻 | 全局步数计数；每个效果 token 记录产生时刻 | 读 token `t` 的 load：所读字节的最后写入时刻 ≤ `t`，且从 `t` 到求值时刻没有写入这些字节 | 效果链完整性、反依赖；F12 |

第三条是 SoN 特有的：浮动 load 在“使用时”求值（[execution.md](execution.md)），它读到的必须是 token 那一刻的内存。断言失败说明某个写没有进入该 load 的 token 链（漏分区、漏逃逸合并）或 placement 把 load 放到了后续写之后（反依赖）。

## 分配的可观察性

`GlobalAlloc` 文档明确：分配函数可以非确定地完全跳过底层分配器，编译器可以合并分配；“分配是否发生不属于程序行为”，即使分配器有打印等副作用。[nodes.md](../../design/ir/nodes.md) 的 `Alloc`/`Dealloc` 配对删除（F35）由此成立。语义应写明：`Alloc`/`Dealloc` 不是可观察事件；用户自定义 `#[global_allocator]` 的调用也不是。

## 字段级不相交

[memory-partitions.md](../../design/ir/memory-partitions.md#字段级不相交) 说它是“地址算术，不依赖 UB 规则”。语义上它依赖一个前提：两次访问的指针有**同一个基址值**。不同基址值、地址相等时，provenance 不同的两个指针仍可能别名同一内存（int→ptr 之后）。所以规则要求同一 SSA 基址节点，不能用 `p == q` 的比较结果把 `q` 换成 `p`（见 [recommendations-soundness.md](recommendations-soundness.md) S9）。

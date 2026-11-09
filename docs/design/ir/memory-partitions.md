# 内存分区：把借用变成独立的效果链

本文是目标设计；当前 Local／Const 标签不等于已经实现独立分区效果链。
token、MemMerge、借用作用域和 Escape 的精确定义见
[constraints-and-effects.md](constraints-and-effects.md)，值事实和访问条件的区别见
[requirements-and-facts.md](requirements-and-facts.md)。

## 问题

首版 SoN（[representations.md](memory-partitions.md)）只有一条保守效果链：任何 load 都排在之前所有 store 和调用之后。正确，但会把 LLVM 的 noalias 推断推迟成“没有”。Rust 的借用规则给出了更细的结构：一个 `&mut` 在 protector 期间是唯一访问路径，一个 `&T`（`T: Freeze`）指向的内存在 protector 期间不变。

## 分区

每个内存操作指定一个或多个分区，每个分区有自己的效果 token 链。

| 分区 | 在哪里创建 | 依据 | rustc+LLVM 的对应 |
|---|---|---|---|
| `Unknown` | `Start` | 保守 | 无 noalias 的内存 |
| `Frozen(p)` | `Start`，参数 `p: &T` 且 `T: Freeze` | protector：整个调用期间不变 | `noalias readonly` |
| `Unique(p)` | `Start`，参数 `p: &mut T` 且 `T: Unpin` | protector：调用期间只经 `p` 访问 | `noalias`（rustc：`optimize && is_unpin`） |
| `Local(s)` | `StackSlot` | 数据流：地址没有逃逸 | 捕获分析 |
| `Heap(a)` | `Alloc` | 数据流：新分配，结果没有逃逸 | `noalias` 返回值 + 捕获分析 |
| `Const` | 常量分配、vtable | 内容不可变 | `readonly`、`!invariant.load` |
| 内联产生的 `Frozen`/`Unique` | 调用点 `Reborrow`（规则 5） | 被调者 protector 的作用域 | `!alias.scope`/`!noalias` |

`Freeze`、`Unpin` 的判定照 rustc 1.94.1：**整类型、浅层**。含一个 `Cell` 字段的 struct 整体不是 `Freeze`，不按字段建 `Frozen`（那会强于 rustc，违反[红线 1](../README.md#设计红线)）；`&&Cell` 只冻结外层的 `size_of::<&Cell>()` 字节。`Box<T>` 参数先归 `Unknown`。

## 规则

1. **根集合：** 每个指针值的根是一个**集合**，沿 `FieldAddr`/`IndexAddr`/`AddrAdd`/`Reborrow`/指针 `Cast` 向上找，经过 `Phi`/`Select` 时取并集，从聚合值中 `Field` 出来的指针取聚合构造时的根。根来自 load、int→ptr、调用返回值时是 `{Unknown}`。访问取根集合里**每个**分区的 token；verifier 检查（`PartitionMismatch`）。
2. **读：** Frozen、Const 的内容不变可支持 CSE 和移动，但 load 仍依赖访问凭据：分配在目标位置存活、地址及范围有效、对齐成立，Frozen 的 protector 也必须有效。不能仅由只读或 sized 推出投机安全。不得越过相关寿命边界；切片等动态范围必须有独立长度／访问证明（W05）。
3. **调用：** 调用接收 `Unknown`，以及**沿实参的值字段可达的全部指针**（包括 `IterMut`、闭包环境里的指针）的根分区，产出它们的新 token；其他分区原样穿过。被调者摘要（[optimization-pipeline.md](optimization-pipeline.md#调用摘要)）可以把“写”缩小为“读”或“无”，把 `captures` 缩小为空。`&T`（`T: Freeze`）实参即使没有摘要，也按 rustc 已有的事实视为只读、只捕获 read_provenance，槽位留在 `Local`。
3a. **激活写：** `Unique(p)` 上新增或上移的写，必须被原程序中同一分区的某个写支配，不能在分区第一次写之前推测性地写（Tree Borrows §5：Reserved → Unique 的激活写不能移动）。循环 store 外提、条件 store 合并都受此约束。
4. **逃逸是数据流状态：** 派生指针被存入可能由外部访问的内存、传给无足够 captures 摘要的调用或暴露给整数访问路径等，导致该点之后不再能证明独立访问。首版可保守使用整函数逃逸；流敏感版本在汇合处取“已逃逸”，先逐前驱将相关精确分区依赖并入 Unknown 视图，再按同一分区做 Phi，不能把不同分区 token 直接当作控制前驱合并。Escape 是知识保守化，不是独立运行时效果或 UB。Frozen、Const 的内容不变资格与寿命仍须另外验证。
5. **内联：** 对满足 producer 参数契约的实参建立 Reborrow 作用域和访问凭据；根为 Unknown 时可按契约分出 Frozen／Unique，已是 Unique／Local／Heap 时可复用父链，但不得丢失契约范围。每个有限的正常和 unwind 出口 EndBorrow 关闭该 scope，将子链依赖及逃逸状态交回父视图。不写回、不 drop，不自动令原地址失效；之后不能使用该 scope 的旧凭据。
6. **不能用的情况：** `T: !Freeze` → 不建 `Frozen`，也没有 `dereferenceable`；`&mut T` 且 `T: !Unpin` → 不建 `Unique`，也没有 `dereferenceable`（rustc `abi.rs` 对这两类和 `Box` 都给 0，#98017）。泛型 `&T` 在 R 级按 `Unknown`；`IsFreeze(T)`/`IsUnpin(T)` 是孔位，S 级得到答案后拆出分区，并以该根上的全部内存节点为种子。
7. **原子与 fence：** 原子访问用地址所属的分区（栈上原子是 `Local`，`&mut AtomicU32` 是 `Unique`）。排序：
   - Acquire 加载、Acquire fence：只排序 `Unknown`（之后的访问不能上移越过它；`Local`/`Unique` 别人看不到）。
   - Release/AcqRel/SeqCst 的原子写和 fence：以 `Unknown` 和**全部 `Unique`、`Local`、`Heap`** token 为输入。原因：`*x = 42; fence(Release)` 之后经 Relaxed 发布 `x` 的地址，另一线程同步后必须看到 42；`Local` 槽在 fence 之后才逃逸也一样（W10）。这些输入只约束 placement，不让 DSE 把 fence 当成读。
   - volatile、内联汇编：`Unknown` 加全部可能可达的分区。

## 分区的两类地位

Local、Heap、Const 的资格是分析结果；Frozen、Unique 还依赖已确认的源语言
protector 契约。源程序违反该契约才是源 UB；错误地建立或使用分区、缺失凭据、
跨 scope 移动都是编译器错误，不能通过标为 Assumed 转嫁给用户。
别名规则仍遵守“kfrc 认定的源分区 UB ⊆ Miri 在 SB 和 TB 下都报的 UB”的验证要求。
布局引入新调用后必须重新验证或失效旧标签，不能按 memcpy/memmove 名字绕过逃逸分析。

## 字段级不相交

同一基址、常量偏移不重叠的两个访问互不影响——这是地址算术，不依赖 UB 规则。R/S 级用结构化投影（`FieldAddr[1]` vs `FieldAddr[2]`）判定，L 级用偏移。`RefCell` 的借用计数与值字段因此不相交（前提是规则 1 穿过 `UnsafeCell::get` 的指针 cast）。

## 明确不做

- **局部 `&mut` 重借用的作用域**（函数体内 `let r = &mut *x;` 在 r 活跃期间 x 不被访问）：rustc 不用，Tree Borrows 对未受保护 `&mut` 能证明的变换也很少。
- **聚合实参里的引用**：rustc 对 Scalar/ScalarPair 聚合中的引用也发 noalias（`tests/ui/codegen/issue-63787.rs`），首版只看直接参数，记为已知差距；扩展时按语义而不是按布局定义。
- **基于类型的别名分析**、**返回值的 noalias**。

## 首版与分阶段

- M2（T17）：节点带分区字段，只建 `Unknown` 和 `Const`；verifier 的根集合检查开启。
- M3：`Local`、`Heap`（不依赖 UB 规则）；逃逸先用整函数版本。
- M6（T38），按顺序逐个开关：流敏感逃逸 → 参数 `Frozen` → 局部 `Frozen`（只读复用，rustc MIR GVN 已在用，#123602）→ 参数 `Unique`（带激活写规则）→ 内联 `Reborrow`。每加一种都用含 unsafe 的 std 代码和 Miri 可运行的用例在 SB 与 TB 两种模式下对拍，W05/W10 的程序进差分集。

**实现形式（Simple 第 11 章的 `MemMerge`）：** 分区不预先全部展开。调用、`Return`、`Resume` 的效果输入是一个 `MemMerge`（默认内存 + 精确访问过的分区），只有被精确访问的分区单独出现，效果输入保持稀疏。

MemMerge 只是同一控制点的分区版本打包，不选择分支、不同步线程、不搬运数据；
控制合流由各分区的效果 Phi 完成。token 进入机器层前可消去表示，但真实访问与
同步依赖必须保留。当前单链实现不得以标签数量宣称已经获得多链并行度。

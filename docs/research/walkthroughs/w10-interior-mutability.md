# W10：内部可变性与 `Frozen` 的边界

检验 [memory-partitions.md](../../design/ir/memory-partitions.md) 的 `Frozen`/`Unique`/`Local`/`Unknown` 划分在 `Cell`、`RefCell`、原子、部分含 `UnsafeCell` 的结构体、`!Unpin` 上是否健全。数字由 `scripts/w10.sh <workdir>` 产生，输出原样节选（`RUSTC` = 1.94.1，`RUSTC_X86` = 1.99.1）。脚本先把 w10.rs 当二进制运行，再以 `--crate-type=lib` 出 IR/汇编，使函数保持外部可见、签名不被 O3 改写。标“推断”的是设计模拟，其余是实测。

## 1. 程序

```rust
pub struct Mixed { pub n: u32, pub c: Cell<u32> }          // 只有 c 是 !Freeze
pub struct Pinned { pub n: u32, _p: PhantomPinned }
pub fn frozen(x: &u32, f: fn()) -> u32 { let a = *x; f(); a + *x }
pub fn cell(c: &Cell<u32>, f: fn()) -> u32 { let a = c.get(); f(); a + c.get() }
pub fn mixed(m: &Mixed, f: fn()) -> u32 { let a = m.n; f(); a + m.n }
pub fn mixed_nocall(m: &Mixed) -> u32 { let a = m.n; m.c.set(7); a + m.n }
pub fn shallow(r: &&Cell<u32>, f: fn()) -> u32 { let a = r.get(); f(); a + r.get() }
pub fn pair(p: (&u32, &Cell<u32>), f: fn()) -> u32 { let a = *p.0; f(); a + *p.0 + p.1.get() }
pub fn refcell(r: &RefCell<u32>) { *r.borrow_mut() += 1; }
pub fn unique_fence(x: &mut u32) { *x = 1; fence(Release); *x = 2; }
pub fn unique_acquire(x: &mut u32, a: &AtomicUsize) -> usize { *x += 1; let s = a.load(Acquire); *x as usize + s }
pub fn frozen_fence(x: &u32) -> u32 { let a = *x; fence(SeqCst); a + *x }
pub fn unique_atomic(x: &mut AtomicU32, f: fn()) -> u32 { x.store(1, Relaxed); f(); x.load(Relaxed) }
pub fn local_atomic() -> usize { let a = AtomicUsize::new(1); a.fetch_add(1, SeqCst); a.load(SeqCst) }
pub fn pinned(p: &mut Pinned, f: fn()) -> u32 { p.n = 1; f(); p.n }
```

全部 `#[inline(never)]`；`f` 是 `black_box` 过的空函数，代表未知调用。完整源码 [scripts/w10.rs](scripts/w10.rs)。

## 2. rustc 现在怎么做（实测）

**源码规则。** 指针参数属性由 `arg_attrs_for_rust_scalar`（[abi.rs#L305–L367](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L305-L367)）按 `PointerKind` 决定，`PointerKind` 来自 `pointee_info_at`（[ty/layout.rs#L1035–L1046](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/layout.rs#L1035-L1046)）：

| 指针 | rustc 发什么 | 依据 |
|---|---|---|
| `&T`，`T: Freeze` | `noalias readonly dereferenceable(size)` | `SharedRef{frozen: is_freeze(T)}` |
| `&T`，`T: !Freeze` | 只有 `nonnull align`；**`dereferenceable` 也没有** | 非 frozen 的共享引用指向的内存可能在调用期间被释放（源码注释引 [#98017](https://github.com/rust-lang/rust/pull/98017)） |
| `&mut T`，`T: Unpin` | `noalias dereferenceable(size)` | `MutableRef{unpin: is_unpin(T)}`；`T` 含 `Cell`/原子也一样 |
| `&mut T`，`T: !Unpin` | 只有 `nonnull align` | 自引用类型（UCG#381） |

`is_freeze` 是对整个类型的 auto trait 查询（`impl !Freeze for UnsafeCell<T>`；`&T`、`*const T` 本身是 Freeze），**不按字段区分**，也**不穿过指针**。1.94.1 判 `Unpin` 只查 `LangItem::Unpin`（`common_traits.rs` 的 `is_unpin_raw`），`UnsafeUnpin` lang item 已定义但没有接进来。`pointee_info_at` 会递归进以 Scalar/ScalarPair 传递的聚合，所以元组里的引用也拿到属性。

```text
== rustc-emitted pointer parameter attributes (unoptimized IR)
  frozen          %x     noalias readonly dereferenceable(4)
  cell            %c     nonnull
  mixed           %m     nonnull
  shallow         %r     noalias readonly dereferenceable(8)
  pair            %p.0   noalias readonly dereferenceable(4)
  pair            %p.1   nonnull
  refcell         %r     nonnull
  unique_fence    %x     noalias dereferenceable(4)
  unique_atomic   %x     noalias dereferenceable(4)
  pinned          %p     nonnull
== O3 IR: non-atomic load/store, atomics, fences, non-intrinsic calls per function
  frozen          load=1 store=0 atomic-load/store=0 rmw=0 fence=0 call=1
  cell            load=2 store=0 atomic-load/store=0 rmw=0 fence=0 call=1
  mixed: merged by MergeFunctions into: cell
  mixed_nocall    load=1 store=1 atomic-load/store=0 rmw=0 fence=0 call=0
  shallow         load=3 store=0 atomic-load/store=0 rmw=0 fence=0 call=1
  pair            load=2 store=0 atomic-load/store=0 rmw=0 fence=0 call=1
  refcell         load=2 store=1 atomic-load/store=0 rmw=0 fence=0 call=1
  unique_acquire  load=1 store=1 atomic-load/store=1 rmw=0 fence=0 call=0
  frozen_fence    load=1 store=0 atomic-load/store=0 rmw=0 fence=1 call=0
  unique_atomic   load=0 store=0 atomic-load/store=2 rmw=0 fence=0 call=1
  local_atomic    load=0 store=1 atomic-load/store=1 rmw=1 fence=0 call=0
  pinned          load=1 store=1 atomic-load/store=0 rmw=0 fence=0 call=1
== unique_fence: non-atomic stores kept across fence(Release), O3 IR, by compiler (host target)
  rustc 1.94.1 / LLVM 21.1.8: stores=2
  rustc 1.99.1 / LLVM 23.1.1: stores=1
== x86 asm instruction counts
    2 asm-inst _RNvCsjnnzfRpktlk_3w1012unique_fence:
    4 asm-inst _RNvCsjnnzfRpktlk_3w1012local_atomic:
    7 asm-inst _RNvCsjnnzfRpktlk_3w107refcell:
```

- **部分 Freeze：** `&Mixed` 的属性和 `&Cell<u32>` 完全相同，`mixed` 在 O3 后与 `cell` 代码相同，被 MergeFunctions 合并。非 cell 字段 `n` 在调用后照样重读。不调用时（`mixed_nocall`）`m.n` 跨 `m.c.set(7)` 被转发：同基址常量偏移不相交，与 `Freeze` 无关。
- **浅层：** `shallow` 外层 `&&Cell` 是 frozen，外层指针只读 1 次；内层 `Cell` 跨调用重读（load=3）。
- **`RefCell`：** MIR 已把 `borrow_mut` 全部内联，只剩冷路径 `panic_already_borrowed`。O3 只剩“读计数、判 0、读改写值”：设 -1 和恢复 0 两次计数写都被删掉。
- **原子与 fence：** `frozen_fence` 跨 `fence(SeqCst)` 不重读（推断：noalias readonly 参数被当作不变内存）；`unique_acquire` 跨 acquire load 转发 `*x`；`unique_atomic` 的两个 relaxed 原子都保留，没有利用 noalias；`local_atomic` 在栈上保留 `lock incq`。`unique_fence` 跨 `fence(Release)` 的覆盖写：LLVM 21 两次 store 都留，LLVM 23 删掉第一次（1.99.1 的 x86 汇编只剩 2 条）。
- `pinned`（`&mut !Unpin`）调用后重读 `p.n`，与 `cell` 同。

## 3. kfrc 编译计划（推断）

**分区划分**（现设计 → 与 rustc 对照）：`frozen`/`frozen_fence`/`shallow` 外层：`Frozen`，一致。`cell`/`mixed`/`refcell`/`pinned`/`pair.1`：`Unknown`，一致。`unique_*`：`Unique`，但 `unique_atomic` 按规则 6/7 必须是 `Unknown`（细化 3）。`pair.0`：现设计只给直接参数建 `Frozen`，是 `Unknown`，比 rustc 弱（细化 6）。`local_atomic`：派生规则要求 `Local`，节点表要求 `Unknown`，矛盾（细化 3）。

**`refcell` 的 S 图**（内联 `borrow_mut`/`deref_mut`/`drop` 后）：

```text
n2  = FieldAddr[borrow](n1) ; n3 = Load[Unknown](n2) ctl=n0 mem=u0 : isize
n4  = If ctl=n0 (Compare.eq(n3, c0))                  // 否 → 冷区域 panic_already_borrowed
n5  = Store[Unknown](n2, c-1) ctl=n4.t mem=u0 → u1
n6  = Cast.ptr(FieldAddr[value](n1)) : *mut u32       // UnsafeCell::get；派生链要穿过 Cast（细化 7）
n7  = Load[Unknown](n6) ctl=n4.t mem=u1 : u32 ; n8 = Add.wrap(n7, c1)
n9  = Store[Unknown](n6, n8) ctl=n4.t mem=u1 → u2
n10 = Store[Unknown](n2, c0) ctl=n4.t mem=u2 → u3             // BorrowRefMut::drop：-1 + 1
```

规则：`n7`/`n9` 与 `n2` 字段不相交，`n7` 的效果输入可越过 `n5`；`n10` 覆盖 `n5` 且其间无对 `n2` 的读 → 删 `n5`；`n4.t` 上有 `Refine{n3 == 0}`，`n10` 写回已知的当前值 → 删 `n10`。结果与 rustc 的 7 条 x86 指令同形。

**`unique_fence`（按细化 1 改后）：** `Fence.release` 以 `Unique(x)` 的当前 token 为**排序输入**（约束 placement，不算读），所以 `*x = 1` 不能被放到 fence 之后；它被 `*x = 2` 覆盖，DSE 仍可删（与 LLVM 23 一致）。

## 4. 代价核算

| 项 | rustc 1.94.1 | kfrc（推断） |
|---|---|---|
| `&!Freeze`/`&mut !Unpin` 参数 | 不发 noalias/dereferenceable | `Unknown`，同等保守 |
| `local_atomic` | 栈槽 + `lock incq`，4 条 | 规则修正后 `Local` 原子降为普通读写，常量 2（1–2 条） |
| `unique_atomic` | 两个原子都留，7 条 | `Unique` 下可把 relaxed load 转发为 1；跨 `f()` 也成立（protector） |
| `pair` | `p.0` 跨调用不重读 | 现设计重读一次（细化 6） |
| 编译工作 | `refcell` 依赖 DSE + GVN | 分区内转发、死 store、当前值 store 三条规则 |

## 5. 设计细化

1. **（健全性）规则 7 让 release 不排序 `Unique`/`Local`，会把先前的写移到 release 之后。** 例：`*x = 42; fence(Release)` 后返回，调用者 `SHARED.store(p, Relaxed)` 发布 `x` 的地址，另一线程 `load(Relaxed)` + `fence(Acquire)` 后读 `*x`，按 C++20 模型必须看到 42；protector 那时已结束，不构成 UB。`Local` 槽在 fence 之后才 `Escape` 也一样。现设计里 store 与 fence 在不同链上，placement 可以交换它们。证据：LLVM 21 跨 release 保留两次 store；LLVM 23 只删被覆盖的那次（删除不等于下移）。建议：Release/AcqRel/SeqCst 的原子与 fence 以全部 `Unique` 分区和非 `Frozen` 的 `Local` 分区的当前 token 为排序输入（placement 约束，DSE 不当作读）；Acquire 只需排序 `Unknown`（本例 `unique_acquire` 的转发与 LLVM 一致）。改 memory-partitions.md 规则 7、[nodes.md](../../design/ir/nodes.md#地址与内存)（`Atomic*`/`Fence` 行）。
2. **（健全性）`dereferenceable` 事实给得过宽。** [values-and-types.md](../../design/ir/values-and-types.md#值事实) 说所有引用都有 `dereferenceable(size_of(T))`，rustc 对 `&!Freeze`、`&mut !Unpin`、`Box` 都给 0：`Arc` 一类代码在 `fetch_sub` 之后、引用仍活着时内存就可能被别的线程释放（#98017）。若据此把 `Unknown` load 上提出条件或循环，就是释放后读。建议：“整个调用期间可解引用”只来自 `Frozen`/`Unique` 分区（protector）；其余引用只有 `nonnull`/对齐；与 W03 细化 1 一起规定 `Load` 默认带 `ctl`。改 values-and-types.md、memory-partitions.md 规则 2。
3. **（一致性）原子只能在 `Unknown` 与派生规则矛盾。** 规则 1（根是 `StackSlot` 就必须用 `Local` token）和 `PartitionMismatch` 会拒绝 `local_atomic`；规则 6 说 `Cell`/原子的内存永远在 `Unknown`/`Local`，但 rustc 对 `&mut AtomicU32` 发 noalias（实测；`&mut Cell` 同理，`MutableRef` 只看 `Unpin`），按分区表应是 `Unique`。建议：原子/volatile 访问用地址所属的分区；“排序”语义另由细化 1 的输入表达；规则 6 改为“`&T` 且 `T: !Freeze`、`&mut T` 且 `T: !Unpin` 不建分区”。收益：`local_atomic` 去掉 `lock`，`unique_atomic` 转发。改 memory-partitions.md 规则 6/7、nodes.md。
4. **`Freeze` 的粒度写明：整类型、浅层。** `&Mixed` 整体进 `Unknown`（rustc 实测同样不给任何属性）；不做按字段的 `Frozen`（比 rustc 强，违反红线 1）。`&&Cell`、`&Vec<Cell<_>>` 是 `Frozen`，但只覆盖 `size_of(T)` 字节，经它 load 出的指针按规则 1 归 `Unknown`。改 memory-partitions.md 规则 6。
5. **泛型 `&T` 的分区依赖孔位。** R 图里 `&T`（`T` 为 `Param`）不知道 `IsFreeze(T)`/`IsUnpin(T)`，只能先当 `Unknown`；S 级得到答案后要拆链。这改变该根上所有内存节点和所有调用的效果输入，不是“只从填了答案的节点开始”的局部化简。建议：R 级为这类参数建“待定分区”（R 级按 `Unknown` 优化），S 级把它及其内存节点、调用作为种子；`IsFreeze`/`IsUnpin` 进答案键。另外“无 `UnsafePinned`”这一条件 1.94.1 并不检查，`Hole` 里也没有对应孔位：要么删，要么加 `IsUnsafeUnpin` 孔位并注明比 rustc 保守。改 memory-partitions.md、[generics-specialization.md](../../design/ir/generics-specialization.md#孔位)。
6. **按值传入的聚合里的引用。** rustc 对 Scalar/ScalarPair 聚合里的引用也发属性（`pair.0`），现设计只看直接参数，`pair` 会多一次重读。这依赖 `BackendRepr`，不应进 R 级语义。建议：首版保持只看直接参数并记为已知差距；若扩展，按“参数值中所有引用字段”定义（与布局无关），并把 [tests/ui/codegen/issue-63787.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/codegen/issue-63787.rs)（`Ref`/`RefMut` 改用 `NonNull` 的原因）加入差分集。改 memory-partitions.md 分区表。
7. **派生链要穿过指针 `Cast`。** `UnsafeCell::get` 是 `as *mut T`，`RefMut` 里是 `NonNull`；规则 1 只列了 `FieldAddr`/`IndexAddr`/`AddrAdd`/`Reborrow`，否则 `RefCell` 的计数与值无法判定不相交。另需“写回已知当前值的 store 删除”（用 `Refine`），O3 的 `refcell` 正是靠它删掉计数恢复。改 memory-partitions.md 规则 1、[optimization-pipeline.md](../../design/ir/optimization-pipeline.md#r-级)。

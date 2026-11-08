# W07：`&dyn` / `Box<dyn>`、vtable 与去虚化

检验 `CallVirtual`、vtable 作为孔位答案和数据、`WidePtr(ptr, vtable)` 的 unsize 表示、`Box<dyn>` 经 vtable 的 drop，以及被调者未知时的调用摘要。命令：[scripts/w07.sh](scripts/w07.sh)（MIR/IR 用 rustc 1.94.1，x86 汇编用 rustc 1.99.1 `x86_64-unknown-linux-musl`），代码块中的数字是脚本输出原样粘贴。标“推断”的是设计模拟，其余是实测。

## 1. 程序

[scripts/w07.rs](scripts/w07.rs)。`Rect` 含 `String`（需要 drop），`Sq` 不需要；`Sq` 用默认方法 `name`。三个函数放在 `mod dynk` 里，`-Ccodegen-units=16` 时与 `main` 分到不同 CGU。

```rust
pub fn total(xs: &[&dyn Shape]) -> u64 { xs.iter().map(|s| s.area()).sum() }   // 被调者未知
pub fn local_devirt(k: u64) -> u64 {
    let s: Box<dyn Shape> = Box::new(Rect { w: k, h: 3, tag: String::new() });   // unsize
    s.area()                                                                     // 内联 Box::new 后类型已知
}                                                                                // 经 vtable drop
pub fn emit(w: &mut dyn Write, x: u64) -> io::Result<()> { writeln!(w, "{x}") } // 默认方法 write_fmt
// main: let xs: [&dyn Shape; 2] = [&r, &q];  let out: Box<dyn Write> = Box::new(io::stdout());
```

## 2. rustc 现在怎么做（实测）

**vtable 内容。** 布局 `[drop_in_place 或 0, size, align, 方法…]`（`words` 是按 u64 解码的字节段）。`Sq`、`Stdout` 不需要 drop，槽 0 是空指针；`Sq` 的 `name` 槽指向默认方法实例 `Shape::name`；`Write` 有 8 个方法，`by_ref` 带 `where Self: Sized` 不进表，剩 7 个：

```text
  @vtable.1: | fn core::ptr::drop_in_place<w07::Rect> | words 40 8 | fn <w07::Rect as w07::Shape>::area | fn <w07::Rect as w07::Shape>::name
  @vtable.2: | words 0 8 8 | fn <w07::Sq as w07::Shape>::area | fn w07::Shape::name
  @vtable.3 fn_entries=7
```

**MIR 不去虚化。** `local_devirt` 里 unsize（`unsize=1`）之后仍是 `<dyn Shape as Shape>::area` 的虚调用（`virtual=1`），`Box::<Rect>::new` 也还是调用。

**LLVM IR 与汇编。**

```text
== pre IR calls（节选）
  inst=57   direct=4   indirect=1  invoke=2  landingpad=2  w07::dynk::local_devirt
  inst=23   direct=0   indirect=1  invoke=0  landingpad=0  w07::dynk::emit
  inst=5    direct=1   indirect=0  invoke=0  landingpad=0  w07::dynk::total
  inst=7    direct=0   indirect=1  invoke=0  landingpad=0  w07::dynk::total::_{{closure}}
== o3 IR calls（节选）
  inst=3    direct=1   indirect=0  invoke=0  landingpad=0  w07::dynk::local_devirt
  inst=10   direct=0   indirect=1  invoke=0  landingpad=0  w07::dynk::emit
  inst=15   direct=0   indirect=2  invoke=0  landingpad=0  w07::dynk::total
  inst=35   direct=2   indirect=1  invoke=1  landingpad=1  core::ptr::drop_in_place<alloc::boxed::Box<dyn std::io::Write>>
== x86
    6 asm-inst _RNvNtCs9fVHEmHMnvJ_3w074dynk12local_devirt:
   16 asm-inst _RNvNtCs9fVHEmHMnvJ_3w074dynk5total:
   12 asm-inst _RNvNtCs9fVHEmHMnvJ_3w074dynk4emit:
   indirect calls via register/memory, GOT excluded: 3
```

- `local_devirt`：O3 剩 3 条指令。LLVM 从常量全局 `@vtable.1` 读出 `area` 槽，内联后 `w*h + len` 变成 `mul k, 3`，**连堆分配也删了**（`__rust_alloc` 的属性见下），只剩 `__rust_no_alloc_shim_is_unstable_v2` 调用。
- `total`：唯一调用点传入长度 2 的数组，O3 把它特化成两次间接调用（不是循环）。`emit`：ArgPromotion 把 `write_fmt` 槽提到调用方 `main` 去读，`emit` 里仍是间接调用。
- `Box<dyn>` 的 drop：每个 `drop_in_place<Box<dyn _>>` 都是“读槽 0 → 判空 → 调用 → 从 vtable 读 size/align → dealloc”（判空在 `rustc_codegen_ssa/src/mir/block.rs` L646–669）。O3 的 `main` 已经把常量 `@vtable.3` 作为实参传给 `drop_in_place<Box<dyn Write>>`，但 drop glue 没被内联，运行时仍读槽 0、size、align。

**vtable 去重。** 每个 CGU 自建一份：`CodegenCx.vtables` 是按 CGU 的缓存（`rustc_codegen_llvm/src/context.rs` L106–107，`rustc_codegen_ssa/src/meth.rs` 的 `get_vtable`），按 `(Ty, trait_ref)` 查，生成 private 常量。同一 `(Rect, Shape)` 在两个 CGU 各一份；跨 crate 同理。三个 dyn drop shim 只差被调符号：

```text
  cgu.0: vtables=5 rect_shape_vtables=1 fns: main 
  cgu.1: vtables=1 rect_shape_vtables=1 fns: emit local_devirt total 
  shims=3 distinct_bodies=3 distinct_with_symbols_masked=1
```

按内容合并合法：core 明确不保证 vtable 地址唯一，也允许不同类型共用相同 vtable（`core/src/ptr/metadata.rs` L154–157，按 Homebrew 1.94.1 rust-src 核对）。

**别名与分配器事实。** `&mut dyn Write` 的数据指针没有 `noalias`（`dyn` 不知道是否 `Unpin`），vtable 指针是 `noalias readonly dereferenceable(80)`（10 项 × 8 字节），每个间接调用前的槽 load 都带 `!invariant.load`（`3 / 6` 里另外 3 个是从内存读宽指针的 vtable 部分和异常指针）；`__rust_alloc` 带分配器属性，这是 LLVM 能删掉分配的依据：

```text
== pre IR: parameter attributes of dynk::emit (&mut dyn Write = data ptr + vtable ptr)
  ptr noundef nonnull align 1 %w.0
  ptr noalias noundef readonly align 8 captures(address, read_provenance) dereferenceable(80) %w.1
== pre IR: pointer loads with !invariant.load / all pointer loads, in dynk fns
  3 / 6
== pre IR: allocator attributes on the __rust_alloc declaration
  nounwind allockind("alloc,uninitialized,aligned") allocsize(0) 
```

## 3. kfrc 编译计划

**Body：** `_2 = move _3 as Box<dyn Shape> (PointerCoercion(Unsize))`，`_0 = <dyn Shape as Shape>::area(&*_2.0.0)`，`drop(_2)`，cleanup 再 `drop(_2)`。 **R 图（非泛型，R = S）：**

```text
n1 = Start : (Control, Effect[P0], u64)
n2 = Aggregate{Rect}(n1.2, Const 3, Const String::new()) : Rect
n3 = Call(Box::<Rect>::new) ctl=n1.0 mem=[P0:n1.1] (n2) : (Control, Control?, Effect[P0], Box<Rect>)
n4 = Cast.Unsize{VTable(Rect, Shape)}(n3.3) : Box<dyn Shape>          // 细化 W07-2
n5 = Field[0](Field[0](Field[0](n4))) : *const dyn Shape                 // Box → Unique → NonNull → 宽指针
n6 = CallVirtual{Shape::area}(n5) ctl=n3.0 mem=[P0:n3.2] : (Control, Control?, Effect[P0], u64)
n7 = DropValue(n4) ctl=n6.0 mem=[P0:n6.2] : (Control, Control?, Effect[P0])
n8 = Return ctl=n7.0 (n6.3)
n9 = Landing(n6.1) : (Control, Exception)  → DropValue(n4) → Resume(n9.1)
```

| 步 | 变换 | 规则 / Rust 事实 |
|---|---|---|
| 1 | 内联 `Box::new`（`#[inline(always)]`）：`p = Call(__rust_alloc)(40, 8)`，空指针进冷区域 `handle_alloc_error`，`Store[P0](p, n2)` | 直接调用 |
| 2 | `Field(Cast.Unsize(Aggregate{Box}(…p…)))` → `WidePtr(p, Const(VTable K))` | 聚合折叠 + Unsize 穿透（细化 W07-2） |
| 3 | `CallVirtual{area}(WidePtr(_, Const K))` → `Call(<Rect as Shape>::area)(p)` | vtable 答案里查槽位实例，不读内存（细化 W07-3） |
| 4 | 内联 `area`；`Load[P0](p)` 的 `w`/`h`/`tag.len` 从步 1 的 `Store` 转发 → `Mul(k, 3)` | 同一 `Unknown` 链上中间没有效果 |
| 5 | `DropValue(Box<dyn Shape>)` 且 vtable 已知 → `DropValue(Box<Rect>)`：`tag` 的 cap 已知为 0，drop 消失，剩 `__rust_dealloc(p, 40, 8)` | dyn drop 的去虚化（细化 W07-4） |
| 6 | `area` 摘要 nounwind → `n9` 的 Landing 删除 | 调用摘要 |

**到这里停住（推断）：** 剩 `__rust_alloc` + `Store` + `__rust_dealloc` + `mul`。设计没有分配器语义（[memory-partitions.md](../../design/ir/memory-partitions.md) 还明确不做返回值 noalias），删不掉这对调用，比 rustc O3 多两次库调用（细化 W07-6）。

**`total` 与 `emit`（推断）：** 元素是从内存读出的宽指针，vtable 未知，`CallVirtual` 保留。L 级变成 `Load(meta + 8×(3+槽号))` + `CallIndirect`；kfrc 不做“唯一调用点 → 常量参数特化”，`total` 保留循环。`emit` 同 rustc，`write_fmt` 槽在 `emit` 内读。

**检验 1：vtable 作为答案和数据。**

- R 级：`T` 是 `Param` 时 `&T → &dyn Shape` 要问一个孔位 `VTable(T, Shape)`；现有 `Hole` 只有 `Resolve`（单个槽位），没有整张表（细化 W07-1）。
- 答案：`VTableKey = hash(size, align, 槽 0 的 drop glue 答案键或空, 每个方法槽的实例答案键或 Vacant, 超 trait vptr 的键)`。S 级是值 `Const(VTable K)`。
- 数据：K 在 build 级存储里只生成一份，不分 CGU、不分 crate；内容相同的不同类型共用（core 允许）。默认方法 `Shape::name` 的 R 图不依赖 `Self`，所有用默认实现的类型共用同一答案键和代码。
- 缓存（推断）：去虚化过的调用者读了 K 的槽位，机器码键要含 K 的内容哈希；只传递 K 的调用者只依赖 K 的符号。

**检验 2：`CallVirtual` 的摘要。** 被调者未知时取顶：`may_unwind` = panic 策略是否 unwind，`memory = Unknown`，全部指针实参可能被捕获。接收者分区与 rustc 的事实一致：`&dyn`/`&mut dyn` 的数据指针在 `Unknown`（`dyn` 不知道 `Freeze`/`Unpin`）。闭世界的改进（二进制 crate 已知全部 vtable 时取同槽实例摘要的并）要等可达性算完，晚于 S 级，首版不做。

**检验 3：unsize 的表示。** `&T → &dyn Tr`：`WidePtr(p, Const(VTable K))`（R 级是 `WidePtr(p, VTable(T, Tr))` 孔位）；`&[T; N] → &[T]`：`WidePtr(p, Const N)`。`Box<T> → Box<dyn Tr>` 的 unsize 作用在嵌套结构的指针叶子上，所以要一个 `Cast.Unsize{meta}` 节点，由“投影穿透”规则落到 `WidePtr`；L 级是 ScalarPair(ptr, ptr)。

## 4. 代价核算

| 项 | rustc | kfrc（推断） |
|---|---|---|
| vtable 数据 | 每个用到它的 CGU 一份 private 常量 | 每个 `VTableKey` 一份 |
| dyn drop glue | 每个 `dyn` 类型一个 shim，体只差符号 | 按图内容键共享一份（细化 W07-7） |
| `local_devirt` 运行时 | 分配被删，只剩乘法 | 去虚化和内联做到，分配删不掉（细化 W07-6） |
| 编译工作 | Box::new 内联、SROA、常量全局 load 折叠、内联 `area`、alloc 消除，分属多个 pass（具体 pass 未测） | 步 1–5 在同一 worklist 里完成，都是局部规则 |

## 5. 设计细化

| # | 问题 | 证据 | 建议改法 | 文件 |
|---|---|---|---|---|
| W07-1 | 没有表示整张 vtable 的孔位和常量 | `Hole` 只有 `Resolve`；vtable 按 `(Ty, trait_ref)` 整体生成 | 加 `Hole::VTable{ty, trait_ref}`，答案为 `VTableKey`；S 级是 `Const(VTable K)`；mentioned 集合记录它 | generics-specialization.md、nodes.md |
| W07-2 | unsize 强转没有节点，`Box<dyn>` 这种嵌套结构写不出来 | MIR `PointerCoercion(Unsize)` 直接作用在 `Box` 上 | 加 `Cast.Unsize{meta}`（对任意 `CoerceUnsized` 类型），加投影穿透规则；`&T`/切片直接产生 `WidePtr` | nodes.md、values-and-types.md |
| W07-3 | 去虚化规则没写 | MIR 不做；O3 靠常量全局 load 折叠 | S 级规则：`CallVirtual{slot}(WidePtr(_, Const K))` → `Call(K.entries[slot])`，用答案表而不是读内存；新调用进内联决策 | optimization-pipeline.md |
| W07-4 | `dyn` 的 drop 没有规定 | `drop_in_place<Box<dyn Write>>` 收到常量 `@vtable.3` 仍运行时读槽 0/size/align | `DropInPlace` 作用于 `dyn` = 读槽 0、判空、`CallIndirect`；`Box<dyn>` 释放用 vtable 的 size/align；vtable 已知时折叠成具体类型的 `DropValue` | ownership-drop-unwind.md |
| W07-5 | 没有“常量内存”概念，vtable 的 load 要挂效果链 | rustc 给 vtable 指针 `readonly noalias dereferenceable(80)`，槽 load 带 `!invariant.load` | 增加只读的 `Const` 分区（vtable、`ConstAlloc`）：load 是纯值，常量偏移的 load 直接折叠 | memory-partitions.md |
| W07-6 | 不能消除未逃逸的堆分配 | O3 `local_devirt` 只剩 3 条指令；`__rust_alloc` 带 `allockind`/`allocsize` | 加 `Alloc{size, align}`/`Dealloc` 节点（由分配器 lang item 产生）：结果是新分区，地址未逃逸且配对时整体删除 | nodes.md、memory-partitions.md、runtime.md |
| W07-7 | shim（dyn drop glue 等）的键没写，按类型做键会每个 `dyn` 一份 | 3 个 shim 原文各不相同，屏蔽符号后只有 1 种 | shim 生成后按图内容键入答案键表，与普通实例同样去重 | generics-specialization.md、size.md |
| W07-8 | `CallVirtual` 的摘要没写 | 被调者未知 | 明确取顶；闭世界的“同槽实例摘要的并”列为二进制 crate 阶段的后续扩展 | optimization-pipeline.md |

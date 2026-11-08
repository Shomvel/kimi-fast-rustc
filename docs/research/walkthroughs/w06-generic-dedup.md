# W06：泛型与去重

检验 [generics-specialization.md](../../design/ir/generics-specialization.md) 的孔位表、答案键、递归去重和 mentioned items。数字由 `scripts/w06.sh <workdir>` 产生，输出原样粘贴。环境变量：`RUSTC` 为 1.94.1，`RUSTC_X86` 为 1.99.1，`RUSTC_NIGHTLY` 为 1.101.0-nightly（2026-10-24），`CXXFILT` 为 llvm-cxxfilt。脚本加 `-Csymbol-mangling-version=v0`，让符号带上类型实参；它只改名字，不改代码。标“推断”的是设计模拟，其余是实测。

## 1. 程序

```rust
#[inline(never)] pub fn largest<T: PartialOrd + Copy>(xs: &[T]) -> T {
    let mut m = xs[0];
    for &x in xs { if x > m { m = x; } }
    m
}
#[inline(never)] pub fn push_one<T>(v: &mut Vec<T>, x: T) { v.push(x) }
// main：largest 用于 u32/i32/char/u64/f64（f64 输入含 NaN），push_one 用于 u32/i32/[u8;4]/char；输入经 black_box。见 scripts/w06.rs。
```

`#[inline(never)]` 让各实例在 O3 后仍能单独比较。std 1.94.1 的调用链：`Vec::push` → `#[inline] push_mut` →（满时）`#[inline(never)] RawVec<T>::grow_one` → 非泛型 `RawVecInner::grow_amortized(len, 1, T::LAYOUT)`。

## 2. rustc 现在怎么做

**MIR。** `largest` 中的比较是 `_7 = <T as PartialOrd>::gt(move _8, move _9)`，参数来自 `_8 = &_6; _9 = &_2`，所以 `x` 和 `m` 被取了地址。`slice::Iter` 已经内联，`switchInt(const <T as std::mem::SizedTypeProperties>::IS_ZST)` 出现两次，多态体里同时保留 ZST 和非 ZST 两套循环。

**实例数**（T 已替换为字面 `T`；drop glue 行略）。每个 T 都有一个 `gt` 实例：多态体里的 trait 调用 MIR 内联不了，单态化后才解析。nightly 与 1.94.1 的差别在 std 的非泛型 inline 函数和 C `main`（推断）。
```text
== instances per definition (unoptimized IR defines: 60; O3 IR defines: 18; O3 without MergeFunctions: 21)
  pre    4 <alloc::raw_vec::RawVec<T>>::grow_one
  pre    4 <alloc::vec::Vec<T>>::push_mut
  pre    5 core::cmp::impls::<impl core::cmp::PartialOrd for T>::gt
  pre    5 w06::largest::<T>
  pre    4 w06::push_one::<T>
  o3    2 <alloc::raw_vec::RawVec<T>>::grow_one
  o3    5 w06::largest::<T>
  o3    3 w06::push_one::<T>
  nightly 1.101.0-nightly (12433c307 2026-10-24): MONO_ITEM count = 56
```

**重复类。** exact：只把函数自己的名字归一后逐行比较。fixpoint：被调者按所属类比较，取不动点（同 `-Zmono-dedup`）。属性组和 `!range`/`!align` 按内容比较，不剥离；内联标签和局部名按函数重新编号。
```text
== duplicate classes, pre IR
  functions=60 inst=1884 exact-classes=53 (removes 79 inst) fixpoint-classes=47 (removes 174 inst)
  merged 2 (2 inst, exact-groups=2): w06::push_one::<i32> | w06::push_one::<u32>
  merged 3 (27 inst, exact-groups=1): <alloc::raw_vec::RawVec<char>>::grow_one | <alloc::raw_vec::RawVec<i32>>::grow_one | <alloc::raw_vec::RawVec<u32>>::grow_one
  merged 2 (43 inst, exact-groups=2): <alloc::vec::Vec<i32>>::push_mut | <alloc::vec::Vec<u32>>::push_mut
== duplicate classes, o3nomerge IR
  functions=21 inst=559 exact-classes=19 (removes 18 inst) fixpoint-classes=18 (removes 33 inst)
== duplicate classes, o3 IR
  functions=18 inst=526 exact-classes=18 (removes 0 inst) fixpoint-classes=18 (removes 0 inst)
== duplicate classes, x86 asm
  functions=17 inst=747 exact-classes=14 (removes 102 inst) fixpoint-classes=14 (removes 102 inst)
  merged 2 (76 inst, exact-groups=1): w06::largest::<char> | w06::largest::<u32>
  merged 2 (20 inst, exact-groups=1): w06::push_one::<char> | w06::push_one::<i32>
```

pre IR 的 drop glue 另有 4 组合并（见完整输出）。

- `push_mut<u32>`/`<i32>` 只差被调者（exact-groups=2）：`grow_one` 先合并，它们才能合并，所以去重必须递归。O3 的 LLVM MergeFunctions（rustc 默认 `Aliases`）做了同样的合并：关掉它是 21 个函数，fixpoint 后也是 18 个。
- x86 汇编比 IR 多出两组。未优化 IR 里 `largest<u32>` 与 `<char>` 只差 char 值域事实（返回值 `range(i32 0, 1114112)`、load 带 `!range`）和被调的 `gt`；`push_mut<u32>` 与 `<char>` 只差参数 `range(i32 0, 1114112)` 和被调的 `grow_one`。
- `largest<u32>` 与 `<i32>` 只差 call 行（`PartialOrd for u32>::gt` 对 `PartialOrd for i32>::gt`）。`grow_one<u32>` 与 `<[u8; 4]>` 只差 elem_layout 实参：`i64 noundef 1, i64 noundef 4, i64 noundef 4` 对 `i64 noundef 1, i64 noundef 1, i64 noundef 4`。
- `&[T]` 的 len 范围由 `size_of::<T>()` 决定：`largest<u32>` 是 `range(i64 0, 2305843009213693952)`，`<u64>` 是 `range(i64 0, 1152921504606846976)`。

```text
== PartialOrd::gt bodies (unoptimized IR, compare lines)
  core::cmp::impls::<impl core::cmp::PartialOrd for f64>::gt   %v = fcmp ogt double %v, %v
  core::cmp::impls::<impl core::cmp::PartialOrd for i32>::gt   %v = icmp sgt i32 %v, %v
  core::cmp::impls::<impl core::cmp::PartialOrd for u32>::gt   %v = icmp ugt i32 %v, %v
  core::cmp::impls::<impl core::cmp::PartialOrd for u64>::gt   %v = icmp ugt i64 %v, %v
  core::cmp::impls::<impl core::cmp::PartialOrd for char>::gt  %v = icmp ugt i32 %v, %v
```

x86 汇编中整数版本是展开 4 倍的 `cmp` + `cmov` 循环；f64 版本用 `maxsd`，操作数顺序保证 NaN 时结果与 `x > m` 一致。

## 3. kfrc 编译计划（推断）

**R 图**（`largest` 的非 ZST 循环，R 级优化后）。`P0 = Frozen(xs)`（`T: Copy` 意味着 T 不含 UnsafeCell），首版是 `Unknown`；`U` 表示 `Unknown`。
```text
n3  = Check.Bounds ctl=n1.0 (n0, n2) : (Control, Control)           // n0 = Const 0，n2 = Field[1](xs) 即 len
n5  = Load[P0]{T}(n4) ctl=n3.0 mem=n1.1 : T                         // xs[0]
n6  = If ctl=n3.0 (ConstValue{IS_ZST,[T]}) : (Control, Control)      // h1：两套循环都在 R 图里
n12 = Phi(n10, n5, n19) : T                                         // m
n13 = Load[P0]{T}(n11) mem=n1.1 : T                                 // x
n14 = StackSlot{T}; n15 = StackSlot{T}                              // &x、&m 要传给 TraitCall
n16 = TraitCall{PartialOrd::gt,[T]} ctl=n10 mem=[U:n9] (n14, n15) : (Control, Effect[U], bool)   // h2
n17 = IndexAddr(n11, n18) : *const T                                // n18 = Const 1，步长 h0
```

**孔位表。** `push_mut` 在 R 级就内联进 `push`：被调者的实参正是本函数的 `T`。

| 定义 | 孔位（括号内为用途） |
|---|---|
| `largest` | h0 `SizeAlign(T)`（步长、`&[T]` len 事实）；h1 `ConstValue{IS_ZST,[T]}`（选循环）；h2 `Resolve{PartialOrd::gt,[T]}`（比较）；h3 `BackendRepr(T)`（L，寄存器类）。没有 `NeedsDrop`：`T: Copy` 意味着不需要 drop |
| `push`/`push_one` | h0 `FieldOffsets(Vec<T>)`（len、ptr、cap）；h1 `ConstValue{IS_ZST,[T]}`（`capacity()`）；h2 `Resolve{RawVec::grow_one,[T,Global]}`；h3 `SizeAlign(T)`（`add(len)`）；h4 `NeedsDrop(T)`（`grow_one` 的 unwind 边上 drop `value`）；h5 `BackendRepr(T)`（L，写入） |
| `grow_one` | h0 `FieldOffsets(RawVec<T>)`（`self.inner`）；h1 `ConstValue{LAYOUT,[T]}`（elem_layout）；h2 `Resolve{grow_amortized,[Global]}` |

**答案向量与键。**

| 实例 | 答案 | 键 |
|---|---|---|
| `largest<u32>` | (4,4), false, K(gt.u32: `Compare.ugt`), Int32 | L1 |
| `largest<i32>` | (4,4), false, K(gt.i32: `Compare.sgt`), Int32 | L2 |
| `largest<char>` | (4,4), false, K(gt.char), Int32 值域 [0, 0x10FFFF] | L3 |
| `largest<u64>` / `<f64>` | (8,8), false, K(gt.u64) / K(gt.f64: `Compare.fogt`), Int64 / F64 | L4 / L5 |
| `grow_one<u32/i32/char>` / `<[u8;4]>` | 布局相同, (4,4) / (4,1), K(grow_amortized) | G1 / G2 |
| `push<u32/i32/char>` | Vec 布局, false, **G1**, (4,4), false, Int32 | P1（需 D1） |
| `push<[u8;4]>` | Vec 布局, false, **G2**, (4,1), false, Memory | P2 |

- 比较语义都通过 `Resolve` 进入键，包括有符号、无符号和浮点有序比较（NaN 时为 false）。所以 `largest` 有 5 个键，与 rustc O3 IR 一致。前提是被调者的 R 图哈希能区分比较模式（D2）。
- **递归：** `push` 的 h2 答案就是 G1/G2，所以 `push` 随 `grow_one` 一起去重。这里没有调用环，不需要类细化。
- 本例不涉及 `repr(packed)` 和 Copy 类型的 `Use` 调用。`&[T]` 的 len 范围由 h0 覆盖。

**mentioned items**（R 级优化前登记）：`largest` 登记 `gt<T>`、`panic_bounds_check`、`IS_ZST<T>`、`Location`；`push` 登记内联前的 `push_mut<T>` 及并入的它的集合（`grow_one<T,Global>`、unwind 上的 `drop_in_place::<T>`、`LAYOUT<T>`）。S 级对 u32 删除 unwind 上的 `DropValue`，但 `drop_in_place::<u32>` 仍要解析（rustc 中是 `DropGlue(None)`，不生成代码）。

**S/L 级。** h2 变成 `Call(gt.u32)` 并内联 gt 体，引用随之消失，`x`、`m` 提升为 SSA，得到 `Phi + Select`；h1 折叠后 ZST 循环被删除。期望 `largest<u32>` 约 10 条指令（`test/je; mov; lea; loop: mov; cmp; cmova; add; cmp; jne; ret`）加一个冷尾块，rustc 见 §2 x86 asm 的 `largest` 行（含展开）。f64 的 `Select(fogt)` 只有在操作数顺序保持 NaN 语义时才能变成 `maxsd`，这条窥孔规则必须看 opcode 的比较模式。

## 4. 代价核算

```text
== S-graph recompute proxy: R-graph size ~ unoptimized-IR inst of the u32 instance.
   instances = rustc instances; keys = kfrc answer keys INFERRED in W06 section 3 (input, not measured)
  w06::largest::<u32>                           n=61  instances=5 keys=5  specialize-nodes: per-instance=305  per-key=305
  <alloc::vec::Vec<u32>>::push_mut              n=43  instances=4 keys=2  specialize-nodes: per-instance=172  per-key=86
  <alloc::raw_vec::RawVec<u32>>::grow_one       n=27  instances=4 keys=2  specialize-nodes: per-instance=108  per-key=54
  TOTAL R-graph nodes(proxy)=131  specialize-nodes per-instance=585 per-key=445
```

- **体积与缓存键：** kfrc 按 §3 的键生成代码，rustc 汇编份数见 §2 的 o3 行（其中 `largest<char>`、`push_one<char>` 与其他实例逐条相同）。kfrc 让 `push<char>` 共享、`largest<char>` 不共享（D1）。P1 被 u32、i32、char 三个 `InstanceKey` 共用，新增布局相同的 T 不产生新项。
- **S 图不缓存：** 同一进程内每个键特化一次，S 图被内联方用完即丢，没有额外代价。跨 crate 时，`Vec<u32>::push` 的 S 图是内联来源：每个内联它而机器码未命中的下游 crate 都要重新特化一次（push_mut n=43）。总量是 crate 数 × 被内联的键数，形状与 ThinLTO 导入体重新简化相同，只是每次代价从 O3 降到种子 worklist。
- **复制浪费：** R 图同时含两套循环。“先复制整图再替换”会把 h1 折叠后要删掉的那一半也复制一遍（D5）。

## 5. 设计细化

| # | 问题 | 证据 | 建议改法 | 文件 |
|---|---|---|---|---|
| D1 | `BackendRepr(T)` 如果照搬 rustc，就含符号位和值域：`push<u32>/<i32>/<char>` 会成为 3 个键，与 cache.md 自己的例子矛盾。反过来，如果不含值域，而 S 级又直接从 `TyId` 读 char 值域，就会错误共享 | rustc_abi 中是 `Primitive::Int(Integer, bool)`；pre IR 中 char 实例只差 `range`/`!range`；汇编里 char 与 u32/i32 实例相同 | 拆成 `ScalarShape(T)`（无符号位）和 `ValidRange(T)`。`ValidRange` 只在 T 值流入 Compare/Switch/Cast/算术/调用实参时登记，只经过 Load/Store/Phi/Return 时不登记。S/L 级只能通过答案表读类型事实；debug 构建下读到未登记的事实就 ICE（相当于 `-Zmono-dedup` 对 `ty.kind()` 的审计） | generics-specialization.md、values-and-types.md、levels.md |
| D2 | 比较语义要靠被调者的 R 图哈希区分 `ugt/sgt/fogt`，但 opcode 模式只为整数算术定义过。非泛型 R 图里的具体 `TyId` 是否进哈希也没有定义；如果进，`largest::<Meters>`（`struct Meters(u32)`）与 `<u32>` 永远不能共享，叶子函数的类型名会切断递归去重 | `gt` 体只差 `icmp ugt/sgt`、`fcmp ogt`；u32 与 char 都是 `icmp ugt i32` | 用 `Compare{int_signed / int_unsigned / float_ordered / float_unordered, pred}` 表示比较；哈希对具体类型使用与孔位答案相同的规范描述（“闭合孔位”），不哈希类型名 | values-and-types.md、nodes.md、generics-specialization.md |
| D3 | `&x`、`&m` 传给没有摘要的 `TraitCall`，按表会进入 `Unknown` 分区。S 级内联后要提升槽位、拆回分区，但“只从被填答案的节点开始”的种子规则覆盖不到这一步 | MIR 有 `_8 = &_6; _9 = &_2`；每个 pre IR 实例都有 `%x`/`%m` alloca | 实参类型为 `&T` 且 `T: Freeze` 时，照 rustc 现有事实处理（readonly，只捕获 read_provenance），槽位留在 `Local`；S 级种子加入“曾作为被删除或被内联调用实参的 StackSlot” | memory-partitions.md（规则 3）、optimization-pipeline.md、generics-specialization.md |
| D4 | “S 图不存”、“内联时拼接被调者的 S 图”和 C2“每个键跨 crate 只特化一次”三条在跨 crate 内联来源上互相冲突 | §4：下游每个 crate 都要重算 push_mut（n=43） | 只存内联来源的 S 图（`size_estimate` 不超过内联阈值，按答案键存，这些图本来就小）；C2 改为“机器码和摘要每键一次” | cache.md、compile-time.md、optimization-pipeline.md |
| D5 | 特化步骤 1 先复制整张 R 图，h1 决定删掉的那套循环也被复制 | MIR 有两处 `IS_ZST` switch | 特化时边复制边做构建期窥孔，常量孔位让不可达分支根本不被复制（与 rustc 的 `mono_reachable` 同理） | generics-specialization.md（特化） |
| D6 | mentioned 只写了“S 级逐个求值”，没写 R 级内联后被调者集合要并入（rustc 会递归检查），也没写后单态错误在同键实例之间怎么报告 | `push` 内联了 `push_mut`；rustc 的 collector 递归检查 mentioned | 内联时并入被调者的 mentioned 集合；常量求值结果按答案键缓存，诊断按每个 `InstanceKey` 用真实类型报告 | generics-specialization.md（mentioned items） |

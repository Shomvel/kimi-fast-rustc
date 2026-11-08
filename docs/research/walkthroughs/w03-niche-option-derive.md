# W03：niche、`Option<&V>` 与 `derive(PartialEq)`

检验变体操作（`MakeVariant`/`VariantOf`/`VariantSwitch`/`VariantField`）、niche 编码推迟到 L 级、`Refine`，以及 derive 的 `eq` 在 S 级如何折叠成变体比较。数字由 `scripts/w03.sh <workdir>` 产生，输出原样粘贴（`RUSTC` = 1.94.1，`RUSTC_X86` = 1.99.1）。标“推断”的是设计模拟，其余是实测。

## 1. 程序

```rust
#[derive(PartialEq)] pub enum Kind { Bin, Lib, Rlib, Dylib, Test }
#[derive(PartialEq)] pub enum CrateType { Bin, Lib, Rlib, Dylib, Cdylib, Staticlib, ProcMacro, Other(String) }
impl Map { pub fn find(&self, k: u32) -> Option<&CrateType> {
    let i = self.keys.iter().position(|&x| x == k)?; self.vals.get(i) } }
#[inline(never)] pub fn doctestable(m: &Map, k: u32) -> bool {
    if let Some(v) = m.find(k) { *v == CrateType::Rlib || *v == CrateType::ProcMacro } else { false } }
#[inline(never)] pub fn any_lib(ts: &[CrateType]) -> bool {   // cargo Target::doctestable 的 any 闭包
    ts.iter().any(|k| *k == CrateType::Rlib || *k == CrateType::Lib || *k == CrateType::ProcMacro) }
#[inline(never)] pub fn kind_eq(a: Kind, b: Kind) -> bool { a == b }
#[inline(never)] pub fn same(a: &CrateType, b: &CrateType) -> bool { a == b }
```

完整源码（含 `black_box` 输入的 `main`）见 [scripts/w03.rs](scripts/w03.rs)。

## 2. rustc 现在怎么做（实测）

**布局。** `CrateType` 是 24 字节，tag 是 `Other` 里 `String` 的 capacity（`UsizeNoHighBit`，2^63 个非法值，比指针的 1 个多），被重排到偏移 0；`ptr`/`len` 在 8/16。`niche_start = 2^63`，变体 0–6 编码为 `2^63+v`，`Other`（7）是 untagged。tag 的 `!range` 是 `[0, 2^63+7)`。`Option<&CrateType>` 的 None 是 null。与 [enum-niche.md](../rustc/layout-abi-mono/enum-niche.md) 的规则一致。下面是脚本输出节选（未删改的行）：

```text
== unoptimized IR (after MIR opts), user functions + reached callees
   67 inst  alloca=2   memcpy=0   lifetime=2   assume=2   call=3   invoke=0   landingpad=0  load=9    store=6    <doctestable>
   71 inst  alloca=1   memcpy=0   lifetime=0   assume=3   call=3   invoke=0   landingpad=0  load=10   store=4    <any_lib::closure>
TOTAL functions=8 inst=286 alloca=13 memcpy=0 lifetime=17 assume=11 call=10 invoke=0 landingpad=0 load=41 store=29
== O3 IR
TOTAL functions=4 inst=78 alloca=0 memcpy=0 lifetime=0 assume=1 call=1 invoke=0 landingpad=0 load=13 store=0
== CrateType niche: !range on the tag load (pre IR) and field offsets used for Other(String)
!6 = !{i64 0, i64 -9223372036854775801}
  %6 = getelementptr inbounds i8, ptr %k, i64 8
  %8 = getelementptr inbounds i8, ptr %k, i64 16
== unoptimized IR: Option/niche decode in doctestable (sub / icmp u* / select)
  %6 = sub i64 %5, -9223372036854775808
  %7 = icmp uge i64 %5, -9223372036854775808
  %_7 = select i1 %7, i64 %6, i64 7
== unoptimized IR: icmp eq against variant constants in doctestable
  %8 = icmp eq i64 %_7, 2
  %9 = icmp eq i64 %_7, 2
  %10 = icmp eq i64 %_7, 7
== unoptimized IR: SlicePartialEq::equal calls (all / with poison args)
12
5
== unoptimized IR: llvm.assume / load ptr with !nonnull / niche decodes (icmp uge 2^63), per function
  any_lib28_: assume=3 load-ptr-!nonnull=3 niche-decode=3
== LLVM passes that changed each function (O3 pipeline, host target)
  doctestable: total 36: 6 SimplifyCFGPass; 6 InstCombinePass; 5 LCSSAPass; 4 LoopSimplifyPass; 2 PostOrderFunctionAttrsPass; 2 JumpThreadingPass; 2 EarlyCSEPass; 1 TailCallElimPass; 1 SROAPass; 1 LoopUnrollPass; 1 InstSimplifyPass; 1 InlinerPass; 1 InferFunctionAttrsPass; 1 IPSCCPPass; 1 GlobalOptPass; 1 GVNPass;
  any_lib: total 22: 7 InstCombinePass; 5 LCSSAPass; 1 SROAPass; 1 PostOrderFunctionAttrsPass; 1 LoopUnrollPass; 1 InstSimplifyPass; 1 InlinerPass; 1 InferFunctionAttrsPass; 1 IPSCCPPass; 1 GlobalOptPass; 1 GVNPass; 1 EarlyCSEPass;
== x86 asm (rustc release printed above)
   25 asm-inst _RNvCsj4z1WP6sg4c_3w0311doctestable:
   21 asm-inst _RNvCsj4z1WP6sg4c_3w037any_lib:
    3 asm-inst _RNvCsj4z1WP6sg4c_3w037kind_eq:
   29 asm-inst _RNvCsj4z1WP6sg4c_3w034same:
== O3 IR: compares/switch in doctestable and any_lib
  [3w0311doctestable17h]
    %6 = icmp slt i64 %5, 0
    %7 = add i64 %5, 9223372036854775806
    %8 = and i64 %7, -5
    %switch.selectcmp3 = icmp eq i64 %8, 0
  [3w037any_lib17h]
    %0 = xor i64 %ptr.val.i, -9223372036854775808
    switch i64 %_5.i.i, label <exit> [
    i64 2, label <exit>
    i64 1, label <exit>
    i64 6, label <exit>
== CrateType size: dereferenceable on same(&CrateType, ..) parameters (pre IR)
  dereferenceable(24) %a
== O3 IR: niche decodes (xor/slt/select) in same
  xor=2 icmp-slt=2 select=2
== x86 asm: tag tests in doctestable / any_lib
  3w0311doctestable: andq $-5, %rax
  3w0311doctestable: movabsq $-9223372036854775806, %rcx
  3w0311doctestable: cmpq %rcx, %rax
  3w0311doctestable: sete %al
  3w037any_lib: movabsq $9223372036854775807, %rdx
  3w037any_lib: movl $35, %esi
  3w037any_lib: addq %rdx, %r8
  3w037any_lib: cmpq $5, %r8
  3w037any_lib: btq %r8, %rsi
```

- **cargo 的现象全部复现。** derive 的 `eq` 先比两边 `discriminant_value`，再 `match (self, other)` 比 `Other` 的字段。MIR 内联后 GVN 把提升常量 `CrateType::Rlib` 的判别值折成 2，却留下同一条件下重复的 `switchInt(_5) -> [2: ..]` 和不可达的 `== 7` 分支；该分支读常量里 `Other` 的字段，codegen 得到 poison：12 个 `SlicePartialEq::equal` 调用里 5 个带 poison 实参。
- 每个 `==` 都重新 `discriminant((*_2))`：any 闭包 3 次 niche 解码；3 个死分支各对 `k+8` 的 `!nonnull` load 再发一次 `assume(ptr != null)`。
- **O3 形状。** `doctestable` 的两次比较在 LLVM 21 IR 中成为 `tag <s 0 && ((tag + 2^63-2) & -5) == 0`，1.99.1 的 x86 汇编进一步成为 `(tag & -5) == 2^63+2`；`any_lib` 的 IR 仍先解码再 `switch {1,2,6}`，汇编变成 `tag + (2^63-1) <= 5` 加 `bt $35`。`same` 两边完整解码后比较。被改动的 pass 次数 36 / 22，与 evidence 中 cargo 的 31 次同量级。

## 3. kfrc 编译计划（推断）

**R 级：`<CrateType as PartialEq>::eq`。** 非泛型 impl，R 图就是最终图；内联进 any 闭包后（`k` 的参数是 `&CrateType`，`CrateType: Freeze`，进 `Frozen(k)`）：

```text
n2  = Load[Frozen(k)](n1) : CrateType            // 三次 discriminant 读经 GVN 成一次
n4  = Const(MakeVariant{CrateType, Rlib}) : CrateType   // 提升常量的 Load 折叠（细化 5）
n5  = Discriminant(n2) ; n6 = Discriminant(n4) → c2      // Discriminant(MakeVariant) 折叠
n7  = Compare.eq (n5, c2) : bool                 // 改写为 VariantOf(n2) == Rlib（细化 3）
n9  = VariantSwitch ctl=n8 (n4) [Other → dead]   // 常量操作数，Other–Other 分支不建，没有 poison 调用
```

三个 `||` 都是对同一 `VariantOf(n2)` 的常量比较，合并为一个 `VariantSwitch ctl=n0 (n2) [Lib, Rlib, ProcMacro → r1; _ → r2]`，结果 `Phi(true, false)`（细化 6）。没有 `assume`：`NonNull` 的非空在字段类型上。

**S 级：`doctestable`。** 非泛型，R=S。`find` 内联；`position` 的 `?` 产生 `VariantSwitch(Phi(MakeVariant None, MakeVariant Some(i)))`，`get` 产生 `Phi(MakeVariant None, MakeVariant Some(IndexAddr))`，再被 `if let` 的 `VariantSwitch` 消费。两处都要“Phi 上的变体线程化”才能让 `Option` 值不出现（细化 6）。`Some` 边上 `v = VariantField[Some,0]`，`*v` 的地址根是从 `m.vals` load 出的堆指针，用 `Unknown` 分区（只读，无影响）。

**L 级。**

| S 级 | L 级 |
|---|---|
| `VariantOf(Load[P](v))`，只喂给 `VariantSwitch` | 只读 tag 字段：`Load[P](v+0) : i64`（细化 4） |
| `VariantSwitch(x) [Rlib, ProcMacro → t; _ → f]` | niche 变体按 tag 值直接比较：`tag == 2^63+2`、`tag == 2^63+6`；untagged `Other` 和其他 niche 值落到默认边。不生成 sub/icmp/select 解码（细化 7） |
| `Option<&CrateType>` 的 `VariantSwitch` | `If(p != null)`；`VariantField[Some,0]` → `Refine{nonnull}(p) ctl=then`（细化 1） |
| `kind_eq`/`same`：`VariantOf(a) == VariantOf(b)` | `Kind`（Direct u8 tag）直接比 tag；`CrateType` 两边先解码成变体号再比（不同 `Other` 的 tag 不同），与 rustc 相同 |

**期望机器码。** `doctestable`：查找循环与 rustc 相同（每元素 5 条），尾部两次 `cmp`+`sete`/`or`，比 rustc 的 `and $-5` 技巧多约 2 条。`any_lib`：有位测试型 switch 降低时与 rustc 的 21 条同形，否则每元素 3 次 `cmp`。`kind_eq` 3 条。

## 4. 代价核算

| 项 | rustc 1.94.1 | kfrc（推断） |
|---|---|---|
| 进入后端的函数 | 8 个，286 条未优化 IR | 4 个对外函数；`find`、闭包、`eq` 只在图里存在 |
| niche 解码 | 每个 `==` 一次（`doctestable` 2、闭包 3），靠 GVN/InstCombine 合并 | R/S 级没有；L 级每个 `VariantSwitch` 一次 tag 读 |
| 死分支 | 5 个 poison 调用、3 个重复 assume，靠 SimplifyCFG/JumpThreading 删除 | 构建期常量折叠，不生成 |
| 缓存 | — | derive `eq` 每个类型一份 R 图；`String` 的内部布局变化只影响 L 级（但见细化 2） |

## 5. 设计细化

1. **（健全性）变体字段与 load 的控制依赖在 L 级会丢。** `Option<&T>` 在 L 级就是 `Ptr`，`VariantField[Some,0](x)` 降成 `x` 本身；`Load(x)` 只依赖地址和效果 token，placement 的“尽早”可以把它放到 `x != null` 判断之前，造成空指针读。裸指针的 `if !p.is_null() { *p }` 同理。证据：[nodes.md](../../design/ir/nodes.md#dump-格式) 的 `Load` 没有 `ctl=`，只有 [memory-partitions.md](../../design/ir/memory-partitions.md#规则) 规则 2 暗示“非 Frozen 不能上提出条件”。建议：(a) `VariantField` 的操作数必须是固定在控制上的 `Refine{variant}`，verifier 检查；L 级把 `Refine{variant=v}` 降成对应 tag 事实（`nonnull`、tag 范围），控制保持不变；(b) `Load`/`Store` 一律带 `ctl`，只有地址在整个区域内 dereferenceable（`Frozen`/`Unique`）时才允许解除固定。改 nodes.md、values-and-types.md、memory-partitions.md。
2. **（缓存健全性）非泛型函数的布局查询不在键里。** `doctestable` 没有孔位，答案键只有 R 图哈希；但 L 级要问 `VariantEncoding(CrateType)`，它取决于 std 中 `String`/`RawVecInner` 的私有字段（本例的 niche 来自 `UsizeNoHighBit`）。上游只改私有布局时 R 图不变，机器码缓存会命中旧编码。建议：L 级对具体类型的布局查询也记入孔位表（以具体 `TyId` 为键），所有函数的机器码键都含布局答案。改 [generics-specialization.md](../../design/ir/generics-specialization.md#孔位)、[cache.md](../../design/build/cache.md#存什么)。
3. **`Discriminant` 与 `VariantOf` 的规范化没有写。** derive（`PartialEq`/`PartialOrd`/`Hash`）一律用 `discriminant_value`，即 `Discriminant`；`VariantSwitch`/`Refine` 讲的是 `VariantOf`。不打通就不能用分支事实折叠 derive 的比较。建议：`Discriminant(x)` 规范化为 `DiscrOf{ty}(VariantOf(x))`（判别值表来自 typeck/常量求值，不是布局，R 级可用）；判别值互不相同，所以 `DiscrOf(a) == DiscrOf(b)` → `VariantOf(a) == VariantOf(b)`，`DiscrOf(x) == c` → `VariantOf(x) == v_c`（无对应变体则为 false）。改 nodes.md、[optimization-pipeline.md](../../design/ir/optimization-pipeline.md#r-级)。
4. **内存中 enum 的变体读取缺形式。** MIR 的 `discriminant((*_2))` 只读 tag；现节点表只能写成整值 `Load : CrateType` 再 `VariantOf`，L 级会按 `Memory` 表示把 24 字节放进 value slot。建议二选一并写明：增加 `VariantOfAt[P](addr)`（L 级 = tag load + 解码），或规定“整值 `Load` 的使用全是投影时按投影窄化”（窄化后的 load 共用原效果输入）。前者还避免整值读对未读字段断言有效性。改 nodes.md、values-and-types.md。
5. **提升常量上的 `Load` 折叠。** `*v == CrateType::Rlib` 的右边是 `&ConstAlloc`；R 级规则表里只有 `Field(Aggregate)` 等，没有“`Load` 来自只读 `ConstAlloc` → 聚合常量”。没有它，rustc 的 poison 分支问题会原样出现。改 optimization-pipeline.md（R 级规则表）。
6. **变体的控制流规则不够。** 需要两条：`VariantSwitch(Phi(MakeVariant…))` 按前驱线程化（复制 Region，有节点预算）；同一 `VariantOf` 上的常量比较链合并成一个 `VariantSwitch`。本例 `?`、`get`、`if let` 和 `||` 链都依赖它们。改 optimization-pipeline.md（R 级）。
7. **niche `VariantSwitch` 的 L 级降低要按 tag 值分支。** 先解码成变体号再比较正是 rustc 留给 InstCombine 的工作。建议写明：niche 变体的 case 直接比较 `niche_start + (v - start)`，untagged 变体用 tag 有效范围判定，多 case 交给 L 级 switch 降低（比较树、范围、位测试）；后者列入 [runtime.md](../../design/goals/runtime.md#必须自己做的部分)。改 nodes.md（`VariantSwitch` 行）、runtime.md。

# W09 大值：sret、`Box::new` 与大 enum

## 1. 程序

```rust
pub struct Big { pub a: [u64; 16], pub n: u32 }                 // 136 字节，Memory
pub enum Msg { Data([u8; 256]), Ping(u32), Quit }
#[inline(never)] pub fn ret_big(x: u64) -> Big { let mut b = make_big(x); b.n += 1; b }
#[inline(never)] pub fn box_call(x: u64) -> Box<Big> { Box::new(make_big(x)) }
#[inline(never)] pub fn box_lit(x: u64) -> Box<Big> { Box::new(Big { a: [x; 16], n: 1 }) }
#[inline(never)] pub fn box_zero() -> Box<[u8; 4096]> { Box::new([0u8; 4096]) }
#[inline(never)] pub fn handle(m: Msg) -> u32 { match m { Msg::Data(d) => d[0] as u32 + d[255] as u32, Msg::Ping(p) => p, Msg::Quit => 0 } }
#[inline(never)] pub fn route(x: u8) -> u32 { let m = if x > 9 { Msg::Data([x; 256]) } else if … ; let m2 = m; handle(m2) }
#[inline(never)] pub fn pipe<const N: usize>(x: u8) -> u8 { let a = make::<N>(x); take(a) ^ take(a) ^ take_opt(Some(make::<N>(x))) }
#[inline(never)] pub fn boxed<const N: usize>(x: u8) -> Box<[u8; N]> { Box::new(make::<N>(x)) }
```

完整源码 [scripts/w09.rs](scripts/w09.rs)，测量脚本 [scripts/w09.sh](scripts/w09.sh)（用 v0 符号，便于从符号读出 N）。阈值扫描取 N ∈ {4 8 16 24 32 64 128 256 4096}（下表节选），`take`/`take_opt` 对整个数组做 xor。

## 2. rustc 现在怎么做（实测）

**命名函数。** 每行从未优化 IR 的 alloca/memcpy 数开始，列出改变计数的 LLVM pass：

```text
7 inst  alloca=0   memcpy=0   lifetime=0   assume=0   call=1   invoke=0   landingpad=0  load=1    store=1    ret_big
  route: a2m1 >MemCpyOptPass> a2m0 >InstCombinePass> a1m0
  handle: a2m1 >SROAPass> a1m1 >InstCombinePass> a0m0
  box_lit: a2m1 >AlwaysInlinerPass> a2m2 >SROAPass> a0m0
  box_call: a1m0 >AlwaysInlinerPass> a1m1
  box_zero: a1m0 >AlwaysInlinerPass> a1m1 >MemCpyOptPass> a1m0 >InstCombinePass> a0m0
  8box_zero:    1 ___rust_alloc_zeroed
  8box_call:    1 @llvm.memcpy    1 ___rust_alloc    1 make_big
  7ret_big: inst=6 memcpy=0 memset=0 rep=0 vmov=0 frame=0
  8box_call: inst=23 memcpy=1 memset=0 rep=0 vmov=0 frame=144
  5route: inst=38 memcpy=0 memset=0 rep=0 vmov=0 frame=264
```

- `ret_big`：MIR 已经把 `make_big` 的结果直接写进 `_0`，字段更新是原地的 `(_0.1: u32) = Add(copy (_0.1: u32), const 1_u32)`；未优化 IR 就没有 memcpy。
- `Box::new` 在 MIR 里没被内联（`#[inline(always)]`，由 LLVM 的 AlwaysInliner 内联），所以它的 memcpy 在内联后才出现。`box_new` intrinsic 本身是先分配、再把表达式直接求值进 `*box`（`rustc_mir_build/src/builder/expr/as_rvalue.rs` 的 `ExprKind::Box`），但用户调用的 `Box::new(x)` 是普通函数，实参先求值，所以没有原地构造。
- `box_lit` 的聚合在 SROA 后直接写进堆；`box_call` 的 136 字节拷贝到最后都在（x86 `call memcpy`）：`make_big` 在分配之前执行，堆地址此时还不存在。
- `box_zero`：MemCpyOpt + InstCombine 把“分配 + 4096 字节零”变成 `__rust_alloc_zeroed`。`handle`：MIR 中 `_3 = copy ((_1 as Data).0: [u8; 256])` 为了读两个字节拷贝 256 字节；SROA + InstCombine 删掉。

**大小阈值。** `pipe.pre`/`boxed.pre` 含被调的 `Box::new<[u8; N]>`：

```text
N     | pipe.pre   pipe.o3    | boxed.pre  boxed.o3   | x86 pipe                                           | x86 boxed
4     | a9 m7      a0 m0      | a6 m4      a0 m0      | inst=6 memcpy=0 memset=0 rep=0 vmov=0 frame=0      | inst=22 memcpy=0 memset=0 rep=0 vmov=0 frame=0
8     | a8 m6      a0 m0      | a6 m4      a0 m0      | inst=6 memcpy=0 memset=0 rep=0 vmov=0 frame=0      | inst=22 memcpy=0 memset=0 rep=0 vmov=0 frame=0
16    | a4 m2      a2 m0      | a2 m1      a1 m1      | inst=28 memcpy=0 memset=0 rep=0 vmov=0 frame=48    | inst=17 memcpy=0 memset=0 rep=0 vmov=2 frame=24
128   | a4 m2      a2 m0      | a2 m1      a1 m1      | inst=28 memcpy=0 memset=0 rep=0 vmov=0 frame=272   | inst=31 memcpy=0 memset=0 rep=0 vmov=16 frame=136
256   | a4 m2      a2 m0      | a2 m1      a1 m1      | inst=28 memcpy=0 memset=0 rep=0 vmov=0 frame=528   | inst=23 memcpy=1 memset=0 rep=0 vmov=0 frame=256
4096  | a4 m2      a2 m0      | a2 m1      a1 m1      | inst=32 memcpy=0 memset=0 rep=0 vmov=0 frame=8208  | inst=23 memcpy=1 memset=0 rep=0 vmov=0 frame=4096
  N=4: i32 make(i8 noundef %x)
  N=8: noundef i8 take_opt(ptr dead_on_return noalias noundef align 1 captures(address) dereferenceable(9) %o)
  N=16: void make(ptr dead_on_unwind noalias noundef writable sret([16 x i8]) align 1 captures(none) dereferenceable(16) %_0, i8 noundef %x)
  N=16: noundef i8 take(ptr dead_on_return noalias noundef readonly align 1 captures(address) dereferenceable(16) %a)
== O3 IR: calls in pipe<256> (callee, pointer argument) and the Some payload address
  call fastcc void make(%a, i8 noundef %x)
  %_4 = call fastcc noundef i8 take(%a)
  %_5 = call fastcc noundef i8 take(%a)
  %0 = getelementptr inbounds nuw i8, ptr %_7, i64 1
  call fastcc void make(%0, i8 noundef %x)
== O3 IR: allocas and lifetime markers in pipe<256> (input to stack coloring)
  %_7 = alloca [257 x i8]
  %a = alloca [256 x i8]
  %_7 lifetime.start
  %_7 lifetime.end
```

| 问题 | 答案（rustc 1.94.1 + 1.99.1 x86） |
|---|---|
| 什么时候有栈上临时值 | 未优化 IR 中每个 `Memory` 值都有 alloca，与大小无关；≤8 字节（`Cast` 成整数传递）反而更多（N=4：9 个 alloca、7 次 memcpy），全是 ABI 转换 |
| PassMode 阈值 | `Memory` ≤8 字节 → `Cast` 整数；>8 字节 → 参数 `Indirect`、返回 sret（N=16 已是 sret；N=8 的 `Option<[u8;8]>` 9 字节已是 `Indirect`）。与 `adjust_for_rust_abi` 一致（`rustc_target/src/callconv/mod.rs`） |
| O3 后还剩什么 | 必须在内存里的：sret 结果槽、`Indirect` 实参槽。`take(copy a)` 的拷贝被 MemCpyOpt 删掉（被调者参数推断为 `readonly`，`rustc_mir_transform/src/deduce_param_attrs.rs`）；`Some(make())` 让 `make` 直接写 `%_7 + 1` |
| memcpy 留到机器码 | 只有“值先于目标地址产生”的 `Box::new(make())`：≤128 字节展开成 SSE 移动（vmov 2…16），≥256 字节调用 `memcpy` |
| 槽位合并 | `pipe<256>` 的 `a` 与 `Some` 槽活跃区间不重叠，但 frame 528 = 两槽之和：O3 IR 中 `%a` 没有 lifetime 标记，栈着色无从合并（推断是 MemCpyOpt 删拷贝时去掉的） |

## 3. kfrc 编译计划

**Body。** `Box::new` 是具体实参的泛型 `#[inline(always)]` 函数，按 W02 细化 2 内联；内联后是 `box_new` 的 Body 形状：`exchange_malloc(size, align)` → `ShallowInitBox` → `*b = move x`。`[x; 256]`、`[0u8; 4096]` 是 `Rvalue::Repeat`。

**R/S 图（节选）。**

```text
// route
n4  = Repeat{256}(p0) : [u8; 256]                      // 现节点表没有，见 E5
n5  = MakeVariant{Msg, Data}(n4) ; n8 = MakeVariant{Msg, Ping}(Cast.zext(p0)) ; n9 = Const Msg::Quit
n11 = Phi(n10) (n5, n8, n9) : Msg                       // let m2 = m 不产生节点
n12 = Call(handle) ctl=n10 mem=[Unknown:n3] (n11) : (Control, Control?, Effect[Unknown], u32)
// handle
n2  = VariantSwitch ctl=n0 (p0) : (Control, Control, Control)
n3  = VariantField[Data, 0](p0) : [u8; 256] ; n4 = Field[0](n3) : u8 ; n5 = Field[255](n3) : u8
// ret_big
n2  = Call(make_big) ctl=n0 mem=[Unknown:n1] (p0) : (Control, Effect[Unknown], Big)
n5  = InsertField[1](n2.2, Add.wrap(Field[1](n2.2), c1)) : Big ; n6 = Return ctl=n2.0 (n5)
// box_lit / box_call
n4  = Aggregate{Big}(Repeat{16}(p0), c1)               // box_call：n4 = Call(make_big)，排在 n5 之前
n5  = Call(exchange_malloc) ctl=n2 mem=[Unknown:n1] (SizeOf(Big), AlignOf(Big)) : (Control, Effect[Unknown], *mut u8)
n6  = Store[Unknown](n5.2, n4) mem=n5.1
```

**L 级放置。**

| 值 | 表示 | 放置 | 期望代码 | rustc O3 |
|---|---|---|---|---|
| `route` 的 `n11: Msg`（260 B） | slot | 规则 2：三个 `MakeVariant` 直接构造进 Phi 槽；`n4` 构造进 `slot + Data 偏移`（E3）；末次使用作 `Indirect` 实参 → 直接传槽 | 一次 256 字节填充 + tag | 同（frame 264） |
| `handle` 的 `n3` | 调用者给的 `Indirect` 指针 + 偏移 | `Field(VariantField(p))` 降为两次字节 load，没有拷贝 | 约 12 条 | 12 条 |
| `ret_big` 的 `n5` | slot | 规则 3 → sret；`n2.2` 在 `n5` 处死亡 → 复用其槽（E7），`make_big` 直接写 `%0` | `call; add [rbx+128], 1` | 6 条 |
| `box_lit` 的 `n4` | slot | 生产者是纯值，可下沉到 `n5` 之后 → 规则 3 构造进堆 | 17 次 store | 30 条，0 memcpy |
| `box_call` 的 `n4` | slot | 生产者是调用，先于分配 → 一次 `CopyMem 136`（E4） | 同 rustc | `call memcpy` |
| `box_zero` | — | 分配 + 整块零 → `__rust_alloc_zeroed`（E6） | 同 rustc | 12 条 |
| `pipe<N>` 的 `a`（N ≤ 8） | `Int{8N}` | 寄存器，不进槽（E1） | 无内存 | 0 alloca |
| `pipe<N>` 的 `a`（N > 8） | slot | 两次 `Indirect` 实参：第一次不是末次使用，`take` 摘要“不写该参数”才能免拷贝（E2） | 0 拷贝 | 0 拷贝（MemCpyOpt） |
| `pipe<N>` 的 `Some(make())` | slot | `make` 的 sret 指向 `slot + 1`（E3）；与 `a` 的槽按 SSA 活跃区间共用（规则 4） | frame ≈ N+1 | frame 2N+16（528） |

## 4. 代价核算

| 项 | rustc | kfrc（推断） |
|---|---|---|
| 未优化中间表示里的大值拷贝 | 每个 `Memory` move 一次 memcpy，靠 SROA/MemCpyOpt/InstCombine/AlwaysInliner 清理 | 0：move 是值转发，拷贝只在 L 级放置失败处生成 |
| 剩余拷贝 | `Box::new(call)`：1 次 | 同（E4 的前移首版不做） |
| 栈 | `pipe<256>` 528、`pipe<4096>` 8208 | 约 272 / 4112：槽合并不依赖 lifetime 标记 |
| 缓存 | — | 调用者机器码依赖被调者摘要（`take` 不写参数、`make_big` 纯），见 E8 |

## 5. 设计细化

| # | 问题 | 证据 | 建议改法 | 文件 |
|---|---|---|---|---|
| E1 | L 级“`Memory` 聚合变成 value slot”与 Rust ABI 不符：≤8 字节的 `Memory` 以整数传递；>16 字节的 `ScalarPair` 返回时走 sret | N=4 签名 `i32 make(…)`，pre IR 为此 9 个 alloca；`adjust_for_rust_abi` 的 `size > 2×指针 → make_indirect` 与 `size <= 指针 → cast_to(Integer)` | L 级类型映射按 `(backend_repr, PassMode)`：`Memory` ≤8 B → `Int{8·size}` 值；`Memory` >8 B → slot；`ScalarPair` 拆值，只在 >16 B 返回的调用边界上用 sret 槽 | levels.md（L 级值类型）、values-and-types.md |
| E2 | `Indirect` 参数归被调者所有、可被就地修改。值作为 `Indirect` 实参而后面还有使用时，要先拷贝；能否省掉取决于被调者写不写这个参数，而 `CallSummary.memory` 只描述指针实参 | MIR `take::<N>(copy _2)`；pre IR `take` 参数 `readonly`；O3 两次 `take(%a)` 共用一槽 | `CallSummary` 加 `writes_indirect_args: SmallBitSet`；L 级只在“不是末次使用且被调者会写”时拷贝 | optimization-pipeline.md（调用摘要）、ownership-drop-unwind.md（大值规则 3） |
| E3 | 规则 3 只写了“唯一使用是 Store/sret/Indirect”，没写嵌套：值唯一的使用是作为某个聚合或变体的字段，而那个聚合有槽 | O3 `make(%0)`，`%0 = %_7 + 1`；`route` 经 MemCpyOpt 后无 memcpy | 规则 3 递归：目标是“父值的槽 + `FieldOffsets`/`VariantEncoding` 给出的偏移”；Phi 槽同样适用 | ownership-drop-unwind.md（大值与原地构造） |
| E4 | 规则 3 的目标地址必须在值产生前就存在。`Box::new(call())` 中调用先于分配，规则 3 静默失效 | `box_call` O3 仍有 `llvm.memcpy`，x86 `call memcpy`；`ExprKind::Box` 只在 `box_new` 直接包住表达式时原地构造 | 写明前提：目标地址支配值的生产者。纯值生产者在 SoN 里本来就能下沉（`box_lit`）。调用生产者首版保留一次 `CopyMem`；把 `exchange_malloc` 前移到调用之前需语言层确认分配可重排，并要求被调者摘要无内存效果（sret 除外）、不 unwind、不发散 | ownership-drop-unwind.md |
| E5 | 节点表没有 `Repeat`：`[x; 256]` 只能展开成 256 个操作数的 `Aggregate`；L 级也没有 `CopyMem`/填充的降低策略 | MIR `_4 = [copy _1; 256]`、`_1 = [const 0_u8; 4096]`；1.99.1 的拷贝 ≤128 B 展开为 SSE 移动，≥256 B 调用 `memcpy` | 加 `Repeat{count}(v)`（RS），常量全零是 `Const`；L 级表加“`CopyMem`/`Repeat` 降低：≤阈值展开，否则调用 memcpy/memset，阈值进 `TargetSpec`（默认 128 B）” | nodes.md、optimization-pipeline.md（L 级） |
| E6 | 没有“分配后整块置零 → `alloc_zeroed`”的规则，`Box::new([0; N])`、`vec![0; n]` 会多一次 memset，且拿不到 calloc 的零页 | `box_zero` O3 只调用 `___rust_alloc_zeroed` | S 级规则：`exchange_malloc` 结果紧接覆盖整块的全零 `Store` → 换成 `__rust_alloc_zeroed`（`GlobalAlloc::alloc_zeroed` 的契约） | optimization-pipeline.md（S 级） |
| E7 | `InsertField` 产生新聚合值；L 级若给它新槽，`b.n += 1` 就是 136 字节拷贝 | MIR 的原地更新 `(_0.1: u32) = Add(…)`；rustc `ret_big` 6 条 | 槽分配加一条：`InsertField`/变体字段更新的结果在操作数末次使用处复用操作数的槽（两地址式） | ownership-drop-unwind.md（大值规则） |
| E8 | 调用者的放置依赖未内联被调者的摘要（E2 的写参数位、E4 的纯度、`may_unwind`），但机器码键只含答案键和被内联者的键 | E2/E4 | 机器码键加入“读过的被调者摘要的哈希”；摘要不变时 early cutoff | cache.md（存什么） |
| E9 | 数组值的动态下标没有节点：`d[i]` 里 `d` 未被取地址时按规则是 SSA 值，但只有常量下标的 `Field[i]` | 设计阅读（本例 `handle` 只用常量下标，MIR 为 `copy _3[0 of 1]`，未触发） | Body → R 构建时，带非常量 `Index` 投影的局部按“地址被取走”处理（`StackSlot` + `Local` 分区），不加值级动态下标节点 | values-and-types.md（什么在内存里） |

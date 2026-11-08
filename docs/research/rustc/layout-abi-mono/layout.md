# 类型布局：数据结构与 struct/union 算法

enum 与 niche 见 [enum-niche.md](enum-niche.md)。链接固定到 `e408947b`。

## 两层分工

- **`rustc_abi`（纯算法）：** `LayoutCalculator<Cx: HasDataLayout>` 只依赖目标数据布局和字段的 `LayoutData`，不碰 `TyCtxt`。源码注释写明 `LayoutData`/`FieldsShape` 对 `FieldIdx`/`VariantIdx` 泛型是为了 rust-analyzer 复用（[lib.rs#L2013](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L2013)、[layout.rs#L114](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L114)）。
- **`rustc_ty_utils::layout`（类型分派）：** `layout_of` 先规范化类型、擦除 region，再 `layout_of_uncached` 按 `ty.kind()` 分派，最后跑 `layout_sanity_check`（[layout.rs#L37](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L37)、[#L178](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L178)、[invariant.rs#L8](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout/invariant.rs#L8)）。

**对 kfrc：** 算法层可以几乎原样移植成 `LayoutId -> LayoutData`；分派层要的是自有类型系统的查询：规范化、`is_sized`、ADT 定义/repr/判别值、数组长度（常量求值）、`Pointee::Metadata`。

## `LayoutData` 字段（[lib.rs#L2013](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L2013)）

| 字段 | 含义 |
|---|---|
| `fields: FieldsShape` | `Primitive` / `Union(n)` / `Array{stride,count}` / `Arbitrary{offsets, in_memory_order}`（[#L1615](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1615)）。`offsets` 按源码顺序，`in_memory_order` 是内存序→源码序的置换 |
| `variants` | `Empty`（无合法值）/ `Single{index}` / `Multiple{tag, tag_encoding, tag_field, variants}`（[#L1858](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1858)） |
| `backend_repr` | `Scalar` / `ScalarPair` / `SimdVector` / `ScalableVector` / `Memory{sized}`（[#L1719](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1719)）；与 `fields` 正交，例如 `ManuallyDrop<Result<isize,isize>>` 是 `ScalarPair` |
| `largest_niche: Option<Niche>` | 无效值最多的叶子标量（offset、primitive、valid_range） |
| `uninhabited` | 独立于 `backend_repr`，影响 ABI 和判别值读取 |
| `align`, `size` | `size` 已按 `align` 向上取整 |
| `max_repr_align`, `unadjusted_abi_align` | 只给 i686-windows / aarch64 ABI 用；x86_64 SysV 不读 |
| `randomization_seed` | 只服务 `-Zrandomize-layout` |

- **`Scalar`**（[#L1511](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1511)）：`Initialized{value: Primitive, valid_range}` 或 `Union{value}`（union/可能未初始化，没有 niche）。`Primitive = Int(Integer, signed) | Float | Pointer(AddressSpace)`。
- **`WrappingRange{start,end}`**（[#L1399](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1399)）：闭区间，可回绕（`start > end` 表示跨过 0），值按无符号位模式解释。
- **谓词：** `is_aggregate()` 对 `ScalarPair` 也返回 true（[#L2072](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L2072)）；`is_zst()` 只对 `Memory{sized:true}` 且 size 0 成立（[#L2187](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L2187)）；`is_1zst` 额外要求 align 1。

## 基本类型（x86_64 Linux）

`layout_of_uncached`（[#L383](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L383) 起）：

- `bool` = `Int(I8,false)`，valid `0..=1`；`char` = `Int(I32,false)`，valid `0..=0x10FFFF`（注意不排除代理区，代理区在常量有效性检查里另判）。
- 整数/浮点 = 满范围 Scalar。数据布局串 `...-i64:64-i128:128-f80:128-...`，即 i128/u128 对齐 16（[x86_64_unknown_linux_gnu.rs#L33](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/spec/targets/x86_64_unknown_linux_gnu.rs#L33)）。`aggregate_align` 默认 1 字节（[lib.rs#L323](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L323)）。
- `fn` 指针：Pointer，valid 从 1 开始（非空）。`FnDef`、闭包无捕获时为 ZST。
- `&T`/`&mut T`/`Box` 内部引用：Pointer，valid 从 1；`*const T` 满范围（[#L410](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L410)）。
- 宽指针：`ScalarPair(data_ptr, meta)`。meta 由 `<T as Pointee>::Metadata` 规范化得到：slice/str = `usize`（满范围），`dyn` = 非空 vtable 指针。metadata 为 1-ZST 时退化为瘦指针。顺序（数据指针在前）由 `scalar_pair` 固定（[simple.rs#L91](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout/simple.rs#L91)）。
- `[T; N]`/`[T]`/`str`：`array_like`，`FieldsShape::Array{stride: elem.size}`，`backend_repr = Memory`（数组永远不是 Scalar/ScalarPair），niche 只在 N>0 时继承（[layout.rs#L123](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L123)）。N 来自常量求值。
- `!` 与无变体 enum：`never_type()`，`Variants::Empty`、size 0、uninhabited（[simple.rs#L32](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout/simple.rs#L32)）。
- 泛型参数 `T` 直接报 `TooGeneric`（[#L762](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L762)）——布局只对单态类型有定义。

## `univariant`：struct、tuple、闭包、enum 变体共用（[layout.rs#L202](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L202)）

1. **字段排序**（`univariant_biased`，[#L1097](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L1097)）：`repr.inhibit_struct_field_reordering()` 为假且字段数 > 1 时排序；可能 unsized 的尾字段不参与。
   - 普通 struct 排序键 `(Reverse(对齐组), niche 键, 字段内 niche 偏移)`（[#L1204](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L1204)）：大对齐在前；对齐组取 `log2(max(align, size))`，所以 `[u8;4]` 和 align-4 字段同组；有 niche 时对齐组会封顶到最大字段对齐。
   - enum 变体（`StructKind::Prefixed`）**升序**排列，同组内 niche 大的放后面（[#L1234](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L1234)），让变体从 tag 后紧凑开始。
   - packed 时对齐组键是 `min(align, pack)`。
2. **两次尝试：** 先 `NicheBias::Start`，若 niche 不在头部且有尾部空间，再算 `NicheBias::End`，niche 离边缘更近者胜（[#L202](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L202) 函数体）。`MaybeUnsized` 不做第二次，保证 `Foo<T>` 与 `Foo<dyn Trait>` 字段偏移一致。
3. **放置：** 按 `in_memory_order` 依次对齐放置；`Prefixed(size, align)` 先跳过 tag 前缀（[#L1260](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L1260)）；pack 把字段对齐截到 pack；`repr(align)` 最后抬高整体对齐；`size = align_to(min_size, align)`；超过 `obj_size_bound`（x86_64 为 2^61）报 `SizeOverflow`。
4. **largest_niche：** 逐字段取更大的 niche（Start 偏向先出现者，End 偏向后出现者）。
5. **backend_repr 提升**（[#L1332](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L1332) 起），只看非 ZST 字段：
   - 恰一个非 ZST 字段，offset 0、size/align 与整体相同：`Scalar`/`SimdVector` 继承——但 `repr(C)`/`repr(simd)` 禁止（`ABI_UNOPTIMIZABLE`）；`ScalarPair` 无论 repr 都继承。
   - 恰两个非 ZST 字段且都是 `Scalar`：按内存序构造 `scalar_pair(a,b)`，只有偏移、size、align 完全一致才用 `ScalarPair`（`repr(C)` 也可以）。
   - 其余 `Memory`。
6. `uninhabited` = 任一字段 uninhabited；`repr(transparent)` 只改 `unadjusted_abi_align`（[#L1410](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L1410)）。

## repr 标志（[lib.rs#L84](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L84)、[#L159](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L159)）

| repr | 字段重排 | newtype Scalar 提升 | enum 布局优化（niche） | 其他 |
|---|---|---|---|---|
| Rust（默认） | 是 | 是 | 是 | — |
| `C` | 否 | 否（ScalarPair 仍可） | 否 | union 不做 ABI 合并；enum tag 至少 `c_int` |
| `u8`…`isize` | 否 | 是 | 否 | 决定 tag 类型 |
| `transparent` | 是（但只有一个非 1-ZST 字段，结果相同） | 是 | — | ABI 依赖布局本身等价 |
| `packed(n)` | 是（键变） | 视对齐结果 | enum 上报 `ReprConflict` | 字段对齐截到 n |
| `align(n)` | 是 | 对齐变化后通常落到 `Memory` | — | 只抬高整体对齐 |

依据：`inhibit_struct_field_reordering`、`inhibit_newtype_abi_optimization`、`inhibit_enum_layout_opt`、`inhibits_union_abi_opt`（[#L221](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L221)–[#L242](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L242)）。

## union（[layout.rs#L333](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L333)）

所有字段 offset 0；size = 最大字段 size 按对齐取整；`largest_niche = None`。非 `repr(C)` 且所有非 ZST 字段的 `backend_repr.to_union()` 相同（且对齐匹配）时继承该 repr，否则 `Memory`。union 不能同时 packed 和 aligned。

## ADT 入口（[ty_utils layout.rs#L645](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L645)）

- 先算每个变体每个字段的布局；union 走 `layout_of_union`；struct/enum 走 `layout_of_struct_or_enum`（[layout.rs#L277](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L277)）。
- `UnsafeCell`/`UnsafePinned` 置 `is_special_no_niche`：清除所有 valid_range 和 `largest_niche`（[#L671](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L671)、[layout.rs#L459](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L459)）。
- `#[rustc_layout_scalar_valid_range_start/end]`（`tcx.layout_scalar_valid_range`）只用于 Scalar/ScalarPair 首元素，收窄 valid_range 并可能产生更大的 niche（同上函数）。core 的 `NonNull`/`NonZero` 用它还是 pattern type（`ty::Pat`，[#L214](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L214)）需对照冻结的 sysroot 源码确认（推断：两条路都要支持）。
- 尾字段可能 unsized 的 struct 用 `MaybeUnsized`；debug 构建会验证“尾部 sized/unsized 两种布局字段偏移一致”（[#L683](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L683)）。

## tricky 边界

- 字段重排 ≠ 按大小排序：键是对齐组，`struct S{a:u8,b:u32,c:u16}` 内存序为 u32,u16,u8；niche 会把 `bool` 拉到组首或组尾。
- **tuple 和尾字段可能 unsized 的泛型 struct 的最后一个字段不参与重排**：非空 tuple 一律按 `MaybeUnsized` 算（[#L563](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout.rs#L563)），所以 `(u8,u32,u16)` 是 u32@0、u8@4、u16@6；`struct W<T:?Sized>{a:u8,t:T}` 即使 `T=u32` 也保持 `t` 在最后。
- 只有“非 ZST 字段数”决定 Scalar/ScalarPair，ZST 带大对齐时靠 size/align 一致性检查拦下。
- `repr(C) struct F(f64)` 是 `Memory`，`struct F(f64)`（Rust repr）是 `Scalar(f64)`——这直接改变 Rust ABI 下传参寄存器类别（见 [fn-abi.md](fn-abi.md)）。
- 数组、`[T;1]` 都是 `Memory`，即使元素是标量。

## 对 kfrc 数据模型的含义

- `LayoutId` 驻留 `LayoutData` 的完整内容（含 `variants` 递归、`largest_niche`），`Place` 投影查 `fields.offset(i)`，enum 先“下转”到变体布局（`for_variant`）再投影。
- 现在全是 i64 的 SoN 需要的最小类型信息就是 `Primitive`（宽度/符号/浮点/指针）+ `valid_range`；`Memory` 类型只以地址形式出现在 SoN。
- 位级一致的范围与理由见 [README.md](README.md#布局一致性范围)。

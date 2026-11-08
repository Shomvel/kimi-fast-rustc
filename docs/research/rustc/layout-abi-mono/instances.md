# Instance、InstanceKind 与解析

单态化遍历见 [mono-collector.md](mono-collector.md)。链接前缀固定到 `e408947b`。

## 数据结构

`Instance{def: InstanceKind, args: GenericArgsRef}`（[instance.rs#L35](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L35)）：`args` 是 region 已擦除的泛型实参；codegen/CTFE 运行时再用 `args` 实例化 MIR。`MonoItem = Fn(Instance) | Static(DefId) | GlobalAsm(ItemId)`（[mono.rs#L54](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/mono.rs#L54)）——static 不带泛型参数。

## `InstanceKind`（[instance.rs#L64](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L64)）

| 种类 | 是什么 | 有自己的函数体 |
|---|---|---|
| `Item(DefId)` | fn、方法、闭包、协程、tuple struct/变体构造器 | 是 |
| `Intrinsic(DefId)` | `#[rustc_intrinsic]` | 否，调用点就地展开；不是 `must_be_overridden` 的 intrinsic 另收集其 fallback body 的 `Item` 实例（[collector.rs#L1005](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L1005)） |
| `VTableShim(DefId)` | `self: Self` 方法进 vtable，shim 以 `*mut Self` 接收 | 是（生成） |
| `ReifyShim(DefId, reason)` | 不能直接取地址的函数做成 fn 指针/vtable 项：`#[track_caller]`、`Virtual` | 是 |
| `FnPtrShim(call_*, fn_ty)` | `fn` 指针类型实现 `Fn*` trait | 是 |
| `Virtual(DefId, vtable_idx)` | 经 vtable 的动态调用 | 否，调用点按下标取函数指针 |
| `ClosureOnceShim{call_once, track_caller}` | `Fn`/`FnMut` 闭包当 `FnOnce` 调用：借用 self 再调 `call_mut` | 是 |
| `ConstructCoroutineInClosureShim{..}` | async 闭包 | 是 |
| `ThreadLocalShim(DefId)` | 平台不支持原生 TLS 导出时的访问器 | 是 |
| `DropGlue(drop_in_place, Option<Ty>)` | `ptr::drop_in_place::<T>`；`None` = 空 drop glue | 是（`None` 时为空函数） |
| `CloneShim(clone, Ty)` | 内建 `Clone`：`Copy` 类型、数组、tuple、闭包 | 是 |
| `FnPtrAddrShim(addr, Ty)` | `<fn(..) as FnPtr>::addr` | 是 |
| `FutureDropPollShim` / `AsyncDropGlueCtorShim` / `AsyncDropGlue` | async drop | 是 |

`has_polymorphic_mir_body`、`requires_inline`（[#L303](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L303)）决定 shim 的 MIR 是否用 `args` 实例化、是否每个 CGU 本地生成。

## 解析 `Instance::try_resolve`（[#L510](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L510)）→ `resolve_instance_raw`（[ty_utils instance.rs#L18](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/instance.rs#L18)）

- **非 trait 关联项**：intrinsic → `Intrinsic`；`drop_in_place::<T>` → `T: needs_drop` 时 `DropGlue(_, Some(T))`（仅 ADT/闭包/tuple/dyn/数组/slice 等），否则 `DropGlue(_, None)`；其余 → `Item(def_id)`。
- **trait 方法**（`resolve_associated_item`，[#L103](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/instance.rs#L103)）：`codegen_select_candidate(trait_ref)`，歧义/未实现返回 `Ok(None)`（调用方 `expect_resolve` 在 type length limit 外报 bug）：
  - `ImplSource::UserDefined`：用 specialization graph 找 `leaf_def`；不是 final 的项只在 `PostAnalysis` 且 trait_ref 不可再特化时解析；`translate_args` 把 trait args 映射到 impl args；结果 `Item(leaf_def)`。
  - `Builtin(Object)`：`Virtual(trait_item, vtable_base + 方法序号)`（[#L238](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/instance.rs#L238)）。
  - `Builtin(Misc|Trivial)`：`Clone` → `CloneShim`；`FnPtr::addr` → `FnPtrAddrShim`；`Fn*` on `fn` 指针 → `FnPtrShim`；闭包 `FnOnce::call_once` 且闭包是 `Fn/FnMut` → `ClosureOnceShim`；其余闭包 → `Item(closure)`（[#L256](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/instance.rs#L256)）。
  - `Param`、trait upcasting → `None`。
- **取 fn 指针** `resolve_for_fn_ptr`（[#L594](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L594)）：`Item` 且 `track_caller` → `ReifyShim(def, FnPtr)`；`Virtual` → `ReifyShim`。
- **进 vtable** `expect_resolve_for_vtable`（[#L639](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L639)）：`self: Self`（可 unsize 的 by-value self）→ `VTableShim`；impl 方法有 `track_caller` 而 trait 声明没有（`should_inherit_track_caller` 为假）→ `ReifyShim(_, Vtable)`；`Virtual` → `ReifyShim`。
- **闭包**：`resolve_closure(def, args, requested_kind)`，需要适配时走 `fn_once_adapter_instance`（[#L717](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L717)、[#L765](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L765)）。
- **drop**：`resolve_drop_in_place(ty)`（[#L731](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L731)）。

## vtable

- 布局：`[drop_in_place 或 null, size, align, 方法…, 超 trait vptr…]`。前三项固定（`COMMON_VTABLE_ENTRIES`，下标 0/1/2，[vtable.rs#L45](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/vtable.rs#L45)）；`!needs_drop` 时 drop 项是空指针。
- 条目由 `vtable_entries`（[trait_selection vtable.rs#L227](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/vtable.rs#L227)）按超 trait 段展开；不可派发方法（如 `where Self: Sized`）不进条目表，谓词不可能成立的方法是 `Vacant`：槽位保留但不写入（[vtable.rs#L84](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/vtable.rs#L84) 中 `continue`）。
- vtable 本身是一个不可变 `Allocation`，函数项是指向 `GlobalAlloc::Function{instance}` 的带 provenance 指针，codegen 时变成重定位；同一 `(Ty, principal)` 查询结果相同（`vtable_allocation_provider`，[#L84](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/vtable.rs#L84)）。

## tricky 边界

- 同一个 `DefId` 可以对应多个不同的 `InstanceKind`（`Item` 与 `ReifyShim`、`ClosureOnceShim`），符号必须区分；kfrc 的 `InstanceKey` 必须包含 kind，不只是 `(DefId, args)`。
- `DropGlue(_, None)` 只有被取地址（进 vtable、fn 指针）时才生成实体；直接调用时什么也不做。
- 特化相关的“延迟解析”：泛型上下文中解析返回 `None`，单态化（`fully_monomorphized`）后才确定；kfrc 的 trait 求解要提供同样的“现在不能决定”结果，而不是挑一个 impl。
- 非 `is_known_rigid` 的 `Self` 若 impl 没有显式 `Self: Sized` 约束则返回歧义：`dyn Trait` 内建 impl 可能与用户 blanket impl 重叠，只影响多态上下文的解析（[#L181](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/instance.rs#L181)）。

## 对 kfrc 的含义

- `InstanceKey = (InstanceKindTag, DefKey, 规范化 args)`，args 中的 const 参数用值树（valtree）表示，见 [const-eval.md](const-eval.md#const-泛型与值树)。
- 首轮可只开放 `Item`、`DropGlue`、`Intrinsic`（就地降低）、`Virtual`；shim 种类按需要逐个加，未支持时在实例图构建阶段明确报错。

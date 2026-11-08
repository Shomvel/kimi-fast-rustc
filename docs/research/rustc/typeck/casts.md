# `as` 转换

源码：`compiler/rustc_hir_typeck/src/cast.rs`、`compiler/rustc_middle/src/ty/cast.rs`。链接指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. 两阶段：检查时只给提示，fallback 之后才判定

- `check_expr_cast`（[expr.rs#L1542](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L1542)）：降低目标类型 `T`，用 `ExpectCastableToType(T)` 检查操作数，然后把 `CastCheck`（[cast.rs#L56](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/cast.rs#L56)）压入 `deferred_cast_checks`，表达式类型立即是 `T`。`CastCheck::new` 只提前拒绝目标是 `dyn`/slice 的情况（[#L239](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/cast.rs#L239)）。
- `check_casts`（[checks.rs#L74](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L74)）在 `type_inference_fallback` **之后**运行（[lib.rs#L228-L230](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/lib.rs#L228) 注释：为兼容，fallback 是比 cast 更强的提示）。
- **字面量例外**：`ExpectCastableToType` 也被 `check_expr_lit` 采纳（`to_option`），所以未后缀字面量直接取目标整型：`256 as u8` 中字面量就是 `u8`，触发 `overflowing_literals`（`tests/ui/lint/issue-109529.rs`）。目标是 `char` 时字面量取 `u8`，目标是裸指针/fn 时取 `usize`。目标为 `bool`/浮点等时字面量仍是 `{integer}`，随后 fallback 为 `i32`：`5 as bool` 报“cannot cast `i32` as `bool`”。
- **变量不受影响**：`let cap = 512 * 512; cap as u8;` 中 `cap` fallback 为 `i32`，cast 不回推（`tests/ui/cast/cast-does-fallback.rs`）。`let i: *const i32 = 0 as _;` 合法，`_` 由外部推出（`cast-to-infer-ty.rs`）。

## 2. `CastCheck::check` 流程

`check`（[cast.rs#L687](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/cast.rs#L687)）：

1. 两边 `structurally_resolve_type`（仍是 `TyVar` 则 E0282）。
2. 目标非 `Sized` → 报错。
3. **先试 coercion**：`try_coercion_cast` = `fcx.coerce(expr, expr_ty, cast_ty, AllowTwoPhase::No)`（[#L1073](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/cast.rs#L1073)）。成功且不是“裸指针→裸指针”时记入 `coercion_casts`（[typeck_results.rs#L156](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L156)）并发 `trivial_casts` lint；ptr→ptr 即使 coercion 成功也按真正的 cast 处理（区分不了只差 region 的 pointee，#113257）。coercion 的 adjustment 照常写入表达式。
4. coercion 失败则 `do_check`（[#L732](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/cast.rs#L732)）按 `CastTy` 分类判定。

## 3. `do_check` 规则表

`CastTy::from_ty`（[ty/cast.rs#L58](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/cast.rs#L58)）：`bool`→`Int(Bool)`，`char`→`Int(Char)`，有符号整数与 `{integer}`→`Int(I)`，无符号→`Int(U(_))`，浮点与 `{float}`→`Float`，无字段 enum→`Int(CEnum)`，`*T`→`Ptr`，fn 指针→`FnPtr`，其他 `None`。

源类型不是 `CastTy` 时：`FnDef` 先 coerce 成 fn 指针再按 `FnPtr` 处理；`&T` 到整数/浮点报 `NeedDeref`（`T` 是数值）或 `NeedViaPtr`；`&[T; N] as *const T` 走 `check_ref_cast`（`ArrayPtrCast`，要求 `m_expr >= m_cast` 且元素类型可等同）；其他 `NonScalar`。

| 源 \ 目标 | 结果 |
|---|---|
| 任意 → `Int(CEnum)` / `FnPtr` | `NonScalar`（目标不能是 enum 或 fn 指针） |
| 任意 → `bool` | `CastToBool` |
| `u8` → `char` | `U8CharCast`；其他 → `char` 报 `CastToChar` |
| `bool`/`char`/C-enum → 浮点 | `NeedViaInt` |
| `bool`/`char`/C-enum/浮点 → 指针；指针/fn 指针 → 浮点 | `IllegalCast` |
| 指针 → 指针 | `check_ptr_ptr_cast`：目标瘦指针总可；目标宽指针要求两边 metadata 种类相同（`PointerKind`） |
| 指针 → 整数 | 源须瘦指针（`PtrAddrCast`）；fn 指针 → 整数 `FnPtrAddrCast` |
| 整数 → 指针 | 目标须瘦指针（`AddrPtrCast`），否则 `IntToWideCast` |
| fn 指针 → 指针 | `FnPtrPtrCast`（目标须瘦） |
| C-enum → 整数 | `EnumCast`（enum 有 `Drop` 时报错） |
| `bool`/`char` → 整数 | `PrimIntCast` |
| 整数/浮点 ↔ 整数/浮点 | `NumericCast` |

规则表直接对应 [cast.rs#L796-L841](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/cast.rs#L796)。typeck 判定出的 `CastKind` **不写入** `TypeckResults`；只记录“是否 coercion cast”。

## 4. 下游如何使用

- THIR：`mirror_expr_cast`（[thir/cx/expr.rs#L251](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L251)）：coercion cast → `ExprKind::Use`（adjustment 已做完转换）；源是引用 → `PointerCoercion::ArrayToPointer`；其他 → `Cast`。
- MIR：`mir_cast_kind(from, to)`（[ty/cast.rs#L76](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/cast.rs#L76)）按源/目标的 `CastTy` 重新分类：`IntToInt`、`IntToFloat`、`FloatToInt`、`FloatToFloat`、`PtrToPtr`、`FnPtrToPtr`、`PointerExposeProvenance`、`PointerWithExposedProvenance`。数值 cast 的语义（截断、符号扩展、浮点饱和）在 MIR/codegen 层，不在 typeck。

## 对 kfrc 的含义

- 标量 M1 的 cast 只涉及 `NumericCast`、`PrimIntCast`（`bool as i64`）和 `CastToBool` 拒绝；但阶段顺序必须照搬：cast 判定排在 fallback 之后，字面量操作数按目标定型。否则 `256 as u8`、`cap as u8` 两类用例结果都会和 rustc 不同。
- Typed Body 应直接记录 kfrc 自己的 cast 种类（IntToInt + 源/目标宽度与符号、IntToFloat 等），而不是像 rustc 那样到 MIR 再重新分类；但分类函数要与 `mir_cast_kind` 一一对应，便于对拍。
- “先试 coercion”意味着 cast 依赖完整的 coercion 实现（`&[T;N] as *const T`、`&mut T as &T`、fn item as fn ptr）；M2 加指针时与 coercion 一起开放。

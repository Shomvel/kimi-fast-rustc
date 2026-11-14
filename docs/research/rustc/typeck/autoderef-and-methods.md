# Autoderef、方法查找、运算符与 place 操作

源码：`compiler/rustc_hir_analysis/src/autoderef.rs`、`compiler/rustc_hir_typeck/src/{autoderef.rs, method/, op.rs, place_op.rs}`；dev-guide `hir-typeck/method-lookup.md`。链接指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. Autoderef 迭代器

- **`Autoderef`**（[hir_analysis/autoderef.rs#L33](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/autoderef.rs#L33)）产出 `(ty, step_count)`，第一项是原类型、0 步。`state.steps: Vec<(Ty, AutoderefKind::{Builtin, Overloaded})>` 记录每步的**源**类型。
- `next`（[#L52](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/autoderef.rs#L52)）：
  1. 超过 `recursion_limit` 报错并停（`silence_errors` 时不报）。
  2. 当前类型是未解 `TyVar` 即停（[#L76](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/autoderef.rs#L76)）——autoderef 不推导。
  3. `builtin_deref(include_raw_pointers)`：`Box<T>`、`&T`/`&mut T`，仅显式要求时含裸指针（[sty.rs#L1572](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/sty.rs#L1572)）。
  4. 否则 `overloaded_deref_ty`（[#L148](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/autoderef.rs#L148)）：用 `predicate_may_hold` 判断 `T: Deref`，再结构化规范化 `<T as Deref>::Target`，产生的 obligation 存进 autoderef，由调用方决定是否注册。
- typeck 侧 `adjust_steps_as_infer_ok`（[hir_typeck/autoderef.rs#L34](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/autoderef.rs#L34)）把 steps 转成 `Adjust::Deref(None | Some(OverloadedDeref{mutbl}))` 列表；重载步骤此时查 `Deref::deref` 签名取得可变性。先记 `Deref`（不可变），需要可变 place 时 `convert_place_derefs_to_mutable` 再改。
- 使用方：coercion 的 `coerce_to_ref`、方法接收者、字段访问 `check_expr_field`、索引 `lookup_indexing`、调用 `check_expr_call`（被调者可经 deref 到 fn/闭包）。显式 `*e` 不走 autoderef，只做一步 `lookup_derefing`（[place_op.rs#L20](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/place_op.rs#L20)）。

## 2. 方法查找：probe

`check_expr_method_call`（[expr.rs#L1479](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L1479)）：接收者**无期望**地检查，`try_structurally_resolve_type`，`lookup_method`（[method/mod.rs#L176](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/mod.rs#L176)）= probe + confirm，然后按方法签名（去掉 self）检查实参。

- **步骤表**：`probe_op`（[probe.rs#L375](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L375)）把 `self_ty` 规范化后调**查询** `method_autoderef_steps`（[#L575](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L575)）。该查询在新 infcx 中跑含裸指针的 autoderef，产出 `CandidateStep { self_ty, autoderefs, from_unsafe_deref, unsize, reachable_via_deref }`（[traits/query.rs#L155](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/traits/query.rs#L155)）。最终类型是数组 `[T; N]` 时追加一步 `[T]`、`unsize = true`（[#L695](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L695)）。最终类型是 `TyVar` 时返回 `opt_bad_ty`，调用方报“需要类型注解”。
- **候选收集**：对每一步的类型 `assemble_probe`（[#L809](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L809)）收集 inherent 候选：ADT/`dyn`/foreign 查 inherent impl，`Param` 查 where 子句，原生类型（含引用、裸指针、数组、元组）查 `incoherent_impls`（标准库给原生类型写的 inherent impl）。`IntVar`/`FloatVar` **没有 inherent 候选**，所以 `2.0.neg()`、`x.pow(2)`（`x: {integer}`）报 E0689。然后对作用域内 trait 收集 extension 候选（[#L1011](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1011)）。
- **挑选顺序** `pick_all_method`（[#L1239](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1239)）：按步骤顺序，每步依次试
  1. **by value** `pick_by_value_method`（[#L1482](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1482)）：unsize 步跳过。若该步类型本身是 `&T`/`&mut T`，记为“再 deref 一次 + 同可变性 autoref”，即**重借用接收者**而不是移动它。
  2. **autoref `&`** `pick_autorefd_method`（[#L1526](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1526)），用 `re_erased` 构造 `&self_ty`。
  3. **autoref `&mut`**。
  4. `*mut T` → `*const T` 方法（`pick_const_ptr_method`，[#L1606](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1606)）；pin 重借用（feature）。

  第一个找到的即结果，不再看后面的步骤。经裸指针 deref 得到的步骤（`from_unsafe_deref`）不参与挑选。
- **inherent 优先**：每个 (步骤, autoref) 组合内，`pick_method`（[#L1634](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1634)）先看 inherent 再看 extension。所以“步骤/autoref 层级”比“inherent vs trait”优先：by-value 的 trait 方法胜过需要 `&` 的 inherent 方法。不可见的 inherent 方法进 `private_candidates`，只在全部失败时报 privacy 错误（`tests/ui/methods/trait-method-resolution-over-inherent-22684.rs` 因此选中 trait 方法）。
- **同层多候选**：`consider_candidates`（[#L1676](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1676)）在 `probe` 中逐个 `consider_probe`（[#L1882](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1882)）：关联接收者类型并评估 where 子句，结果 `Match / NoMatch / BadReturnType`；多个匹配时尝试合并到同一 trait，否则 `MethodError::Ambiguity`。
- **遮蔽检查**：选中 by-value 或 `&` 的 inherent 方法时，`check_for_shadowed_autorefd_method`（[#L1389](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L1389)）检查更后面的 autoref 层是否有可能被遮蔽的方法（为 arbitrary self types 预留）。

## 3. 方法确认：confirm

`ConfirmContext::confirm`（[confirm.rs#L98](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/confirm.rs#L98)）：

1. `adjust_self_ty`（[#L167](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/confirm.rs#L167)）：在真实 infcx 中**重跑** autoderef 到 `pick.autoderefs` 步，写入 `Deref` 步骤；按 pick 追加 `Borrow(Ref(m, AllowTwoPhase::Yes))`（新 region 变量 `Autoref`），unsize 时再加 `Pointer(Unsize)`；`ToConstPtr` 加 `MutToConstPointer`。一次性 `apply_adjustments(receiver, ..)`。
2. 为 impl/trait 参数建新变量，套用显式 turbofish，实例化方法签名（late-bound region 换新变量）。
3. `unify_receivers`：调整后的接收者类型 `sup` 方法签名的 `self` 类型。
4. 注册方法 predicates；返回 `MethodCallee { def_id, args, sig }`，记入 `type_dependent_defs` 与 `node_args`。

## 4. 调用与路径（摘要）

`check_expr_call`（[callee.rs#L65](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/callee.rs#L65)）对被调者做 autoderef：`FnDef`/`FnPtr` 为内建调用；kind 未定的闭包延迟解析；其他尝试 `Fn*` trait。详见 [calls-and-closures.md](calls-and-closures.md)。

## 5. 二元运算符

- `check_expr_binop`（[op.rs#L92](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L92)）：`&&`/`||` 两侧 coerce 到 `bool`。其他运算 `check_overloaded_binop`（[#L200](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L200)）：左侧无期望检查后 coerce 到新变量；rhs 先给一个新变量 `?R`；`lookup_op_method` 登记 `Lhs: Add<?R>` 等 trait；再以 `?R` 为期望检查右侧。
- 两侧都不是 `TyVar`（`IntVar` 不算 `TyVar`）且 `is_builtin_binop`（[#L1209](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L1209)，允许剥一层 `&`）时，`enforce_builtin_binop_types`（[#L159](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L159)）：
  - 算术、位运算：`rhs <: lhs`，结果是 lhs 类型；比较：`rhs <: lhs`，结果 `bool`。
  - **移位不统一两侧**，结果是 lhs 类型：`1u8 << 2` 的 `2` fallback 为 `i32`。
  - 结果类型 = lhs 类型，因此 `let _: u8 = 0 + 0;` 中外层的 `u8` 经结果类型约束到两侧字面量。
- 按引用传递的比较运算（`==`、`<` 等）先给两侧加 autoref adjustment；writeback 的 `fix_scalar_builtin_expr`（[writeback.rs#L151](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L151)）在两侧都是标量时删掉方法记录和这些 autoref，MIR 构建据此生成内建运算。

## 6. 索引与显式解引用

- `lookup_indexing`（place_op.rs）对 base autoderef，数组/slice 配 `usize` 走内建索引，否则 `Index`/`IndexMut`；数组末步可 unsize 成 slice。和运算符一样先记成方法调用，writeback 的 `fix_index_builtin_expr`（[writeback.rs#L220](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L220)）在元素类型与索引类型确为内建时撤销。

## 对 kfrc 的含义

- 标量 M1 不需要 autoderef/方法查找，但运算符规则需要：直接实现 §5 的内建分支（含“移位不统一”“结果取 lhs”），并对无法判定内建的情况（操作数是未解 `TyVar`）明确报未支持，而不是猜。
- 方法查找依赖“步骤 × autoref × (inherent, trait)”三重循环和 trait 求解器的 `predicate_may_hold`/候选评估接口。步骤计算在 rustc 中是 canonical 查询，kfrc 可直接在本地 infcx 的 probe 中算，但要保证结果不泄漏变量（rustc 用规范化达到这一点）。
- 运算符、索引在 kfrc 中应直接记“内建 / 重载(callee)”二选一，不必模仿 rustc“先记方法再在 writeback 撤销”的做法；但判定条件要与 writeback 的条件一致（两侧标量；索引为 `usize` 且元素类型相符）。

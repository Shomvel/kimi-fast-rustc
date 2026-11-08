# 闭包：捕获分析与 region 要求的上传

只写与借用检查交互的部分。链接固定到 `e408947b`。

## 1. 捕获分析（HIR typeck，`rustc_hir_typeck/src/upvar.rs`）

- `analyze_closure`（[upvar.rs#L165](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L165)）用 `ExprUseVisitor`（[expr_use_visitor.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr_use_visitor.rs)）遍历闭包体，委托 `InferBorrowKind` 记录每次对外部变量 Place 的使用：consume → `ByValue`；`borrow(bk)` → `ByRef(bk)`（[#L2084](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2084)）；mutate → `ByRef(Mutable)`（[#L2118](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2118)）；经裸指针 deref 的一律 `Immutable`。
- **截断规则**（每条都可能降低精度）：
  - `restrict_capture_precision`（[#L2186](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2186)）：在 `Index`/`Subslice` 处截断（数组整体捕获）；不越过裸指针 deref 和 union 字段。
  - `restrict_precision_for_drop_types`（[#L2125](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2125)）：不能从有 `Drop` impl 的类型中按值捕获字段。
  - `move` 闭包 `adjust_for_move_closure`（[#L2216](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2216)）：在第一个 `Deref` 处截断并改为 `ByValue`；非 move 闭包的 `ByValue` 同样截到第一个 `Deref`（[#L2247](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2247)）。
  - `truncate_place_to_len_and_update_capture_kind`（[#L2473](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2473)）：被截掉的部分含 `&mut` 的 deref 且原为 `Mutable` ⇒ 改为 `UniqueImmutable`。
  - `truncate_capture_for_optimization`（[#L2570](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2570)）：最右一个 deref 是 `&` 时，丢掉其后的字段，捕获引用本身。
- `compute_min_captures`（[#L765](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L765)）：同一根变量的祖先/后代 Place 合并为祖先，捕获方式取较强者。结果存 `closure_min_captures`，闭包种类（Fn/FnMut/FnOnce）由此推出。
- **edition**：`enable_precise_capture`（[#L2594](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2594)）看**闭包 span** 的 edition ≥ 2021。否则对每个提到的变量补一条整变量捕获（[#L324](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L324)），min capture 就退化成整变量。宏生成的闭包按宏定义处的 edition 走。

## 2. 在 MIR 中的形状

- **父函数**：闭包创建是 `Rvalue::Aggregate(Closure, operands)`。每个按引用捕获的操作数是父函数里的一次普通借用：`Immutable` → `Shared`，`UniqueImmutable` → `Mut { ClosureCapture }`，`Mutable` → `Mut { Default }`（[rustc_mir_build/src/thir/cx/expr.rs#L1432](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L1432)）。闭包里再捕获外层闭包的 upvar 时由 `limit_capture_mutability`（[as_rvalue.rs#L710](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_rvalue.rs#L710)）按外层可变性定 kind。这些 loan 的 region 流进闭包类型的 upvar 字段，只要闭包值 live，loan 就在作用域内。
- **`ClosureCapture`（unique immutable）**：冲突上按 `&mut` 处理；可变性检查用 `LocalMutationIsAllowed::Yes`（[lib.rs#L2341](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L2341) 起），所以不可变绑定 `x: &mut i32` 也能被 `|| *x += 1` 捕获。
- **闭包体**：`_1` 是环境（`closure_env_ty`，[rustc_middle/src/ty/util.rs#L670](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/util.rs#L670)：Fn → `&Closure`，FnMut → `&mut Closure`，FnOnce → 按值），upvar 访问是 `(*_1).i`，按引用捕获的再加一层 deref。`is_upvar_field_projection`（[path_utils.rs#L134](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/path_utils.rs#L134)）用于识别。
- 父函数检查创建闭包的 `Aggregate` 时，从 `root_cx.used_mut_upvars(def)` 取闭包实际改动过的 upvar，标记父函数的 `mut` 绑定为已用（[lib.rs#L1566](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L1566)），只影响 `unused_mut` lint。

## 3. Region 要求的上传

- 闭包的 universal region：`'static`、external（闭包实参里的全部 free region：父函数实参、闭包签名、upvar 类型，以及父作用域的 late-bound）、local（闭包签名自己的 late-bound 与 `ClosureEnv`）。见 [lifetimes-origin.md](lifetimes-origin.md)。
- typeck child 求解时 `outlives_requirements = Some(vec)`（[region_infer/mod.rs#L509](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L509)）。`check_universal_region` 发现 `longer: shorter` 无证据时，`try_propagate_universal_region_error`（[#L1272](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L1272)）：
  1. `longer-` = `non_local_lower_bounds(longer)`（沿已知关系往下找 external region），为空则报错；
  2. `shorter+` = `non_local_upper_bounds(shorter)`（往上找，至少有 `'static`，[free_region_relations.rs#L86](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/free_region_relations.rs#L86)）；
  3. 候选是笛卡尔积；若其中已有被 `longer-` 满足的，只上传这些，否则全部上传（不支持“或”约束，所以是近似）。
- `T: 'a` 类 type test 同样可上传（`ClosureOutlivesSubject::Ty`，`try_promote_type_test`）。
- 结果 `ClosureRegionRequirements { num_external_vids, outlives_requirements }`（[lib.rs#L193](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L193)）；vid 是 `closure_mapping` 的下标：`['static, 闭包实参中的 free region 按出现顺序, 父作用域 late-bound]`（[universal_regions.rs#L287](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/universal_regions.rs#L287)）。
- **父函数应用**：MIR typeck 在 `Aggregate(Closure)` 处 `prove_closure_bounds`（[type_check/mod.rs#L2495](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L2495)）登记延迟项 `(def_id, args, Single(loc))`；`root_cx` 在子 body 求解后用 `ConstraintConversion::apply_closure_requirements`（[constraint_conversion.rs#L86](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/constraint_conversion.rs#L86)）把要求按映射换成父函数的约束（`from_closure = true`），位置是闭包创建点。

例：`escape-upvar-ref.rs` 中 `|| p = &y` 捕获 `p: &'1 mut &'2 i32`、`y: &'3 i32`，闭包上传 `'3: '2`；父函数里 `y` 的 loan 因此必须活到 `p` 的最后使用 ⇒ E0597。

## 4. tricky 边界

- `borrowck-closures-unique.rs`：两个闭包都只读 `*x` 可以共存；其一写 `*x` 时需要 unique 访问（E0500/E0501/E0524 系列）。
- edition 2021 下闭包只捕获 `p.x`，外面可以同时 `&mut p.y`（`tests/ui/closures/2229_closure_analysis/run_pass/capture-disjoint-field-struct.rs`）；2018 下整变量捕获 `p`，同样代码应报 E0502（推断，未找到对应 2018 用例）。
- 父函数和闭包在同一个 typeck root 里检查；闭包 body 必须先完成，`used_mut_upvars` 和闭包要求才可用（[flow.md](flow.md)）。

## 5. 对 kfrc 的含义

- 捕获分析是 typeck 的产物（依赖 Place 分类、Copy 判断、`Drop` impl 查询、edition），要在构造父函数和闭包的 Typed Body **之前**完成；它与借用检查的接口是“每个捕获的 (根变量, 投影, 捕获方式, 可变性)”。
- 建议把 [T22](../../roadmap/m3-semantics.md#t22) 的“closure 捕获”拆成三步：(1) 只按值/move 捕获、不含引用；(2) 按引用捕获但闭包要求为空（只检查父函数里的捕获 loan）；(3) 闭包要求上传与应用。第 (3) 步需要完整的 universal region 关系表与 `closure_mapping` 编号约定。
- CheckedBody 的身份与缓存键按 typeck root 计，闭包要求是 root 内部的中间产物。

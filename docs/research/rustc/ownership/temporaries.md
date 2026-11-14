# Region scope tree 与临时值生命周期

源码：`compiler/rustc_hir_analysis/src/check/region.rs`（建树）、`compiler/rustc_middle/src/middle/region.rs`（`ScopeTree` 与查询）、MIR 侧 `compiler/rustc_mir_build/src/builder/expr/as_temp.rs`、`thir/cx/{expr,block}.rs`。1.94 已没有单独的 `rvalue_scopes.rs`，延长规则并入 `ScopeTree::extended_temp_scopes`。

## 数据结构

- [`Scope { local_id, data }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L85)，[`ScopeData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L112)：`Node`、`CallSite`、`Arguments`、`Destruction`、`IfThen`、`IfThenRescope`（2024）、`MatchGuard`、`Remainder(i)`（块中第 i 条 `let` 之后的后缀）。
- [`ScopeTree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L208)：`parent_map`（子→父）、`var_map`（绑定→所在 scope）、[`extended_temp_scopes`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L229)（被延长的表达式 → 新 scope，`None` 表示 'static 不 drop）、`backwards_incompatible_scope`（仅 lint）。
- [`TempLifetime { temp_lifetime, backwards_incompatible }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L239)。

## 查询：临时值在哪个 scope 结束

[`temporary_scope(expr_id)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L346)：先查 `extended_temp_scopes`；否则 [`default_temporary_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L308) 从 `Node(expr)` 向上走：父是 `Destruction` → 返回当前（被包住的 terminating Node）；父是 `IfThenRescope`/`MatchGuard` → 返回父（[L318](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/region.rs#L318)）；其他继续向上。

MIR 侧：THIR 给每个表达式包一层 `ExprKind::Scope{Node}` 并记 `temp_scope_id`（[`mirror_expr_inner`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L51)）；构建时每层 `Scope` 都 `in_scope` 压栈，[`as_temp`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_temp.rs#L16) 把临时值调度到 `temp_lifetime` 对应的那层。`Destruction` scope 只存在于树里，MIR 不压它。

## 建树：哪些位置是 terminating

[`enter_node_scope_with_dtor(id, terminating)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L740) 在 terminating 时先进 `Destruction` 再进 `Node`。terminating 位置：

- 函数体表达式（[`visit_body`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L769)），外面是 `CallSite`、`Arguments`；
- 每条语句（[`resolve_stmt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L224)，`super let` 除外）；
- `if` 条件（[`resolve_cond`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L172)：条件不是 `let`、不是 `&&`/`||` 时）、then/else 块；match guard；
- `&&`/`||` 的操作数（非 `let`、非嵌套逻辑运算）；
- `loop` 体（`while` 降为 `loop { if cond {..} else { break } }`，[ast_lowering expr.rs L555](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/expr.rs#L555)）；
- match arm（[`resolve_arm`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L194)，有 guard 时 arm 内再套 `MatchGuard`）；
- `DropTemps(e)`（`for` 整体包在 `DropTemps(match into_iter(head) {..})` 里，[expr.rs L1950](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/expr.rs#L1950)，所以 head 的临时值活到整个 for 结束）；
- `let-else` 的 else 块；
- **块尾表达式，仅 edition ≥ 2024**（[L147](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L147)）。

非 terminating 的典型：match 的 scrutinee（临时值活到所在语句结束，`match m.lock().unwrap().x {..}` 整个 match 期间持锁）；2021 的块尾（临时值活到外层语句，晚于块内局部变量 drop）；`let` 条件。

`if`（[L355](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L355)）：then 外包 `IfThen`（2021）或 `IfThenRescope`（2024），条件在它里面，else 在它外面。2024 下 `IfThenRescope` 是临时值边界，`if let` scrutinee 的临时值在进入 else 前 drop；2021 下 `IfThen` 不是边界，这些临时值活过 else 到语句结束。THIR 对应 [expr.rs L930](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L930)。edition 取自**表达式 span**（宏展开出的代码按定义宏的 crate 的 edition）。

`AssignOp` 在树里先访问 RHS 再访问 LHS（[L350](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L350)），与原生类型的求值顺序一致。

## 临时值生命周期延长

[`resolve_local`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L406) 处理 `let pat = init`，把下列子表达式的临时 scope 设为绑定所在的 `var_parent`（Remainder scope）：

- `E&` 文法 [`record_rvalue_scope_if_borrow_expr`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L620)：`&ET`，以及穿过结构体/元组/数组字面量、`as`、块尾、`if`/`match` 各分支、tuple-struct/variant 构造调用（`Some(&temp())`）。
- `P&` 文法 [`is_binding_pat`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L551)：模式里有显式 `ref` 绑定时，整个 `ET` 被延长（隐式 ref 的 match ergonomics 不算）。
- `ET` 文法 [`record_subexpr_extended_temp_scopes`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L702)：沿 `*e`、`e[..]`、`e.f`、`&e` 向下，链上每个表达式都记录（建 MIR 时可能对外层表达式取临时 scope）。
- `const`/`static` 初值：`var_parent = None`，延长后的临时值 `temp_lifetime = None`，不调度 drop（as_temp 注释）。
- `super let`（[L491](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/region.rs#L491) 附近）：被外层 `let` 延长时使用外层绑定的 scope，否则用所在块的默认临时 scope。`format_args!` 的降低就生成 `super let`（[format.rs L450](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/format.rs#L450)），`pin!` 也依赖它。

## tricky 边界

- 2021 块尾临时值晚于块内局部 drop，经典报错“borrowed value does not live long enough”在 2024 消失；反之块尾临时值被存进外层局部时，2024 报 E0716 而 2021 通过（`tests/ui/drop/tail-expr-drop-order-negative.rs`；顺序见 `tail-expr-drop-order.rs`）。
- `if let` 在 2021 与 2024 的 else 分支中锁/借用是否仍持有不同（`drop_order_if_let_rescope.rs`、`if-let-rescope-borrowck-suggestions.rs`）。
- 延长只看语法，不看类型；`let x = f(&temp());` 不延长（函数调用不在 `E&` 里），`let x = Some(&temp());` 延长。
- let-else：else 块回到上一层 scope，使初值里被延长的临时值也在 else 中 drop。

## 对 kfrc 的含义

- region scope tree 只依赖语法（AST/HIR 级）和 span edition，可以在 T06 之前或与之并行移植；kfrc 需要一个与 rustc HIR 节点对应的稳定“表达式 ID”，Body builder 按它查 temp lifetime。
- Typed Body 不必存 scope tree 本身，但 builder 必须用它决定每个临时值/绑定的 `Drop`/`StorageDead` 插在哪；表达式降低接口要带 `TempLifetime`。
- 必须在 AST lowering 层复刻 `while`/`for`/`?`/`format_args!`/`if let` 等脱糖形状，否则临时值时机会偏离 rustc（`for` 的 `DropTemps`、`format_args!` 的 `super let`）。
- edition 必须是 per-span 属性随宏展开传播（与 T23 hygiene 耦合）；ripgrep（2021）与新版 Cargo（2024）两种规则都要支持（推断：Cargo 当前 manifest edition 需在 M0 冻结时确认）。
- 移植量（推断）：region.rs + ScopeTree 查询约 1000 行，无类型依赖。

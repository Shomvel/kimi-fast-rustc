# FnCtxt 主流程、Expectation、表达式检查与 fallback

源码根：`compiler/rustc_hir_typeck/src/`。链接指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. 一个 typeck root 的固定阶段顺序

`typeck_with_inspect`（[lib.rs#L106](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/lib.rs#L106)）。闭包不单独 typeck，查询转发到外层函数（[#L113](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/lib.rs#L113)）。

1. 取 `fn_sig`，`liberate_late_bound_regions`，逐个参数/返回类型 `normalize`。
2. `check_fn`（[check.rs#L27](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/check.rs#L27)）：返回类型建 `ret_coercion = CoerceMany::new(ret_ty)`；`GatherLocalsVisitor` 给每个绑定建类型（有注解用注解，否则新 `TyVar`，[gather_locals.rs#L107](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/gather_locals.rs#L107)）；参数模式 `check_pat_top`；函数体作为尾表达式走 `check_return_or_body_tail`（[expr.rs#L967](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L967)）；最后 `CoerceMany::complete`。
3. `check_repeat_exprs` → `type_inference_fallback`（[lib.rs#L226](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/lib.rs#L226)）→ `check_casts`（**cast 在 fallback 之后**，[#L230](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/lib.rs#L230)）→ `select_obligations_where_possible`。
4. `closure_analyze`（upvar 捕获与闭包 kind，fallback 之后）→ deferred sized → select → `report_ambiguity_errors`（剩余 obligation 报“需要类型注解”）。
5. `resolve_type_vars_in_body`（writeback，[lib.rs#L262](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/lib.rs#L262)，见 [writeback.md](writeback.md)）。

`FnCtxt`（[fn_ctxt/mod.rs#L44](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/mod.rs#L44)）的可变状态：`ret_coercion`、`diverges: Cell<Diverges>`、`enclosing_breakables`（每个 loop/labeled block 一个 `CoerceMany`）、never fallback 配置；推导状态在共享的 `TypeckRootCtxt`。

## 2. Expectation：向下传的“提示”

`Expectation`（[expectation.rs#L11](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expectation.rs#L11)）：`NoExpectation | ExpectHasType(T) | ExpectCastableToType(T) | ExpectRvalueLikeUnsized(T)`。

- 提示不是约束。约束由调用方事后施加：`check_expr_coercible_to_type`（coerce，[expr.rs#L155](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L155)）、`check_expr_has_type_or_error`（子类型 + 允许 `!`→任意，[#L94](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L94)）、`demand_eqtype`。
- `only_has_type`（[#L114](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expectation.rs#L114)）只认 `ExpectHasType`；`to_option`（[#L103](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expectation.rs#L103)）还接受 castable/unsized。字面量定型和闭包签名推导用 `to_option`，所以 `as` 的目标类型会影响字面量（见 [casts.md](casts.md)）。
- 分支结构用 `try_structurally_resolve_and_adjust_for_branches`（[#L43](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expectation.rs#L43)）：期望若仍是 `TyVar` 就降为 `NoExpectation`，避免各分支的具体类型过早约束外层变量。`coercion_target_type`（[#L123](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expectation.rs#L123)）在无硬期望时建新变量作为 `CoerceMany` 初值。
- `rvalue_hint`（[#L77](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expectation.rs#L77)）：期望是 unsized（`[T]`/`str`/`dyn`）时改成 `ExpectRvalueLikeUnsized`。

## 3. `check_expr` 骨架与关键分支

`check_expr_with_expectation_and_args`（[expr.rs#L238](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L238)）：保存并清空 `diverges` → `check_expr_kind` 分派（[#L343](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L343)）→ `resolve_vars_if_possible` → 结果为 `!` 且表达式确实构成读取时置 `diverges = Always`（[#L318](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L318)）→ `write_ty` → 与旧 `diverges` 合并。

| 表达式 | 规则（函数） |
|---|---|
| 字面量 | `check_expr_lit`（[checks.rs#L736](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L736)）：未后缀整数——期望已是 `Int/Uint` 则直接用；期望 `char` 用 `u8`、裸指针/fn 用 `usize`（服务 `0x61 as char` 一类 cast）；否则 `next_int_var`。期望是 `{integer}` 变量时也新建变量（不复用）。浮点同理。 |
| 一元 `-`/`!` | 把期望传给操作数；`*` 不传（[expr.rs#L425](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L425)）。始终走 `check_user_unop`（trait `Neg`/`Not`）；操作数是整数/bool（`!`）或数值（`-`）时结果取操作数类型。`-1` 赋给 `usize` 报 E0600。 |
| 二元运算 | 见 [autoderef-and-methods.md §5](autoderef-and-methods.md)。 |
| `let` | `check_decl`（[checks.rs#L901](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L901)）：先查初始化表达式（coerce 到局部类型；有 `ref` 绑定时用 eqtype），再查模式；`let-else` 的 else 块必须是 `!`。 |
| 语句 | `check_stmt`（[checks.rs#L952](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L952)）：无分号表达式语句要求 `()`（允许 `!`）；有分号的不加约束。 |
| 块 | `check_expr_block`（[checks.rs#L1014](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L1014)）：块本身是 breakable，带 `CoerceMany`；尾表达式参与 coercion；无尾表达式时，若块不发散则强制 `()`（`coerce_forced_unit`），若发散则什么都不加，块类型可为 `!`（[#L1085](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L1085)）。 |
| `if` | `check_expr_if`（[expr.rs#L1198](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L1198)）：条件 `has_type_or_error(bool)`；两分支用同一 expectation，结果走 `CoerceMany`（LUB）；无 else 时 `if_fallback_coercion` 要求 then 为 `()`；发散 = 条件发散 ∨ (then ∧ else)。 |
| `match` | `check_expr_match`（[_match.rs#L18](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/_match.rs#L18)）：先查完所有 arm 的模式，再查 arm 体；`CoerceMany` 初值为期望类型，但期望是 `()` 时改用新变量（[#L73](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/_match.rs#L73)）；零 arm 的 match 类型为 `!`。 |
| `loop` | `check_expr_loop`（[expr.rs#L1433](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L1433)）：`loop` 带 `CoerceMany` 收集 `break` 值；没有 `break` 则类型 `!`、`diverges = Always`；`while`/`for` 类型 `()`。 |
| `break`/`return` | 值进入对应 `CoerceMany`；无值时 `coerce_forced_unit`；表达式类型 `!`（[expr.rs#L887](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L887)）。 |
| 赋值 | `check_expr_assign`（[expr.rs#L1264](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L1264)）：**先查左侧**（`Needs::MutPlace`），再以左侧类型为提示查右侧并 coerce；类型 `()`。期望是 `bool` 时直接报“可能想写 `==`”。typeck 顺序与运行时求值顺序无关。 |

## 4. obligation 何时注册、何时 select（与 trait 求解器的接口）

- 注册：`register_predicate`（[typeck_root_ctxt.rs#L113](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/typeck_root_ctxt.rs#L113)）进入 `fulfillment_cx`。来源：路径实例化的 where 子句（`add_required_obligations_for_hir`）、方法确认的 predicates、运算符 trait、WF、`Sized`、coercion/关系产生的 `Subtype`/`Coerce`/`CoerceUnsized`、autoderef 的 `Deref` 投影。
- 主动推进 `select_obligations_where_possible`（[_impl.rs#L646](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L646)）在以下时机：`resolve_vars_with_obligations`（类型仍含变量时先 select 再 resolve，[#L88](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L88)）——几乎所有 `try_structurally_resolve_type` 都经过它；实参检查在处理闭包实参之前；fallback 前后；cast 检查后；最终。
- `structurally_resolve_type`（[#L1429](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L1429)）：select 后仍是 `TyVar` 就报 E0282；`IntVar`/`FloatVar` 允许留下。方法调用、字段访问、解引用、cast 检查都要求接收者“结构上已知”。
- 默认求解器：1.94.1 的 `NextSolverConfig` 默认 `coherence: true, globally: false`（[config.rs#L993](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L993)），HIR typeck 用的是旧求解器（`FulfillmentContext` + `SelectionContext`）。很多 typeck 代码有 `if self.next_trait_solver()` 两套分支，移植时以旧分支为行为基准。

## 5. 整数/浮点 fallback

- `type_inference_fallback`（[fallback.rs#L36](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fallback.rs#L36)）：select → 对所有未解变量 `fallback_if_possible` → 若有变化再 select。
- `fallback_if_possible`（[#L97](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fallback.rs#L97)）：有错误时一律 `{error}`；`IntVar → i32`、`FloatVar → f64`；被判为 diverging 的 `TyVar → ()`/`!`；其他 `TyVar` 不动（之后报歧义或 E0282）。用 `demand_eqtype` 落地。只有一次 fallback，没有用户自定义默认。

## 6. never 类型与 diverging fallback（含 edition 2024）

- `!` 到任意类型的 coercion 记 `Adjust::NeverToAny { target }`；target 是 `TyVar` 时进 `diverging_type_vars`（[_impl.rs#L265](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L265)）。
- `calculate_diverging_fallback`（[fallback.rs#L138](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fallback.rs#L138)）：diverging 变量的根 vid 集合 D；未解变量的根在 D 中就 fallback。`create_coercion_graph`（[#L332](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fallback.rs#L332)）由待证的 `Coerce`/`Subtype` 谓词建图，只用于 lint。
- 目标类型：`default_fallback`（[fn_ctxt/mod.rs#L504](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/mod.rs#L504)）——**edition 2024 起为 `!`，之前为 `()`**；`#![rustc_never_type_options]` 可覆盖。依赖 `()` fallback 的代码在旧 edition 触发 `DEPENDENCY_ON_UNIT_NEVER_TYPE_FALLBACK`，1.94.1 中是 **Deny** 级 future-incompat lint（[builtin.rs#L4195](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_lint_defs/src/builtin.rs#L4195)），流入 unsafe 调用的情况由 `NEVER_TYPE_FALLBACK_FLOWING_INTO_UNSAFE` 处理（同为 Deny）。
- 块级规则不随 edition 变：`DivergingBlockBehavior::Never` 是默认（[checks.rs#L51](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L51)），`{ return; }` 的类型是 `!`。
- 只有“确实构成读取”的 `!` 表达式才允许 `NeverToAny`（`expr_guaranteed_to_constitute_read_for_never`，[hir/mod.rs#L239](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/hir/mod.rs#L239)），`let _ = *never_ptr;` 不发散。

## 对 kfrc 的含义

- **i32 是 fallback 的必需品。** kfrc 现在只有 `i64/u64/bool/()`，而 rustc 对无约束整数字面量取 `i32`。要与 rustc 对拍，M1 必须支持 `i32`，或对“fallback 才能定型”的字面量明确报未支持；不能把它定成 i64。
- 阶段顺序（全部表达式 → fallback → cast 检查 → 歧义报告 → writeback）是语义的一部分，例如 `let cap = 512*512; cap as u8;` 中 `cap` 是 `i32` 而不是 `u8`。kfrc 的 typeck 也应是“先收集约束、末尾统一 fallback”，而不是逐表达式立即定型。
- 即使没有 trait，标量版也需要一个“延迟谓词”队列：`-x` 中 `x: {integer}` 稍后定为 `u64` 时要报 E0600（推断：rustc 靠挂起的 `{integer}: Neg` obligation 实现）。
- `!` 与 diverges 标记在 M1 就需要：`return`、`loop {}`、发散块的类型规则直接决定 if/match 的合并结果和 Body 的不可达块。

# 生命周期从哪来：省略、early/late-bound、`'static`、隐含界

链接固定到 `e408947b`。这些事实在借用检查之前就由名称解析、HIR 降低和签名收集产出；借用检查只消费（[regions.md](regions.md)）。

## 1. 名称解析阶段的 lifetime rib（`rustc_resolve/src/late.rs`）

`LifetimeRibKind`（[late.rs#L315](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L315)）决定一个省略的 `&`/`'_` 解析成什么：

| rib | 用在哪 | 结果 |
|---|---|---|
| `Generics` | 显式 `<'a>` | 具名参数 |
| `AnonymousCreateParameter` | fn/闭包**参数**类型 | 每处新建一个 lifetime 参数（`create_fresh_lifetime`，[#L2083](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L2083)；AST lowering 把它变成 HIR 泛型参数，[rustc_ast_lowering/src/lib.rs#L870](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/lib.rs#L870)） |
| `Elided(res)` | fn 返回类型（省略成功时）、函数体（`Infer`）、static/const 类型（`Static`，[#L2846](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L2846)） | 统一替换为给定结果；`Infer` 即借用检查中的 existential 变量 |
| `ElisionFailure` | fn 返回类型（省略失败时） | E0106 missing lifetime specifier |
| `AnonymousReportError` | where 子句、`for<>` 闭包等 | 报错 |
| `StaticIfNoLifetimeInScope` | 关联常量等 | 作用域里没有 lifetime 时取 `'static` |

### 返回类型省略规则（`resolve_fn_signature` [#L2347](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L2347) → `resolve_fn_params` [#L2384](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L2384)）

1. 逐个参数收集出现的 lifetime（具名、省略都算）。第一个含 lifetime 的参数若恰有一个**不同的** lifetime ⇒ 候选；再出现第二个含 lifetime 的参数 ⇒ 失败。
2. 第 0 个参数是 `self` 时，`find_lifetime_for_self` 找“指向含 `Self` 的类型”的引用的 lifetime：恰好一个 ⇒ 用它并压过其他参数；多个 ⇒ 失败；没有 ⇒ 丢弃 `self` 参数自身带来的候选，从后续参数重新计。
3. 规则在 AST 上执行：藏在类型别名里的 lifetime 不算（`type StaticStr = &'static str; fn j(_: StaticStr) -> &isize` 报错，见 `lifetime-elision-return-type-requires-explicit-lifetime.rs`）。
- 闭包（[#L1134](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L1134)）：参数用 `AnonymousCreateParameter`，返回类型用 `Elided(Infer)`——闭包返回类型里的省略 lifetime 是推导变量，不套用 fn 省略规则。

## 2. early-bound 与 late-bound（`rustc_hir_analysis/src/collect/resolve_bound_vars.rs`）

- query `resolve_bound_vars`（[#L256](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/collect/resolve_bound_vars.rs#L256)）把每个具名 lifetime 使用解析为 `ResolvedArg::{StaticLifetime, EarlyBound, LateBound(debruijn, idx, decl), Free, Error}`（[rustc_middle/src/middle/resolve_bound_vars.rs#L12](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/resolve_bound_vars.rs#L12)）。
- `is_late_bound_map`（[#L2306](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/collect/resolve_bound_vars.rs#L2306)）：fn 上的 lifetime 参数默认 late-bound，以下情况改为 early-bound：
  - 出现在 where 子句或泛型参数的界里；
  - 没被输入类型“约束”（投影 `<T as Tr<'a>>::X` 的参数不算约束）却出现在返回类型里；
  - 返回类型含“捕获全部参数”的 `impl Trait` 时，所有参数都视为出现在返回类型里。
  - 类型/const 参数永远 early-bound；impl 块上的 lifetime 也是 early-bound（只有 fn 自己的参数会 late-bound）。
- 意义：early-bound 进入 `generics_of`、出现在 `FnDef` 的泛型实参里，调用点按普通实参处理；late-bound 留在 `fn_sig` 的 binder 里，**每次调用**实例化为新变量（[type_check/mod.rs#L767](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L767)）。在被检查的函数内部，两者都是 local universal region：early-bound 来自 identity 实参（`compute_indices`，[universal_regions.rs#L660](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/universal_regions.rs#L660)），late-bound 由 `build` 把签名 binder 解放成 `ReLateParam`（[#L524](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/universal_regions.rs#L524)）。

## 3. `'static`

- 名称解析为 `LifetimeRes::Static` → `ReStatic` → 借用检查的 `fr_static`（vid 0）。
- `UniversalRegionRelations` 预置 `'static: 'r`（所有 universal r），值含全部点和全部 `end(..)`；约束图里 `'static` 到所有 region 有隐式边（[regions.md](regions.md)）。
- 用户写 `&'static` 的标注用 `Locations::All`，使被借局部的 loan 永不失效（`cast_static_lifetime.rs`：`(&x) as &'static u32` ⇒ E0597）。

## 4. 已知的 universal 关系与隐含界（`free_region_relations::create`，[#L182](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/free_region_relations.rs#L182)）

1. `explicit_outlives_bounds(param_env)`：where 子句里的 `'a: 'b`。
2. 每个 universal：`r: r`、`'static: r`、`r: 'fn_body`。
3. param_env 里的 `T: 'a` 进 `region_bound_pairs`（供 type test 用）。
4. 对每个输入/输出类型，未规范化和规范化后各算一次 `ImpliedOutlivesBounds`（`add_implied_bounds`，[#L360](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/free_region_relations.rs#L360)；#87748）。
5. 方法/关联常量：再加 impl 头的 `assumed_wf_types`（只用规范化后的）。

`compute_implied_outlives_bounds_inner`（[rustc_trait_selection/.../implied_outlives_bounds.rs#L51](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/query/type_op/implied_outlives_bounds.rs#L51)）：反复取类型的 WF 义务；`'a: 'b` 直接记录；`T: 'a` 经 `push_outlives_components`（[rustc_type_ir/src/outlives.rs#L56](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/outlives.rs#L56)）拆成 region/参数/投影分量。例：参数类型 `&'a &'b T` 的 WF 要求 `&'b T: 'a`，得到 `'b: 'a` 和 `T: 'a`。ADT 的 WF 带上它的 where 子句和推断出的 outlives（`inferred_outlives_of`，[rustc_hir_analysis/src/outlives/mod.rs#L11](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/outlives/mod.rs#L11)，RFC 2093）。

## 5. 外部 region 怎样进入借用检查

- 闭包/inline const 的 universal region 分三段：`'static`、external（父函数 identity 实参和父函数作用域内的 late-bound，`for_each_late_bound_region_in_recursive_scope`，[universal_regions.rs#L949](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/universal_regions.rs#L949)）、local（闭包签名自己的 late-bound，含闭包环境 region `ClosureEnv`）。
- 闭包违反 external 之间的关系时不报错，而是上传给父函数（[closures.md](closures.md)）。
- 父函数在闭包创建点把闭包实参的父段与自己的 identity 实参相等（`prove_closure_bounds`，[type_check/mod.rs#L2495](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L2495)；#98589）。

## 6. 对 kfrc 的含义

- 名称解析（[T05](../../../plan/m1-vertical-chain.md#t05)）要产出：每个 fn 的 lifetime 参数表（显式 + 省略新建，带 early/late 标记）、返回类型的省略结果或 E0106、body 内省略 = 推导变量、static/const 类型里省略 = `'static`。规则只依赖 AST，规模小，可直接移植。
- early/late 判定规则短（只看输入/输出/where 子句），首轮就该实现；否则调用点的 region 实例化与 rustc 不一致。
- 隐含界首轮只需“引用嵌套”和“含 lifetime 参数的 struct 字段”两类：WF + outlives 分量拆分可自写；涉及 trait 投影、`assumed_wf_types`、`inferred_outlives_of` 的部分依赖 [T21](../../../plan/m3-semantics.md#t21) 的 trait/规范化，推迟并显式拒绝。

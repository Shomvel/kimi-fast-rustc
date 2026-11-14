# param env、where 子句、elaboration 与 implied bounds

## param env 的构造

[`param_env`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/ty.rs#L152) 查询：

1. 取 `predicates_of(def_id).instantiate_identity()`：包含父项（impl/trait）的谓词与自身 where 子句。[`predicates_of`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/collect/predicates_of.rs#L49) 对 trait 额外加 `Self: Trait`；每个类型参数默认加 `Sized`（[`add_default_traits`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/collect/predicates_of.rs#L200)；`?Sized` 去掉；1.94.1 的层级是 `Sized: MetaSized: PointeeSized`，`PointeeSized` 在 lowering 时去掉）。
2. 有默认体的 trait 方法：把 RPITIT 的 `NormalizesTo(projection → opaque)` 加进去；条件 const 项加 `[const]` 条件。
3. [`normalize_param_env_or_error`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/mod.rs#L323)：
   - `util::elaborate` 展开全部谓词（supertrait、outlives 分量等）；
   - 不含 alias 则直接返回；
   - 否则（旧求解器）**急切规范化**：先在“未规范化的 elaborated env”里规范化非 outlives 谓词（[`do_normalize_predicates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/mod.rs#L253)，`TypingMode::non_body_analysis`，忽略 region），再在规范化后的 env 里规范化 outlives 谓词；出错就报告并退回未规范化 env；
   - 非 `generic_const_exprs` 且非 globally 时，先把 param env 里的匿名常量求值。

新求解器不依赖急切规范化：param env 候选匹配时经 `ReplaceAliasWithInfer` / `AliasRelate` 惰性规范化，并在**规范化之后**判断 where-bound 是否全局（[`characterize_param_env_assumption`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L1219)），以模拟旧求解器急切规范化的效果（`global-param-env-after-norm.rs`）。

`ParamEnv` 只是 `Clauses` 列表（[`ParamEnv`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/mod.rs#L1030)）；`TypingEnv = (TypingMode, ParamEnv)`（[L1093](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/mod.rs#L1093)）。

## Elaboration

[`Elaborator`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/elaborate.rs#L84)（`rustc_type_ir`，两个求解器共用）：工作栈 + `visited` 集合（按匿名化绑定变量后的 `PredicateKind` 去重），两种模式：

- `Filter::All`：用 `explicit_implied_predicates_of`（supertrait + trait where 子句中约束 `Self` 及其关联类型的谓词）；
- `Filter::OnlySelf`：只用 `explicit_super_predicates_of`；[`supertraits`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/elaborate.rs#L339) = OnlySelf + 只保留 trait 谓词，[`supertrait_def_ids`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/elaborate.rs#L315) 只看 DefId。

[`elaborate` 单步](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/elaborate.rs#L127)：

- trait 谓词（只有正极性）：把父谓词用 `instantiate_supertrait` 代入当前 trait ref（处理绑定变量的平移）；
- `HostEffect`：展开 `[const]` supertrait；
- `TypeOutlives(T: 'a)`：按 outlives 分量拆解（`&'b U: 'a` ⇒ `'b: 'a`、`U: 'a`），`for<'a> T: 'a` 不展开；
- **`Sized` 默认不展开**（[L135](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/elaborate.rs#L135)），由候选匹配时“`Sized` 假设满足 `MetaSized` goal”补上。

`Self: Trait` 也出现在 trait 自身的 param env 里，所以 trait 方法体内 `Self: Super` 来自 elaboration。

## where 子句在求解中的角色

- 被检查的函数：param env 的谓词是**假设**（ParamEnv 候选）。
- 被调用的泛型项：调用方要证明 `predicates_of(callee).instantiate(args)`（wf / 调用义务）。
- impl 候选：impl 的 `predicates_of` 是嵌套 goal（`GoalSource::ImplWhereBound`），决定环的种类。
- trait 定义的 where 子句：`T: Trait` 成立的条件在 impl wf 检查时证明；使用时只经 elaboration 当作 `T: Trait` 的推论。

## Implied bounds

- [`assumed_wf_types`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/implied_bounds.rs#L23)：函数签名的输入/输出类型、impl 头的类型被假定是 well-formed 的。
- [`implied_outlives_bounds`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/outlives_bounds.rs#L34) 从这些类型的 WF 推出 region outlives 关系（例如 `&'a T` ⇒ `T: 'a`），只用于 region 求解（regionck/borrowck），**不进入 trait 求解的 param env**。
- 新求解器的 impl 候选额外加入 `impl_super_outlives`（elaborate 出的 supertrait outlives），注释说明这是余归纳尚未完善时的权宜做法。

## tricky 边界

- caller bound 的顺序可观察（第一个总是适用的 ParamEnv 候选胜出），而顺序来自 `predicates_of` 和 elaboration 的工作栈顺序。
- 全局 where-bound（不含参数，如 `i32: Copy`）不压过 impl，只有全是全局 where-bound 时才用（[candidates](candidates.md)）。
- 旧求解器在 param env 规范化失败时继续使用未规范化 env，并已报错；新求解器在用到时才失败。
- 带绑定变量的 where-bound（`for<'a> T: Tr<'a>`）一律视为非全局。

## 对 kfrc 数据模型的含义

- 需要的派生查询：`predicates_of`（含默认 `Sized`、`Self: Trait`、父项谓词）、`explicit_super_predicates_of`、`explicit_implied_predicates_of`、`item_bounds`（alias bound）。都是确定顺序的谓词列表，可以作为 Interner 风格的接口。
- `ParamEnvId` = 驻留后的有序谓词列表（已 elaborate）。T12 先不做急切规范化，采用新求解器的惰性方式；记录这一点与默认 oracle（旧求解器）的差异风险。
- implied bounds 属于 T22（region/borrowck）；T12/T21 只需 trait 部分。

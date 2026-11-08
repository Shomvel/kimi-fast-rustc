# Goal、canonical 输入输出与 EvalCtxt

新求解器把每个 goal 当作纯函数：`CanonicalInput → QueryResult`。调用方先把 goal canonicalize（去掉推导上下文），求值在全新的推导上下文里进行，结果再 canonicalize 成“对输入变量的约束”，由调用方实例化并合并回自己的推导状态。

## 数据结构

| 类型 | 内容 |
|---|---|
| [`Goal<I, P>`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L43) | `param_env` + `predicate` |
| [`QueryInput`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L105) | goal + `predefined_opaques_in_body`（当前 body 已登记的 opaque 隐藏类型） |
| [`CanonicalQueryInput`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/canonical.rs#L22) | `Canonical<QueryInput>` + [`TypingMode`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/infer_ctxt.rs#L26)；它就是缓存键 |
| [`Canonical`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/canonical.rs#L38) | `value` + `max_universe` + `var_kinds` |
| [`CanonicalVarKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/canonical.rs#L96) | `Ty{ui, sub_root}` / `Int` / `Float` / `Region(ui)` / `Const(ui)` / `Placeholder{Ty,Region,Const}` |
| [`CanonicalVarValues`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/canonical.rs#L226) | 每个 canonical 变量对应的 `GenericArg` |
| [`Response`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L242) | `certainty` + `var_values`（输入变量被约束成什么）+ `external_constraints` |
| [`ExternalConstraintsData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L255) | region 约束、新登记的 opaque、`normalization_nested_goals` |
| [`Certainty`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L290) / [`MaybeCause`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L395) | `Yes` / `Maybe{Ambiguity \| Overflow{suggest_increasing_limit, keep_constraints}}`；`QueryResult = Result<Canonical<Response>, NoSolution>`，三值结果 |

`TypingMode` 五种：`Coherence`、`Analysis{defining_opaque_types_and_generators}`、`Borrowck`、`PostBorrowckAnalysis`、`PostAnalysis`。它决定 opaque 是否刚性、`default` 关联项能否投影、归纳环的结果等，因此是缓存键的一部分。

**合并规则：** 嵌套 goal 用 [`Certainty::and`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L369)（任何 `Maybe` 拉低为 `Maybe`，overflow 压过 ambiguity）；候选之间用 `or`（ambiguity 与 overflow 合并后保留约束）。

## Canonicalizer

[`canonicalizer.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/canonicalizer.rs#L37)，模式 `Input(ParamEnv | Predicate)` 与 `Response{max_input_universe}`。

- **跳过：** 值上没有 [`NEEDS_CANONICAL`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/canonicalizer.rs#L12) 标志（infer/placeholder/param/自由 region/erased）就原样返回。
- **输入（[`canonicalize_input`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/canonicalizer.rs#L197)）：** 先处理 param_env，再处理其余部分并沿用同一变量表。推导变量变成存在变量；泛型参数 `Param` 变成**匿名 placeholder**（[L364](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/canonicalizer.rs#L364)）；所有自由 region（early/late param、placeholder、erased、推导变量）一律变成存在 region 变量；`'static` 只在 param_env 中保留，用来判断 where-bound 是否全局（[`fold_region`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/canonicalizer.rs#L411)）。输入的所有 universe 都压到 ROOT。
- **编号：** 按 fold 遍历中首次出现的顺序；同一个 `GenericArg` 复用同一变量（超过 16 个变量后改用哈希表）。类型变量另记 `sub_root`，即同一个 sub-unification 根的第一个变量。
- **param_env 缓存：** 没有非 region 推导变量的 param_env 的 canonical 结果全局缓存（`canonical_param_env_cache_get_or_insert`）。
- **输出（[`finalize`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/canonicalizer.rs#L282)）：** 响应里调用方能命名的 universe 映射回 ROOT，query 内部新建的 universe 减去 `max_input_universe` 后保留。

**生成响应：** [`evaluate_added_goals_and_make_canonical_response`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L1221) 依次：求完嵌套 goal → 对 query 内新 universe 做 leak check → 若当前是 `NormalizesTo` 且浅层结果为 `Yes`，把未完成的嵌套 goal 交给调用方 → overflow 且 `keep_constraints == false` 时返回不带任何约束的歧义响应 → region 约束**只在 `Yes` 时**返回（[L1339](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L1339)）→ 只返回本次新登记的 opaque → 去重 region 约束 → canonicalize。

**应用响应：** [`instantiate_and_apply_query_response`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/mod.rs#L94)：先为 query 内新建的 universe 在调用方创建对应 universe；存在变量若在响应中原样映射回自己，直接复用调用方原值（Ty 变量因 `sub_root` 不走此捷径，见 [L125](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/canonical/mod.rs#L125)）；placeholder 映射回原值；再把 `var_values` 与原值逐一 unify，登记 region 约束与 opaque，返回 normalization 嵌套 goal。

## EvalCtxt 与嵌套 goal 循环

- [`EvalCtxt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L76)：私有的推导上下文 `delegate`、`var_values`、`max_input_universe`、共享的 `search_graph`、`nested_goals: Vec<(GoalSource, Goal, Option<GoalStalledOn>)>`、`tainted`、[`CurrentGoalKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L43)（`Misc` / `CoinductiveTrait` / `NormalizesTo`）。
- [`enter_canonical`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L350)：从 canonical 输入建全新的 InferCtxt，并登记预定义 opaque。
- [`compute_goal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L564)：先 `enter_forall`（高阶绑定实例化为新 universe 的 placeholder），再按 `PredicateKind` 分派：Trait、HostEffect、Projection、TypeOutlives、RegionOutlives、ConstArgHasType、UnstableFeature、Subtype、Coerce、DynCompatible、WellFormed、ConstEvaluatable、NormalizesTo、AliasRelate、Ambiguous；`ConstEquate` 直接 panic。
- [`evaluate_goal_raw`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L430)：goal 若曾停滞（[`GoalStalledOn`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/mod.rs#L412)）而相关变量、sub-unification 根和 opaque 数量都没变，直接复用上次的 certainty；否则急切解析变量 → canonicalize → `search_graph.evaluate_goal` → 应用响应 → 判断 `HasChanged`。
- [`add_goal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L760)：先用 [`ReplaceAliasWithInfer`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L1384) 把谓词里每个不含逃逸绑定变量的 alias 换成新变量，并加 `AliasRelate` goal，goal 来源记为 `NormalizeGoal(子 goal 的路径种类)`，以免改变环的种类。
- [`try_evaluate_added_goals`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L622)：最多 [`FIXPOINT_STEP_LIMIT = 8`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/mod.rs#L46) 轮，仍有进展就返回 overflow。每轮（[`evaluate_added_goals_step`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L641)）先试 delegate 快路径；`NormalizesTo` 的右侧先换成新变量，求值后再结构化地等同；任何 goal 失败即 `NoSolution` 并把上下文标为 tainted。
- [`probe`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/probe.rs#L108)：每个候选都在快照里求值并回滚，只有 canonical 响应留下。
- **根入口：** [`evaluate_root_goal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L204) 以 `recursion_limit`（默认 [128](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/limits.rs#L38)）作为搜索深度；typeck 侧由 [`FulfillmentCtxt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/solve/fulfill.rs#L47) 反复求值根 goal，直到一轮没有进展。

## tricky 边界

- `?x: Trait`（self 类型是未解析的类型变量）在快路径和候选收集中都直接返回歧义，不靠 impl 猜类型（[`compute_goal_fast_path`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/solve/delegate.rs#L61)）；旧求解器同样如此。
- 只有一个候选适用时，它的响应约束会写回调用方变量，这就是“唯一 impl 引导推导”的来源。
- overflow 响应故意丢弃约束；歧义响应可以携带约束。
- 输入里同一个 region 出现两次也只得到一个变量；`'static` 在 param_env 与谓词中的处理不同。

## 对 kfrc 数据模型的含义

- `CanonicalGoal` 是可驻留、可哈希的值：`(TypingMode, ParamEnvId, Predicate, PredefinedOpaques)`；变量编号必须确定（固定遍历顺序），参数统一映射为 placeholder。这样缓存键与并发调度顺序无关。
- 推导表至少要支持：快照/回滚、universe、类型变量的 sub-unification 根、int/float 变量、变量是否已改变的查询（停滞检测）。
- 首版没有 region 时，`CanonicalVarKind` 仍预留 `Region` 槽，所有 region 视为 erased；以后接入 T22 时不改变键的布局。
- 响应只描述“输入变量的约束”，不包含选中了哪个 impl。单态化（T25）另需选择结果，见 [dyn-and-builtins](dyn-and-builtins.md#codegen-选择)。

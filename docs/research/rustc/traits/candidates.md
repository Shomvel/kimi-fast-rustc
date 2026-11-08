# 候选收集、合并与优先级

## 候选与来源

- [`Candidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L35) = `source` + 该候选的 canonical 响应 + `head_usages`（用到了哪些环头，供 search graph 判断能否忽略）。
- [`CandidateSource`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L138)：`Impl(ImplId)`、`BuiltinImpl(BuiltinImplSource)`、`ParamEnv(NonGlobal|Global)`、`AliasBound(SelfBounds|NonSelfBounds)`、`CoherenceUnknowable`。
- [`BuiltinImplSource`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L224)：`Trivial`（`Sized`/`MetaSized`）、`Misc`、`Object(vtable_base)`、`TraitUpcasting(idx)`。
- [`GoalKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L42) 由 `TraitPredicate`（trait_goals.rs）、`NormalizesTo`（normalizes_to/mod.rs）和 `HostEffect` 实现；每种 builtin 都有一个 `consider_*` 钩子，收集逻辑共用。

## 收集顺序：[`assemble_and_evaluate_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L395)

1. 结构化规范化 self 类型；失败则没有候选。规范化后若是类型变量，只尝试已登记 opaque 的路径（[L982](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L982)），否则没有候选（最终歧义）。
2. `TypingMode::Coherence` 下若 trait ref 不可知（下游/兄弟 crate 或上游未来可能实现），只返回一个歧义的 `CoherenceUnknowable` 候选，并顺带证明 supertrait（[L881](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L881)），见 [coherence](coherence.md)。
3. **alias bound**（[L675](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L675)）：self 是刚性 projection/opaque 时取 `item_self_bounds`，再对 projection 的 self 类型递归取 `item_non_self_bounds`。
4. **param env**（[L634](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L634)）：逐个 caller bound 先做 [`fast_reject_assumption`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/trait_goals.rs#L130)，再在 probe 中把绑定实例化为推导变量后 `eq`，最后**在规范化之后**判断 Global/NonGlobal（[`characterize_param_env_assumption`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L1219)；带绑定变量的一律视为 NonGlobal）。`Sized` 假设可以满足 `MetaSized` goal。
5. `AssembleCandidatesFrom::All` 时：
   - **builtin**（[L520](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L520)）总是收集：按“含 error → auto trait → trait alias → lang item”分派；`Unsize` 另可产生多个结构化候选。
   - **impl**（[L493](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L493)）与 **object**（[L789](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L789)）：只有在不存在“无约束成立的 NonGlobal ParamEnv 或 AliasBound 候选”时才收集（性能优化，coherence 下总是收集）。impl 经 `for_each_relevant_impl` 枚举：先按简化 self 类型取非 blanket impl，再取全部 blanket impl（[context.rs L554](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L554)）；int/float 变量枚举所有整数/浮点简化类型；`default impl` 跳过。
   - `EnvAndBounds` 模式（规范化用）只收集 3、4 两类；env 中没有候选且 self 是 `dyn` 时补 object 候选。

**impl 候选**（[`consider_impl_candidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/trait_goals.rs#L56)）：`DeepRejectCtxt` 快速拒绝 → 极性检查（reservation impl 在 coherence 中歧义、其余模式不算 impl；正负极性不匹配则拒绝）→ 新鲜化 impl 参数并 `eq` trait ref → impl 的 `predicates_of` 以 `ImplWhereBound` 加入，`impl_super_outlives` 以 `Misc` 加入。

## 合并：[`merge_trait_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/trait_goals.rs#L1356)

Coherence 模式下直接 `try_merge_candidates`，失败则 flounder。其余模式按顺序：

1. 有 `BuiltinImplSource::Trivial`（即 `Sized`/`MetaSized` 的 builtin）就直接用它，**不管它是否还有嵌套 goal**（修 #53123，防止 where-bound 延长生命周期）。
2. [`CandidatePreferenceMode::Marker`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L115)（sizedness / auto / default trait）且存在 `AliasBound(SelfBounds)`：只看 alias bound。
3. 存在 NonGlobal ParamEnv：只看 ParamEnv 候选（含 Global）；若某个候选“总是适用”，忽略其他 ParamEnv 候选（含失败者）的环头使用。
4. 存在任意 AliasBound：只看 alias bound。
5. [`filter_specialized_impls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L922)；[`unsound_prefer_builtin_dyn_impl`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/trait_goals.rs#L1328)：object 候选只有 region 约束时丢掉所有 impl 候选（#57893 兼容 hack）。
6. 若全部是 Global ParamEnv，结果记为“经 ParamEnv 证明”；否则丢掉 Global ParamEnv 再合并。

[`try_merge_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/mod.rs#L256)：第一个“`Yes` 且没有任何推导/外部约束”的候选胜出；否则要求所有响应完全相同；否则 [`flounder`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/mod.rs#L301)：没有候选为 `NoSolution`，多个候选为**不带约束**的歧义（合并各 `Maybe` 原因）。

返回值附带 [`TraitGoalProvenVia`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/trait_goals.rs#L1300)（`Misc` / `ParamEnv` / `AliasBound`），`NormalizesTo` 据此限制自己能用哪些候选（见 [normalization](normalization.md)）。

## tricky 边界

- **where-bound 遮蔽 impl：** `fn foo<T: Foo>()` 内 `T: Foo` 由 env 证明，所以 `<T as Foo>::Assoc` 不会经 blanket impl 规范化成 `i32`，保持刚性（`oracle-cases/c09`：两种求解器都报 E0271）。
- **全局 where-bound** 规范化后才判定是否全局：`T: Trait<Assoc = u32>, i32: From<T::Assoc>` 中后者规范化后是全局的。
- 第一个总是适用的 ParamEnv 候选胜出，因此 caller bound 的顺序可观察。kfrc 必须保留 rustc 的谓词顺序（`predicates_of` + elaboration 的顺序）。
- 多个 impl 都适用而响应不同时，结果是**不带约束**的歧义，不会挑一个用来推导。

## 旧求解器对照

[`winnow_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L1833) 的顺序几乎一样：`SizedCandidate`（仅当嵌套必然成立，否则歧义）> 去重 where-bound > Marker 模式下的 self alias bound > 唯一的 NonGlobal where-bound（同时有 Global 则歧义）> alias bound（取最小下标）> object（取最小下标）> upcasting > impl（按特化取舍）> 其余歧义。差别：旧求解器在 `SelectionCandidate` 上直接选出一个（[`SelectionCandidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/traits/select.rs#L98)），并以 `ImplSource` 返回；新求解器比较的是响应是否相同。

## 对 kfrc 数据模型的含义

- 需要一个 trait 数据库：按 trait 分组的 impl 列表，分为 blanket 与按简化类型索引的非 blanket（对应 [`TraitImpls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/trait_def.rs#L102)），以及 [`simplify_type`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/fast_reject.rs#L114) / [`DeepRejectCtxt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/fast_reject.rs#L195)。impl 顺序必须确定（影响诊断，也影响“第一个总是适用”的选择）。
- `CandidateSource` 是稳定、可序列化的值，供诊断、选择结果和缓存证据使用。
- T12 只需 impl、param env 和 `Sized` builtin 三类候选，再加完整的合并规则 1、3、6 与 `try_merge_candidates`；alias bound 在出现 `T::Assoc: Bound` 用法时再加。

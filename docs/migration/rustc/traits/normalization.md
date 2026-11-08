# 关联类型规范化（lazy normalization）

新求解器不在进入求解前规范化类型。alias 一直保留在类型里，需要时通过 goal 求值：

```
Projection(<T as Tr>::A == U)  → AliasRelate(<T as Tr>::A, U, Equate)
AliasRelate(lhs, rhs)          → 两侧 alias 各换成新变量 + NormalizesTo(alias, ?v)，再关联结果
NormalizesTo(alias, ?v)        → 按 alias 种类：projection / inherent / opaque / free / anon const
```

## 三种 goal

- [`compute_projection_goal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/project_goals.rs#L13)：只是改写成 `AliasRelate`（来源 `TypeRelating`）。
- [`compute_alias_relate_goal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/alias_relate.rs#L32)：两侧的 alias 都换成新变量并加 `NormalizesTo`，求完后：两边都不是 alias → 普通 relate；一边仍是（刚性）alias → `relate_rigid_alias_non_alias`；两边都是刚性 alias → 结构化 relate。
- [`compute_normalizes_to_goal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/mod.rs#L28)：断言右侧是完全未约束的变量（[`term_is_fully_unconstrained`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L810)），按 `AliasTermKind` 分派。`NormalizesTo` 只作为 `AliasRelate` 的子 goal 出现，不会泄漏到求解器外。

**右侧替换：** 嵌套 goal 循环里，`NormalizesTo` 的右侧总是先换成新变量，求值后再与原右侧做 `eq_structurally_relating_aliases`（[L641](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L641)）。目的有两个：提高缓存命中，以及避免期望结果影响候选选择。作为补偿，`NormalizesTo` 成功时把自己未完成的嵌套 goal 交还给调用方（`normalization_nested_goals`）。

**结构化规范化：** [`structurally_normalize_term`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/mod.rs#L342) 在“要匹配类型构造器”之前调用（候选收集的 self 类型、alias bound 递归、opaque 参数等）：加 `AliasRelate(term, ?v)`，求值后返回解析结果，可能是具体类型、刚性 alias 或仍是变量。

## projection 规范化

1. 先在 probe 中证明对应的 trait goal，得到 `TraitGoalProvenVia`。
2. [`assemble_and_merge_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/assembly/mod.rs#L1131)：
   - trait 经 **ParamEnv/AliasBound** 证明：只收集 env 与 alias bound 的投影候选（`EnvAndBounds`）。一个都没有 → alias 视为**刚性**，`NormalizesTo` 用 alias 本身实例化右侧（[`structurally_instantiate_normalizes_to_term`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/mod.rs#L113)）。有 ParamEnv 候选时优先于 alias bound。
   - trait 经 **Misc**（impl/builtin）证明：收集全部候选；有 ParamEnv 投影谓词时只留它们；再做特化过滤（允许推导约束）后合并。
   - trait goal 没有结果（歧义/overflow）：直接返回歧义。
3. GAT 的自身参数里有完全未约束的变量时，返回歧义而不是选某个具体实例的候选（#92917）。

**impl 投影候选**（[`consider_impl_candidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/mod.rs#L209)）：负 impl 忽略；`eq` trait ref；impl where-clause 以 `ImplWhereBound` 加入并**立即**求值，失败就不再查 `type_of`（避免 RPITIT 查询环）；加入 GAT 自身的 where-clause（`AliasWellFormed`）；[`fetch_eligible_assoc_item`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/mod.rs#L274) 沿特化图找定义该项的 impl：
- 项被特化隐藏（`default type`）：coherence 中歧义，其余模式视为刚性；
- impl 缺这一项：若 impl 的 self 类型保证不定长，同上处理，否则错误；
- 找到：[`translate_args`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/mod.rs#L960) 把参数从选中 impl 搬到定义 impl，`type_of(item).instantiate(args)` 写入右侧。

## 其他 alias

- **opaque**（[`normalize_opaque_type`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/opaque_types.rs#L16)）：`Coherence` → 加 item bound 与 `Ambiguous`，结果总是歧义；`Analysis/Borrowck` → 定义域内则登记或复用 opaque 存储中的隐藏类型（参数先结构化规范化），定义域外视为刚性；`PostBorrowckAnalysis` → 用已定义的隐藏类型；`PostAnalysis` → 直接 `type_of`。刚性判断见 [`opaque_type_is_rigid`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/mod.rs#L368)。
- **free alias**（[`normalize_free_alias`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/free_alias.rs#L17)）：加 where-clause（`Misc`），右侧 = `type_of`。普通 `type A<T> = ...` 在 HIR 类型 lowering 时直接展开；只有 `lazy_type_alias` 特性或别名内含 opaque 时才生成 `ty::Free`（[hir_ty_lowering L1129](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/hir_ty_lowering/mod.rs#L1129)、[`type_alias_is_lazy`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/collect/type_of.rs#L527)）。
- **inherent 关联类型**（[`normalize_inherent_associated_term`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/normalizes_to/inherent.rs#L18)）：特性门控，非稳定。
- **anon const**：`normalize_anon_const` 调 `evaluate_const`。

## tricky 边界

- 刚性 alias 是合法终态：`fn f<T: Tr>(x: T::A)` 中 `T::A` 永远不会变成具体类型，但可以和自身相等；kfrc 的类型表示必须允许 alias 作为普通类型存在。
- 规范化的 `ImplWhereBound` 是**归纳**步（`normalizes-to-is-not-productive.rs`），即使 trait 是余归纳的。
- 用户写的 where-bound 没有给出关联类型时，不会退回 impl 去规范化（`param-candidate-shadows-project.rs`，`oracle-cases/c09`）。
- 规范化失败（trait 不成立）与“刚性”不同：前者 `NoSolution`。

## 旧求解器对照

旧求解器在类型检查前**急切规范化**：[`AssocTypeNormalizer`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/normalize.rs#L147) 遍历类型，遇到 projection 调 [`opt_normalize_projection_term`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/project.rs#L297)（带 projection cache），不能规范化时换成推导变量加 `Projection` obligation。[`project`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/project.rs#L650) 的候选顺序：param env → trait 定义上的 item bound → object 类型 → impl（[`assemble_candidates_from_impls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/project.rs#L892) 先 `select` trait obligation，再查特化图上的项是否 final）。param env 有候选时同样不再看 impl。

## 对 kfrc 数据模型的含义

- 类型里保留 `Alias(kind, def, args)`；Typed Body 中的类型可以含刚性 alias，单态化后（`PostAnalysis`）再全部规范化。
- 推导表要能创建 term 变量（类型或常量），并支持“结构化关联 alias”（不再展开成 `AliasRelate`）。
- T12 只需 `Projection → AliasRelate → NormalizesTo(projection)` 这条链与 impl/param-env 两类投影候选；opaque、free alias、inherent、GAT、特化全部推迟，但 `TypingMode` 字段从第一天起就放进 goal。

# Search graph：环检测、余归纳、fixpoint、overflow 与缓存

通用实现在 `rustc_type_ir`（[`search_graph/mod.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L575)，1437 行），通过 [`Cx`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L40) / [`Delegate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L60) 与求解器解耦，可以单独做 fuzz（lcnr/search_graph_fuzz）。求解器侧的 `Delegate` 实现只有 149 行（[`search_graph.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/search_graph.rs#L46)）。`sources/rustc-map.md` 只列了后者，核心算法在前者。

## 数据结构

- `SearchGraph { root_depth, stack, provisional_cache }`；每次根求值新建一个，栈结束时必须为空。
- [`StackEntry`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/stack.rs#L19)：`input`、`step_kind_from_parent`、`available_depth`、`provisional_result`、`required_depth`、`heads`（依赖的环头及到环头的路径）、`encountered_overflow`、`usages`、`candidate_usages`、`nested_goals`（供判断全局缓存是否适用）。
- [`PathKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L125)：`Inductive < Unknown < Coinductive`，路径拼接取最大值（[`extend`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L153)）；`ForcedAmbiguity` 只给 fuzz 用。

## 每一步的路径种类：[`step_kind_for_source`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/eval_ctxt/mod.rs#L268)

| [`GoalSource`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/solve/mod.rs#L71) | 路径种类 |
|---|---|
| `ImplWhereBound`，当前 goal 是余归纳 trait | `Coinductive` |
| `ImplWhereBound`，当前 goal 是 `NormalizesTo` | `Inductive`（规范化取出关联项后立即离开 impl） |
| `ImplWhereBound`，其他 trait | `Unknown` |
| `TypeRelating`（含所有 `NormalizesTo` 子 goal） | `Inductive` |
| `NormalizeGoal(k)` | 继承 `k` |
| `Misc`、`AliasBoundConstCondition`、`AliasWellFormed` | `Unknown` |

余归纳 trait = auto trait 或带 `#[rustc_coinductive]` 的 trait（[collect.rs L936](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/collect.rs#L936)）；core 中 `Sized` 与 `MetaSized` 带此属性（[marker.rs L160](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/library/core/src/marker.rs#L160)、L178，按 Homebrew 1.94.1 附带的 rust-src 核对行号）。

**环首轮的临时结果**（[`initial_provisional_result`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/search_graph.rs#L46)）：`Coinductive` → 无约束的 `Yes`；`Unknown` → `overflow(false)`（对外表现为歧义/overflow 错误）；`Inductive` → `NoSolution`，但 coherence 中仍给 overflow。因此**一条环只要含有至少一个余归纳步就成立**（#136824），旧求解器则要求环上每一步都是余归纳 trait。

## [`evaluate_goal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L754) 的顺序

1. **深度：** [`allowed_depth_for_nested`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L275)：子 goal 的可用深度为父深度减 1；父 goal 已遇到 overflow 时改为父深度除以 [`DIVIDE_AVAILABLE_DEPTH_ON_OVERFLOW = 4`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/search_graph.rs#L44)，防止指数爆炸。深度为 0 时进入 [`handle_overflow`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L879)：返回 `overflow(true)`（建议提高 recursion_limit），并把父 goal 标为遇到 overflow。根深度 = `recursion_limit`，默认 128。
2. **provisional cache**（[L1071](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L1071)）：只有“最高环头到当前位置的路径种类”与条目记录的一致才命中；遇到过 overflow 的条目只在当前 goal 已处于同一个环时使用。
3. **全局缓存**（[L1212](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L1212)）：要求条目的 `required_depth` 不超过可用深度，且条目依赖的 `nested_goals` 既不在栈上，也没有会命中的 provisional 条目。
4. **栈上环**（[`check_cycle_on_stack`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L1242)）：按 canonical 输入相等判定；计算从环头到这里的路径种类，记到环头的 usages，返回环头当前的 `provisional_result`，首轮则返回上面的初始结果。
5. 入栈，在 `with_cached_task`（增量依赖跟踪）中运行 [`evaluate_goal_in_task`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L1303)。
6. 出栈后更新父 goal；没有依赖环头 → 写全局缓存（[`insert_global_cache`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L1427)），否则写 provisional cache。

## Fixpoint：[`evaluate_goal_in_task`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L1303)

循环：计算 → 出栈 → 不是环头则结束；[`reached_fixpoint`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L1280)（结果等于上一轮的临时结果；或首轮结果等于该路径种类的初始结果且只有一种使用方式）则 rebase provisional 条目后结束；没有使用 → 结束；结果是不带约束的歧义 → 不再重跑，把歧义传播给依赖者；迭代次数达到 `FIXPOINT_STEP_LIMIT = 8` → `overflow(false)`；否则清除依赖本 goal 临时结果的 provisional 条目，以本轮结果作为临时结果重跑。**每个环头都会重跑**，不只是根（`fixpoint-rerun-all-cycle-heads.rs`）。

## 缓存

- **全局缓存** [`GlobalCache`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/global_cache.rs#L37)：`HashMap<Input, CacheEntry { success, with_overflow: HashMap<depth, _> }>`（[`get`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/global_cache.rs#L75)）；放在 `TyCtxt` 上加锁共享（[L1603](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L1603)）。设计目标是**完全不可观察**；只有环根和不在环中的 goal 进入全局缓存；并发重复插入时断言结果相同。
- **provisional cache**：每个 SearchGraph 私有，条目在其环头结果改变前有效；出栈时 [`rebase_provisional_cache_entries`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/search_graph/mod.rs#L955)。模块注释说明它**可以影响行为**，只要结果仍然正确（`provisional-cache-impacts-behavior.rs`）。
- 生成 proof tree 时不查全局缓存。

## 旧求解器对照

- [`check_evaluation_cycle`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L1145)：栈上有相同 param_env 和 freshened trait 谓词即成环；[`coinductive_match`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L1248) 要求环上**全部**谓词都是余归纳 trait，成立则 `EvaluatedToOk`，否则 `EvaluatedToAmbigStackDependent`。
- fulfillment（ObligationForest）里的环由 [`process_backedge`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/fulfill.rs#L821) 处理：余归纳则成功，否则 `Cycle` 错误。
- 深度按 obligation 的 `recursion_depth` 计，超过即 E0275。实测 `c15`：旧 E0275、新 E0277；`c08`：旧 E0275、新通过。

## 对 kfrc 的含义

- 必须实现：canonical 输入相等的栈上环检测、`PathKind` 与 `GoalSource` 的映射、三种初始结果、fixpoint 重跑与 8 次上限、深度预算与 overflow 时除以 4。这些直接决定接受/拒绝和 E0275。
- 全局缓存可推迟（设计上不可观察，只影响性能）；一旦加入，必须带上 `required_depth`、overflow 分深度存放和 `nested_goals` 适用性检查，否则结果会随求值顺序变化。
- provisional cache 可推迟，代价是少数环形用例的结果可能与 rustc 不同；这类用例单独标记。
- 并行：每个任务各有自己的 SearchGraph，可以共享全局缓存；rustc 已在锁内断言并发插入结果一致，kfrc 可以用同样的约束保证结果与 worker 调度无关。

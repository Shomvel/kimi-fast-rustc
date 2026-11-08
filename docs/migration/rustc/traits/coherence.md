# Coherence：overlap、孤儿规则与负推理

## 入口与流程

[`coherent_trait`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/coherence/mod.rs#L154)（每个有本地 impl 的 trait 一次）：

1. `specialization_graph_of(trait)`（[provider](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/specialize/mod.rs#L386)）：逐个插入 impl；[`Children::insert`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/specialize/specialization_graph.rs#L76) 按简化 self 类型筛出可能的兄弟，对每个兄弟调 `overlapping_trait_impls`。重叠时：marker trait 允许重叠；一方特化另一方则调整图；否则报 E0119（只因 leak check 才重叠的情况报 future-compat lint）。
2. 每个本地 impl：`check_impl`（手动实现限制、marker trait 必须为空、负 auto impl 规则）、[`check_object_overlap`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/coherence/mod.rs#L182)、unsafety、[`orphan_check_impl`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/coherence/orphan.rs#L21)、[`builtin::check_trait`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/coherence/builtin.rs#L30)（Drop/Copy/CoerceUnsized/DispatchFromDyn 等）。

## Overlap：[`overlap`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L240)

1. 先用 `DeepRejectCtxt::relate_infer_infer` 比较两个 impl 头的参数，不可能统一就直接判为不重叠（[`overlapping_trait_impls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L130)）。
2. 新 InferCtxt（默认新求解器，`TypingMode::Coherence`，**空 param env**）；两个 impl 头的参数都换成新推导变量并规范化（[`fresh_impl_header_normalized`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L222)），[`equate_impl_headers`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L346)；不能统一则不重叠。
3. **implicit negative**（`OverlapMode::Stable` 默认启用，[`OverlapMode`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/traits/specialization_graph.rs#L52)）：两个 impl 的 where 子句合在一起，[`impl_intersection_has_impossible_obligation`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L401) 中只要有一条**一定不成立**就判为不重叠。新求解器实现：先对每条做深度为 8 的 `root_goal_may_hold_with_depth`（[L413](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L413)），再整体跑 fulfillment；真错误 → 不重叠；剩下的歧义里 overflow 的谓词记下来用于诊断。
4. leak check 失败 → 不重叠；否则报告重叠，并在新求解器下用 proof tree 计算 intercrate 歧义原因（“上游 crate 可能在未来版本添加 impl”等提示）。
5. `with_negative_coherence` 特性下另有显式负 impl 推理（[`impl_intersection_has_negative_obligation`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L491)），稳定版不用。

**Coherence 模式下求解器的完备性要求：** 不能把“可能成立”判成“不成立”。因此：不可知的 trait ref 产生歧义的 `CoherenceUnknowable` 候选；reservation impl 给出歧义；opaque 规范化总是歧义；特化隐藏的关联项规范化为歧义；归纳环给 overflow 而不是 `NoSolution`；不做候选优先级，只合并或 flounder。

## 可知性：[`trait_ref_is_knowable`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/coherence.rs#L47)

1. 以 `InCrate::Remote` 做孤儿检查成功 → 下游或兄弟 crate 可能实现 → `Conflict::Downstream`（不可知）。
2. trait 是本地的或 `#[fundamental]` → 可知。
3. 以 `InCrate::Local{Proper}` 做孤儿检查成功 → 可知；否则 `Conflict::Upstream`（上游可在次版本中添加非 blanket impl，RFC 1023）。

`oracle-cases/c14`：`impl<T: Display> Mine for T` 与 `impl Mine for Local` 不冲突（`Local: Display` 可知不成立）；与 `impl Mine for Vec<u8>` 冲突（std 将来可能 `impl Display for Vec<u8>`），两种求解器都报 E0119。

## 孤儿规则：[`orphan_check_trait_ref`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/coherence.rs#L221)

规则全文在[源码注释](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/coherence.rs#L130)。算法：`OrphanChecker` 按 `Self, P1, P2, ...` 顺序访问 trait ref 的类型参数（常量参数全部忽略，[L466](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/coherence.rs#L466)），每个类型先 shallow resolve 并惰性规范化，然后（[`visit_ty`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/coherence.rs#L324)）：

| 类型 | 处理 |
|---|---|
| 本地 ADT / 本地 foreign 类型 / principal 为本地 trait 的 `dyn` / `ty::Error` | 找到本地类型 → 成功 |
| `#[fundamental]` ADT（`Box`、`Pin` 等） | 递归进参数 |
| `&T`、`&mut T` | 递归进 `T`（引用也视为 fundamental，[L410](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/coherence.rs#L410)） |
| 推导变量 / placeholder（代表 impl 参数） | Local 模式：**未覆盖的类型参数** → 失败；Remote 模式：视为可能本地 → 成功 |
| 含推导变量的 projection | Local+`Compat`：视为已覆盖（不健全的兼容模式）；Local+`Proper`：未覆盖 |
| 其他（基本类型、元组、数组、指针、非本地 ADT 等） | 记为非本地，继续下一个参数 |

全部参数都非本地 → `NonLocalInputType`（E0117）；先遇到未覆盖参数 → `UncoveredTyParams`（E0210）。[`orphan_check_impl`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/coherence/orphan.rs#L21) 先用 `Proper` 检查，失败再用 [`OrphanCheckMode::Compat`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/coherence.rs#L20)；Compat 通过则只发 lint。trait 本身是本地的则直接通过。auto trait 的 impl 另要求 self 是名义类型。

实测：`c11`（`impl<T> Add<Local> for T`）E0210；`c12`（`Display for Box<Local>` 和 `&Local`）通过；`c13`（`Display for Vec<Local>`）E0117。

## dyn 与 coherence

[`check_object_overlap`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/coherence/mod.rs#L182)：`impl Tr for dyn Sub` 中若 `Tr` 是 `Sub` 的 supertrait（或自身）且 `implement_via_object`，与编译器自动提供的 object impl 冲突，报 E0371（`coherence-impl-trait-for-trait.rs`）。

## 对 kfrc 数据模型的含义

- 需要**所有上游 crate 的 impl 列表**以及 `#[fundamental]`、`is_local`、marker、`implement_via_object` 等属性。这把 coherence 与 metadata（M4）绑在一起；M1/M3 的单 crate 用例只能覆盖本地 impl 和“标准库已有 impl”的近似。
- `TypingMode::Coherence` 必须作为一等模式贯穿候选收集、规范化与 search graph 初始结果，不能只在顶层判断。
- 孤儿检查与可知性检查是同一个访问器的两种模式（Local/Remote），按上表移植即可，规模约 450 行，与 `Interner` 的耦合只有 `is_local`、`is_fundamental`、`trait_is_fundamental`。
- overlap 的 implicit negative 推理依赖完整求解器；T21 之前可以先只做“impl 头不能统一”这一级。

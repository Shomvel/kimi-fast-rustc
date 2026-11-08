# rustc 实现精读：trait 求解、关联类型规范化、coherence 与 dyn

依据官方 rustc 1.94.1（commit [`e408947b`](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1)）源码，2026-11-08 阅读。链接都钉在该 commit；`library/` 的行号按 Homebrew 1.94.1 附带的 rust-src 核对。oracle 结果来自本机同 commit 的 rustc。标“推断”的内容没有直接核实。

| 文件 | 内容 |
|---|---|
| [which-solver.md](which-solver.md) | 1.94.1 默认用哪个求解器、各场景分派、实测分歧、对 oracle 的影响 |
| [goals-and-canonical.md](goals-and-canonical.md) | Goal/TypingMode、canonicalizer、Response、EvalCtxt 与嵌套 goal 循环 |
| [candidates.md](candidates.md) | 候选收集顺序、impl/env/alias/object 候选、合并与优先级 |
| [search-graph.md](search-graph.md) | 环检测、归纳/余归纳、fixpoint、overflow、全局与 provisional 缓存 |
| [normalization.md](normalization.md) | Projection → AliasRelate → NormalizesTo、刚性 alias、opaque/free/inherent |
| [param-env.md](param-env.md) | param env 构造、elaboration、where 子句、implied bounds |
| [coherence.md](coherence.md) | overlap、implicit negative、可知性、孤儿规则、object overlap |
| [dyn-and-builtins.md](dyn-and-builtins.md) | dyn 兼容性、object 候选、vtable 布局、auto trait、Sized/Copy/Clone/Fn/Drop、codegen 选择 |
| [test-cases.md](test-cases.md) | 本目录 oracle 用例（含双求解器结果）与 rustc UI 代表用例 |
| [oracle-cases/](oracle-cases/run.sh) | 15 个自包含用例与复现脚本 |

## 概览

```
HIR typeck / wf / borrowck 登记 obligation
  → FulfillmentCtxt：反复 evaluate_root_goal，直到没有进展           (solve/fulfill.rs)
    → canonicalize(goal, TypingMode, 已登记 opaque)                  (canonical/)
      → SearchGraph::evaluate_goal：深度、provisional/全局缓存、环    (rustc_type_ir::search_graph)
        → EvalCtxt::compute_goal：按 PredicateKind 分派
          → Trait：收集候选（alias bound、env、builtin、impl、object）→ 每个在 probe 中求值 → 合并
          → Projection/AliasRelate/NormalizesTo：惰性规范化
        → 嵌套 goal 循环（≤8 轮）→ canonical Response（约束 + Certainty）
  → 调用方实例化响应并合并到自己的推导状态
```

结果是三值的：`Ok(Yes)`、`Ok(Maybe{Ambiguity|Overflow})`、`Err(NoSolution)`；成功与歧义都可以携带对输入变量的约束。

## 选哪个求解器做参考

**算法参考新求解器（`rustc_next_trait_solver` + `rustc_type_ir` 的 search graph / canonical / elaborate / fast_reject），oracle 同时记录默认（旧求解器）与 `-Znext-solver=globally` 两种结果。** 理由：

1. **与 rustc 运行时解耦：** `rustc_next_trait_solver` 只依赖 `rustc_type_ir`，通过 [`Interner`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/interner.rs#L19)、[`InferCtxtLike`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/infer_ctxt.rs#L139)、[`SolverDelegate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/delegate.rs#L6) 访问外部；旧求解器的 `SelectionContext`/`ObligationForest`/freshening/InferCtxt 内缓存都直接绑在 `TyCtxt`/`InferCtxt` 上。
2. **结构符合 kfrc 的任务模型：** goal 是纯函数 `CanonicalInput → QueryResult`，缓存键不含推导上下文，全局缓存设计成不可观察且允许并发插入（断言结果一致）。
3. **是现状与方向：** stable 的 coherence 已经用它（`unproductive-in-coherence.rs` 注释原文）；rust-analyzer 以 `ra-ap-rustc_next_trait_solver` 复用它（[开发指南](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/src/doc/rustc-dev-guide/src/solve/sharing-crates-with-rust-analyzer.md)）。
4. **目标项目已验证：** ripgrep 15.2.0 在 `-Znext-solver=globally` 下 check 与 build 均通过（[which-solver](which-solver.md)）。

代价：默认 oracle 是旧求解器。15 个探针中有 3 个两种模式结果不同（c08 接受/拒绝相反；c10、c15 错误码不同）；UI 测试中 305 个带双 revision。分歧用例单独归类，见 [which-solver](which-solver.md#对-kfrc-的含义)。

## kfrc 最小闭包与移植建议

### T12（[m1-vertical-chain.md#t12](../../roadmap/m1-vertical-chain.md#t12)）要移植的部分

| 部分 | 参考 | 说明 |
|---|---|---|
| canonical goal 与响应 | [goals-and-canonical](goals-and-canonical.md) | 输入/响应两种模式、首次出现编号、`Param → placeholder`、`sub_root`；首版 region 全部视为 erased 但保留槽位 |
| EvalCtxt 嵌套 goal 循环 | 同上 | `add_goal` 的 alias 替换、≤8 轮、`NormalizesTo` 右侧替换、probe 回滚、停滞检测可后加 |
| 候选：impl、param env、`Sized` builtin | [candidates](candidates.md) | 简化类型索引 + 快速拒绝；合并规则（Trivial > NonGlobal env > impl；Global env 规则）与 `try_merge_candidates` |
| 规范化链 | [normalization](normalization.md) | `Projection → AliasRelate → NormalizesTo(projection)`，`TraitGoalProvenVia`，刚性 alias |
| search graph（无缓存版） | [search-graph](search-graph.md) | 栈上环、`PathKind`、三种初始结果、fixpoint ≤8、深度 128 与 overflow 除以 4 |
| param env | [param-env](param-env.md) | `predicates_of`（默认 `Sized`、`Self: Trait`）+ elaboration，惰性规范化 |
| fulfillment 外层 | [`FulfillmentCtxt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/solve/fulfill.rs#L47) | 根 goal 反复求值到无进展，剩余歧义报 E0283/E0282 类错误 |

**自有表示（草案）：** 类型与谓词采用与 `rustc_type_ir` 同构的种类划分，保证将来可以实现 `Interner`：

```
TyKind: Bool | Int | Uint | Float | Adt(AdtId, Args) | Ref | RawPtr | Tuple | Array | Slice | Never
      | FnDef | FnPtr | Param(ParamIdx) | Placeholder(Universe, Idx) | Infer(TyVar|IntVar|FloatVar)
      | Bound(Canonical, Var) | Alias(Projection|Opaque|Free, AliasId, Args) | Dynamic | Error
Predicate: Trait(TraitRef, Polarity) | Projection(AliasTerm, Term) | NormalizesTo | AliasRelate
         | WellFormed | Ambiguous             (Outlives、HostEffect 等后加)
CanonicalGoal { typing_mode, param_env: ParamEnvId, predicate, opaques, var_kinds, max_universe }
Response { certainty: Yes|Maybe(Ambiguity|Overflow{..}), var_values, constraints, selection? }
```

类型驻留成 `TyId`，`CanonicalGoal` 可哈希、跨任务共享；推导表支持快照/回滚、universe、sub-unification 根。`selection` 只在 `PostAnalysis` 模式下填写（`CandidateSource + impl_args`），供 T25 使用（[dyn-and-builtins](dyn-and-builtins.md#codegen-选择)）。

**T12 需要的依赖闭包：** generics/predicates_of、impl 列表（按 trait 与简化类型索引）、`impl_trait_ref`、关联项 `type_of`、lang item `Sized`、ADT 尾字段（`sizedness_constraint`）、recursion_limit。没有 `TyCtxt`/query：都是 kfrc 语义层的确定性表。

**M1 用例需要 mini core：** `Copy`/`Clone` 对基本类型、`&T`、指针、数组的实现来自 core 的 impl，不是求解器 builtin；`Sized`/`MetaSized` 的余归纳来自 core 的 `#[rustc_coinductive]`。M4 之前的用例要么只用用户 trait，要么带一个声明 lang item 与这些 impl 的 mini core；oracle 侧对应用 `#![no_core]` + `RUSTC_BOOTSTRAP=1`（推断：需要 T12 实测确认可行）。

**可推迟（列入 T12 未覆盖清单）：** alias bound；auto trait；闭包/`Fn*`；dyn/unsize/object 候选；coherence（overlap、孤儿、可知性）；opaque 与 `TypingMode::{Analysis 定义域, Borrowck, PostBorrowckAnalysis}`；高阶约束、universe 与 leak check；region/outlives；GAT；特化；const trait（HostEffect）；负 impl；全局缓存与 provisional cache；proof tree 诊断。

### T21（[m3-semantics.md#t21](../../roadmap/m3-semantics.md#t21)）顺序建议

alias bound 与 elaboration 完整化 → auto trait（余归纳）→ 闭包与 `Fn*` → dyn 兼容性、object 候选与 unsize → coherence：孤儿规则（约 450 行，耦合很小）→ overlap 的 implicit negative（需要 `TypingMode::Coherence` 贯穿全部路径）→ opaque 与 TypingMode 全集 → 高阶约束与 leak check → 全局缓存。**sysroot 额外需要非稳定特性：** 1.94.1 的 core 打开了 `const_trait_impl`、`min_specialization`、`negative_impls`、`auto_traits`（core `lib.rs`），约 40 个 core 源文件使用 `[const]`；要自编 core/alloc/std（M4），T21 必须覆盖 HostEffect goal（`effect_goals.rs`）、特化过滤与负 impl，否则只能在 M4 明确报错。

### T25（[m3-semantics.md#t25](../../roadmap/m3-semantics.md#t25)）需要的信息

`PostAnalysis` 下的选择结果（哪个 impl、impl 参数）；vtable 段算法（`MetadataDSA` + 首个 supertrait 前缀 + 其余 supertrait 带 vptr）、`is_vtable_safe_method`、`impossible_predicates` 判 `Vacant`；dyn 兼容性；`needs_drop`/drop glue 不经 trait 求解。见 [dyn-and-builtins](dyn-and-builtins.md)。

### 与 TyCtxt/Interner 的耦合，能否直接复用 crate

**事实：**
- `rustc_next_trait_solver`（10,002 行）只依赖 `rustc_type_ir`（14,445 行）等；两者 `default-features = false` 时不需要 nightly，rust-analyzer 即以稳定工具链构建（[开发指南](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/src/doc/rustc-dev-guide/src/solve/sharing-crates-with-rust-analyzer.md)）。
- 使用方要提供：`Interner`（70 个关联类型、96 个方法）、`inherent` 模块的 30 个 trait（159 个方法）、`InferCtxtLike`（48 个方法：变量表、universe、region 约束、opaque 存储、probe）、`SolverDelegate`（约 12 个方法，含 leak check、wf goal、关联项查找、常量求值）。`search_graph::{Cx, Delegate}` 已在求解器 crate 内实现。
- rustc 自己的实现很薄（`TyCtxt` 的 `Interner` 实现约 700 行，`InferCtxtLike` 335 行，`SolverDelegate` 315 行），因为背后有完整的查询系统和 `rustc_infer`（11,653 行）。kfrc 要先有对应的语义表和推导表。
- 复用意味着 kfrc 的类型表示就是 `rustc_type_ir::TyKind<KfrcInterner>` 等泛型枚举，并随 rustc 版本迁移；ra-ap 系列按 rust-lang/rust 主干定期发布，与 1.94.1 的对应版本需另行核实（推断）。

**建议：** T12 按上表自写最小求解器（推断规模 2–3 千行），同时做 1–2 天的复用 spike：让 kfrc 的最小类型表实现 `Interner` 子集，接上钉死版本的 `ra-ap-rustc_next_trait_solver`，跑通 oracle c01–c09。T14 用两组数据决定 T21 路线：复用能免费得到 const trait、特化、opaque、coherence、auto trait 等 sysroot 必需的语义；自写保持数据模型完全自有，但这些语义都要逐项移植。复用 crate 不引入 `TyCtxt`/query 运行时，但会让类型表示依赖外部 crate，属于“固定技术决策”层面的选择，需要明确拍板。

### 工作量与风险（推断）

- T12 自写最小版：2–3 千行，主要风险是 canonical 编号与响应应用的细节、`NormalizesTo` 右侧替换、环的初始结果。
- T21 全部自写：求解器约 1 万行量级 + 推导表/泛化/leak check；最大风险是与 rustc 在环、歧义和缓存上的细微差异，以及 sysroot 所需非稳定特性的工作量。
- 复用路线：主要风险是 API 跟随成本和类型表示被外部定义约束；语义差异风险最低。
- 两条路线共同风险：oracle 默认是旧求解器，分歧用例必须单独统计；`-Znext-solver` 是不稳定开关，行为可能随版本变化。

## 对现有计划文件的修正建议

本笔记不改这些文件，只列出建议：

- [sources/rustc-map.md](../../sources/rustc-map.md) 的 trait 行：search graph 核心在 `rustc_type_ir/src/search_graph/`（`rustc_next_trait_solver/src/solve/search_graph.rs` 只是 149 行的 delegate）；还应列出 `rustc_type_ir` 的 `canonical.rs`、`elaborate.rs`、`fast_reject.rs`、`solve/`，以及 `rustc_trait_selection/src/traits/{coherence,dyn_compatibility,vtable}.rs`、`rustc_hir_analysis/src/coherence/`。并注明默认 oracle 是旧求解器。
- T12：输入闭包补 mini core；oracle 记录两种求解器模式；“循环边界”至少包含 c06（overflow）和 c08（两种求解器结果相反）。
- M0：对冻结的 Cargo 版本跑 `-Znext-solver=globally`（ripgrep 已通过）。
- [validation/](../../validation/README.md)：差分结果增加 `solver-divergence` 类别，比较粒度为接受/拒绝 + 错误码集合。
- T21：显式列入 sysroot 需要的 const trait、`min_specialization`、负 impl。

## 旧求解器对照索引

| 概念 | 旧求解器 | 新求解器 |
|---|---|---|
| 选择 | [`SelectionContext::select`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L289)，返回 [`ImplSource`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/traits/mod.rs#L690) | `compute_trait_goal`，返回 canonical 响应 |
| 候选取舍 | [`winnow_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L1833) | [`merge_trait_candidates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src/solve/trait_goals.rs#L1356) |
| 环 | [`check_evaluation_cycle`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L1145)：全部余归纳才成立 | search graph：含一个余归纳步即成立 |
| fulfillment | [`FulfillmentContext`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/fulfill.rs#L61)（ObligationForest） | [`FulfillmentCtxt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/solve/fulfill.rs#L47)（根 goal 循环） |
| 规范化 | 急切：[`AssocTypeNormalizer`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/normalize.rs#L147) + [`project`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/project.rs#L650) | 惰性：`AliasRelate` + `NormalizesTo` |
| param env | 急切规范化 | 惰性，规范化后判全局 |

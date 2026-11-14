# 1.94.1 默认用哪个 trait 求解器

**结论：** 官方 1.94.1 默认在 typeck、wf、borrowck、投影规范化、codegen 选择中用**旧求解器**（`rustc_trait_selection::traits::{select, fulfill, project}`）；只有 **coherence 的 overlap 检查**默认用**新求解器**（`rustc_next_trait_solver`），另有少数内部检查硬编码新求解器。`-Znext-solver` 不带参数等于 `globally`。因此 rustc 默认 oracle 体现的是旧求解器的行为。

## 证据（源码）

| 位置 | 事实 |
|---|---|
| [`NextSolverConfig`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L993) | 默认 `coherence: true, globally: false` |
| [`parse_next_solver_config`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/options.rs#L1666) | `-Znext-solver=no\|coherence\|globally`；不带参数时 `coherence: true, globally: true` |
| [`TyCtxt::next_trait_solver_globally`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L3486) / [`..._in_coherence`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L3490) | 直接读上面两个开关 |
| [`InferCtxtBuilder` 默认值](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L526) | 未调用 `with_next_trait_solver` 的 InferCtxt 一律取 `globally`，默认即旧求解器 |
| [`TraitEngineExt::new`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/engine.rs#L35) | fulfillment 二选一：`NextFulfillmentCtxt` 或旧 `FulfillmentContext`（ObligationForest） |
| [`SelectionContext::select`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L289)、[`evaluate_obligation`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/query/evaluate_obligation.rs#L86) | 按 `infcx.next_trait_solver()` 分派 |
| [`overlapping_impls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L154) | coherence 开关为真（默认）时只跑一次带歧义原因追踪的 `overlap`；关闭时旧求解器跑两遍 |
| [`overlap` 建 InferCtxt](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L265) | `with_next_trait_solver(tcx.next_trait_solver_in_coherence())` + `TypingMode::Coherence` |
| [`orphan_check`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/coherence/orphan.rs#L301) | `infer_ctxt().build(TypingMode::Coherence)`，没有指定新求解器，默认用旧求解器规范化；孤儿规则本身在 `rustc_next_trait_solver::coherence`，两边共用 |
| [`codegen_select_candidate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_traits/src/codegen.rs#L24) | 单态化时选 impl 用 `SelectionContext::select`，默认走旧求解器 |

**硬编码新求解器**（与开关无关）：
- [`impossible_predicates`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/mod.rs#L790)：`vtable_entries` 用它把方法槽判为 `Vacant`。
- [`is_impossible_associated_item`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/mod.rs#L844)（`TypingMode::Coherence`）。
- [`impl_intersection_has_negative_obligation`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/coherence.rs#L491)：只在 `with_negative_coherence` 特性下用到。
- `globally` 模式下另有路径切换，例如 [`compare_impl_item`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/check/compare_impl_item.rs#L255) 改用 `deeply_normalize_param_env_ignoring_regions`，[`with_post_analysis_normalized`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/mod.rs#L1138) 不再预先揭示 opaque。

## 场景对照

| 场景 | 默认 1.94.1 | `-Znext-solver`（=globally） |
|---|---|---|
| HIR typeck / 方法探测 / wf / borrowck | 旧 | 新 |
| param env 规范化（`normalize_param_env_or_error`） | 旧，急切规范化 | 新，惰性（见 [param-env](param-env.md)） |
| coherence overlap 的 implicit negative 推理 | **新** | 新 |
| 孤儿检查中的类型规范化 | 旧 | 新 |
| codegen 选 impl（`codegen_select_candidate`） | 旧 | 新（proof tree 上 winnow，见 [dyn-and-builtins](dyn-and-builtins.md)） |
| vtable 槽的 impossible predicates | 新 | 新 |

## 实测（本机 Homebrew rustc 1.94.1，commit `e408947b`）

- [oracle-cases/](oracle-cases/run.sh) 的 15 个用例中 12 个两种模式结果一致；分歧见 [test-cases](test-cases.md)：
  - `c08`：只有一个余归纳步的环，旧求解器报 E0275，新求解器通过；
  - `c15`：旧求解器报 E0275 overflow，新求解器报 E0277；
  - `c10`：新求解器在 E0038 之外多报一个 E0308。
  - `c06` 的错误码相同，但新求解器报 4 条 E0275，旧求解器报 1 条。
- ripgrep 15.2.0（`e89fff89`，`--offline --locked`）在 `RUSTC_BOOTSTRAP=1 RUSTFLAGS=-Znext-solver=globally` 下 `cargo check --all-targets` 和 `cargo build` 都成功（65 个 fingerprint 全部带该 flag），产物 `rg --version` 能运行。Cargo 因离线依赖不全没有测。
- 仓库内 305 个 UI 测试带 `current/next` 双 revision，`*.next.stderr` 148 个、`*.current.stderr` 137 个：两种求解器在诊断层面差异广泛。

## 对 kfrc 的含义

1. **语义参考与 oracle 分开选：** 算法结构参考新求解器（理由见 [README](README.md#选哪个求解器做参考)）；但 rustc 默认的接受/拒绝结果来自旧求解器。差分测试要同时记录两种模式（`rustc` 与 `RUSTC_BOOTSTRAP=1 rustc -Znext-solver=globally`）。
2. **比较粒度：** 只比较“接受/拒绝 + 错误码集合”，不比较诊断条数和措辞（`c06` 两种模式条数不同）。
3. **分歧单独归类：** 两种模式不一致的用例标为 `solver-divergence`，kfrc 以新求解器为目标，但不计入与默认 oracle 的一致率；真实项目验收（M5）仍以“默认 rustc 能编译”为准，再加一道“`-Znext-solver=globally` 也能编译”的检查。
4. **M0 补一项：** 对冻结的 Cargo 版本跑一次 `-Znext-solver=globally` 的 check/build，确认目标项目不依赖旧求解器独有的行为（ripgrep 已确认）。

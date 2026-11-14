# 类型推导与 trait 求解：RA 的 `hir-ty` 与 next-solver 复用

对应任务：[T06](../../plan/m1/t06-typeck-and-body.md)、[T12](../../plan/m1/t12-trait-experiment.md)、[T21](../../plan/m3-semantics.md#t21)。rustc 一侧见 [typeck/](../rustc/typeck/README.md)、[traits/](../rustc/traits/README.md)。

## RA 的类型推导

- 入口 [`infer_query`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/infer.rs#L120)，头注释直接对照 `rustc_hir_typeck/check.rs`。阶段顺序跟随 rustc：先 `type_inference_fallback`，再检查 cast（[L238–L245](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/infer.rs#L238)，注释说明“为了向后兼容，cast 在 fallback 之后”），再做闭包分析。`infer/` 下的 `coerce.rs`、`cast.rs`、`pat.rs`、`closure/analysis/expr_use_visitor.rs` 都是 rustc 对应文件的近似移植。
- **有意的偏离：**
  - 环：推导查询的 Salsa 环回退是一个空结果加 `has_errors: true`（[`infer_cycle_result`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/infer.rs#L187)）；rustc 报 E0391 一类的循环错误。
  - 诊断：[`InferenceDiagnostic`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/infer.rs#L281) 共 46 种，覆盖“IDE 值得提示”的错误；其余错误只体现为 error 类型。
  - 借用检查：当前 commit 的 `hir-ty/src/mir/` 只有 `lower`、`eval`、`monomorphization`，没有借用检查模块。
  - 常量求值：自有 MIR 解释器（`mir/eval`），不是 CTFE。
  - **求解器：** 所有 trait goal 走 next-solver；而 rustc 1.94.1 的 typeck 默认是旧求解器（[which-solver.md](../rustc/traits/which-solver.md)）。RA 的结果更接近 `-Znext-solver=globally`，不能当 oracle。
- **结论：** 类型推导“不参考（以 rustc 为准）”。RA 的 `infer/` 只在一种情况下值得读：想看 rustc 的 typeck 算法脱离 `TyCtxt`/HIR 后长什么样。

## RA 复用 next-solver 的真实成本

RA 的类型就是 `rustc_type_ir` 的泛型枚举，`DbInterner<'db>` 实现 [`Interner`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/next_solver/interner.rs#L875)，`InferCtxt` 实现 [`InferCtxtLike`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/next_solver/infer/context.rs#L22)，`SolverContext` 实现 [`SolverDelegate`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/next_solver/solver.rs#L53)。

| 部分 | 行数 | 内容 |
|---|---|---|
| `next_solver/` 合计 | 19,838 | 下面各项 + const、valtree、fold、proof tree 格式化 |
| `next_solver/infer/` | 8,182 | rustc_infer 的移植：canonicalizer、instantiate、generalize、lattice、region constraints、type variable、`at`、select |
| `interner.rs` | 2,508 | `Interner` 实现、lang item、impl 枚举 |
| `ty.rs` / `predicate.rs` / `generic_arg.rs` / `def_id.rs` | 1,517 / 910 / 841 / 681 | 驻留类型与 RA ID 的映射 |

[traits/README.md](../rustc/traits/README.md#与-tyctxtinterner-的耦合能否直接复用-crate) 估计 rustc 自己的 `Interner`+`InferCtxtLike`+`SolverDelegate` 实现约 1,350 行，因为背后有 `rustc_infer`。RA 的数字说明：**没有 rustc_infer 的前端要多写约 8k 行推导表，再加约 1 万行类型与映射**。kfrc 的推导表本来就要自写（[types.md](../../design/frontend/types.md#推导表)），所以增量成本小于 2 万行（推断）。

**RA 自己跟不上发布节奏。** master 锁在 `ra-ap-rustc_type_ir` 0.166.0（2026-05-10），crates.io 最新 0.184.0（2026-11-08）。按行首 `fn`/`type` 统计 `pub trait Interner`：1.94.1 为 88 个方法 / 62 个关联类型，0.166.0 为 93 / 69，0.184.0 为 109 / 73；1.94.1 → 0.184.0 方法名增删 51 处（实测；口径与 traits/README 的“96 方法、70 类型”不同）。`rustc_next_trait_solver` 同期 diff 5,591 行。

## 对 kfrc

| 部分 | 档 | 做法 |
|---|---|---|
| 类型推导 | 不参考（以 rustc 为准） | 按 typeck 笔记移植 rustc_hir_typeck；推导表按 types.md |
| trait 求解 | 参考设计、自写（复用待 T14 决策 D2） | 维持 T12：自写最小求解器 + 1–2 天复用 spike |
| mini core | 参考设计、自写 | 借 RA `test-utils/src/minicore.rs`（2,542 行、78 个带依赖的特性标志）的“按标志裁剪”机制 |

**T12 spike 的具体做法：**

1. 钉 `ra-ap-rustc_type_ir` / `ra-ap-rustc_next_trait_solver` `=0.148.0`（与 1.94.1 逐字节相同，stable 1.94.1 可构建，见 [components.md](components.md#实测哪个-ra-ap-版本等于-rustc-1941)）。不要跟 RA 的 0.166。
2. 照 RA 的 `interner.rs` 看哪些 `Interner` 方法在 c01–c09 用例中真被调用；其余先 `unimplemented!` 并记录（只限 spike，产品代码不允许，红线 1）。
3. `InferCtxtLike` 直接接 kfrc 自己的推导表；如果为此要把推导表改成 RA `next_solver/infer` 的形状，记为复用路线的成本。
4. 产出给 T14 的三个数：适配层行数、c01–c15 一致率、`=0.148.0` 之外升级一次的改动量（对 0.150.0 试编即可）。

**mini core：** RA 的 minicore 用 `//- minicore: flag1, flag2` 选子集，标志带依赖（如 `copy: clone`、`fmt: option, result, …`）。kfrc 的 mini core 用同样机制，但内容要能被 rustc 1.94.1 以 `#![no_core]` 编译，作为 oracle 输入；RA 的 minicore 只给 RA 用，未验证能被 rustc 接受（推断），不能直接拿来。

## 要避开的坑

- RA 的 `Ty` 带 `'db` 生命周期，源码注释承认“把生命周期当作建议，按需 transmute”（[next_solver.rs L3–L5](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/next_solver.rs#L3)）。kfrc 用 `TyId` 索引，不要引入这种 unsafe。
- RA 的类型测试（`hir-ide/src/tests/`，31,696 行）期望值是 RA 的输出；可以拿输入，期望值由 rustc 生成，并按 next-solver/旧求解器分两列记录。
- 复用路线下，类型表示由外部 crate 定义，属于固定决策层面的取舍（[README](README.md#需要用户决策)）。

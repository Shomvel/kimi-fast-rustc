# 模式穷尽检查与布局计算：两个“叶子算法” crate

对应任务：穷尽检查目前**没有任务归属**（见文末建议）；布局对应 [T15](../../plan/m2-types-memory-abi.md#t15)。rustc 一侧见 [match-lowering.md](../rustc/ownership/match-lowering.md)、[layout.md](../rustc/layout-abi-mono/layout.md)。

## `rustc_pattern_analysis`

**rustc 的分工。** 算法在 [`compute_match_usefulness`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_pattern_analysis/src/usefulness.rs#L1835)（`usefulness.rs` 1,890 行 + `constructor.rs` 1,219 行），通过 [`PatCx`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_pattern_analysis/src/lib.rs#L44) trait 访问类型系统。rustc 自己的实现 `RustcPatCtxt` 在 `rustc.rs`（1,090 行，`rustc` 特性开启才编译），调用方是 [`check_match`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/pattern/check_match.rs#L34)（1,529 行，E0004/E0005、`let` 可反驳性、绑定检查）。

**`PatCx` 的接口面：** 6 个关联类型（`Ty`、`Error`、`VariantIdx`、`StrLit`、`ArmData`、`PatData`）+ 约 12 个方法，核心是 `ctors_for_ty`（一个类型有哪些构造器：整数区间、bool、char 区间、ADT 变体、slice 长度、引用、`non_exhaustive`、不可居住变体）、`ctor_arity`、`ctor_sub_tys`，其余是诊断与 lint 回调。

**RA 的做法。** [`MatchCheckCtx` 实现 `PatCx`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ide/src/diagnostics/match_check/pat_analysis.rs#L305)，整个 `match_check` 1,020 行。偏离：`type StrLit = Void`（字符串字面量模式不参与精确分析）；只在没有类型错误时报告缺失分支。

**对 kfrc：直接依赖（待决策 D1）。** 理由：

- 接口窄，kfrc 的类型和模式 IR 保持自有，只实现 `PatCx`（参照 RA 的 518 行与 rustc 的 `RustcPatCtxt`，推断 600–900 行）。
- 算法本体约 3k 行，正确性难点在区间分裂、slice 构造器、不可居住类型与 `exhaustive_patterns`、复杂度上限（`complexity_exceeded`），自写的分歧风险高，且对 kfrc 没有设计自由度上的收益。
- 依赖闭包 16 个 crate，无 nightly，0.148.0 与 1.94.1 相同。

不依赖时：照 `usefulness.rs` 顶部的长篇算法说明移植，把 crate 作为 dev-dependency 做差分。无论哪种，`check_match` 的调用逻辑（何时检查、错误码、`let` 与 `if let` 的差别）自写，参照 rustc。

**前提：** 需要冻结一个“THIR 级模式 IR”。match-lowering 笔记已指出 `rustc_mir_build/src/builder/matches/` 与穷尽检查输出强耦合；模式 IR 应同时服务 typeck（[patterns.md](../rustc/typeck/patterns.md)）、穷尽检查和 Body 构建。

## `rustc_abi` 的布局计算

**rustc 的分工。** [`LayoutCalculator<Cx: HasDataLayout>`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L114) 只依赖目标数据布局和字段的 `LayoutData`：`univariant`、`layout_of_struct_or_enum`、`layout_of_union`、`array_like`、`simd_type`、`coroutine`（`layout.rs` 及子模块约 2.3k 行）。类型分派在 `rustc_ty_utils/src/layout.rs`（1,035 行，依赖 `TyCtxt`）。`LayoutData`/`FieldsShape` 对 `FieldIdx`/`VariantIdx` 泛型，源码注释写明是为了 RA 复用。

**RA 的做法。** `hir-ty/src/layout.rs`（574 行）+ `layout/`（202 行）就是分派层：定义 [`Layout = LayoutData<RustcFieldIdx, RustcEnumVariantIdx>`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/layout.rs#L73)，在 `layout_of_ty_query` 里按类型分派，ADT 交给 `calc.layout_of_struct_or_enum`。错误类型有 `HasErrorType`、`NotImplemented`、`Unknown` 等降级出口。

**对 kfrc：直接依赖（待决策 D1），只用 `LayoutCalculator`。**

- 计划已要求布局与 rustc 位级一致、“按 `rustc_abi::layout` 移植”（T15）。逐行移植得到的就是同一份算法；依赖 crate 省掉移植和对拍维护。
- 冲突点：kfrc 的 `LayoutId → LayoutData` 将直接是 `rustc_abi::LayoutData<FieldIdx, VariantIdx>`，L 级 SoN 和 machine-abi 契约会读它的字段（`BackendRepr`、`Scalar`、`Niche`）。如果希望 L 级数据模型自有，就在边界上转换成自有结构（推断：转换几百行，但要跟随字段变化）。
- 不依赖时：逐行移植，crate 作 dev-dependency；用 `#[rustc_layout(debug)]` 输出做第二层对拍（[layout-abi-mono/test-cases.md](../rustc/layout-abi-mono/test-cases.md)）。
- 分派层（类型 → 字段布局、判别值、`repr`、数组长度常量求值、`Pointee::Metadata`）无论如何自写，参照 `rustc_ty_utils::layout`，不参照 RA 的降级出口。

## 要避开的坑

- 关闭 `rustc_abi` 的 `nightly` 特性后没有 `ty` 模块（`TyAndLayout`、`TyAbiInterface`、rustc 自己的 `FieldIdx`/`VariantIdx`，[layout.rs L20–L24](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs#L20)），也没有稳定哈希与编码派生。kfrc 要像 RA 一样自带索引类型，布局的持久化编码也自写。
- 依赖 `rustc_abi` 不等于得到 `extern "C"` 分类：x86_64 SysV 的分类在 `rustc_target/src/callconv/`，不在可独立的 crate 里，仍按 [sysv-x86_64.md](../rustc/layout-abi-mono/sysv-x86_64.md) 自写。
- RA 的 `StrLit = Void`、“有类型错误就不报”都不能照搬：kfrc 有错误类型时整函数已经被拒绝，穷尽检查只在无错误的 TypedBody 上运行。

## 建议

1. 在 plan 中为穷尽与可反驳性检查建一个子项：位置在 T06 之后、Body 构建之前（M1 的 `match` 只有字面量、范围、`_`，已经需要 E0004）；M3 随 enum/slice 补全。
2. 冻结 THIR 级模式 IR 时，按 `PatCx` 的需要（构造器 + 子类型）检查表示是否完备。

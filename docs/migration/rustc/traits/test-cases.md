# 边界用例与 oracle 结果

## 本目录的 oracle 用例（T12 起点）

[oracle-cases/](oracle-cases/) 下 15 个自包含程序；`t12_base.rs` 是 T12 的输入闭包：一个 trait `Tr`、关联类型 `A`、两个 impl（`u32`、`bool`）、`fn via_param<T: Tr>(t: T) -> T::A` 与 `fn via_eq<T: Tr<A = u64>>`，以及跨函数调用。用 `oracle-cases/run.sh [rustc]` 复现，只比较退出码与错误码集合。2026-11-08 在 Homebrew rustc 1.94.1（commit `e408947b`，aarch64-apple-darwin）上的结果：

| 用例 | 内容 | 默认（旧） | `-Znext-solver=globally` | 考察点 |
|---|---|---|---|---|
| c01_ok | 两个 impl + where 约束 + 跨函数调用 | 通过 | 通过 | impl 选择、投影规范化、`via_eq → via_param` 证明 `T: Tr` 来自 env |
| c02_no_impl | `via_param(1i8)` | E0277 | E0277 | 无候选 → `NoSolution` |
| c03_proj_mismatch | `via_eq(true)` | E0271 | E0271 | `<bool as Tr>::A == u64` 规范化后不等 |
| c04_ambiguous | `mk::<_>()`，`T: Tr` 的 self 是变量 | E0283 | E0283 | self 为类型变量直接歧义，不靠两个 impl 猜 |
| c05_overlap | 再加 `impl<T: Copy> Tr for T` | E0119 | E0119 | overlap：`u32: Copy` 成立 |
| c06_inductive_cycle | `impl Tr2 for S where S: Tr2` | E0275（1 条） | E0275（4 条） | `Unknown` 环 → overflow；诊断条数不同 |
| c07_coinductive_auto | 递归链表 `Send` | 通过 | 通过 | auto trait 余归纳 |
| c08_one_coinductive_step | `Foo<T>: Send` 经非余归纳 trait 成环 | **E0275** | **通过** | 新求解器：环中有一个余归纳步即成立 |
| c09_env_shadows_impl | `T: Foo` 来自 env，要求 `Foo<Assoc = i32>` | E0271 | E0271 | env 候选遮蔽 impl，alias 保持刚性 |
| c10_dyn_generic_method | 含泛型方法的 trait 转 `dyn` | E0038 | E0038 + E0308 | dyn 兼容性；后续诊断不同 |
| c11_orphan_uncovered | `impl<T> Add<Local> for T` | E0210 | E0210 | 未覆盖的类型参数在本地类型之前 |
| c12_orphan_fundamental | `Display for Box<Local>`、`&Local` | 通过 | 通过 | `Box`、引用是 fundamental |
| c13_orphan_vec | `Display for Vec<Local>` | E0117 | E0117 | `Vec` 非 fundamental |
| c14_neg_reasoning | blanket `impl<T: Display>` + `Local` + `Vec<u8>` | E0119 | E0119 | `Local: !Display` 可知；`Vec<u8>` 上游可能实现 |
| c15_overflow_depth | `issue-18400` 的递归 impl | **E0275** | **E0277** | 深度溢出的报告方式不同 |

T12 的完成标志（正例、缺 impl、冲突/歧义、循环边界）分别对应 c01、c02/c03、c04/c05、c06/c08。

## rustc UI 测试中的代表用例

路径相对于官方源码树 `tests/ui/`。

**环与 search graph**
- `traits/next-solver/cycles/coinduction/only-one-coinductive-step-needed-trait.rs`：c08 的原型，`current` 报错、`next` 通过。
- `traits/next-solver/cycles/normalizes-to-is-not-productive.rs`：规范化经 impl where 子句是归纳步，必须报错。
- `traits/next-solver/cycles/inductive-cycle-but-err.rs` / `inductive-cycle-but-ok.rs`：provisional cache 与归纳环的交互，前者应为 `NoSolution` 而非 overflow。
- `traits/next-solver/cycles/fixpoint-rerun-all-cycle-heads.rs`：非根的环头也要重跑到 fixpoint。
- `traits/next-solver/cycles/provisional-cache-impacts-behavior.rs`：provisional cache 会改变证明树（结果仍正确）。
- `traits/next-solver/cycles/unproductive-in-coherence.rs`：coherence 中归纳环给 overflow；注释写明“coherence already uses the new solver on stable”。
- `traits/next-solver/cycles/cycle-modulo-ambig-aliases.rs`：`ReplaceAliasWithInfer` 的作用。
- `sized/coinductive-1.rs`：`Sized` 余归纳，依赖 `NormalizeGoal(path_kind)` 继承路径种类。
- `traits/issue-18400.rs`：深度溢出（c15 的来源）。

**候选优先级与规范化**
- `traits/next-solver/normalization-shadowing/param-candidate-shadows-project.rs`：env 遮蔽 impl 投影（c09）。
- `traits/next-solver/normalization-shadowing/alias-bound-shadowed-by-env.rs`：elaborate 出的 where-bound 不应阻止经 item bound 规范化。
- `traits/next-solver/global-param-env-after-norm.rs`：where-bound 是否全局要在规范化后判断。
- `traits/lifetime-incomplete-prefer-sized-builtin-over-wc.rs`：`Sized` builtin 优先于 where-bound（两种求解器 stderr 不同）。
- `traits/dyn-any-prefer-vtable.rs`：`dyn Any` 优先 object 候选而不是 blanket impl。
- `traits/copy-guessing.rs`：唯一 impl 引导推导的副作用。
- `associated-types/associated-types-binding-in-where-clause.rs`、`associated-type-projection-from-supertrait.rs`：where 子句中的关联类型等式、supertrait 上的投影。
- `associated-types/associated-type-projection-ambig-between-bound-and-where-clause.rs`：两个 bound 都有同名关联类型时的歧义（HIR 层报错）。
- `generic-associated-types/no-incomplete-gat-arg-inference.rs`：GAT 参数未约束时不规范化。

**Coherence**
- `coherence/coherence-blanket-conflicts-with-blanket-unimplemented.rs`：两个 blanket impl 即使条件都不成立也冲突（下游可能实现）。
- `coherence/coherence-bigint-param.rs`（aux crate）：E0210。
- `coherence/coherence-fundamental-trait-objects.rs`：`#[fundamental]` trait 的 `dyn` 不算 fundamental 类型。
- `coherence/const-generics-orphan-check-ok.rs`：孤儿检查忽略常量参数。
- `coherence/coherence-impl-trait-for-trait.rs`：E0371，与自动 object impl 冲突。
- `traits/next-solver/coherence/trait_ref_is_knowable-normalization-1.rs`：可知性检查前先规范化。
- `traits/trivial-unsized-projection-in-coherence.rs`：coherence 中缺失的关联项规范化为歧义。

**dyn 兼容性**
- `dyn-compatibility/generics.rs`、`sized-2.rs`、`mentions-Self.rs`、`supertrait-mentions-Self.rs`、`by-value-self.rs`、`associated-consts.rs`：各类违规与 `where Self: Sized` 豁免。
- `traits/next-solver/dyn-incompatibility.rs`：新求解器下的 dyn 不兼容报错。

## 规模提示

- `tests/ui/traits/next-solver/` 下 130 个条目；`tests/ui/coherence/` 294 个文件；`tests/ui/associated-types/` 402 个。
- 305 个 UI 测试带 `current/next` 双 revision；它们是“求解器分歧”类别的现成来源，可以整体纳入 kfrc 的差分语料并按 revision 分别记录期望。

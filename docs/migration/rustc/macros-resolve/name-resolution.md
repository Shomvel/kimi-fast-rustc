# 名称解析：定义收集、模块图、作用域与可见性

源码：`compiler/rustc_resolve/src/{lib.rs,def_collector.rs,build_reduced_graph.rs,ident.rs,macros.rs,late.rs,effective_visibilities.rs}`，定义表 `compiler/rustc_hir/src/definitions.rs`。导入不动点单独见 [imports.md](imports.md)，卫生规则见 [hygiene.md](hygiene.md)。

## 两个阶段

- **早期解析**（展开期间，每个新片段一次）：DefCollector 建 `LocalDefId`，BuildReducedGraphVisitor 把名字种进模块、登记导入和占位；宏路径和导入在这里推测性解析，可以返回 Undetermined。
- **收尾 + 晚期解析**：[`resolve_crate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L1847) 依次 `finalize_imports` → `compute_effective_visibilities` → `lint_reexports` → [`finalize_macro_resolutions`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L829)（用最终状态重解析每个宏路径，结果与展开时不同即报错）→ `late_resolve_crate`（函数体、类型、模式里的路径）→ `resolve_main` → `check_unused` → `report_errors`。

## 定义收集与 DefKey

- [`collect_definitions`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/def_collector.rs#L18)：父定义取自占位记录的 `InvocationParent { parent_def, impl_trait_context, in_attr, const_arg_context }`（[lib.rs L193](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L193)）；遇到占位只记录 [`visit_macro_invoc`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/def_collector.rs#L104)。
- [`Definitions::create_def`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir/src/definitions.rs#L388)：`DefKey { parent, DisambiguatedDefPathData { data, disambiguator } }`，`DefPathHash` = 父哈希 + key 的稳定哈希。disambiguator 来自 [`DisambiguatorState`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir/src/definitions.rs#L101)：按 `(parent, data)` 计数，**按创建顺序**分配。
- DefKey 里没有展开或卫生信息。两个不同展开产出的同名项（例如 2.0 宏卫生地生成的同名 `fn`）靠创建顺序的 disambiguator 区分。

## 模块图

- [`ModuleData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L603)：`parent`、`kind`（[`ModuleKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L521)：`Block` 匿名模块 | `Def(DefKind, DefId, name)`，enum 和 trait 也是模块）、`lazy_resolutions: Map<BindingKey, NameResolution>`、`unexpanded_invocations`、`no_implicit_prelude`、`glob_importers`、`globs`、`traits` 缓存、`expansion`、`self_decl`。外部 crate 的模块按需从 `module_children` 填充（[`build_reduced_graph_external`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L246)）。
- [`BindingKey { ident: Macros20NormalizedIdent, ns, disambiguator }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L562)：`disambiguator` 只给 `_` 用（`const _` 可以有多个，查不到但可被 glob 带走）。
- 名字声明 [`DeclData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L807)：`kind`（[`DeclKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L838)：`Def(Res)` | `Import { source_decl, import }`）、`ambiguity`、`expansion`、`vis`、`parent_module`。
- 命名空间 [`Namespace`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir/src/def.rs#L672) = Type/Value/Macro；MacroNS 再分 bang 与 attr/derive 两个子空间（[`sub_namespace_match`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L85)）。单元/元组结构体同时在 Type 和 Value 定义。
- [`BuildReducedGraphVisitor`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L235)：[`build_reduced_graph_for_item`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L809) 定义名字（[`define_local`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L61)）；`use` 树 → [`add_import`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L502)；含项或宏调用的块建匿名模块（[`block_needs_anonymous_module`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L493)）；占位 → [`visit_invoc`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L1220) 记录 [`ParentScope { module, expansion, macro_rules, derives }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L172)，项位置的还加入 `module.unexpanded_invocations`。

## 作用域遍历 `visit_scopes`

[`visit_scopes`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L52) 按 [`ScopeSet`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L154) 依次访问 [`Scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L118)：

| 命名空间 | 顺序 |
|---|---|
| Type | 模块非 glob → 模块 glob →（块向外到最近的非块模块）→ `ExternPreludeItems` → `ExternPreludeFlags` → `ToolPrelude` → `StdLibPrelude` → `BuiltinTypes` |
| Value | 模块非 glob → 模块 glob →（同上）→ `StdLibPrelude` |
| Macro | `DeriveHelpers`（沿展开链）→ `DeriveHelpersCompat` → `MacroRules` 链 → 模块（同上）→ `MacroUsePrelude` → `StdLibPrelude` → `BuiltinAttrs` |

- 从具名 `mod` 不向父模块继续（项不继承外层模块的名字）；只有块会走到外层，2.0 宏会经 `hygienic_lexical_parent` 跳到定义处模块。到达最外层后 ctxt 调整到根再查前奏。
- `#![no_implicit_prelude]` 关掉前奏；MacroNS 仍访问 `StdLibPrelude`，但只接受内建宏（[`resolve_ident_in_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L516)）。
- 局部变量和泛型参数在 late 阶段先查 rib（[`resolve_ident_in_lexical_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L305)）：倒序走 rib；`RibKind::Block(Some(module))` 处查块内项（块内项在整个块可见，局部变量声明后才遮蔽它们）；遇到 `RibKind::Module` 改用 `ScopeSet::All`。

**推测模式与歧义**：[`resolve_ident_in_scope_set`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L392) 在早期（非 late finalize）找到最内层结果后继续向外，[`maybe_push_ambiguity`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L759) 判断最内层是不是“弱”结果，产生 [`AmbiguityKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L884)：`BuiltinAttr`、`DeriveHelper`、`MacroRulesVsModularized`、`GlobVsOuter`、`GlobVsGlob`、`GlobVsExpanded`、`MoreExpandedVsOuter`（基于展开偏序的 [`may_appear_after`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L1039)）。late 阶段第一个结果即返回。

**Undetermined 的来源**（[`Determinacy`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L105)）：MacroRules 链上有未展开的 `Invocation`；模块还有 `unexpanded_invocations`；有待定的单项导入可能定义该名；glob 来源模块未解析；crate 根仍有未展开调用时查 `ExternPreludeItems`/`MacroUsePrelude`（`extern crate` 可能由宏产生）。

## 路径

- [`resolve_path_with_ribs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L1701) 逐段解析；首段特判（[L1780](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L1780)）：`self`、`::`（按 `::` token 的 span edition：2018+ → `ExternPrelude`；2015 宏在 2018 crate → `ModuleAndExternPrelude(crate_root)`；2015 → crate 根）、`crate`、`$crate`。中间状态 [`ModuleOrUniformRoot`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L448)，结果 [`PathResult`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L468) = `Module` | `NonModule(PartialRes)` | `Indeterminate` | `Failed`。`PartialRes` 允许 `Type::Assoc` 这种只解析前缀、余下留给类型检查。
- 宏路径 [`resolve_macro_or_delegation_path`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L728)：单段 → `ScopeSet::Macro(kind)`；多段 → `MacroNS` 路径解析；都记入 `single/multi_segment_macro_resolutions` 供收尾复核。[`smart_resolve_macro_path`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L549) 再检查宏种类、内属性、稳定性。

## 前奏层次

- extern prelude：`--extern` 名字（Flags）+ crate 根的 `extern crate` 项（Items）。
- std prelude：注入的 `#[prelude_import] use std::prelude::rust_20xx::*;`（`no_std` → core，`no_core` → 无）记为 `self.prelude`。注入的 `extern crate std;` **没有** `#[macro_use]`（[tests/ui/unpretty/exhaustive.expanded.stdout L34–L36](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/unpretty/exhaustive.expanded.stdout#L34)），所以 `println!`、`vec!` 等经 `StdLibPrelude` 找到（推断：library 的 prelude 模块重导出了这些宏；library 不在本源码中）。
- 语言前奏：内建类型、内建属性（[`builtin_attr_decls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L1616)）、工具模块（`rustfmt`、`clippy`、`diagnostic`、`#![register_tool]`）。

## 晚期解析

[`LateResolutionVisitor`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L752)：每个命名空间一套 rib 栈 + `label_ribs`；[`RibKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L195)：`Normal`、`Block`、`AssocItem`、`FnOrCoroutine`、`Item`、`ConstantItem`、`Module`、`MacroDefinition`、`ForwardGenericParamBan`、`ConstParamTy`、`InlineAsmSym`；rib 表 `FxIndexMap<Ident, Res>`（[L292](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L292)）。[`smart_resolve_path`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L4360) 把结果写入 `partial_res_map` 供降低使用；方法调用处收集 `traits_in_scope`（[lib.rs L1870](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L1870)）给类型检查。

## 可见性

- [`resolve_visibility`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L367)：`pub` → `Public`；不写 → `Restricted(最近的父 mod)`，enum 变体和 trait 项则继承 enum/trait 的可见性；`pub(crate)`/`pub(super)`/`pub(in path)` → `Restricted(模块)`。查找时不可访问的非 glob 绑定直接给出确定的失败。
- [`compute_effective_visibilities`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/effective_visibilities.rs#L70)：沿模块与重导出链迭代到不动点，得到每个定义在 [`Level`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/middle/privacy.rs#L20)（Direct/Reexported/Reachable/ReachableThroughImplTrait）上的有效可见性。resolver 只算基于名字与重导出的部分；类型接口泄露的可达性由 `rustc_privacy` 补（推断，未细读）。

## 对 kfrc（T05）的含义

- 模块表键 = `(Macros20 规范化的 name+ctxt, ns, '_'序号)`，值是 `NameResolution`（非 glob、glob、待定单项导入集合）。T05 现在就按这个形状做，M3 加宏时只需把 ctxt 从“恒为根”变成真实值。
- 解析接口返回三态 `Ok | Determined 失败 | Undetermined`，并区分“推测”与“finalize”两种调用；展开结束后所有宏路径与导入用 finalize 模式复核。T05 的“先登记声明再检查函数体”对应 early/late 两阶段，early 部分必须能被按片段反复驱动。
- T05 完成标志“完成顺序变化不改变 DefKey”：rustc 的 disambiguator 依赖创建顺序，而创建顺序由确定的展开队列保证；kfrc 若并行收集片段，需要用片段在展开树中的位置定序。[identities.md](../../architecture/identities.md) 要求 DefKey “携带宏卫生”，rustc 并不这样做，建议在 T02 明确取舍。

# 展开主循环、属性顺序与 cfg

源码：`compiler/rustc_expand/src/{expand.rs,base.rs,config.rs,placeholders.rs,module.rs}`，入口 `compiler/rustc_interface/src/passes.rs`，派生入口 `compiler/rustc_builtin_macros/src/derive.rs`，解析侧 `compiler/rustc_resolve/src/macros.rs`。

## 入口顺序

[`configure_and_expand`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L132)：

1. crate 级属性先过 [`pre_configure_attrs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/config.rs#L156)（展开 `cfg_attr`，遇到假的 `#![cfg]` 截断），再算 [`features`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/config.rs#L46)；`recursion_limit` 缺省 128（[limits.rs L36](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/limits.rs#L36)）。
2. [`register_builtin_macros`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L151) 把内建展开器登记进 resolver，再 [`standard_library_imports::inject`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/standard_library_imports.rs#L11) 注入 `extern crate std;` 和 `#[prelude_import] use std::prelude::rust_20xx::*;`。
3. [`monotonic_expander().expand_crate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L218) → [`fully_expand_fragment(Crate)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L493)。
4. test/proc-macro harness 注入，然后 [`resolver.resolve_crate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L304)（finalize + late resolution，见 [name-resolution.md](name-resolution.md)）。

## 数据结构

- [`Invocation { kind, fragment_kind, expansion_data }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L392)；[`InvocationKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L398)：`Bang { mac, span }`、`Attr { attr, pos, item, derives }`、`Derive { path, is_const, item }`、`GlobDelegation`（不稳定 `fn_delegation`）。
- [`ExpansionData { id: LocalExpnId, depth, module, dir_ownership, lint_node_id, is_trailing_mac }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L1216)。`id` 在收集时用 `LocalExpnId::fresh_empty()` 分配，`ExpnData` 在解析成功后才填（见 [hygiene.md](hygiene.md)）。
- `AstFragmentKind` 共 20 种（[ast_fragments! L195–L283](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L195)）。[`supports_macro_expansion`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L300)：Expr/Stmts/Ty/Pat 可放宏调用和属性宏（不支持内属性）；Items/各类 AssocItems/ForeignItems/Crate 支持内属性；Arms、字段、参数、泛型参数、变体、where 谓词只接受 `cfg` 和惰性属性。
- [`SyntaxExtension`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L827)：`kind`、`edition`（定义 crate 的）、`helper_attrs`、`builtin_name`、`local_inner_macros`、`allow_internal_unstable` 等；[`SyntaxExtensionKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L735)：`MacroRules`、`Bang`/`Attr`/`Derive`（token 型，过程宏）、`LegacyBang`/`LegacyAttr`/`LegacyDerive`（AST 型，内建宏）、`NonMacroAttr`（惰性属性）、`GlobDelegation`。
- [`ExpandResult::{Ready, Retry}`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L251)；[`Indeterminate`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L1070)；与 resolver 的接口是 [`trait ResolverExpand`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L1082)（`visit_ast_fragment_with_placeholders`、`resolve_imports`、`resolve_macro_invocation`、`resolve_derives`/`take_derive_resolutions`、`cfg_accessible` 等）。

## 主循环 `fully_expand_fragment`

1. [`collect_invocations`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L646)：`InvocationCollector` 遍历片段，把每个宏调用替换成 [`placeholder`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/placeholders.rs#L12)，其 `NodeId = placeholder_from_expn_id(新 LocalExpnId)`；随后调用 `resolver.visit_ast_fragment_with_placeholders`，即对这个片段跑 DefCollector + BuildReducedGraph，把名字种进模块，并为每个占位记录 `invocation_parent_scopes`。
2. `resolve_imports()`：先把能定的导入定下来，多数导入宏可以直接解析。
3. 工作栈（`invocations.reverse()` 后从尾部弹出，按源码顺序处理）：没有 ext 就 [`resolve_macro_invocation(invoc, eager_root, force)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L539)；`Err(Indeterminate)` 放进 `undetermined_invocations`。
4. [`expand_invoc`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L720)：`Ready(fragment)` 时先 [`take_derive_resolutions(expn_id)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L566)，为每个 derive 路径建 `Derive` 调用并在输出里加占位；再对输出 `collect_invocations`；派生调用**先于**输出中的宏调用出栈处理（[L605](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L605)）。输出按 depth 存进 `expanded_fragments`。`Retry` 进 undetermined。
5. 栈空：再 `resolve_imports()`；undetermined 为空就结束，否则换入并置 [`force = !progress`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L520)。一整轮没有任何展开成功就进入 force 模式：解析失败变成确定的错误，宏产出 dummy 片段做恢复。
6. 结束后 [`PlaceholderExpander`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L625) 由深到浅把片段替换回占位。

递归限制按 `depth` 检查（[L730](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L730)）：超限报错、限制减半、返回 dummy。

**急切展开**：`format_args!`、`concat!`、`env!`、`include_*!` 用 [`ecx.expander()`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/base.rs#L1291)（`monotonic = false`）立即展开参数里的宏：不调用 `visit_ast_fragment_with_placeholders`/`resolve_imports`，父作用域继承 `eager_expansion_root`（[macros.rs L247](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L247)）。参数宏暂时解析不了时内建宏返回 `Retry`。

## InvocationCollector 与属性顺序

[`flat_map_node`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L2254) 循环调用 [`take_first_attr`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L2107)：

- 按顺序扫属性，跳过 doc 注释和已标记的惰性属性；**只要出现 `cfg`/`cfg_attr` 就优先处理它**（不管前面有没有属性宏）；否则取第一个不是内建属性名（`is_builtin_attr_name`）的属性，同时收集它后面 `#[derive]` 里的路径，供旧式 derive helper 解析。
- `cfg`：[`expand_cfg_true`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L2210) 为真则原位留 `cfg_trace` 并继续循环；为假则删除节点，并把节点声明的名字登记给 `append_stripped_cfg_item`（仅用于“被 cfg 掉了”的诊断）。
- `cfg_attr`：原位插入展开出的属性后继续循环（嵌套 `cfg_attr` 由循环逐层处理）。
- 其他属性：`collect_attr` 生成 `Attr` 调用。解析结果决定行为：真属性宏；`LegacyAttr` 内建（`derive`、`test`、`cfg_eval`、`global_allocator`…）；`NonMacroAttr`（工具属性、derive helper 等）则把属性插回原位并记入 `expanded_inert_attrs`，不再收集（[L892](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L892)）。
- 宏调用节点 → `collect_bang`；宏调用上的属性只做检查。
- `mod foo;` 在收集器里加载（[expand.rs L1361–L1395](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L1361)、[`parse_external_mod`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/module.rs#L51)）：读文件是展开的一部分，被 cfg 掉的模块不会被读取。

tricky：凡不是内建属性名的属性（含 `#[rustfmt::skip]` 这类工具属性和 derive helper）都要走一轮“解析 → NonMacroAttr”调用，所以名称解析在展开期间就要能回答属性路径。

## `#[derive]` 流程

1. `derive` 本身是 `LegacyAttr` 内建：[`derive::Expander`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/derive.rs#L14) 调 [`resolver.resolve_derives`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L374)，闭包为每个路径生成 `DeriveResolution`，`item` 是 `cfg_eval` 过的项（字段上的 `cfg` 先剥离），多个 derive 共享同一份。任一路径未定就整体 `Retry`。
2. resolver 一次性解析容器内所有 derive 路径，登记 helper 属性（按 derive 序排序）和 `has_derive_copy`；原 item 原样返回。
3. 主循环取回结果，每个路径成为独立的 `Derive` 调用，输出**追加**在 item 后。
4. 同一 item 上的多个 `#[derive]` 属性逐个处理：第一个 derive 的输入里还带着后面的 `#[derive(..)]`（[tests/ui/proc-macro/derive-expand-order.stdout](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/derive-expand-order.stdout)）。

## cfg 剥离（config.rs）

- [`StripUnconfigured`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/config.rs#L36)：`configure` = `process_cfg_attrs` + `in_cfg`；[`expand_cfg_attr`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/config.rs#L289) 先留 `cfg_attr_trace`，谓词为真时按需递归展开；`config_tokens = true`（`cfg_eval`、derive 输入）时连 token 流里的 `cfg` 一并剥。
- 谓词求值走 `rustc_attr_parsing::eval_config_entry`；`cfg!` 内建宏展开成布尔字面量。

## 对 kfrc 的含义

- 展开是“调用记录 + 工作队列 + 不定重试 + force 兜底”的不动点，不是递归下降。每条调用在收集时就要冻结父作用域快照（模块、macro_rules 作用域、所在展开、derive 列表），否则重试时语义会漂移。
- rustc 用占位节点再回填 AST。kfrc 可让 AST 持有 `MacroSlot(ExpnId)`，片段存在按 ExpnId 索引的 arena 里，省掉回填；但“片段产出后立刻把名字种进 resolver”这一步必须保留。
- DefPath disambiguator 按创建顺序分配（见 [name-resolution.md](name-resolution.md)），并行展开时必须给出与队列顺序无关的确定性编号。
- `GlobDelegation`、`macro_rules` 的 attr/derive 规则等不稳定能力可以明确拒绝。

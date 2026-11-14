# Hygiene：SyntaxContext、ExpnData 与名字比较

源码：`compiler/rustc_span/src/{hygiene.rs,lib.rs,symbol.rs}`；使用点在 `compiler/rustc_resolve/src/{ident.rs,late.rs,lib.rs}` 和 `compiler/rustc_expand/src/mbe/transcribe.rs`。官方说明见 rustc-dev-guide [macro-expansion.md#hygiene-and-hierarchies](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/src/doc/rustc-dev-guide/src/macro-expansion.md)。

## 数据结构

- [`ExpnId { krate, local_id }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L105)，本 crate 用 [`LocalExpnId`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L120)。收集调用时 [`fresh_empty`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L222) 分配，解析出宏后 [`set_expn_data`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L255) 填数据（[macros.rs L315](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L315)）。
- [`ExpnData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L992)：`kind`（[`ExpnKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L1139)：`Root`/`Macro(MacroKind, name)`/`AstPass`/`Desugaring`）、`parent`（宏调用所在的展开，构成展开树）、`call_site`、`def_site`、`edition`（定义 crate 的）、`macro_def_id`、`parent_module`、`allow_internal_unstable`、`local_inner_macros`、`collapse_debuginfo`。展开树判祖先用 [`is_descendant_of`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L317)。
- [`SyntaxContext(u32)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L52) 是“标记链” `(ExpnId, Transparency)*` 的驻留 ID；[`SyntaxContextData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L65) = `{ outer_expn, outer_transparency, parent, opaque, opaque_and_semiopaque, dollar_crate_name }`，键 `(parent, expn, transparency)`。不实现 `Ord`（增量确定性）。
- [`Transparency`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L186)：`Transparent`（总在调用处解析）、`SemiOpaque`（局部变量、标签、`$crate` 在定义处，其余在调用处——`macro_rules` 与多数内建宏）、`Opaque`（总在定义处——`macro` 2.0、proc macro 的 `def_site`）。
- `Span` 携带 ctxt。`Ident` 的相等和哈希是 `name + ctxt`（[symbol.rs L2640](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/symbol.rs#L2640)）；包装类型 [`MacroRulesNormalizedIdent`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/symbol.rs#L2753)、[`Macros20NormalizedIdent`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/symbol.rs#L2781) 在类型上标明已规范化。

## 算法

- [`alloc_ctxt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L577)：按键查表，新建时同时算出两个规范化形式：`opaque` 去掉 Transparent 和 SemiOpaque 标记，`opaque_and_semiopaque` 只去掉 Transparent 标记。
- [`normalize_to_macros_2_0`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L449) = 取 `opaque`；[`normalize_to_macro_rules`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L454) = 取 `opaque_and_semiopaque`。都是 O(1) 查表。
- [`apply_mark(ctxt, expn, t)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L538)：`Opaque` 直接追加；否则取 `call_site` 的 ctxt（SemiOpaque 用 macros_2_0 规范化，Transparent 用 macro_rules 规范化），若为根就直接追加；不为根说明是“macro 2.0 里调用 macro_rules”，把 ctxt 的全部标记重放到调用处 ctxt 上再追加，使外层 2.0 宏保持卫生（[tests/ui/hygiene/legacy_interaction.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/legacy_interaction.rs)）。
- [`adjust(ctxt, expn)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L530)：去掉所有“不是 `expn` 祖先”的外层标记，返回最后去掉的标记（用于隐私检查的定义作用域）。[`glob_adjust`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L840)/[`reverse_glob_adjust`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L868) 处理带卫生的 glob 导入。
- Span 便捷函数（[lib.rs L1256–L1270](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/lib.rs#L1256)）：`with_def_site_ctxt` = 根 ctxt + Opaque 标记，`with_call_site_ctxt` = + Transparent，`with_mixed_site_ctxt` = + SemiOpaque。它们就是 proc macro 的 `Span::def_site/call_site/mixed_site`。

## 谁打什么标记

| 来源 | 标记 |
|---|---|
| `macro_rules` 转录 | rhs 自带 token 加 `(expn, SemiOpaque)`；代入的 `$x` 保持调用方 ctxt（[macro-rules.md](macro-rules.md)） |
| `macro` 2.0 | 同上但 Opaque（`#[rustc_macro_transparency]` 可改） |
| 内建宏 | 自己选：`std_path` 的 `$crate` 用 def_site，`assert!` 生成的 `panic!` 路径用 call_site，`stringify!`/`line!` 结果用 def_site |
| 过程宏 | 输入 token 保持原 span；宏新造的 token 用 `call_site`/`mixed_site`/`def_site` 全局 span |
| 编译器注入（std prelude、harness） | `AstPass` 展开 + def_site/call_site |

## 名字比较规则（mixed-site 的来源）

| 名字 | 比较前的规范化 | 对 `macro_rules` 的效果 |
|---|---|---|
| 模块级名字（项、导入；`BindingKey`） | `normalize_to_macros_2_0` | SemiOpaque 标记被去掉 → 宏定义/引用的项**不卫生** |
| 局部变量（ValueNS rib） | `normalize_to_macro_rules`（[`fresh_binding`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L4207)、[`resolve_ident_in_lexical_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L305)） | 标记保留 → **卫生** |
| 标签 | `normalize_to_macro_rules`（[`resolve_label`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L2615)） | 卫生 |
| 泛型参数（`contains_params` 的 rib） | `normalize_to_macros_2_0` | 不卫生 |
| `Self` | 根 ctxt | — |
| `$crate` | [`resolve_crate_root`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L2095)：先 macro_rules 规范化，从最外层标记开始依次越过连续的 Opaque 标记、再越过连续的 SemiOpaque 标记，取最后越过的那个；结果是它 `expn_def_scope` 所在 crate 的根，没有标记则为本 crate 根 | 指向宏定义所在 crate |
| `#[macro_use]` 前奏 | 只比 `Symbol` | — |
| 匹配器字面 token | 只比名字 | — |
| 元变量名 | `MacroRulesNormalizedIdent` | — |

模块内查找还要按模块的展开调整 ctxt：[`resolve_ident_in_module`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L891) 用 `normalize_to_macros_2_0_and_adjust(module.expansion)`；词法向上时 [`hygienic_lexical_parent`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L240) 在“模块的展开不是 ctxt 外层展开的祖先”时跳到宏定义所在模块（`remove_mark`）——这是 2.0 宏在定义处解析项名的机制；`macro_rules` 的 ctxt 规范化后没有 SemiOpaque 标记，因此不会跳。

块内定义的 `macro_rules`：[`resolve_block`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L4900) 在其后压 `RibKind::MacroDefinition(def)`；查局部变量/标签越过该 rib 时，若名字由这个宏产生就去掉一层标记，于是宏体能看到定义之前的局部变量。

## 跨 crate 与 `$crate` 打印

- metadata 只编码被引用到的 `ExpnData`/`SyntaxContextData`（[`HygieneEncodeContext`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L1298)、[`decode_syntax_context`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L1448)）；std 的宏在下游展开时靠它保持卫生和 `$crate`。
- [`update_dollar_crate_names`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L647)（由 `resolve_dollar_crates` 在每次收集前调用）为每个 ctxt 记下 `$crate` 应打印成的 crate 名，供 pretty-print 和传给过程宏的文本使用。

## 对 kfrc（T04/T23）的含义

- T04 的 `Span` 要从第一天起带 `ctxt: SyntaxContextId`，与源码区间分开存放；token、AST 标识符都带它。`Ident` 的相等必须是 `(name, ctxt)`，resolver 里不能只按 `Symbol` 建表。M1 无宏时 ctxt 全为根，成本只是一个字段。
- 卫生表 = `ExpnData` arena + 驻留的 `SyntaxContextData`（两种规范化预先算好）。宏产出的 token 保留定义处位置、换 ctxt，诊断回溯走 `ExpnData.call_site` 链——这满足计划里“hygiene 不随诊断 span 丢失”。
- rustc 的 `ExpnIndex` 按分配顺序编号，再用 `ExpnHash` 做稳定身份。kfrc 若要并行展开或跨构建缓存，ExpnId 应由“父展开 + 在父片段中的序号”确定性派生。
- 自有 metadata 要能序列化卫生表；否则下游无法正确展开 std 的 `macro_rules`。

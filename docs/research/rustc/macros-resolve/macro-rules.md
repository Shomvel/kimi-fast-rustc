# macro_rules!：定义、匹配、转录与作用域

源码：`compiler/rustc_expand/src/mbe.rs`、`mbe/{quoted,macro_parser,macro_rules,transcribe,macro_check,metavar_expr}.rs`，fragment 解析在 `compiler/rustc_parse/src/parser/nonterminal.rs`，作用域在 `compiler/rustc_resolve/src/{build_reduced_graph,macros}.rs`。

## 定义的编译

[`compile_declarative_macro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L679)：

- 规则以 `;`（`macro_rules!`）或 `,`（`macro` 2.0）分隔；每条 `lhs => rhs`。规则种类 [`MacroRule`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L129)：`Func`、`Attr`、`Derive`（后两者是不稳定的 `macro_attr`/`macro_derive`）。
- lhs/rhs 经 [`quoted::parse`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/quoted.rs#L60) 变成 [`mbe::TokenTree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe.rs#L69)：`Token`、`Delimited`、`Sequence(SequenceRepetition { tts, separator, kleene, num_captures })`、`MetaVar`、`MetaVarDecl { name, kind }`、`MetaVarExpr`。rhs 里的 `$crate` 变成单个 `kw::DollarCrate` 标识符 token（[L302](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/quoted.rs#L302)）。
- 检查：[`check_lhs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L874)（follow-set、不能有可空重复）、`check_rhs`、[`check_meta_variables`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_check.rs#L196)（重复绑定、自由变量、重复深度、Kleene 一致；嵌套宏下允许误报漏报，见文件头注释）。
- transparency：`#[rustc_macro_transparency]`，缺省 [`Transparency::fallback(macro_rules)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs#L203)：`macro_rules` → `SemiOpaque`，`macro` → `Opaque`。
- lhs 线性化为 [`MatcherLoc`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L100) 序列（[`compute_locs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L167)）。
- 外部 crate 的宏以 token 形式存在 metadata 里，首次使用时重新编译：[`get_macro_by_def_id`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L206) → `load_macro_untracked`（tokens、属性、**定义 crate 的 edition**）→ [`compile_macro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L1156)；带 `builtin_name` 的宏在这里换成内建展开器。

## fragment specifier 与 edition

[`NonterminalKind::from_symbol`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast/src/token.rs#L1133)。edition 取 matcher 中该 token 的 span：不是宏产物就用宏所在 crate 的 edition，是宏产物就用 `span.edition()`（[quoted.rs L126](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/quoted.rs#L126)）。

| 写法 | ≤2018 | 2021 | 2024 |
|---|---|---|---|
| `pat` | `PatParam`（顶层不吃 `\|`） | `PatWithOr` | `PatWithOr` |
| `pat_param` | `PatParam` | 同 | 同 |
| `expr` | `Expr2021` | `Expr2021` | `Expr`：另外接受 `_` 与 `const { }` |
| `expr_2021` | `Expr2021` | 同 | 同 |

开头 token 过滤在 [`nonterminal_may_begin_with`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_parse/src/parser/nonterminal.rs#L18)：`Expr2021` 拒绝 `let`、`const`；`Expr` 拒绝 `let`、接受 `_`。follow-set 见 [`is_in_follow`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L1581)：`expr`/`stmt` 后只能 `=>` `,` `;`；`PatParam` 后可 `=>` `,` `=` `|` `if` `in`；`PatWithOr` 去掉 `|`；`path`/`ty` 后可 `{` `[` `=>` `,` `>` `=` `:` `;` `|` `as` `where` 或 `$b:block`；`vis` 后可 `,`、非 `priv` 标识符、能开始类型的 token；`ident`/`lifetime`/`literal`/`meta`/`tt`/`item`/`block` 后任意（[`frag_can_be_followed_by_any`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L1555)）。重复体的 FIRST/LAST 集由 [`FirstSets`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L1037) 和 [`check_matcher_core`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L1333) 计算。

## 匹配：NFA + 黑盒解析器

[`try_match_macro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L509) 按顺序试每条规则，每条用 [`TtParser::parse_tt`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L615)（算法说明在[文件头](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L1)）：

- 线程集合 `cur_mps`/`next_mps`/`bb_mps`/`eof_mps`；[`parse_tt_inner`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L463) 做 ε 转移（进入/跳过/重复序列、分隔符）。
- 每步按 `(next_mps.len(), bb_mps.len())`：`(0,0)` → `Failure`，试下一条规则；`(_,0)` → 吃一个 token；`(0,1)` → 调黑盒 `parse_nonterminal(kind)`；其余 → **local ambiguity** 硬错误。
- 黑盒解析**提交且不回溯**：fragment 解析失败返回 `ErrorReported`，不会再试后面的规则。规则顺序因此是语义的一部分。
- 字面 token 比较 [`token_name_eq`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L392)：标识符/生命周期只比名字和 raw 标记（不比 hygiene），不可见分隔符永不相等。
- 结果 `NamedMatches: Map<MacroRulesNormalizedIdent, NamedMatch>`，[`NamedMatch`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L386) = `MatchedSeq(Vec<NamedMatch>)` | `MatchedSingle(ParseNtResult)`；[`ParseNtResult`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_parse/src/parser/mod.rs#L1701)：`Tt`、`Ident`、`Lifetime`，以及带捕获 token 的 AST 节点（Item/Block/Stmt/Pat/Expr/Literal/Ty/Meta/Path/Vis）。

## 转录

[`transcribe`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/transcribe.rs#L163)：`Frame` 栈（Delimited/Sequence）+ `repeats: Vec<(idx, len)>`。

- rhs 自带 token：[`Marker::mark_span`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/transcribe.rs#L81) 对 ctxt 做 `apply_mark(expn_id, transparency)`（按原 ctxt 缓存）。位置不变，只换 ctxt。
- `$( … ) sep op`：[`lockstep_iter_size`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/transcribe.rs#L771) 统计其中所有变量在当前深度的重复次数：无变量 → 错；次数不一致 → 错；`+` 而次数为 0 → 错。
- [`transcribe_metavar`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/transcribe.rs#L360)：本宏没绑定的 `$x` 原样输出 `$` `x`（加标记）——这是嵌套宏定义能工作的原因；到叶子仍是序列 → 错。
- [`transcribe_pnr`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/transcribe.rs#L386)：`tt` 直接输出原 token；`ident`/`lifetime` 输出 `NtIdent`/`NtLifetime`，**标识符保留调用方 ctxt**；其他种类输出 `Delimited(Invisible(MetaVar(kind)))` 包住 `TokenStream::from_ast(node)`（同种嵌套只包一层）。所以 `$e * 3` 中 `$e = 1 + 2` 得 `(1+2)*3`；转发出去的 fragment 不能再被另一个宏的字面 token 匹配。
- 元变量表达式 `${count}`/`${index}`/`${len}`/`${ignore}`/`${concat}`（[metavar_expr.rs L17](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/metavar_expr.rs#L17)）是不稳定特性；冻结的 std 是否使用要在 T31 冻结输入时核对（library 不在本源码中）。

输出由 [`ParserAnyMacro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L47) 按调用位置的 fragment kind 解析（[`parse_ast_fragment`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L1096)），必须吃完所有 token。同一段输出在表达式位置和语句/项位置会得到不同 AST。

## 作用域与可见性

- **文本作用域**：[`MacroRulesScope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L64) = `Empty` | `Def(MacroRulesDecl { decl, parent_macro_rules_scope, ident })` | `Invocation(LocalExpnId)`，单向链。[`define_macro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L1265) 在当前链上压 `Def`；[`visit_item`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L1378) 处理完普通项后恢复链，只有 `#[macro_use] mod` 让子模块的链漏到后面；块（[`visit_block`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L1434)）结束时恢复。项/语句位置的宏调用压 `Invocation`（[`visit_invoc_in_module`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L1229)），未展开时查到它返回 Undetermined，展开后被路径压缩成输出作用域。
- **`#[macro_export]`**：可见性 `Public`，并以 `ImportKind::MacroExport` 种进 crate 根的 MacroNS，于是 `crate::m!`、`$crate::m!` 和下游 crate 能按路径使用。未导出的 `macro_rules` 可见性是 `Restricted(crate)`，不进模块表；`use m;`/`pub(crate) use m;` 可行是因为单段导入在 `CurrentScope` 下走 `ScopeSet::All`，会经过 MacroRules 链（[ident.rs L975](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L975)）。
- **`#[macro_use] extern crate`**：[`process_macro_use_imports`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L1108) 把宏放进 `macro_use_prelude`，按 `Symbol` 查，不带 hygiene；只能在 crate 根。
- **`local_inner_macros`**：这类宏里的单段 bang 调用被前缀成 `$crate::m!`（[macros.rs L742](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/macros.rs#L742)）。
- 同名 `macro_rules` 遮蔽：后定义的遮蔽先定义的；宏展开产生的定义遮蔽外层定义时，若使用点会受“更晚展开”影响，报 `MoreExpandedVsOuter` 歧义（见 [imports.md](imports.md)）。

## 对 kfrc 数据模型的含义

- 宏定义存 token tree + 定义 crate edition + transparency + 每条规则的 `MatcherLoc`；metadata 要导出宏的 token 与 span（含 hygiene），见 [hygiene.md](hygiene.md)。
- kfrc 解析器要提供“从 token 游标解析一个 fragment”的入口，并在所有能出现 fragment 的位置接受 `Invisible(MetaVar(kind))` 分组。
- 匹配到的 fragment 建议保存为“原始 token 区间 + 已解析节点”，转录时直接输出原 token 包在不可见分组里，不需要从 AST 反打印 token。
- 黑盒解析不回溯、规则按序、`local ambiguity` 都是可观察语义，差分测试要覆盖。

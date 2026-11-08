# 词法与语法：`rustc_lexer`、RA parser 与 rowan

对应任务：[T04](../../plan/m1/t04-syntax.md)。

## rustc 的分层

| 层 | 位置 | 规模 | 职责 |
|---|---|---|---|
| 纯词法 | [`rustc_lexer`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_lexer/src/lib.rs) | 1,654 行 | `&str` → `(TokenKind, len)`；不报错，错误记为 token 上的标志（如未终止字面量）；不驻留、不建 span |
| cooking | [`rustc_parse/src/lexer/`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_parse/src/lexer/mod.rs#L162) | 2,296 行 | 粘合多字符运算符、按 edition 处理保留前缀与关键字、字面量校验与诊断、doc 注释、建 token tree |
| 解析 | `rustc_parse` 其余部分 | 约 2.8 万行 | AST、错误恢复、fragment 入口 |

`rustc_lexer` 的头注释写明它就是为复用而拆出来的（“separating out pure lexing and rustc-specific concerns”），并承诺可用 stable 构建（[lib.rs L1–L25](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_lexer/src/lib.rs#L1)）。入口是 [`tokenize`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_lexer/src/lib.rs#L329)。

## RA 怎么做

- [`LexedStr`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/parser/src/lexed_str.rs#L37) 直接调用 `rustc_lexer::tokenize`，再做 RA 自己的 cooking（按 edition 拆分、错误文本）。即词法和 rustc 同源，cooking 自写。
- parser 不认识树，只消费 `Input`、产出事件流（[lib.rs L1–L19](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/parser/src/lib.rs#L1)）；事件再由 `syntax` 建成 rowan 树。语法规则在 `grammar/`（5,110 行）。
- 片段入口：[`TopEntryPoint`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/parser/src/lib.rs#L88)（SourceFile、MacroStmts、MacroItems、Pattern、Type、Expr、MetaItem）和 [`PrefixEntryPoint`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/parser/src/lib.rs#L150)（Vis、Block、Stmt、Pat、PatTop、Ty、Expr、Path、Item、MetaItem）。后者正是 `macro_rules` 匹配 fragment 时用的“解析一个前缀”。
- 很多拒绝不在 parser 里：[`validation.rs`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/syntax/src/validation.rs#L21) 事后检查字面量、可见性、范围表达式、路径关键字、`dyn`/`impl` 的 `+`、`let` 位置等。grammar 本身接受超集。
- 语料：`crates/parser/test_data/` 有 526 个 `.rs` 输入，其中 180 个是错误用例。期望输出是 RA 的树，不是 rustc 的判定。

## 对 kfrc

| 部分 | 档 | 做法 |
|---|---|---|
| 纯词法 | **直接依赖 `rustc_lexer`（待决策 D1）** | 钉 `ra-ap-rustc_lexer = "=0.148.0"`（与 1.94.1 源码相同）。kfrc 的 token 类型仍自有：`TokenKind` 只在 cooking 内部出现 |
| cooking | 参考设计、自写 | 照 `rustc_parse::lexer` 移植：jointness（`Joint`/`JointHidden`）、保留前缀（2021）、guarded string（2024）、字面量校验用 `rustc-literal-escaper` 或自写 |
| parser | 参考设计、自写 | 语义与错误码以 `rustc_parse` 为准；结构上借 RA 的“parser 只出事件” |
| 语法树 | 不参考 | 不用 rowan |

**为什么词法可以依赖、解析不行。** 词法是 `&str → (kind, len)`，没有数据模型可言；kfrc 自写只会在原始字符串、Unicode 标识符、frontmatter、shebang 上制造与 rustc 的分歧。解析器的产物是 kfrc 的 AST 和 HIR 级节点 ID（T04 要求的 token 区间、`Span { lo, hi, ctxt }`），必须自有。

**为什么不用 rowan。** rowan 树保留全部 trivia，节点按偏移相对存放，为“每次按键后增量重解析”设计。kfrc 一次性解析、不编辑源码，AST arena + token 区间足够；无损树的内存和遍历成本没有回报（推断，未测）。T04 要的“AST 节点保留来源 token 区间”用 arena 里两个下标即可。

**借 RA 的事件式结构。** parser 输出 `Start { kind } / Token / Finish / Error` 事件（[event.rs L22](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/parser/src/event.rs#L22)，另有一个拆浮点字面量的 `FloatSplitHack`），再由 builder 决定建 AST 还是只建 token 区间。这样同一个 grammar 可以服务三种调用者：整文件、`macro_rules` 的 fragment 匹配（只要消费了多少 token）、过程宏输出的重新解析。T04 的“按 fragment 种类的解析入口”可直接对照 `PrefixEntryPoint` 列表核对完整性（RA 缺 `literal`、`lifetime`、`ident`，这些在 `mbe` 里直接按 token 判断）。

## 要避开的坑

- **不要照搬 RA 的错误恢复策略。** RA 尽量“继续解析出一棵树”，例如把缺失的表达式留成空节点；kfrc 出错后只需继续收集诊断，错误恢复的产物不能进入名称解析之后的阶段（[overview.md](../../design/frontend/overview.md) 的“错误恢复值不进入 codegen”）。
- **不要把 validation 推迟当作合法。** RA grammar 接受、validation 才拒绝的形状，kfrc 的 parser 要在同一位置报 rustc 的错误码。用 RA 的 `test_data/parser/err` 作为负例输入，期望值由 rustc 1.94.1 生成。
- **edition 是逐 token 的。** RA 的 `LexedStr::new(edition, text)` 按文件 edition 词法；kfrc 的 span edition 来自展开来源（T04），宏展开后的 token 要按定义处 edition 重新判断关键字。
- **依赖 `rustc_lexer` 时仍要差分。** 0.148.0 与 1.94.1 源码相同，但 kfrc 的 cooking 是新代码；cooking 的差分需要 token 级 dump：1.94.1 的 `-Zunpretty` 没有 token 模式（[config.rs L2790 起](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L2790)，只有 source/AST/HIR/THIR/MIR 各级），可行的 oracle 是一个打印 `TokenStream` 的 bang 过程宏（能看到 `Spacing` 与字面量原文；推断，T04 实测）。

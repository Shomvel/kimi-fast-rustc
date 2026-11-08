# T04 lexer、AST 与标量函数语法 — 细化

`M1 · A · 依赖：T02, T03` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t04) · rustc 笔记：[macros-resolve](../../rustc/macros-resolve/README.md#t04token-tree-与-span-预留)、[ownership/temporaries](../../rustc/ownership/temporaries.md)

## 现状

[frontend.rs](../../../../src/frontend.rs) 的 `Lexer`（:117）产出扁平 `Vec<Token>`，`Span` 只有位置；`Parser` 直接建 SoN。没有 token tree、没有 AST、没有 edition。

## 现在就要预留的结构（晚加会改遍下游）

| 结构 | 内容 | 为什么现在 |
|---|---|---|
| `Span { lo, hi, ctxt: SyntaxContextId }` | 根 ctxt 为 0 | 标识符身份是“名字 + ctxt”；晚加要改所有 resolver 查表点 |
| `SyntaxContextData` / `ExpnData` arena | 结构定义好，M1 只有根 | T23 填充；edition 从展开来源取 |
| edition 按 span 查询 | `span.edition()` | 临时值作用域、模式规则、闭包捕获都按 span 的 edition，不按 crate |
| token tree | 分隔符含 `Invisible(origin)`；`Spacing::{Alone, Joint, JointHidden}`；raw 标识符；字面量保留原文（kind、symbol、suffix） | 宏代入、`stringify!`、过程宏都需要 |
| AST 节点保留 token 区间 | 起止 token 下标 | 宏输入、derive、`stringify!` |
| 按 fragment 种类的解析入口 | expr/ty/pat/pat_param/stmt/item/block/path/vis/meta/literal/lifetime/ident | `macro_rules` 匹配要按种类调用解析器 |
| HIR 级节点 ID | 每个表达式/块/语句一个稳定 ID | region scope tree、TypeckResults、Body 构建都以它为键 |

## M1 语法范围

函数（参数、返回类型、`pub`/`const fn` 修饰）、`let`/`let mut`（含类型标注）、赋值与复合赋值、块与块尾表达式、`if`/`else`、`match`（字面量、范围、`_`、guard）、`while`、`for a..b`/`a..=b`、`loop`/`break 值`/`continue`/`return`、整数与 bool 字面量（全部后缀）、`as`、一元/二元运算、函数调用、`const` 项、`mod` 块与 `use` 路径。范围外语法给出“未支持”诊断。

`for`、`while` 的脱糖形状影响临时值析构点（rustc 用 `DropTemps` 包装 `for` 的迭代器表达式），脱糖放在 AST → HIR 降低里，按 rustc 的形状实现。

## 测试

- 词法：全部整数后缀与进制、下划线、raw 标识符、注释嵌套、`'a` 生命周期 token。
- 解析正反例：沿用现有 `tests/integer_syntax.rs`、`tests/assignment_syntax.rs`、`tests/expression_statements.rs` 等的输入，改成 `.rs` fixture。
- 往返：AST pretty-print 后再解析得到同一 AST。

## 完成标志

上述结构存在且有单元测试；M1 语法范围的正反例通过；解析器不生成任何 SoN。

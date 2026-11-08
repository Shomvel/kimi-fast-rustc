# 代表性边界用例（tests/ui）

全部路径相对 [`tests/ui/`](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui)。`run-pass` 可直接进 x86 真实执行对拍；`check-pass`/报错用例对拍“接受/拒绝”和错误类别，不比较诊断文本。带 `aux-build`/`proc-macro` 的需要跨 crate metadata 或过程宏，最早在 M4 才能原生跑，M3 只能标记为待办。

链接前缀：`https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/`。

## 展开顺序、cfg 与 derive

| 用例 | 验证点 | 用于 |
|---|---|---|
| [macros/macro-deep_expansion.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-deep_expansion.rs) | `concat!(foo!(), "bar")`：内建宏急切展开参数里的嵌套宏 | T23 |
| [macros/macro-in-fn.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-in-fn.rs) | 函数体内定义 2.0 `pub macro`（定义收集与可见性解析） | 2.0 支持时 |
| [proc-macro/derive-expand-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/derive-expand-order.rs) + `.stdout` | 多个 `#[derive]` 属性逐个处理，前面的 derive 看得到后面的 `#[derive]` | T30 |
| [proc-macro/attribute-after-derive.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/attribute-after-derive.rs) | derive 前/后的属性宏看到的输入；`#[derive]` 先把字段上的 `cfg` 剥掉 | T30 |
| [proc-macro/cfg-eval.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/cfg-eval.rs) | `#[cfg_eval]` 让属性宏收到已配置的 token | T30 |

## macro_rules 匹配与转录

| 用例 | 验证点 | 用于 |
|---|---|---|
| [macros/local-ambiguity-multiple-parsing-options.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/local-ambiguity-multiple-parsing-options.rs) | `$($i:ident)* $j:ident` → local ambiguity 硬错误 | T23 |
| [macros/macro-follow.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-follow.rs) | 各 fragment 的 follow-set 拒绝表（≤2021） | T23 后期 |
| [macros/macro-at-most-once-rep-2018.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-at-most-once-rep-2018.rs) | `?` 是 Kleene 运算符、不接分隔符；`$(a)?+` 合法 | T23 |
| [macros/macro-stmt-matchers.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-stmt-matchers.rs) | `$s:stmt;` 匹配 `vec![].push(0);` | T23 |
| [macros/macro-interpolation.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-interpolation.rs) | `ty`/`block`/`expr`/`pat`/`path` 代入后的解析；`<$ty as $path>::$ident` | T23 |
| [macros/meta-variable-misuse.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/meta-variable-misuse.rs) | 宏生成宏 + 嵌套重复（`$(macro_rules! $f …)+`），在 `deny(meta_variable_misuse)` 下运行（run-pass） | T23 |
| [macros/same-sequence-span.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/same-sequence-span.rs) | 多个同 span 序列的 follow-set 检查不混淆（需过程宏辅助） | 参考 |
| [macros/macro-comma-support-rpass.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-comma-support-rpass.rs) | std 宏（`assert!`、`format!`、`panic!`…）尾逗号支持，运行期行为 | T23/T31 |

## edition 差异

| 用例 | 验证点 |
|---|---|
| [macros/expr_2021_inline_const.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/expr_2021_inline_const.rs) | `expr_2021` 不吃 `const { }`；2024 的 `expr` 吃 |
| [macros/expr_2024_underscore_expr.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/expr_2024_underscore_expr.rs) | 2024 的 `expr` 接受 `_` |
| [macros/macro-pat-pattern-followed-by-or.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-pat-pattern-followed-by-or.rs) | ≤2021 的 `pat` 后可跟 `\|`（run-pass） |
| [macros/macro-pat-follow-2018.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-pat-follow-2018.rs) | 2018 的 `pat` follow 规则 |

## hygiene

| 用例 | 验证点 | 用于 |
|---|---|---|
| [hygiene/hygiene.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/hygiene.rs) | `macro_rules` 体里的 `x` 绑定到定义处的局部变量而非调用前最近的 `let x`；宏生成宏的组合（run-pass） | T23 必过 |
| [hygiene/hygienic-labels.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/hygienic-labels.rs)、[hygienic-labels-in-let.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/hygienic-labels-in-let.rs) | 宏引入的 `'x:` 不捕获调用方的 `break 'x`（run-pass） | T23 必过 |
| [hygiene/legacy_interaction.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/legacy_interaction.rs) | 2.0 宏里调用 `macro_rules`：`apply_mark` 的标记重放 | 2.0 支持时 |
| [hygiene/items.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/items.rs)、[arguments.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/arguments.rs)、[fields.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/fields.rs) | 2.0 宏的项名、类型参数、字段卫生与隐私 | 2.0 支持时 |
| [hygiene/generic_params.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/generic_params.rs) | 泛型参数总按 2.0 规则比较 | T23 |
| [hygiene/no_implicit_prelude.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/no_implicit_prelude.rs) | 2.0 宏在定义处模块（`#[no_implicit_prelude]`）解析前奏名 | 2.0 支持时 |
| [hygiene/local_inner_macros.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/local_inner_macros.rs) | `#[macro_export(local_inner_macros)]` 的 `$crate::` 前缀（跨 crate） | M4 |
| [hygiene/dollar-crate-modern.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/dollar-crate-modern.rs)、[nested-dollar-crate.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/hygiene/nested-dollar-crate.rs) | 嵌套与跨 crate 的 `$crate` | M4 |
| [proc-macro/mixed-site-span.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/mixed-site-span.rs) | `Span::mixed_site` 等同 `macro_rules` 卫生 | T30 |

## 作用域、遮蔽与歧义

| 用例 | 验证点 | 用于 |
|---|---|---|
| [macros/macro-shadowing.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-shadowing.rs) | 宏展开出的 `macro_rules! foo` 在外部使用 → 歧义；只在展开内部使用则合法 | T23 |
| [macros/restricted-shadowing-legacy.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/restricted-shadowing-legacy.rs)、[-modern.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/restricted-shadowing-modern.rs) | 定义/调用/使用三者展开偏序的全组合表 | T23 |
| [macros/macro-use-scope.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-use-scope.rs) | `#[macro_use] extern crate` 对之前的代码也可见（前奏而非文本作用域） | M4 |
| [macros/macro-path-prelude-shadowing.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/macro-path-prelude-shadowing.rs) | 宏展开加入的 `macro_use` 前奏与内建属性/std 前奏的优先级 | M4 |
| [imports/local-modularized-tricky-fail-1.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/local-modularized-tricky-fail-1.rs) | 宏展开出的 `#[macro_export]` 与 glob/前奏中的同名宏（`panic`、`include`）歧义 | T23 |
| [imports/shadow_builtin_macros.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/shadow_builtin_macros.rs) | 显式 `use` 可遮蔽 `panic!`；glob 或宏展开的 `use` 不行 | T23 |
| [imports/glob-shadowing.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/glob-shadowing.rs) | glob 引入的 `env` 与前奏 `env!` 歧义 | T23 |
| [resolve/block-with-trait-parent.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/resolve/block-with-trait-parent.rs) | 含项的块是匿名模块，外层 trait 仍在作用域 | T05 |

## 导入

| 用例 | 验证点 | 用于 |
|---|---|---|
| [imports/import-loop.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/import-loop.rs) | 自引用导入报 unresolved 而非死循环 | T05 |
| [imports/glob-cycles.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/glob-cycles.rs) | glob 互相导入成环仍收敛 | T05 |
| [imports/duplicate.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/duplicate.rs) | 重复定义 vs glob 冲突只在使用时报 | T05 |
| [imports/issue-53269.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/issue-53269.rs) | 失败导入恢复成 `Res::Err` 后与 `macro_rules` 的歧义 | T23 |

## 过程宏与内建宏

| 用例 | 验证点 | 用于 |
|---|---|---|
| [proc-macro/derive-helper-shadowing.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/derive-helper-shadowing.rs) | derive helper 作用域与同名导入、宏生成代码的可见性 | T30 |
| [proc-macro/expand-expr.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/expand-expr.rs) | `TokenStream::expand_expr` 只返回字面量 | T30 |
| [proc-macro/nonterminal-token-hygiene.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/nonterminal-token-hygiene.rs) | `macro_rules` 代入后传给过程宏的 token 卫生 | T30 |
| [unpretty/exhaustive.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/unpretty/exhaustive.rs) + `.expanded.stdout`/`.hir.stdout` | std 注入形状、`format_args!` 的 HIR 形状 | T23 对照 |

## 建议的 kfrc 自有回归（rustc 测试未直接覆盖或依赖诊断）

- 黑盒提交：`($e:expr) => …; ($t:ty) => …` 输入一个类型，必须报错而非落到第二条。
- `$e * 3` 在 `$e = 1 + 2` 时结果为 9（不可见分组保持优先级）。
- 同一宏在表达式、语句、项位置展开出不同 AST。
- `assert!(a == b)` 失败消息文本与 rustc 一致（pretty printer），`stringify!` 同理。
- `format_args!` 的宽度/精度/填充/`#?`/位置参数/隐式捕获组合，对拍输出字符串。

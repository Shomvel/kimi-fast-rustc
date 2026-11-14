# 组件清点：rust-analyzer crate 与 `ra-ap-rustc_*`

RA 链接固定到 [`03ae6be5`](https://github.com/rust-lang/rust-analyzer/tree/03ae6be57aa063bb373f69eead3b8e15168a45d2)，rustc 链接固定到 `e408947b`。行数是 `src/` 下全部 `.rs` 的物理行数（含注释和单元测试），只用于比较量级。

## rust-analyzer 的前端 crate

整个 workspace 的许可是 `MIT OR Apache-2.0`，`rust-version = "1.98"`、edition 2024（[Cargo.toml L6–L9](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/Cargo.toml#L6)），因此无法用 rustc 1.94.1 构建；除 proc-macro-srv 外都在 stable 上构建。

| crate | 行数（其中测试） | 职责 | 关键依赖 | 与 rustc 的一致程度 |
|---|---|---|---|---|
| `parser` | 13,170（3,982） | 事件式递归下降；不含 lexer，按 `TopEntryPoint`/`PrefixEntryPoint` 解析片段 | `ra-ap-rustc_lexer`、`edition` | 容错、接受超集；部分拒绝推迟到 `syntax/src/validation.rs`（476 行） |
| `syntax` | 25,115 | rowan 无损树、类型化 AST、校验 | `rowan 0.18.0-alpha.2`、`parser` | 为编辑设计，保留全部 trivia |
| `tt` / `mbe` / `syntax-bridge` | 2,630 / 3,466 / 1,823 | token tree、`macro_rules` 引擎、树 ↔ token 转换 | `span`、`salsa` | 匹配/转录跟随 rustc，测试在 `hir-def` |
| `span` | 1,948 | `SpanAnchor`、`AstId`、`SyntaxContext` | `salsa`、`vfs` | 卫生结构与 rustc 同构；位置编码不同 |
| `hir-expand` | 9,846 | `MacroCallId`、内建宏、急切展开、过程宏接口、语法修补 | `mbe`、`cfg`、`base-db` | 有 IDE 专用的修补（`fixup.rs`）和降级 |
| `hir-def` | 47,219（6,988） | item tree、`DefMap`、签名、函数体（`expr_store`）、resolver、lang item | `ra-ap-rustc_{parse_format,abi,type_ir}`、`fst` | 名称解析单调、不报歧义；见 [resolve-defmap.md](resolve-defmap.md) |
| `hir-ty` | 63,314 | 类型推导（`infer/` 14.5k + `infer.rs` 2.8k）、next-solver 接口（`next_solver/` 19.8k）、方法解析、布局、自有 MIR 与解释器（`mir/` 8.8k） | `ra-ap-rustc_{abi,index,ast_ir,type_ir,next_trait_solver}`、`ena` | 推导跟随 rustc 阶段顺序；不做借用检查；常量求值是自有解释器 |
| `hir-ide` | 38,794（31,696） | 显示、额外诊断（`match_check` 用 `rustc_pattern_analysis`）、类型推导测试 | `ra-ap-rustc_pattern_analysis` | 穷尽检查算法与 rustc 相同 |
| `base-db` | 1,914 | Salsa 输入、crate 图、durability | `salsa 0.28.5` | 不适用 |
| `intern` | 2,071 | 全局驻留（`Symbol`、`Interned<T>`） | `dashmap`、`hashbrown` | 不适用 |
| `cfg` | 1,141 | `CfgExpr`、`CfgOptions`、启用建议 | `tt`、`syntax` | 求值规则简单，与 rustc 一致（推断） |
| `project-model` | 6,180 | `cargo metadata`、sysroot 发现、build script 结果 | `cargo_metadata`、`toml` | 不适用 |
| `proc-macro-api` | 2,887 | 客户端：进程池、两种协议、版本协商 | `postcard`、`serde` | 不适用 |
| `proc-macro-srv` | 3,128（954） | 服务端：加载宏 dylib、实现 bridge | `rustc_private` | 与工具链同版本，行为即 rustc 的 bridge |

RA 也把全部 crate 以 `ra_ap_*` 名义每周发布（例如 `ra_ap_hir_def` 0.0.360，2026-10-26，`rust_version = 1.98`，crates.io 查询）。

## RA 复用的 rustc crate

rustc-dev-guide 列出共享的 8 个 crate，由 rustc-auto-publish 以 `ra-ap-rustc_*` 名义发到 crates.io（[sharing-crates-with-rust-analyzer.md](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/src/doc/rustc-dev-guide/src/solve/sharing-crates-with-rust-analyzer.md)）。版本号与 rustc 版本无关，许可均为 `MIT / Apache-2.0`。

| crate | 1.94.1 行数 | 依赖闭包（crate 数） | stable | RA 在哪用 | RA 当前版本 |
|---|---|---|---|---|---|
| `rustc_lexer` | 1,654 | 4（memchr、unicode-ident、unicode-properties） | 是，源码 `#![deny(unstable_features)]` | `parser`、`mbe`、`tt` | 0.180.0 |
| `rustc_parse_format` | 1,563 | 6 | 是，同上 | `hir-def`（`format_args!`） | 0.180.0 |
| `rustc_index` | 4,033 | 6（含过程宏 `rustc_index_macros`） | `default-features = false` | `hir-ty` | 0.166.0 |
| `rustc_abi` | 5,400 | 15 | 关闭 `nightly` | `hir-def`（repr）、`hir-ty`（布局） | 0.166.0 |
| `rustc_type_ir` | 14,445 | 含在下一行 | 关闭 `nightly` | `hir-def`、`hir-ty` | 0.166.0 |
| `rustc_next_trait_solver` | 10,002 | 28（含 type_ir、ast_ir、index 和 4 个过程宏 crate） | 关闭 `nightly` | `hir-ty` | 0.166.0 |
| `rustc_pattern_analysis` | 5,206 | 16 | 关闭 `rustc` 特性 | `hir-ide` | 0.166.0 |

版本来自 RA 的 [Cargo.toml L86–L93](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/Cargo.toml#L86) 与 Cargo.lock。依赖闭包由 `cargo tree -e normal` 统计（实测）。

## 实测：哪个 `ra-ap` 版本等于 rustc 1.94.1

发布包不含 `.cargo_vcs_info.json`，不能直接反查 rustc commit。改为把 crate 的 `src/` 与官方 1.94.1 源码逐文件 diff（实测，2026-11-08）：

| 版本（发布日期） | type_ir | next_trait_solver | abi | pattern_analysis | lexer | parse_format | index |
|---|---|---|---|---|---|---|---|
| 0.148.0（2026-01-18） | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| 0.149.0（2026-01-25） | 2 | 6 | 20 | 13 | 0 | 0 | 0 |
| 0.150.0（2026-02-01） | 482 | 57 | 20 | 11 | 0 | 2 | 0 |
| 0.166.0（2026-05-10） | 3,046 | 2,850 | — | — | — | — | — |
| 0.184.0（2026-11-08） | 6,747 | 5,591 | — | — | — | — | — |

数字是 diff 的增删行数。**0.148.0 的 7 个 crate 与 1.94.1 逐字节相同**；用 Homebrew rustc 1.94.1 对 `=0.148.0` 的 7 个 crate（`default-features = false`）运行 `cargo check` 成功，15.8 s（实测）。

## 不在 RA 里、但同样可独立的

`rustc_literal_escaper`（字面量转义，已在 crates.io 独立发布，RA 也用）。`rustc_parse`、`rustc_expand`、`rustc_resolve`、`rustc_hir_typeck`、`rustc_infer` 都依赖 `TyCtxt`/`Session`，没有可独立的形态；RA 的对应部分是自写的。

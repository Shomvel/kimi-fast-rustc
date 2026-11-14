# 过程宏：加载、server 端、TokenStream 转换与 span

源码：`compiler/rustc_expand/src/{proc_macro.rs,proc_macro_server.rs}`、`compiler/rustc_builtin_macros/src/proc_macro_harness.rs`、`compiler/rustc_metadata/src/{creader.rs,rmeta/decoder.rs}`。client 端与 bridge 协议在 `library/proc_macro/src/bridge/`（经 `compiler/rustc_proc_macro` 引入），**不在本源码 checkout 中**；下文关于协议内部的描述标“推断”。

## 宏 crate 一侧：harness 与导出符号

- 编译 `proc-macro` crate 时 [`proc_macro_harness::inject`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/proc_macro_harness.rs#L51) 收集 `#[proc_macro]`、`#[proc_macro_attribute]`、`#[proc_macro_derive(Trait, attributes(..))]` 函数，[`mk_decls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/proc_macro_harness.rs#L283) 生成 `#[rustc_proc_macro_decls] static DECLS: &[ProcMacro] = &[ProcMacro::custom_derive(..), ProcMacro::attr(..), ProcMacro::bang(..)]`。
- 该静态以 `__rustc_proc_macro_decls_{stable_crate_id:08x}__` 导出（[session.rs L383](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/session.rs#L383)），crate 产物是 host 平台的动态库。
- 同一 crate 内这些函数只是桩（[`proc_macro_stub`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L1235)），使用会报 `ProcMacroSameCrate`。

## 编译器一侧：加载

- [`dlsym_proc_macros`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/creader.rs#L907) 打开 dylib、取上述符号，得到 `&'static [ProcMacro]`。
- [`load_proc_macro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/decoder.rs#L1038)：`CustomDerive { trait_name, attributes, client }` → `SyntaxExtensionKind::Derive(DeriveProcMacro)`，`attributes` 成为 `helper_attrs`；`Attr { name, client }` → `Attr(AttrProcMacro)`；`Bang` → `Bang(BangProcMacro)`。edition 取宏 crate 的。proc-macro crate 的 metadata 只描述这些宏，不含可链接代码。

## 展开入口

- [`BangProcMacro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro.rs#L42)：`client.run(strategy, server, input)`；输出 token 交给 `parse_ast_fragment` 按调用位置解析。
- [`AttrProcMacro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro.rs#L72)：输入 `(属性参数, 被修饰项的 tokens)`，输出**替换**原项。
- [`DeriveProcMacro`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro.rs#L105)：输入 `item.to_tokens()`（已 `cfg_eval`），输出逐个 `parse_item` 后**追加**；语句位置包成语句。可选查询缓存 `derive_macro_expansion`（`-Z cache-proc-macros`，键为 `(LocalExpnId, 输入 TokenStream)`，宏 crate 的 `crate_hash` 变化即失效）。
- 宏 panic → 编译错误（`ProcMacroPanicked` 等）。执行策略 [`exec_strategy`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro.rs#L35)：同线程，或 `-Z proc-macro-execution-strategy=cross-thread` 时用 [`MessagePipe`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro.rs#L14)（容量 1 的 `sync_channel`）跨线程。

## server：`Rustc`

[`struct Rustc { ecx, def_site, call_site, mixed_site, krate, rebased_spans }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L436)，[`new`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L446) 从当前 `ExpnData` 算出三个全局 span：`def_site = with_def_site_ctxt(def_site)`（Opaque），`call_site = with_call_site_ctxt(call_site)`（Transparent），`mixed_site = with_mixed_site_ctxt(call_site)`（SemiOpaque）。关联类型（[L463](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L463)）直接是编译器自己的 `TokenStream`/`Span`/`Symbol`。

server 方法集合（即 bridge 的 RPC 面）：

| trait | 方法 |
|---|---|
| [`FreeFunctions`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L470) | `injected_env_var`（`--env-set`）、`track_env_var`（写 `env_depinfo`）、`track_path`（写 `file_depinfo`）、`literal_from_str`、`emit_diagnostic` |
| [`TokenStream`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L560) | `is_empty`、`from_str`（以 call_site 解析源码文本）、`to_string`、[`expand_expr`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L578)（急切展开，只允许结果是字面量）、`from_token_tree`、`concat_trees`、`concat_streams`、`into_trees` |
| [`Span`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L680) | `debug`、`file`、`local_file`、`parent`、`source`、`byte_range`、`start`、`end`、`line`、`column`、`join`、`subspan`、`resolved_at`（位置取前者、ctxt 取后者）、`source_text`、`save_span`/[`recover_proc_macro_span`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L831)（`quote!` 里的 span 从 metadata 取回，换成 def_site ctxt） |
| [`Symbol`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L841) | `normalize_and_validate_ident` |
| [`Server`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L848) | `globals() -> ExpnGlobals { def_site, call_site, mixed_site }`、`intern_symbol`、`with_symbol_string` |

## TokenStream 转换

[`from_internal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L106)（编译器 → 宏）：

- `Delimited` → `Group`；连续同层的 `Invisible(MetaVar)` 合并成一层，不可见分组对宏表现为 `Delimiter::None`。
- 多字符运算符拆成单字符 `Punct`，前面的都 `joint = true`，最后一个继承原 token 的 jointness；`JointHidden` 对宏表现为非 joint。
- 标识符（含 raw、`NtIdent`）→ `Ident`；生命周期 → `'` Punct + `Ident`；字面量原样（kind、symbol、suffix）；doc 注释 → `#` `[doc = "…"]`（内注释另加 `!`）。

[`to_internal`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro_server.rs#L326)（宏 → 编译器）做反向映射，`Delimiter::None` 变 `Invisible(ProcMacro)`，解析器会跳过它（[`InvisibleOrigin::skip`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast/src/token.rs#L63)）。

## Span 与 hygiene

- 输入 token 保留原 span（含 ctxt），宏原样输出的输入 token 保持输入方的卫生。
- 宏新造的 token：`Span::call_site()` 等同手写在调用处；`Span::mixed_site()` 与 `macro_rules` 相同（局部变量、标签、`$crate` 卫生）；`Span::def_site()` 不稳定。行为对照 [tests/ui/proc-macro/mixed-site-span.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/proc-macro/mixed-site-span.rs)。
- `proc_macro::quote!` 本身注册为内建 `Bang` 过程宏，在编译器进程里运行（[lib.rs L147](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_builtin_macros/src/lib.rs#L147)）。

## bridge 协议形态（推断）

从 `client.run(&strategy, server, input, backtrace)` 和 server trait 形状推断：`Client<I, O>` 是宏 dylib 里的一组 `extern "C"` 函数指针；调用时 client 把请求编码进字节缓冲，server 解码并分派到上表方法，`TokenStream`/`Span` 等以句柄（整数）跨边界传递，server 持有句柄表。方法枚举和编码是**按编译器版本固定、无稳定性承诺**的：宏 dylib 静态链接了自己那份 `libproc_macro`，必须与加载它的编译器 bridge 完全一致。

## kfrc 的 host 方案

| 方案 | 做法 | 优点 | 风险 |
|---|---|---|---|
| A. 兼容 rustc 1.94.1 bridge | 实现与 1.94.1 一致的 server 端，`dlopen` stock rustc 编出的宏 dylib | 最快让 Cargo 依赖跑起来 | ABI 未公开，需逐字段对照 library 源码；属于混合 bootstrap，按红线 4 单独标记，不计入 full-native |
| B. 自编 proc_macro | kfrc 自带 `proc_macro` 库（稳定 API 面：`TokenStream`、`TokenTree`、`Group`、`Punct`、`Ident`、`Literal`、`Span`、`Delimiter`、`Spacing`、`quote!`）和自有 bridge；kfrc 把 proc-macro2/syn/quote/serde_derive 编成 host 动态库 | 全链自有、协议可控 | 需要 kfrc 先能编出这些 crate 和动态库，并处理 host/target 分离 |
| C. 进程外 server | 宏在独立进程执行，token 树序列化传输（rust-analyzer 的 proc-macro-srv 是同类先例） | 隔离崩溃、易做缓存键和超时 | 多一次序列化；可与 A 或 B 组合 |

建议：B + C 作为目标形态；A 只作明确标记的过渡。缓存键至少包含：宏 crate 产物哈希、输入 token（含影响输出的 span 信息）、属性参数、`track_env_var`/`track_path` 记录、`--env-set`。宏能读取未声明的环境和文件，不可完整跟踪的展开不跨构建缓存（与 [T30](../../../plan/m4-cargo-sysroot.md#t30) 一致）。

## 目标项目需求

- ripgrep 15.2.0（lockfile 见 [t00-freeze-inputs.md](../../../plan/m0/t00-freeze-inputs.md)）在 Linux 默认 features 下的 normal/build 依赖闭包**没有任何 proc-macro crate**；`serde_derive` 只是 ripgrep 的 dev-dependency（`cargo tree --offline --target x86_64-unknown-linux-gnu -e normal,build` 与 `-i serde_derive -e all` 核对）。
- Cargo 的 lockfile 含 serde_derive、thiserror-impl、tracing-attributes、displaydoc、zerovec/yoke/zerofrom-derive、openssl-macros、rustversion 等；Cargo 原生构建必须有过程宏。

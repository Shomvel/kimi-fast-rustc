# 过程宏进程外执行：RA 的 proc-macro-srv

对应任务：[T30](../../plan/m4-cargo-sysroot.md#t30)（T30.b 过程宏协议）。rustc 一侧与 kfrc 的 A/B/C 方案见 [proc-macros.md](../rustc/macros-resolve/proc-macros.md)。

## RA 的结构

| 部分 | 位置 | 说明 |
|---|---|---|
| 服务端库 | [`proc-macro-srv`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/proc-macro-srv/src/lib.rs#L11) | 整个 crate 在 `#![cfg(feature = "in-rust-tree")]` 下，开启 `proc_macro_internals`、`rustc_private` 等不稳定特性，链接 `rustc_metadata`、`rustc_proc_macro`、`rustc_span` |
| 加载 dylib | [`ProcMacroLibrary::open`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/proc-macro-srv/src/dylib.rs#L76) | 直接调用 `rustc_metadata::locator::get_proc_macros`，按 rustc 版本字符串校验 |
| 服务端二进制 | `proc-macro-srv-cli` | 随 rustup 工具链发布为 `libexec/rust-analyzer-proc-macro-srv`；RA 在 sysroot 里查找它（[sysroot.rs L206](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/project-model/src/sysroot.rs#L206)） |
| 客户端 | [`proc-macro-api`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/proc-macro-api/src/lib.rs#L1) | 进程管理、进程池（按活跃请求数挑选 worker）、两种协议 |
| 协议 | `legacy_protocol`（JSON）、`bidirectional_protocol`（postcard） | API 版本 0–6，当前 `HASHED_AST_ID = 6`（[lib.rs L35–L45](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/proc-macro-api/src/lib.rs#L35)） |

**关键事实：服务端与工具链绑定。** 宏 dylib 静态链接了自己那份 `libproc_macro`，bridge 无稳定性承诺；RA 的解决办法不是实现兼容 bridge，而是让服务端成为工具链的一部分（`rustc_private` 构建、同版本发布）。这对 kfrc 的方案 A（兼容 1.94.1 bridge）是一条反证：连 RA 也没有在工具链之外实现 bridge（推断：RA 早期曾自带 ABI 副本，已放弃，未查历史）。

## 协议（bidirectional）

- 请求：`ListMacros`、`ExpandMacro`、`ApiVersionCheck`、`SetConfig`。
- [`ExpandMacro`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/proc-macro-api/src/bidirectional_protocol/msg.rs#L156)：dylib 路径、环境变量、当前目录、`ExpandMacroData { macro_body, macro_name, attributes, has_global_spans: ExpnGlobals { def_site, call_site, mixed_site }, span_data_table }`。
- 响应 `ExpandMacroResponse`：结果树、span 表、**`tracked_env_vars`、`tracked_paths`**。
- 子请求（服务端回调客户端）：[`SubRequest`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/proc-macro-api/src/bidirectional_protocol/msg.rs#L18) = `FilePath`、`SourceText`、`LocalFilePath`、`LineColumn`、`ByteRange`、`SpanSource`、`SpanParent`、`SpanJoin`。即 `Span::source_text()`、`line()`、`join()` 等不随请求预先传输，而是按需回问。
- token 树编码：[`FlatTree`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/proc-macro-api/src/legacy_protocol/msg/flat.rs#L92) 把 subtree、literal、punct、ident、token tree 拆成几个 `Vec<u32>` 加一个字符串表；span 走单独的 `span_data_table`（锚点 + 相对区间 + ctxt）。

## 对 kfrc：参考设计、自写

| 借什么 | 怎么用 |
|---|---|
| 进程外 + 进程池 | 方案 C：宏崩溃、死循环不拖垮编译器；超时算失败（[pipeline.md](../../design/build/pipeline.md#失败与取消)）；池大小计入 jobserver 预算 |
| 版本协商 | 首个请求协商协议版本；kfrc 自编的 `proc_macro` 与服务端同仓同版本，协商失败即报错，不降级 |
| 响应带依赖记录 | `tracked_env_vars`/`tracked_paths` 正是 [proc-macros.md](../rustc/macros-resolve/proc-macros.md#kfrc-的-host-方案) 要求的缓存键成分；kfrc 还要加上宏 crate 产物哈希与 `--env-set` |
| span 按需回问 | 只把“锚点 + 相对区间 + ctxt”传过去，源文本、行列在服务端需要时回问，请求体小，且缓存键里不会混入绝对位置 |
| 扁平编码 | 稠密数组 + 字符串表，便于哈希作缓存键 |

不借：`proc-macro-srv` 的实现本身（`rustc_private`、只能 nightly/树内构建，加载的是 stock rustc 的 dylib）。kfrc 的服务端加载**kfrc 自己编出的**宏 dylib，bridge 由 kfrc 的 `proc_macro` 库定义（方案 B）。

## 要避开的坑

- **不要兼容 RA 的协议。** 兼容意味着跟随它的版本号（已到 6，且仍有 `Prototype` 字样的协议名），而 kfrc 不需要与 RA 互通。
- **`SetConfig`/`SpanMode` 的两种 span 模式**（`Id` 与 RA span）是为兼容旧服务端；kfrc 只有一种。
- **span 回调要保证确定性。** 子请求让宏能读源文本；kfrc 若缓存展开结果，`SourceText` 读到的内容也必须进入键（推断：最简单是声明“读过源文本的展开不跨构建缓存”，与 [cache.md](../../design/build/cache.md#跟踪不全的输入) 一致）。
- **RA 把宏 panic 编码成 `PanicMessage` 响应，变成展开错误后继续分析**；kfrc 按 rustc：宏 panic 是编译错误（`ProcMacroPanicked`），该 crate 编译失败。

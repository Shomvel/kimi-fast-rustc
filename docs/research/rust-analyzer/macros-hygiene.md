# 宏展开与 hygiene：展开身份、span 锚点、`macro_rules` 引擎

对应任务：[T04](../../plan/m1/t04-syntax.md)（span 预留）、[T23](../../plan/m3-semantics.md#t23)。rustc 一侧见 [macros-resolve/](../rustc/macros-resolve/README.md)。

## 三处对照

| 概念 | rustc 1.94.1 | RA `03ae6be5` |
|---|---|---|
| 一次展开的身份 | `LocalExpnId`，按创建顺序分配的整数；`ExpnData` 存调用点、宏定义、edition | [`MacroCallId`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-expand/src/lib.rs#L1554) = Salsa 驻留的 [`MacroCallLoc { def, krate, kind, ctxt }`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-expand/src/lib.rs#L239)；`kind` 里是调用点的 `AstId` |
| 展开结果所在“文件” | 片段直接并入 crate AST | `HirFileId = 真实文件 | MacroFile(MacroCallId)`，可递归；宏文件有自己的 item tree 和 AstId |
| 卫生上下文 | `SyntaxContextData`：外层展开、透明度、父 ctxt、两种规范化 | [`SyntaxContextData`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/span/src/hygiene.rs#L38)：`outer_expn`、`outer_transparency`、`edition`、`parent`、`opaque`、`opaque_and_semiopaque`；源码注释称与 rustc“基本相同” |
| span | `lo/hi` 绝对字节偏移（增量模式下相对父项编码）+ ctxt | `Span { range, anchor: SpanAnchor { file_id, ast_id }, ctx }`，[`range` 相对锚点项](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/span/src/lib.rs#L123) |

## 值得借的设计

**1. 展开身份由调用位置决定，与完成顺序无关。** `MacroCallId` 是 `(宏定义, crate, 调用点 AstId, 调用处 ctxt)` 的驻留值；调用点 AstId 又属于某个 `HirFileId`，后者可以是外层宏文件。所以同一调用无论第几个被展开，身份都一样。rustc 的 `LocalExpnId` 依赖展开顺序确定，kfrc 计划里“并行展开时按展开树中的位置定序”（[T05](../../plan/m1/t05-resolve.md#defkey-与-hygiene)）就是 RA 这套做法。差别在持久化：RA 的身份是会话内驻留号，kfrc 需要把这条链哈希成持久键（见 [resolve-defmap.md](resolve-defmap.md#身份方案)）。

**2. span 相对锚点项。** RA 的 span 记“锚点项 + 项内偏移”，在函数体里加一行不改变其他项内 token 的 span，从而不使其他项的展开结果失效。kfrc [cache.md](../../design/build/cache.md#源位置隔离) 要求行号相对定义起始行，同一思路；RA 证明它可以贯穿宏展开（过程宏协议 v6 `HASHED_AST_ID` 也按锚点传 span）。

**3. 卫生表结构。** `SyntaxContextData` 预存两种规范化结果（`opaque`、`opaque_and_semiopaque`），比较时不必沿链走。这与 rustc 的 `normalize_to_macros_2_0`/`normalize_to_macro_rules` 对应，T04 定义卫生表时可直接采用“驻留时算好两种规范化”的形状。

**4. `macro_rules` 匹配器。** RA 的 [`mbe/src/expander/matcher.rs`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/mbe/src/expander/matcher.rs#L1) 头注释写明是 rustc `macro_parser.rs` 的 NFA 移植。它展示了“匹配器只依赖 token tree + 一个按 fragment 种类解析前缀的回调”的边界：`ident`/`lifetime`/`literal`/`tt` 在匹配器里直接按 token 判断（[L816](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/mbe/src/expander/matcher.rs#L816)），其余交给 parser 的 `PrefixEntryPoint`。kfrc T23 的模块边界可以照此划分；算法细节仍以 rustc 为准。

## 不能照搬的部分

| RA 行为 | 位置 | 为什么 kfrc 不能这样做 |
|---|---|---|
| 内建 derive 不真正展开，只登记 `BuiltinDeriveImplId`，再由类型层合成 impl | [hir-def/src/builtin_derive.rs](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/builtin_derive.rs#L1)（“To save time and memory, builtin derives are not really expanded”） | kfrc 要生成函数体，形状必须与 rustc 展开一致（`Clone` 对 `Copy` 类型用 `*self`、`Debug` 用 `debug_struct_field*_finish`）。直接从 ADT 数据合成 Body 可以作为以后的性能选项（推断），但输出必须与 rustc 展开逐项对拍 |
| 给属性宏的输入先做语法修补 | [hir-expand/src/fixup.rs](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-expand/src/fixup.rs#L1) | 编辑中的残缺代码才需要；kfrc 遇到解析错误就停在该项 |
| 未实现的内建宏登记为 `UnimplementedBuiltIn` 继续 | [lib.rs L267](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-expand/src/lib.rs#L267) | 红线 1：明确报“未支持” |
| 属性过程宏展开可被配置关闭 | [hir-def/src/lib.rs L119](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/lib.rs#L119) | 语义配置不可关 |
| 展开错误返回“部分结果 + 错误”继续用 | `ExpandResult` 贯穿 `hir-expand` | 错误展开的片段不能进入解析之后的阶段 |

## 建议做法

1. **T04：** span 采用 `(锚点定义, 相对区间, ctxt)`；锚点在名称解析前只能是“文件内项序号”，等 DefKey 确定后映射（推断：需要在 T02 冻结时决定锚点是 AST 级还是 DefKey 级）。卫生表按 RA 的形状预存两种规范化。
2. **T23：** 展开身份 `ExpnKey` 按调用位置定义（外层展开键 + 调用点在该片段内的位置 + 宏定义 DefKey），不按创建顺序；`ExpnData` 的其余字段跟 rustc。
3. **T23 测试：** RA 的 `hir-def/src/macro_expansion_tests/`（`mbe.rs` 2,059 行、`mbe/regression.rs` 1,207 行、`builtin_*` 两个文件）可作为输入语料；期望输出用 rustc 1.94.1 的 `-Zunpretty=expanded` 重新生成，不用 RA 的快照。
4. **不读：** `fixup.rs`、`eager.rs` 的降级路径、`prettify_macro_expansion_.rs`（只为显示）。

## 阅读顺序

| 先读（RA，结构） | 再核对（rustc，规则） |
|---|---|
| `span/src/hygiene.rs`、`hir-expand/src/hygiene.rs` | [`rustc_span/src/hygiene.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs)、[hygiene.md](../rustc/macros-resolve/hygiene.md) |
| `hir-expand/src/lib.rs`（`MacroCallLoc`、`HirFileId`） | [`rustc_expand/src/expand.rs`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs)、[expansion-loop.md](../rustc/macros-resolve/expansion-loop.md) |
| `mbe/src/expander/{matcher,transcriber}.rs` | [`rustc_expand/src/mbe/`](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe)、[macro-rules.md](../rustc/macros-resolve/macro-rules.md) |
| `span/src/ast_id.rs`、`span/src/map.rs` | rustc 无对应（相对 span 只在增量编码里），按 kfrc [cache.md](../../design/build/cache.md#源位置隔离) |

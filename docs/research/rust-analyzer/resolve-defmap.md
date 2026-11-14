# 名称解析：RA 的 DefMap 与 kfrc 的 DefKey、展开-解析迭代

对应任务：[T05](../../plan/m1/t05-resolve.md)、[T23](../../plan/m3-semantics.md#t23)。rustc 一侧见 [name-resolution.md](../rustc/macros-resolve/name-resolution.md)、[imports.md](../rustc/macros-resolve/imports.md)。

## RA 怎么做

- **产物。** [`DefMap`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres.rs#L174)：模块树 + 每个模块的 `ItemScope`（三个命名空间的 `PerNs`）+ 前奏 + `macro_use` 前奏 + derive helper + 诊断。整个 crate 一份（[`crate_local_def_map`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres.rs#L398)），**每个含项的块再懒建一份**（[`block_def_map`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres.rs#L432)）。
- **输入。** 每个文件（含宏文件）的 item tree；`DefCollector` 遍历模块树，种名字，同 crate 的 `macro_rules` 在收集时按文本顺序立即展开（[nameres.rs 头注释](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres.rs#L1)）。
- **不动点。** [`resolution_loop`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/collector.rs#L420) 三层循环：导入到不动点 → 解析并展开一批宏 → 重复；全部稳定后取一个未解析的属性宏，[当作不存在](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/collector.rs#L489)把被修饰项重新喂回。超过 [`FIXED_POINT_LIMIT = 8192`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/collector.rs#L61) 轮只记 `tracing::error!("name resolution is stuck")` 并退出。
- **导入状态。** [`PartialResolvedImport`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/collector.rs#L124)：`Unresolved | Indeterminate(PerNs) | Resolved(PerNs)`。`Indeterminate` 指“部分命名空间已找到”，与 rustc 的 `Undetermined`（“以后可能改变”）不是一回事。解析到的结果立即写入作用域（[`resolve_imports`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/collector.rs#L792)），收尾时连未解析的导入也记录下来，“供补全使用”。

## 与 rustc 的差异（源码核实）

| 规则 | rustc | RA |
|---|---|---|
| 歧义（E0659）、受限遮蔽 | glob/宏展开名字不能遮蔽外层名字，冲突即错误 | `DefDiagnosticKind` 没有歧义或重复定义的变体（检索 `ambigu`/`duplicate`/`E0659`/`E0428` 无结果）；先到先得 |
| finalize 复核 | 展开结束后重解析每个导入和宏路径，结果变化即报错 | 无 |
| 不动点失败 | `force` 模式把未定的都报错 | 8192 轮上限后静默退出；未解析属性宏当作不存在 |
| 块内的项 | 匿名模块，属于同一张模块图 | 每块一份懒建的 `DefMap` |
| 块内的 impl | 全局可见（coherence、方法解析都看得到） | 方法解析只收集 crate `DefMap`、`const _: () = { … }` 里的块和当前位置的外层块（[method_resolution.rs L600–L650](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/method_resolution.rs#L600)）；别的函数体里的 impl 看不到 |

这些都是为 IDE 的懒惰和容错有意为之，结论：**名称解析的算法“不参考（以 rustc 为准）”**。T05 已经要求三态查找、导入不动点、歧义和 finalize，保持不变。

## 身份方案

两边都要回答：“定义的身份怎样不随无关修改、不随完成顺序变化”。

| | rustc / kfrc 当前设计 | RA |
|---|---|---|
| 定义身份 | `DefKey { parent, data, disambiguator }`，disambiguator 按 `(parent, data)` **创建顺序**计数；不含 hygiene 和展开 | `ItemLoc { container: ModuleId, id: AstId }`（[lib.rs L127](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/lib.rs#L127)），会话内驻留 |
| 文件内编号 | — | [`AstId`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/span/src/ast_id.rs#L1) = 种类 + 名字（嵌套项再加父项）的 16 位哈希 + 同哈希序号；注释说明 AstId 失效会连锁让所有 trait 求解失效，所以力求稳定 |
| 宏生成的项 | 父模块下的普通子项，靠 disambiguator 区分 | 位于宏文件 `MacroFile(MacroCallId)` 中，身份天然带上展开链 |

RA 的方案说明了一件事：**把展开链放进身份**，就不需要“按完成顺序分配”，也就没有并行展开的定序问题。kfrc 有两个选择（交给 T02 冻结）：

- **A. rustc 形状。** DefKey 不含展开；同一 `(parent, data)` 的 disambiguator 按展开树的先序位置分配（调用点在父片段中的位置排序）。代价：一个父模块的 disambiguator 要等该模块下所有展开完成才能确定（推断：只在出现同名冲突时才需要等待，可以先发临时键）。
- **B. 借 RA。** 宏生成项的 `DefPathData` 额外带 `ExpnKey`（调用位置链的哈希），disambiguator 只在同一展开内计数。没有屏障；DefKey 与 rustc 的 `DefPathHash` 不再一一对应（kfrc 自有 metadata 和符号命名，不需要一致）。

两种都要把 RA 的 16 位哈希换成完整指纹：RA 的 AstId 是会话内身份，碰撞时靠序号兜底，同名同种类项的插入会让后面的序号改变。

## 建议做法

1. 读 `collector.rs` 的 `resolution_loop`、`resolve_imports`、`update_recursive`（glob 传播）看工程组织；规则一律对照 `rustc_resolve/src/imports.rs` 移植。
2. 借 `PerNs`（三命名空间一组结果）和 `ItemScope` 的存储形状；不借 `Indeterminate` 的语义。
3. 块内项：保持 rustc 的“一张模块图”语义，但可以像 RA 一样按块分片建表，只要 impl、trait、`macro_rules`（`#[macro_export]`）这些全局可见的东西在 crate 级登记（见 [item-tree.md](item-tree.md#体内的项)）。
4. 负例语料：RA `hir-def/src/nameres/tests/`（`macros.rs` 1,772 行、`mod_resolution.rs` 1,046 行等）作输入，期望值由 rustc 生成；RA 接受而 rustc 拒绝的用例正好覆盖歧义规则。

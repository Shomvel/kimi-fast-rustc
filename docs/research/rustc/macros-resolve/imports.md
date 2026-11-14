# 导入解析：不动点、glob、遮蔽与歧义

源码：`compiler/rustc_resolve/src/imports.rs`，查找谓词在 `ident.rs`，登记在 `build_reduced_graph.rs`。

## 数据结构

- [`ImportData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L145)：`kind`、`module_path: Vec<Segment>`、`parent_scope`（导入所在的完整 `ParentScope`，含 macro_rules 链）、`imported_module: Cell<Option<ModuleOrUniformRoot>>`、`vis`、各种 span。
- [`ImportKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L62)：`Single { source, target, decls: PerNS<Cell<PendingDecl>>, type_ns_only, nested, id }`（每个命名空间独立成败）、`Glob { max_vis, id }`、`ExternCrate`、`MacroUse`、`MacroExport`。[`PendingDecl`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L45) = `Ready(Option<Decl>)` | `Pending`。
- [`NameResolution { single_imports, non_glob_decl, glob_decl }`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L243)：每个 `(module, BindingKey)` 一份。`binding()` 在仍有待定单项导入时隐藏 glob 结果。
- [`build_reduced_graph_for_use_tree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/build_reduced_graph.rs#L550) 把嵌套 use 树拆成多条 `Import`；单项导入登记时就把自己加进目标名字的 `single_imports`。

## 不动点 `resolve_imports`

[`resolve_imports`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L576)：反复扫描 `indeterminate_imports`，对每条调用 [`resolve_import`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L864)，它返回剩余不定数（模块路径未定记 3，每个命名空间未定记 1）；总数不再下降就停。展开器在每个 `fully_expand_fragment` 开头和每轮结束各调一次（[expansion-loop.md](expansion-loop.md)）。

`resolve_import`：

1. 模块路径用 `maybe_resolve_path(..., ignore_import = 自己)`，避免“自己证明自己”；`Indeterminate` → 3；失败 → 0（留给 finalize 报错）。
2. `Glob` → [`resolve_glob_import`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L1505)：登记到源模块的 `glob_importers`，把源模块当前**已确定**的绑定逐个复制过来（可见性检查 + `reverse_glob_adjust` 卫生调整）。之后源模块每出现新绑定，[`update_local_resolution`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L483) 会推给所有 glob 导入者（推送模型，可级联、可成环）。
3. `Single`：每个命名空间 `maybe_resolve_ident_in_module`：成功 → `new_import_decl` 并种进本模块；确定失败 → 从 `single_imports` 移除；不定 → 计数。

## 种名字与遮蔽

[`try_plant_decl_into_local_module`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L422)：

| 旧 \ 新 | 非 glob | glob |
|---|---|---|
| 无 | 写 `non_glob_decl` | 写 `glob_decl` |
| 非 glob | **重复定义错误** | 都保留，非 glob 优先 |
| glob | 都保留，非 glob 优先 | [`select_glob_decl`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L352)：同一 `Res` 取更大可见性；不同 `Res` 在声明上记 `ambiguity`（只有被使用时才报错） |

`Res::Err` 不覆盖真实声明（错误恢复）。`_` 名字每次分配新的 disambiguator，所以从不冲突。

## 查找时的确定性谓词

- 非 glob（[ident.rs L1001](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L1001)）：有非 glob 绑定 → 确定（不可访问则确定失败）；有待定单项导入可能定义它（[`single_import_can_define_name`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L1268)）→ 阻塞；模块还有未展开调用 → 不定但允许继续向外找；否则确定不存在。
- glob（[ident.rs L1064](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L1064)）：glob 绑定只有在 [`determined()`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L1055)（导入所在模块已无未展开调用且源绑定也确定）、或在 MacroNS、或处于受限遮蔽查找时才算确定；没有绑定时要确认没有未展开调用、所有可访问的 glob 源模块都已解析且确定不含该名。
- crate 根的特殊性：任何模块里的 `#[macro_export]` 都能往根里种宏，但为避免卡死（#53144）不把其他模块的未展开调用算进根；代价是宏展开出的 `macro_export` 宏被绝对路径访问时只报 lint（[ident.rs L1223](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L1223)）。
- 导入环：`resolution_or_default(..).try_borrow_mut` 失败即视为确定失败，防止无限递归。

## 受限遮蔽（restricted shadowing）

词法查找（非 `ScopeSet::Module`）时，来自 glob 或宏展开的名字是“弱”的：它们**不能**遮蔽外层作用域的同名项，冲突时报歧义而不是选内层（`GlobVsOuter`、`MoreExpandedVsOuter`、`GlobVsExpanded`、`MacroRulesVsModularized`，见 [`maybe_push_ambiguity`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/ident.rs#L759)）。这样早期解析得到的结果在之后的展开中不会被推翻，整个过程不需要回溯。几个兼容性特例（`issue_145575_hack`、`issue_149681_hack`、2024 及更早 edition 下 core/std `panic` 冲突降为警告）写在同一函数里。

[`may_appear_after`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/lib.rs#L1039)：展开构成偏序；若某声明的展开既不是使用点展开的祖先，也不是另一候选展开的祖先，它就“可能更晚出现”，与外层结果冲突时报 `MoreExpandedVsOuter`。完整组合表见 [tests/ui/macros/restricted-shadowing-legacy.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/macros/restricted-shadowing-legacy.rs)。

## 收尾 `finalize_imports`

[`finalize_imports`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L595) → 每条 [`finalize_import`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L966)：以 finalize 模式重解析，报告未解析/歧义/隐私错误，检查重导出可见性（`pub use` 私有项），合并同一 use 树的错误；[`finalize_resolutions_in`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L1568) 生成 `module_children`（写入 metadata 的重导出表）。force 模式下仍不定的导入种 `Res::Err` 占位（[`import_dummy_binding`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L538)）避免级联错误。

## 宏相关的导入

- `use some_macro_rules;` 能成立：单项导入在 `CurrentScope` 下走 `ScopeSet::All(MacroNS)`，经过导入所在位置的 MacroRules 链。
- `#[macro_export]` → `MacroExport` 导入种进 crate 根；`#[macro_use] extern crate` → `MacroUse` 导入进 `macro_use_prelude`（按 Symbol，非宏产生时允许遮蔽）。
- 内建宏同名的 glob 冲突例子：[tests/ui/imports/glob-shadowing.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/imports/glob-shadowing.rs)（`use m::*` 引入的 `env` 与前奏 `env!` 冲突报歧义）。

## 对 kfrc 的含义

- 状态单元是 `(module, BindingKey) → NameResolution`。最直接的正确移植是复刻 rustc 的“全量重扫 + 不定计数下降”；改成事件驱动（新绑定、模块解析完成、调用展开完成时唤醒等待者）可以更快，但三类确定性谓词必须一字不差，否则会在不同顺序下得到不同结果。
- glob 需要“导入者列表 + 推送”与环检测；歧义要记在声明上、使用时才报。
- finalize 阶段的重解析与一致性检查是安全网：kfrc 应保留，并把“早期结果 ≠ 最终结果”当作内部错误。
- `module_children`（含重导出、可见性、歧义标记）就是下游 crate 名称解析的输入，T29 的自有 metadata 要原样提供。

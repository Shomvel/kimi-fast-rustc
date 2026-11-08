# Salsa durability 与 kfrc 缓存

对应设计：[design/build/cache.md](../../design/build/cache.md)、[design/build/pipeline.md](../../design/build/pipeline.md)、[design/ids.md](../../design/ids.md)。先例综述见 [ct-incremental-cache.md](../prior-art/ct-incremental-cache.md)（Salsa 与 durability 一行）。

## RA 怎样用 Salsa（源码核实）

- **版本与特性：** `salsa 0.28.5`，特性 `rayon`、`salsa_unstable`、`macros`、`inventory`、`triomphe`（[Cargo.toml L126](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/Cargo.toml#L126)）。没有启用任何持久化特性；在 `base-db`、`hir-def`、`rust-analyzer` 中检索 `persist` 无结果。**RA 的派生数据全在内存里，进程重启从头算。**
- **durability 分配：** 输入写入时带级别。

  | 输入 | 库（sysroot、registry） | 工作区 |
  |---|---|---|
  | 文件文本 | `HIGH` | `LOW` |
  | source root | `MEDIUM` | `LOW` |
  | crate 图、crate 数据 | `MEDIUM` | `MEDIUM` |
  | 过程宏表、属性宏开关 | `HIGH` | `HIGH` |

  出处：[change.rs L93–L99](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/base-db/src/change.rs#L93)、`input.rs` 中 crate 数据的 `with_durability(MEDIUM)`、[proc_macro.rs L98](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-expand/src/proc_macro.rs#L98)。
- **机制（Salsa 文档，见 prior-art）：** 每个 durability 级别记录“最后一次变化的修订号”。一个派生值只依赖高级别输入、而该级别自上次验证后没变，就整片跳过逐边验证。没有它时，编辑 `src/lib.rs` 要重验 std 相关查询约 300 ms（matklad 2023）。
- **backdating：** 重算结果与旧值相等时保留旧的“变化修订号”，下游不再重算，即 early cutoff。
- **LRU：** 函数体查询 `lru = 512`，丢掉的可以重算（[body.rs L89](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/expr_store/body.rs#L89)）。
- **驻留：** `MacroCallId`、`SyntaxContext`、各种 `*Id` 都是 `#[salsa::interned(revisions = usize::MAX)]`，会话内永不回收。
- **环与取消：** 环用 `cycle_result` 给回退值（推导给空结果 + `has_errors`）；新修订到来时，长查询在 `unwind_if_revision_cancelled` 处以 unwind 方式取消（[collector.rs L428](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/collector.rs#L428)）。

## 对照

| 维度 | Salsa / RA | kfrc |
|---|---|---|
| 生命周期 | 一个编辑会话、一个进程 | 跨构建、跨进程、跨 crate，可落盘 |
| 失效判定 | 修订号 + 沿依赖边验证（验证型 trace） | 键 = 实际读过的输入指纹，命中即复用（构造型 trace + 内容寻址） |
| early cutoff | backdating | 下游键用中间结果哈希（R 图哈希、答案键） |
| 分级跳过 | durability 三级 | 尚无；prior-art 建议“sysroot/registry 一次验证” |
| ID | 驻留 ID，会话内有效 | 会话 ID + 持久键分层（ids.md） |
| 内存上限 | LRU，可随时重算 | 不存 AST、S 图；产物发布后不可变 |
| 依赖记录 | 读数据库自动记边 | 依赖清单（cache.md），由生成者记录 |
| 环 | 回退值 | 各类环按语言规则报错 |
| 等待 | 按需拉取，在途查询阻塞等待（推断） | 任务返回“依赖未就绪”，不占 worker |

## 借什么

1. **durability 分级，改成持久版。** 每个依赖源属于一个级别：`HIGH` = sysroot（键：工具链哈希）、`MEDIUM` = registry/git 依赖（键：lockfile 条目 + features + target）、`LOW` = 工作区。每个缓存产物记录它读过的**最低级别**。构建开始时先比对各级别的总指纹：只读 `HIGH`/`MEDIUM` 且该级总指纹未变的产物，直接命中，不逐项验证依赖清单。这是 [ct-incremental-cache.md](../prior-art/ct-incremental-cache.md) 第 4 节第 5 条的具体形状；RA 的分配表说明三级就够，crate 图本身归 `MEDIUM`。
2. **依赖记录必须自动化。** Salsa 的正确性来自“读即记边”，用户代码绕不开。kfrc 的依赖清单若由各阶段手写，漏记就是错误命中（W03/W05/W08 的教训）。建议语义表只能经由一个记录读集的访问接口读取，阶段代码拿不到裸表；缓存测试里“故意漏记一次读取必须被发现”（[cache.md](../../design/build/cache.md#指标)）就测这一层。
3. **位置与语义分开存，backdating 才有效。** RA 的 `Body`/`BodySourceMap` 分离让“只移动位置”被 backdating 截断（见 [item-tree.md](item-tree.md)）。kfrc 的等价物是 CheckedBody 哈希不含 span 表。
4. **驻留只追加。** 与 ids.md“驻留表只追加”一致；RA 的 `revisions = usize::MAX` 说明长会话里也不回收驻留值，内存靠 LRU 丢派生数据，而不是丢 ID。

## 不借什么

- **不把 Salsa 作为运行时依赖。** kfrc 的调度是显式任务图 + 一次性发布（[pipeline.md](../../design/build/pipeline.md#任务状态与发布)），与 Salsa 的按需拉取、在途阻塞、修订号模型不同；而且 Salsa 不落盘，kfrc 的主要收益在跨构建复用。固定决策“不把 query 运行时包进产品”虽然针对 `TyCtxt`，同样的理由适用。
- **不用回退值处理环。** RA 的 `cycle_result` 让 IDE 在环上给出“有错误”的空结果；kfrc 必须报出具体的环。
- **不用 unwind 取消。** kfrc 的取消在安全点检查标志，任务结果不发布即可。
- **不照搬 RA 的 16 位 AstId 哈希作持久键**（见 [resolve-defmap.md](resolve-defmap.md#身份方案)）。

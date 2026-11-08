# Body、CheckedBody 与产物序列化

对应 [body.md](../../design/frontend/body.md)、[cache.md](../../design/build/cache.md#存储形式)、[pipeline.md](../../design/build/pipeline.md#产物依赖)。

## Body 的紧凑表示

[body.md](../../design/frontend/body.md#body-结构) 的草案沿用 MIR 形状：`BlockData { stmts: Vec<Stmt>, term }`、`Rvalue::Aggregate(_, Vec<Operand>)`、`Call { args: Vec<Operand>, … }`。形状应保留（借用检查、move 分析依赖它），**存储**可以扁平化。

rustc 1.94.1 对照【来源】：`Statement` 56 B、`Terminator` 96 B、`BasicBlockData` 152 B、`Rvalue` 40 B、`Place` 16 B；`Assign` 把 `(Place, Rvalue)` 装箱，每条赋值一次堆分配。

建议（【推断】）：

```rust
pub struct Body {
    locals: IndexVec<LocalId, LocalDecl>,
    blocks: IndexVec<BlockId, BlockRec>,      // { stmts: Range<StmtId>, term: TermId, flags }
    stmts:  IndexVec<StmtId, StmtRec>,        // 全函数连续；块内语句是一段区间
    terms:  IndexVec<TermId, TermRec>,
    operands: Vec<Operand>,                   // Aggregate/Call 的变长实参，按区间引用
    projs:  Interned<ProjList>,               // Place 的投影列表驻留（会话内）
    spans:  IndexVec<StmtId, SpanId>,         // 源位置旁表，冷数据单独一列
}
pub struct Place { local: LocalId, proj: ProjListId }          // 8 B（rustc 16 B）
pub enum Operand { Copy(Place), Move(Place), Const(ConstId) }  // 12 B
```

| 设计点 | 说明 |
|---|---|
| 语句连续存放 | Body builder 按块顺序生成，块就是 `stmts` 中的区间；插入 drop/`StorageDead` 等“清理”步骤生成新数组而不是在中间插入 |
| `StmtRec` ≤ 32 B，不装箱 | `Assign` 的 `Rvalue` 内联；变长部分（Aggregate 字段、Call 实参）进 `operands` 区间 |
| span 放旁表 | 源位置不进代码身份（设计红线 2），也是借用检查之后的冷数据 |
| `TypeckResults` | 按 `ExprId`/`PatId` 稠密编号用 `IndexVec`，不用哈希表；rustc 的 `node_types` 是 `ItemLocalMap`（哈希表）【来源】 |
| CheckedBody | = Body + 借用检查结果（只留下游要的：drop flag 分类、闭包捕获）；借用检查内部的 region 推导表任务结束即丢 |

估算（【推断】）：rustc MIR 每条赋值约 56 B + 56 B 装箱 ≈ 112 B，外加每块一个 `Vec`；扁平后约 24–32 B/语句、每块 0 次分配。

## 产物的两种编码

| 产物 | 读法 | 编码 | 依据 |
|---|---|---|---|
| R 图（metadata 中的函数体）、作为内联源的 S 图 | 整体读入、特化时解码 | **varint 节点流**：op、flags、立即数、输入写成相对本节点的回指距离（zigzag） | T4：5.4–5.7 B/节点，解码 11–18 ns/节点【实测】；rustc metadata 也用 LEB128 + 类型 shorthand【来源】 |
| 定义表、接口签名、孔位表、符号/实例索引、调用摘要 | 随机查找、多个下游反复读 | **定宽小端记录**，mmap 后零拷贝 | T4：打开只要 1.7–5 ns/节点（越界校验）【实测】；Zig ZIR 4 个数组一次 `writev`【来源】 |
| CheckedBody（只供常量求值） | 少量读 | varint 流 | 冷数据 |
| 机器码片段 | 拼接 | 原始字节 + 定宽重定位表 | 由链接器消费 |

### varint 节点流格式（草案）

```text
graph   := header node* type_table const_table hole_table
header  := magic schema_version node_count(varint) …
node    := op(u16 LE) flags(u8) [imm(varint)] n(varint) (backref(zigzag varint))* ty(varint 文件内类型下标)
```

- 序列化前**按原顺序压实** `NodeId`：回指距离小，varint 才短；顺序打乱后距离变成均匀分布，字节数会上升。
- 会话 ID（`TyId`、`DefId`、`InstanceId`）全部换成文件内表的下标；表的内容用持久键写入（[ids.md](../../design/ids.md#规则)）。
- 读时一次解码进 [node-storage.md](node-storage.md) 的 `FunctionGraph`；解码与“边复制边折叠”的 `specialize` 合成一遍（【推断】：省一次全图写入）。

### 定宽零拷贝的条件

| 条件 | 做法 |
|---|---|
| 对齐 | 每个数组起点按 8 B 对齐；记录用 `#[repr(C)]` 和显式小端 |
| 校验 | 打开时检查魔数、版本、长度；下标越界检查可按数组做一次顺序扫描（T4 实测的成本），之后访问不再检查 |
| 不可信输入 | 缓存目录可能被篡改或损坏：校验失败当 miss（现状 `native_cache.rs` 已是“错误降级为 miss”），不能 panic |
| 字节序 | 只支持小端主机读写；大端主机按 miss 处理 |

## 对现状的差距

现状没有序列化 SoN 图：`native_cache.rs` 只缓存对象文件，键是整个程序图的 SHA-256。新链第一次需要序列化的是发布到 metadata 的 R 图，格式应与 T02 冻结 ID 一起定；在那之前不要让任何缓存键依赖 `NodeId` 数值。

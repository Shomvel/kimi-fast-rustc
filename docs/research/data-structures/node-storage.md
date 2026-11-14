# SoN 节点存储：布局、编号、GVN 表

对应 [graph.md](../../design/ir/graph.md) 的“函数图”“节点”两节。标注：【来源】【实测】【推断】。

## 结论

| 问题 | 建议 | 依据 |
|---|---|---|
| AoS 还是 SoA | **AoS 16 B 核心记录 + 两列 SoA（`ty`、`ctl`）** | T1：遍历类操作 AoS/SoA 差距 ≤20%；Zig 的 SoA 让 `zig fmt` cache miss +34.5%【来源】；Carbon `Inst`、Cranelift `InstructionData` 都是 16 B 定长【来源】 |
| 输入内联还是外部边表 | 内联 3 个，更多进共享溢出池（Cranelift `EntityList` 式）；已发布的只读图用 CSR | T1：三者遍历相近，但内联省一次间接；溢出池每列表 4 B 而 `Vec` 24 B【来源】 |
| use-list | 不持久维护；转发表 + 近似 users 索引 + 饱和计数 | [use-lists.md](use-lists.md) |
| NodeId | u32，级别内只追加、不复用；`DEAD` 标志代替 `Option<Node>`；在级别边界按原顺序压实 | Graal 70% 阈值压缩且保序、C2 重编号【来源】；T1 乱序存放让 DFS 慢 2.6–4.0×【实测】 |
| GVN 表 | `hashbrown::HashTable<(NodeId, u32)>`，存 32 位哈希，Fx | T3：与 `FxHashMap` 同速或更快，内存 43%；SipHash 慢 2.4–3.8×【实测】 |

## 建议布局

```rust
#[repr(C)]
pub struct NodeRec {            // 16 B，Vec 中连续存放
    op: Opcode,                 // u16：种类 + 子操作（Binary.Add、Check.Bounds、Cast.ZExt …）
    n: u8,                      // 输入数（不含 ctl）
    flags: u8,                  // DEAD | IN_GVN | ON_WORKLIST | USES(2 位饱和计数) | OVERFLOW
    slots: [u32; 3],            // 内联输入；或 [溢出池起点, 立即数, _]；格式由 op 决定
}

pub struct FunctionGraph {
    nodes: IndexVec<NodeId, NodeRec>,   // 16 B/节点
    ctl:   IndexVec<NodeId, NodeId>,    // 4 B/节点；NodeId::NONE 表示浮动
    ty:    IndexVec<NodeId, TyRef>,     // 4 B/节点；见下
    extra: Vec<u32>,                    // 溢出输入、多字立即数（Call 的 callee + 实参）
    consts: ConstPool,                  // 常量值去重，Const 节点只存 ConstId
    holes: Vec<Hole>, level: GraphLevel, state: GraphState, …
}
```

| 设计点 | 说明 |
|---|---|
| 效果输入不带 `PartitionId` | 分区是效果 token 的**类型**（`TyRef::Effect(P)`），由产生 token 的节点决定；契约草案的 `SmallVec<[(PartitionId, NodeId); 1]>` 退化为普通输入。`Call`/`Return`/`Resume` 本来只收一个 `MemMerge`（[nodes.md](../../design/ir/nodes.md#地址与内存)），所以多数节点至多一个效果输入 |
| `ctl` 单独一列 | placement、verifier、冷区域识别沿控制骨架走，只读这一列；纯节点为 `NONE`。`Region` 的多个前驱放在 `slots`/溢出池，`ctl` 列留空 |
| `TyRef` 压成 u32 | 高 3 位是种类（Rust、Int、Float、Ptr、Control、Effect、Tuple、Machine），低 29 位是 `TyId`/`PartitionId`/`TupleTyId`/位宽。R/S/L 三级共用一列，L 级降低时原地改写 |
| 立即数 | `Field[i]`、`Proj(i)`、`Check.kind` 等放空闲槽或 opcode 里；`Const` 存 `ConstId`；`Call` 的 `InstanceId` 放溢出池头部 |
| 访问接口 | `inputs(id) -> &[NodeId]`、`set_input`、`op`、`ty`；**字段私有**，换布局不影响 pass |

### 每节点字节估算（【推断】，按 [bench-method.md](bench-method.md) 的合成分布）

| 部分 | 字节/节点 |
|---|---|
| `NodeRec` + `ctl` + `ty` | 24 |
| 溢出池（`Store` 4 个输入不再溢出，因 `ctl` 移出；`Call`、大 `Phi`/`Region` 溢出） | 约 1–2 |
| 常量池、GVN 表（只含纯节点，约 19 B/条目 × 55%） | 约 1 + 10 |
| **合计（含 GVN）** | **约 36**；不含 GVN 约 26 |
| 对照：现状（T1 实测，未计 malloc 头部） | 96 + 每节点 1.33 次分配 |
| 对照：契约草案（`NodeKind` 16 + `ValueTy` 8 + `Option<NodeId>` 8 + 两个 `SmallVec`） | 约 88，溢出时另有堆分配 |

一个 1k 节点的函数图约 26 KiB，放得进 x86 的 32–48 KiB L1d；现状约 96 KiB 加分散的堆块，放不进。按 cargo 的规模（82,130 个函数、未优化 431 万条 LLVM 指令，[evidence.md](../evidence.md)），假设节点数与指令数同量级，全部 R 图同时驻留时现状约 0.4 GB 以上，新布局约 0.11 GB。

## NodeId 与 tombstone

| 规则 | 理由 |
|---|---|
| 级别内只追加，不复用编号 | 复用会让 worklist、转发表、旁表里残留的 id 指向新节点；Cranelift 同样不提供删除【来源】 |
| 删除 = 置 `DEAD` | 不改变其他节点的 id（现状测试 `removal_leaves_other_ids_stable` 的语义保留） |
| 压实点：R 图发布前、`specialize` 复制时、L 级 placement 前（存活 < 70% 时） | 都是本来就要全图遍历的地方；按原顺序重编号保留创建顺序的局部性（T1）和小的 varint 回指（T4） |
| 旁表一律 `IndexVec<NodeId, T>`/位集 | 1 bit/节点的 worklist 标记、4 B/节点的转发表；不用 `HashMap<NodeId, _>`/`BTreeMap<NodeId, _>` |

## GVN 表

- 键 = `(op, ty, 输入, 立即数)` 的 Fx 哈希，存在表项里（32 位）；比较时才回读节点。
- `IN_GVN` 标志等价于 Simple 的 `_hash != 0`：在表中的节点**不能原地改输入**，要先移出表、改完再插回。违反时 debug 构建 ICE。
- 构建期窥孔用全局表（每图一个）；placement 的作用域 GVN 用另一张表 + 撤销日志（离开支配树作用域时按日志删除）。
- 只放纯节点；固定在控制上的节点不进表（与现状 `expression_key` 一致）。
- 容量按 `nodes.len()` 预留，避免构建中多次扩容（rustc #137354 的教训是预留过大会涨 RSS，只按本图大小预留）。

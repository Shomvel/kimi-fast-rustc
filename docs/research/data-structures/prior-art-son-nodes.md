# 先例（二）：SoN 系统的节点表示——V8、C2、Graal、Simple

全部为【来源】，除标注外均为原始来源（源码按 2026-11 主干读取，行号会漂移）。

## V8：TurboFan（SoN）→ Turboshaft（CFG）

Mercadier, [Land ahoy: leaving the Sea of Nodes](https://v8.dev/blog/leaving-the-sea-of-nodes)（2025-03）：

| 量 | 数字 / 原话 |
|---|---|
| 无效访问 | “平均每 20 次访问才改一次节点” |
| L1 dcache miss | SoN 平均约为新 CFG IR 的 3 倍，个别阶段 7 倍 |
| miss 的代价 | “最多占编译时间 5%，这个数字有点粗略” |
| 整体编译时间 | 换成 CFG 后约减半 |
| 节点为什么大 | “节点在内存中相当大（主要因为每个节点同时有指向输入和使用的指针）” |
| 新 load elimination | “最多快 190 倍，内存少得多”（未给内存数字） |
| Turboshaft 内存降幅 | **未找到原始来源** |

源码中的布局：

| 系统 | 结构 | 来源 |
|---|---|---|
| TurboFan `Node` | `op_`、`type_`、`mark_`（u32）、`bit_field_`（Id 24 位、内联输入数 4 位、内联容量 4 位）、`first_use_`；最多 14 个输入内联，更多时换成 `OutOfLineInputs` | `src/compiler/node.h` |
| TurboFan `Use` | `{next, prev, bit_field_(inline:1, input_index:31)}`，放在节点**前面**，内存顺序依次是 `Use#N…Use#0`、`Node`、`Input#0…`，靠指针运算找回节点 | 同上 |
| TurboFan 节点大小 | “40+ 字节/节点”的开销 | Swirski & McIlroy 幻灯片（约 2017，未注日期） |
| Turboshaft `OperationBuffer` | zone 分配、8 B 槽位数组、只追加；只有循环 Phi 可原地覆写（新操作不更大时） | `src/compiler/turboshaft/graph.h` |
| `OpIndex` | 32 位字节偏移，`id = offset / 16`；注释：“紧凑的 id 空间很重要，它让旁表更小” | `turboshaft/index.h` |
| `Operation` 头 | `opcode: u8` + 饱和的 `use_count: u8` + `input_count: u16`，输入紧跟在操作后面；**只有使用计数，没有 use-list**（“通常只关心 0、1 还是大于 1”）；每个操作至少 2 槽 = 16 B | `turboshaft/operations.h` |
| 每个阶段复制整图 | “非常紧凑，基于每个阶段把整个图完整复制一遍” | 作者 [HN 评论](https://news.ycombinator.com/item?id=43481215)，半原始 |

## HotSpot C2

| 事实 | 细节 | 来源 |
|---|---|---|
| 边 | `Node **_in`（use-def）、`Node **_out`（def-use），两个外置数组 | `opto/node.hpp` |
| 计数 | `_cnt` 必需输入数、`_max` 数组长度（多出来的是优先边）；`_outcnt`/`_outmax`；数组按 2 的幂增长 | 同上 |
| `_idx` | “小而稠密的唯一编号，用来索引辅助数组” | 同上 |
| 分配 | `operator new` 从 `C->node_arena()` 取；“delete 是空操作”；两个 node arena 在变换中轮换新/旧空间 | `node.hpp`、`compile.hpp` |
| 死节点 | 只有最后一个节点能归还编号；其余记入 `_dead_node_list`；`live_nodes = unique - dead` | `node.cpp`、`compile.hpp` |
| 重编号 | `live_nodes() + NodeLimitFudgeFactor(2000) < unique()` 时跑 `PhaseRenumberLive`；`MaxNodeLimit = 80000` | `compile.cpp`、`c2_globals.hpp` |

## Graal

| 事实 | 细节 | 来源 |
|---|---|---|
| 输入 | 节点类的字段，用 `@Input` 注解；按 `NodeClass` 记录的字段偏移经 `Unsafe` 读写 | `graph/Node.java`、`Edges.java` |
| 每节点字段 | `graph`、`id`、`usage0`、`usage1`、`extraUsages[]`、`extraUsagesCount`、`predecessor`、`nodeClass` 等；前 2 个使用内联 | `Node.java` |
| 节点表与压缩 | `Graph.nodes[]` 按 id 索引；存活节点少于 70% 时 `maybeCompress()` 把存活节点挪到前面，**保持原顺序**并重编号 | `graph/Graph.java` |
| 论文 | 边作为字段“紧凑，无需额外数组存直接边”；没有字节/节点或内存测量 | Duboscq 等, [Graal IR](https://ssw.jku.at/General/Staff/GD/APPLC-2013-paper_12.pdf), APPLC '13 |

## Simple（Cliff Click）

| 事实 | 细节 | 来源 |
|---|---|---|
| 节点字段 | `_nid`（稠密 id）、`_inputs`、`_outputs`（`Ary` = 数组 + 长度）、`_type`、`_deps`、`_hash` | `chapter23/.../node/Node.java` |
| def-use | 有：“输出与输入直接对应，可以双向遍历” | 同上 |
| GVN | `HashMap`，键是 opcode + 输入 + 额外位；`_hash` 缓存哈希，同时是“在 GVN 表中”的标志：非零时边被锁定，改边前先移出表 | 第 9 章 README、`Node.java` |
| worklist | `IterPeeps`：数组 + 按 `_nid` 的 `BitSet` 去重 + **随机弹出**（固定种子）；理由：从两端弹出可能退化成二次，随机化击败坏模式 | `IterPeeps.java`、第 9 章 |
| 对 V8 的回应 | [ASimpleReply.md](https://github.com/SeaOfNodes/Simple/blob/main/ASimpleReply.md)（2025-04）：“加上回边后一切都快多了”；维护边“全是 L1 命中”；“C2 的编译即使大量内联，也很容易放进 L2，几乎总在 L1”；**没有给数字** | 原始 |

## 对照与启示（【推断】）

| 维度 | TurboFan | C2 | Graal | Simple | Turboshaft | Cranelift |
|---|---|---|---|---|---|---|
| 输入存放 | 内联 ≤14，否则外置 | 外置数组 | 字段 | 数组 | 紧跟操作 | 内联/池 |
| use 信息 | 双向链表 | `_out` 数组 | 2 内联 + 数组 | `_outputs` | 饱和计数 1 B | 按需计算 |
| 编号 | 24 位 id | `_idx` + 重编号 | id + 70% 压缩 | `_nid` | 字节偏移 | u32 实体 |

1. 保留 SoN 的系统（C2、Graal、Simple、TurboFan）全部维护 def-use；离开或没用 SoN 的（Turboshaft、Cranelift）都不维护，只留计数或按需计算。V8 明确把节点大归因于“同时指向输入和使用”。kfrc 选择不持久维护 use-list，见 [use-lists.md](use-lists.md)。
2. Click 的论点是“编译单元小，全在 L1”。这在节点小时成立：1k 节点 × 16 B = 16 KiB；但现状 kfrc 约 96 B/节点加堆分配，1k 节点就超出 x86 的 L1d（[bench-layout.md](bench-layout.md)）。
3. Graal 压缩时保持顺序、C2 定期重编号、Turboshaft 每阶段复制：三者都在恢复稠密和顺序。kfrc 的 R→S 复制和 S→L 降低天然是压缩点，见 [node-storage.md](node-storage.md)。

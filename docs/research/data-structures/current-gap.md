# 与现有实现的差距和迁移

范围：`src/` 下 [son.rs](../../../src/son.rs)、[son_optimize.rs](../../../src/son_optimize.rs)、[value_numbering.rs](../../../src/value_numbering.rs)、[placement.rs](../../../src/placement.rs)、[regalloc.rs](../../../src/regalloc.rs)、[scheduler.rs](../../../src/scheduler.rs)、[native_cache.rs](../../../src/native_cache.rs)。`son::` 被 21 个源文件引用，所以迁移必须先换存储、保留接口。

## 逐项评估

| 位置 | 现状 | 问题 | 量级（T 表为本机实测） |
|---|---|---|---|
| `son::Node` | `kind: NodeKind`（16 B，`Const(i64)`、`Call(usize)` 撑大）+ 3 个 `Vec<NodeId>`；`Vec<Option<Node>>` | 88 B/节点 + 每节点 1.33 次堆分配；节点无类型 | T1：95.9 B/节点，紧凑布局 19–22 B；克隆 36–64 vs 0.3–1.0 ns/节点 |
| `son::Graph` | 一个程序一张图，`function_graph` 用 `HashSet`/`HashMap` 切出函数 | 新建节点（如折叠出的常量）追加在**全程序图**末尾，离所属函数很远；切分时还要把它们当“外部节点”拉进来 | 局部性随程序变大而变差（T1 乱序存放 DFS 慢 2.6–4.0×） |
| `Graph::replace_uses` | 每次扫描全图所有节点的 3 个通道 | O(N)/次 | T2：64k 节点每次约 0.3 ms，比转发表/use-list 慢约 2,400 倍 |
| `son_optimize::optimize` | 循环跑整图的 `constant_fold`、`algebraic_simplify`、DCE 直到不动点 | 不是变化驱动；`algebraic_simplify` 每次改写调一次 `replace_uses`，整体 O(K·N)；`constant_fold` 每次折叠新建 `Const`，不查重 | 同上 |
| `eliminate_dead_nodes` | `HashSet<NodeId>` 记存活 | 用位集即可 | 哈希表 vs 1 bit/节点 |
| `value_numbering` | 支配信息 `BTreeMap<NodeId, NodeId>`；表达式表 `HashMap<u64, Vec<(Key, NodeId)>>`（标准库 SipHash 再套一层自算哈希）；替换表 `BTreeMap`；外层循环到不动点 | 每次查找 O(log n) 指针追逐；双重哈希；每个桶一个 `Vec` | T3：SipHash 比 Fx 慢 2.4–3.8× |
| `placement::place_graph` | `BTreeMap` 入度/依赖者、`BTreeSet` 就绪集；最后逐节点 `clone()` 生成新图 | O(n log n) + 每节点分配；克隆是 T1 中最贵的操作之一 | T1 克隆列 |
| `regalloc` | `BTreeMap<VirtualReg, Location>` 结果；`BTreeMap<VirtualReg, BTreeMap<…>>` 提示 | `VirtualReg` 本来就是稠密 u32 | 应为 `IndexVec` |
| `native_cache` | 自写 SHA-256，逐字段哈希整个程序图（含每个槽位的 `NodeId` 和 tombstone） | 慢；键依赖删除历史；全程序重算 | T5：逐字段 SHA-256 约 107 ns/节点，SipHash-128 约 6 |
| `scheduler` | 固定 DAG，`Vec<TaskId>` 依赖 | 数据结构没有问题；缺的是字节预算（设计已列） | — |

## 已经做对、应保留的

- `NodeId(u32)` 稠密编号；删除不改变其他 id（测试 `removal_leaves_other_ids_stable`）。
- 控制/数据/内存三个通道分开；`value_numbering` 不合并带控制或内存输入的节点。
- `native_cache` 的原子写、完整性校验、错误降级为 miss。
- `constant_fold` 先收集替换表、最后一次扫描改输入——这正是转发表方案，只是没推广到 `algebraic_simplify`。

## 迁移顺序（【推断】，每步可独立验收）

| 步 | 改动 | 验收 | 对应任务 |
|---|---|---|---|
| 0 | 加计数器：每 pass 访问数/改动数/耗时，图的字节/节点、每节点分配次数（[metrics.md](metrics.md)） | 现有 walkthrough 和测试输出基线 | 先于一切 |
| 1 | `replace_uses` 改为转发表；`algebraic_simplify` 批量生效；`optimize` 改为 worklist；`HashSet`/`BTreeMap<NodeId, _>` 换 `IndexVec`/位集；`value_numbering` 用 `FxHash` 的 `HashTable<(NodeId, u32)>` | 归一化 dump 与改前相同、差分执行一致；O(K·N) 消失 | 不依赖新契约，可立即做 |
| 2 | `Node` 字段改私有，所有调用方经 `inputs(id, channel)`/`set_input`/`kind(id)` 访问 | 21 个文件只改调用方式；行为不变 | T02 前 |
| 3 | 存储换成 `NodeRec` 16 B + `ctl`/`ty` 列 + 溢出池（[node-storage.md](node-storage.md)）；`NodeKind::Const(i64)` 改常量池 | T1 型指标：字节/节点 ≤ 30，分配次数/节点 ≈ 0 | 与 graph.md 的 `FunctionGraph` 一起做 |
| 4 | 一个实例一张 `FunctionGraph`，删掉 `function_graph` 切分；常量在本图内创建 | 新节点落在本图；切分代码删除 | graph.md“一个实例一个图” |
| 5 | placement 改数组 + CSR users，直接输出 Machine IR；`regalloc` 改 `IndexVec` | placement 不再克隆图 | L 级 |
| 6 | 缓存键改为“压实后 varint 流的 128 位哈希” | 改动删除历史不改变键；jobs 不同键相同 | 新缓存 namespace |

## 对契约草案的修改建议

| 文件 | 节 | 改法 |
|---|---|---|
| [graph.md](../../design/ir/graph.md#节点) | 节点 | `Node` 的字段列表改为“逻辑字段”，存储写成 `NodeRec` 16 B + `ctl`/`ty` 列；`effects` 去掉 `PartitionId`（分区由效果 token 的类型给出）；字段私有、经访问器读写 |
| [graph.md](../../design/ir/graph.md#函数图) | 函数图 | `nodes: Vec<Option<Node>>` 改为 `IndexVec<NodeId, NodeRec>` + `DEAD` 标志；写明三个压实点和“压实保序” |
| [graph.md](../../design/ir/graph.md) | 新增“边与使用” | 不持久维护 use-list；转发表、近似 users 索引、饱和使用计数（[use-lists.md](use-lists.md)） |

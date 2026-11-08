# 访问模式：worklist、placement、创建顺序

对应 [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#共同机制) 与 [levels.md](../../design/ir/levels.md#风险v8-离开-son-的教训)。

## 事实

| 事实 | 数字 | 类别 |
|---|---|---|
| V8：SoN 窥孔从 return 反向遍历，平均每 20 次访问才改一次节点 | 20 | 【来源】V8 博客 2025 |
| V8：SoN 的 L1 dcache miss 约为 CFG 的 3 倍（个别阶段 7 倍），估计最多占编译时间 5% | 3× / 7× / 5% | 【来源】同上 |
| Simple：worklist 随机弹出（固定种子），理由是两端弹出会退化成二次 | — | 【来源】`IterPeeps.java` |
| LLVM InstCombine 83% 的访问没有改动 | 83% | 【来源】kfrc 已有剖析，见 [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#共同机制) |
| 1M 节点：随机顺序访问比顺序扫描慢约 3 倍；64k 时约 1.5 倍；4k 时无差别 | T1 | 【实测】 |
| 1M 节点：按随机打乱的存放顺序做 DFS，比创建顺序慢 2.6–4.0 倍 | T1 | 【实测】 |
| cargo：82,130 个函数、431 万条未优化 LLVM 指令（平均约 52 条），48% 的函数 ≤20 条 | — | 【来源】[evidence.md](../evidence.md) |

推论（【推断】）：绝大多数函数图在紧凑布局下只有几 KiB，访问顺序和存放顺序对它们几乎没有影响；影响集中在内联后的大函数（数万节点）。所以**先把节点做小**（[node-storage.md](node-storage.md)），再在大图上管顺序。V8 的 3× L1d miss 是在节点大（输入 + 使用指针）、遍历顺序差两者叠加下测得的。

## worklist

| 设计点 | 建议 | 理由 |
|---|---|---|
| 数据结构 | `Vec<NodeId>` 栈 + `ON_WORKLIST` 标志位（在 `NodeRec.flags` 中，免一张位集） | 去重 O(1)，无哈希 |
| 初始种子 | 按 `NodeId` 升序入队，即创建顺序 | 顺序访问；R 级图按 Body 块顺序建，创建顺序≈程序顺序 |
| 弹出顺序 | LIFO：刚改动节点的使用者先处理，它们刚被碰过，多半还在缓存里 | 局部性；确定性 |
| 防退化 | 每个 pass 的燃料上限（已在设计中）；若某类规则出现二次行为，再对该 pass 换成随机弹出（固定种子，仍确定） | Simple 的经验；先测再换 |
| 入队使用者 | 经近似 users 索引（[use-lists.md](use-lists.md)），多入队是安全的 | 无持久 use-list |
| 计数 | 每个 pass 记录 `visits`、`changes`、`visits_per_change` | 对标 V8 的 20、InstCombine 的 83% |

构建期窥孔（每建一个节点就折叠 + GVN）不经过 worklist，它只碰新节点和它的输入，访问天然是最近创建的节点。

## placement（L 级末尾一次）

现状 `placement.rs` 用 `BTreeMap<NodeId, usize>` 记入度、`BTreeMap<NodeId, Vec<NodeId>>` 记依赖者、`BTreeSet` 做就绪集，最后克隆节点生成新图。每次查找 O(log n) 且是指针追逐；`Vec` 套 `Vec` 每节点一次分配。

建议（【推断】）：

| 步骤 | 数据 | 访问模式 |
|---|---|---|
| 进入时建反向索引 | CSR：`user_start: IndexVec<NodeId, u32>` + `users: Vec<NodeId>`，12 B/节点，6–19 ns/节点（T2） | 两次顺序扫描 |
| 控制骨架 RPO、支配树 | `IndexVec<BlockId, _>`，块号按 Body 块顺序 | 骨架只读 `ctl` 列 |
| schedule early | `block: IndexVec<NodeId, BlockId>`、`depth` 数组；沿输入 DFS | 与 T1 的 DFS 同型；创建顺序存放时最快 |
| schedule late | 沿 CSR 使用者求 LCA，选循环深度最浅的块 | CSR 连续读 |
| 块内顺序 | 同一块内按 `NodeId` 升序为平局规则，再按依赖拓扑 | 与 [levels.md](../../design/ir/levels.md) 的“保留 Body 块顺序作为平局规则”一致 |
| 输出 | 直接生成 Machine IR，不先克隆出一张新的 SoN 图 | T1：现状布局克隆 36–64 ns/节点 |

作用域 GVN、LICM、再物化在 schedule late 的同一遍完成（Cranelift elaboration 的做法，[egraph-rewrite.md](../prior-art/egraph-rewrite.md)）。

## 创建顺序要保住的地方

| 环节 | 风险 | 做法 |
|---|---|---|
| R 级 builder | 先建所有块的 Phi、最后补回边会打乱 | Braun 式 sealed 构造按块顺序推进；Phi 在块开始时建 |
| 内联拼接 | 被调者节点追加到图尾，离调用点远 | 内联后若图增长超过一半，在下一个压实点按“调用点位置插入被调者区间”的顺序重编号 |
| `specialize` 复制 | 按 worklist 顺序复制会打乱 | 按原 `NodeId` 升序复制，跳过不可达节点，即保序压实 |
| L 级原地降低 | 一个 R 节点展开成多个 L 节点追加在尾部 | 同上，在 placement 前的压实点恢复 |

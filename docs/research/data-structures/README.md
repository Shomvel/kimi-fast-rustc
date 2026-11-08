# 编译器自身的数据结构

kfrc 要比 rustc 编译快，又选了 V8 因缓存局部性差而放弃的 Sea of Nodes。本目录回答：节点、类型、Body 和产物该怎么存，现有 `src/` 差多少，以后测什么。

标注：【来源】有原始出处，记年份；【实测】本目录微基准的本机数字（Apple M3 Pro），只作相对比较；【推断】由前两者推出的设计判断。找不到原始来源的写明“未找到原始来源”。

## 结论

1. **V8 的局部性问题主要来自“节点大 + 遍历乱”，不是 SoN 本身。** V8 把节点大归因于“同时指向输入和使用”，测得 SoN 的 L1d miss 约为 CFG 的 3×，个别阶段 7×，估计最多占编译时间 5%【来源】。保留 SoN 的 C2、Graal、TurboFan 都维护 use-list；Turboshaft 和 Cranelift 不维护，只留计数或按需计算【来源】。
2. **节点做到 16 B 定长 + 两列 4 B**，比现状小约 4 倍。现状每节点 96 B、1.33 次堆分配；紧凑布局 19–22 B、几乎零分配。克隆快 1–2 个数量级；1M 节点时 DFS 快约 2.6×【实测】。Zig、Carbon、Cranelift、Turboshaft 的节点都在 8–16 B【来源】。
3. **不持久维护 use-list。** 它每节点多 30 B，比节点本身还大。改用转发表（4 B/节点）、在复制图时顺便建的近似 users 索引和饱和使用计数，替换速度相同【实测】。
4. **现状最大的问题是算法，不是布局。** `replace_uses` 每次扫描全图，64k 节点时比替代方案慢约 2,400 倍；`optimize` 循环跑整图直到不动点；placement 和 value numbering 用 `BTreeMap`；GVN 用 SipHash（比 Fx 慢 2.4–3.8×）【实测】。这些不依赖新契约，可以先改。
5. **创建顺序值得保住。** 1M 节点图乱序存放时 DFS 慢 2.6–4.0×【实测】。Graal 压缩时保序、C2 重编号、Turboshaft 每阶段复制整图【来源】。kfrc 的 R→S 复制和 S→L 降低天然是保序压实点。
6. **并行驻留要让读路径不写共享缓存行。** 只按 rustc 方式分 32 片，8 线程时吞吐基本不涨（争用率只有 0.4–2.3%）；加每任务本地缓存后 3.4×【实测】。rustc 并行前端内存最多 +35%，Zig 多线程峰值 RSS +28.1%【来源】。
7. **哈希分两类。** 会话内用 Fx；持久键用 128 位 SipHash 或 BLAKE3。逐字段 SHA-256（现状缓存键的写法）比 SipHash-128 慢约 16 倍。R 图哈希直接取压实后 varint 流的哈希：varint 流只有原样记录 29% 的字节【实测】。
8. **产物两种编码。** 冷的、整体读的 R 图用 varint 节点流，约 5.5 B/节点；热的、随机查找的表用定宽零拷贝。rustc metadata 是 LEB128 加 shorthand，Zig 的 ZIR 是 4 个数组一次写盘【来源】。

## 建议总表

| # | 问题 | 证据 | 改法 | 目标文件 |
|---|---|---|---|---|
| 1 | 契约草案节点约 88 B（`NodeKind` 16、`ValueTy` 8、`Option` 8、两个 `SmallVec`） | T1：80 B/节点；16 B 记录 19–22 B | `NodeRec` 16 B + `ctl`、`ty` 两列；字段私有，经访问器读写 | [graph.md](../../design/ir/graph.md#节点) |
| 2 | `effects: SmallVec<[(PartitionId, NodeId); 1]>` 每条效果边多存分区号 | 分区可以由 token 的类型确定；`Call`/`Return` 只收一个 `MemMerge` | 效果输入是普通 `NodeId`；`TyRef::Effect(P)` | [graph.md](../../design/ir/graph.md#节点)、[memory-partitions.md](../../design/ir/memory-partitions.md) |
| 3 | `Vec<Option<Node>>` tombstone 只增不压 | Graal 70% 保序压缩；T1 乱序 2.6–4.0× | `DEAD` 标志；在 R 发布、`specialize`、placement 前三处按原顺序压实 | [graph.md](../../design/ir/graph.md#函数图) |
| 4 | use-list 未定义 | T2；V8 的“节点大”；Turboshaft 只留计数 | 新增“边与使用”节：转发表 + 近似 users 索引 + 饱和计数 | [graph.md](../../design/ir/graph.md) |
| 5 | GVN 表形式未定 | T3：存哈希的 `HashTable<(NodeId,u32)>` 与 `FxHashMap` 同速，内存 43% | 规定 Fx + 存哈希；`IN_GVN` 节点改输入前先移出 | [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#共同机制) |
| 6 | worklist 顺序与计数未定 | V8 20 次访问/改动；T1 随机顺序 3× | 种子按 `NodeId` 升序，LIFO；记 `visits/changes`；退化时才换随机弹出 | [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#共同机制) |
| 7 | 风险表的“局部性”指标不可操作，L1d 无采集方式 | macOS 无 perf；V8 按阶段报 | 改为字节/节点、分配次数、存活率、打乱对照臂；L1d miss/指令 按阶段在 E2B 采 | [levels.md](../../design/ir/levels.md#风险v8-离开-son-的教训) |
| 8 | `TyId` 驻留“初版一把锁” | T6 单锁 8 线程 0.65× | 只追加分块存储（无锁读）+ 分片 + 每任务本地缓存 + 片内编号；单线程不建分片 | [ids.md](../../design/ids.md#规则) |
| 9 | 类型是否缓存指纹未定 | rustc 2022 年加、2026 年删（每类型省 16 B） | 不缓存；持久指纹按需算并备忘 | [types.md](../../design/frontend/types.md) |
| 10 | 持久键算法未定，现状用 SHA-256 | T5 | 128 位 SipHash-1-3 或 BLAKE3-128；R 图哈希 = 压实后 varint 流的哈希 | [cache.md](../../design/build/cache.md#存储形式)、[ids.md](../../design/ids.md) |
| 11 | “节点流 + 变长整数”没有区分冷热 | T4：varint 5.5 B/节点、解码 11–18 ns；零拷贝打开 2–5 ns | R 图用 varint（相对回指）；接口、孔位、索引表用定宽零拷贝 | [cache.md](../../design/build/cache.md#存储形式) |
| 12 | Body 每块 `Vec<Stmt>`、`Vec<Operand>` | rustc `Statement` 56 B + 装箱 `Assign` | 语句全函数连续、块为区间；`Place` 8 B；变长实参进池；span 旁表 | [body.md](../../design/frontend/body.md#body-结构) |
| 13 | 内存预算没有单位 | rustc +35%、Zig +28.1% | 准入用“节点数 × 字节/节点”估算，报告并行增量 | [pipeline.md](../../design/build/pipeline.md#cpu-与内存预算) |
| 14 | 现状 O(K·N) 替换与整图不动点循环 | T2：2,400× | 迁移第 1 步，先于新契约 | [current-gap.md](current-gap.md) |

## 文件

| 文件 | 内容 |
|---|---|
| [prior-art-compact-ir.md](prior-art-compact-ir.md) | Zig、Carbon、Cranelift 的紧凑 IR 与字节数 |
| [prior-art-son-nodes.md](prior-art-son-nodes.md) | V8 TurboFan/Turboshaft、C2、Graal、Simple 的节点与 use 表示 |
| [prior-art-rustc.md](prior-art-rustc.md) | rustc 驻留、arena、`IndexVec`、FxHash、分片与并行前端数据 |
| [use-lists.md](use-lists.md) | LLVM/Swift/MLIR 的 use-list 代价，kfrc 的替代方案 |
| [node-storage.md](node-storage.md) | 节点布局、字节估算、NodeId 与压实、GVN 表 |
| [access-patterns.md](access-patterns.md) | worklist、placement、创建顺序 |
| [interning-and-keys.md](interning-and-keys.md) | `TyId` 驻留、孔位表、答案键、指纹 |
| [body-and-artifacts.md](body-and-artifacts.md) | Body 紧凑表示、varint 流与零拷贝 |
| [parallel-memory.md](parallel-memory.md) | 每任务 arena、并行驻留、峰值内存预算 |
| [current-gap.md](current-gap.md) | `src/` 现状评估与迁移顺序 |
| [metrics.md](metrics.md) | 持续测量的指标，接入风险表 |
| [bench-method.md](bench-method.md)、[bench-layout.md](bench-layout.md)、[bench-io-parallel.md](bench-io-parallel.md) | 微基准方法与原样结果（T1–T6） |

相关：[prior-art/sea-of-nodes.md](../prior-art/sea-of-nodes.md)、[prior-art/egraph-rewrite.md](../prior-art/egraph-rewrite.md)、[prior-art/ct-fast-compile.md](../prior-art/ct-fast-compile.md)。

**未找到原始来源：** Turboshaft 相对 TurboFan 的内存降幅；Graal 的字节/节点或内存测量；Carbon 每行耗时预算（只有演讲的二手转述）；Kelley 演讲中的 64→5、120→15.6、54→20.3 B（只有第三方转录，PR #7920 和 0.8.0 发布说明的数字是原始的）；LLVM 2025 年去掉 `ConstantData` use-list 的性能数字（PR 未公布）；Swift 关于 use-list 代价的文档。

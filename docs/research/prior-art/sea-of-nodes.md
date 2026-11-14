# Sea of Nodes 的现状

## 1. 是什么

| 系统 | 来源 | 年份 | 现状 |
|---|---|---|---|
| HotSpot C2 | Paleczny, Vick, Click, [The Java HotSpot Server Compiler](https://www.usenix.org/legacy/events/jvm01/full_papers/paleczny/paleczny.pdf)（JVM '01）；IR 源自 Click & Paleczny, *A Simple Graph-Based Intermediate Representation*（IR '95） | 2001 | 仍在用 |
| V8 TurboFan → Turboshaft | Darius Mercadier, [Land ahoy: leaving the Sea of Nodes](https://v8.dev/blog/leaving-the-sea-of-nodes) | 2025-03 | JS 后端和整个 Wasm 流水线已换成 CFG（Turboshaft）；SoN 只剩 builtin 流水线和将被 Maglev 替代的 JS 前端 |
| Graal | Duboscq 等, [Graal IR: An Extensible Declarative IR](https://ssw.jku.at/General/Staff/GD/APPLC-2013-paper_12.pdf)（APPLC '13）；[speculative IR](https://dl.acm.org/doi/10.1145/2542142.2542143)（VMIL '13）；Stadler 等, [Partial Escape Analysis](https://ssw.jku.at/Research/Papers/Stadler14/Stadler2014-CGO-PEA.pdf)（CGO '14） | 2013–2014 | 仍在用；固定节点 + 浮动节点 |
| Cliff Click 的 Simple | [SeaOfNodes/Simple](https://github.com/SeaOfNodes/Simple)（Java，29 章）；对 V8 的回应 [ASimpleReply.md](https://github.com/SeaOfNodes/Simple/blob/main/ASimpleReply.md) | 2023–2026 | 教学实现，含 x86-64/RISC-V/ARM64 代码生成、图着色分配 |
| Cranelift | Fallin, [The acyclic e-graph](https://cfallin.org/blog/2026/04/09/aegraph/) | 2026-04 | “CFG 骨架 + 浮动纯节点”的 SoN 变体，见 [egraph-rewrite.md](egraph-rewrite.md) |

## 2. 关键证据

**V8 离开的理由（原文列举）：**

| 问题 | V8 的数据或表述 | kfrc 是否同样暴露 |
|---|---|---|
| 大多数节点挂在效果/控制链上 | 内存操作、检查、甚至除法都要效果输入；“SoN is just CFG where pure nodes are floating” | 部分：kfrc 的聚合是值、借用是分区，但 `Unknown` 分区上的调用和 `Check` 仍在链上 |
| 手工维护多条效果链易错 | 一个函数要同时维护 3 条效果链，混用的 bug 几个月没被发现 | **是**：分区就是多条效果链 |
| 访问顺序差 | 窥孔从 return 反向遍历，“平均每 20 次访问才改一次节点” | worklist 可避免 |
| 缓存局部性 | L1 dcache miss 平均约 3×，个别阶段 7×；估计占编译时间最多 5% | 待测 |
| 控制相关的类型化 | 纯的 `CheckedAdd` 在调度前无法去掉检查 | 否：kfrc 的 `Check` 带控制，`Refine` 固定在控制上 |
| 编译时间 | 换成 CFG 后整体编译时间约减半；作者注明“对 AOT 编译器可能不是大问题” | 待测 |

**Click 的回应（不是数据，是做法）：** 强类型语言的等价类别名（ECA）“免费”，由 parser 给出，效果链因此稀疏；用格上的 `Cast`（类似 kfrc 的 `Refine`）代替守卫的控制边，消掉 Java 90% 以上的类型检查；用 worklist（线性时间）代替遍历顺序；简单展开（min/max）放到寄存器分配后、复杂展开放到 GCM 后；用 ASCII dump 调试。

**Simple 第 11 章的内存实现：** 每个结构体字段一个别名编号；parser 只产生一个内存值，优化器按需懒拆分：`MemMerge`（默认内存 + 精确别名覆盖，不产生代码）、`MemPhi`、`BulkMemPhi`（除排除集外的所有别名）。不同别名类之间没有反依赖。

**Graal：** 分层流水线 High（内联、PEA、snippet lowering）→ Mid（内存优化、`FloatingReadPhase` 把 load 变成浮动读、guard lowering）→ Low → LIR（[Wimmer, CGO 2015 教程](https://www.slideshare.net/slideshow/2015-cgo-graal/46075272)）。图状态用 `StructuredGraph.GuardsStage` 等字段记录（`FLOATING_GUARDS` → `FIXED_DEOPTS` → `AFTER_FSA`）。不同 `LocationIdentity` 的访问保证不别名（[LocationIdentity](https://www.graalvm.org/sdk/javadoc/org/graalvm/word/LocationIdentity.html)）。PEA 在对象只在部分分支逃逸时仍做标量替换；Stadler 博士论文报告分配减少最多 58.5%、性能提升最多 33%；一份 C2 的 PEA RFC 引用 Stadler 的数字：PEA 占 Graal 编译时间 3.5–4.5%，其中一半是调度（[RFC gist](https://gist.github.com/navyxliu/62a510a5c6b0245164569745d758935b)，二手来源）。

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| 保留 SoN | **采纳（不变）** | V8 的问题主要来自 JS 的动态别名；kfrc 是 AOT、强类型、有借用信息，与 C2/Graal 同类。Cranelift 2026 也认为“SoN + CFG 骨架”效果好 |
| 效果节点固定在控制骨架上，只有纯节点和可证明不变的 load 浮动 | **采纳** | Graal 的 FixedNode、Cranelift 的 side-effect skeleton；V8 观察到内存操作本来就很少浮动。省掉效果链的重排，placement 只调度浮动节点 |
| Simple 的懒拆分内存（MemMerge） | **改编** | 作为分区的实现形式：调用、`Return` 收一个 `MemMerge`，只有被精确访问的分区单独出现，效果输入保持稀疏 |
| 按类型/字段的 ECA | **不采纳** | Rust 的 unsafe 代码可以类型双关，Rust 不保证 TBAA（memory-partitions.md 已列为“明确不做”）；字段级不相交只用结构化投影和常量偏移 |
| Graal 的分层与图状态字段 | **采纳（已一致）** | `GraphLevel` 就是 `GuardsStage` 的对应物 |
| PEA | **改编，M6 之后** | kfrc 的聚合已是值；适用面是 `StackSlot` 只在冷区域逃逸：热路径保持 `Local`，在冷区域入口物化 |
| 初始顺序 | **采纳** | V8 指出 SoN 丢掉初始调度。kfrc 保留 Body 块顺序作为固定节点的骨架和 placement 的平局规则 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [levels.md](../../design/ir/levels.md) | 风险：V8 离开 SoN 的教训 | 时间改为“约 2022 年起，2025-03 博客”；指标表增加两行：“每次改动的访问数”（V8 参照值 20）、“L1d miss / 节点”（V8 参照：SoN 约为 CFG 的 3×）；把记录起点从 M2 提到 M1（T07/T09 已有图） |
| [levels.md](../../design/ir/levels.md) | 为什么不是三种 IR | 补一句：效果节点固定在控制骨架上（Graal FixedNode / Cranelift skeleton），浮动的只有纯节点、`Frozen` load 和不逃逸的 `Local` load |
| [nodes.md](../../design/ir/nodes.md) | 地址与内存 | 新增 `MemMerge`（RSL）：一组分区 token 的合并视图，不产生代码；`Call`、`Return`、`Resume` 的 `mem=` 收一个 `MemMerge`，dump 只列出非默认的分区 |
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) | L 级 | 写明：`min/max/abs`、`cmov` 类展开放在 placement 之后的 Machine IR 窥孔；跨块控制的展开（`Check` → `If`）在 placement 之前 |
| [ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md) | 冷区域 | 增加“部分逃逸”扩展（M6 后）：`StackSlot` 的地址只在冷区域逃逸时，热路径保持 `Local` 分区 |

# rustc 1.94.1 的 LLVM 流水线与流水线编译

基于官方 rustc 1.94.1（`e408947bfd200af42db322daf0fadfe7e26d3bd1`，LLVM 21.1.8）。rustc 源码链接固定到该 commit，LLVM 链接固定到 `llvmorg-21.1.8`。回答两个问题：rustc+LLVM 在构建流程的哪个位置做了哪些优化；kfrc 的 R/S/L 三级在哪里承接它们。数字来源分**实测**（本目录 [scripts/](scripts)）、**前期 E2B**（2026-10 剖析数据，经脚本汇总或引原文）、**源码核实**和**推断**，口径见 [measurements.md](measurements.md)。

## 结论

1. **rustc 默认不往 LLVM 里加任何优化 pass。** `LLVMRustOptimize` 只按阶段选 LLVM 的现成流水线、设置 PTO（展开、向量化、MergeFunctions）和 TLI/AA；回调只服务 sanitizer、coverage、校验、自剖析（[pass-inventory-o3.md](pass-inventory-o3.md)）。
2. **release 默认是“化简两遍”。** 16 个 CGU 各跑 ThinLTO pre-link（O3 有 77 个叶子 pass，只化简）→ 屏障与全局导入 → post-link（108 个，化简半段整段重跑 + 优化半段）→ 后端（x86-64 O3 有 203 个 pass）。InstCombine 静态出现 5 + 8 次。单遍的 `PerModule-O3`（115 个）只在 `-Clto=off` 或 CGU=1 时出现（实测，[opt-levels.md](opt-levels.md)）。
3. **post-link 的重复化简是最大的单块浪费。** 导入体的再化简占 post-link 的 40%；ThinLocal 让 LLVM 优化 CPU 多约 75%；但关掉它运行时慢 +1.50%…+27.83%，价值在跨 CGU 内联（前期材料，[thinlto-postlink.md](thinlto-postlink.md)）。
4. **时间分布平坦。** E2B rg A 臂：pre-link、post-link、后端分别占 LLVM 计时的 39.1%、31.9%、29%；最大的族是 InstCombine 类 13.6%、指令选择 10.2%、循环 8.7%、内联器 7.1%（下表）。没有能单独砍掉的 pass，只能少处理代码或同一份代码少处理几遍。
5. **rmeta 之前的一切都串行且在关键路径上。** 库 crate 的 MIR 优化、单态化收集和 CGU 划分都在写 rmeta 之前（metadata 要导出泛型符号表）；LLVM 工作在 rmeta 之后按 CGU 并行，但受 IR 生成串行喂料、ThinLTO 屏障和“二进制 crate 等全部 rlib”限制（[timeline-rustc.md](timeline-rustc.md)，8 处放错位置的优化）。
6. **kfrc 对“结构重建”的承接是结构性的**：值形式聚合、分区、变体操作让 SROA/DSE/MemCpyOpt 与 niche 折叠的大部分工作不产生；共享已优化 S 图让“导入体再化简”、LocalCopy 和 ThinLTO 屏障消失（[timeline-kfrc.md](timeline-kfrc.md)、[research/llvm-mir/mapping-to-design.md](mapping-to-design.md)）。
7. **缺口在过程间、循环和后端**：IPSCCP/GlobalOpt、改签名（ArgPromotion/DeadArgElim）、循环规范化与 loop-idiom、switch 降低、调试变量位置都没有设计；新的串行点是二进制 crate 末端生成和跨进程 R 图屏障（[recommendations.md](recommendations.md) R1–R12）。
8. **8 核上墙钟主要靠降 CPU 总量。** 签名优先的 −28.5% 是 CPU 不设上限的模型，8 vCPU 实测 −3.3%（前期 p2b）。kfrc 的收益应主要来自 C1/C2/C6 去掉的工作。

## 总表

按 LLVM pass 族汇总。“pre/post 次数”是 ThinPre-O3 / ThinPost-O3 中该族 pass 的静态出现次数（实测）；“rg A %”“cargo Z %”是前期 E2B 逐 pass 重放中该族占三阶段 LLVM 计时的比例（Z 臂已含 post-link 跳过等补丁）。类：A 值在内存，B 薄包装，C enum/niche 与比较，G 全局，M 机器级（evidence.md 的分类）。表由 `make-tables.sh` 从 `families.tsv` 与测量输出生成。

| 族 | 类 | 位置 | pre/post 次数 | rg A % | cargo Z % | 做什么 | kfrc 承接 |
|---|---|---|---:|---:|---:|---|---|
| InstCombine 类 | C/通用 | pre+post | 6/10 | 13.6 | 12.9 | 窥孔化简；niche 解码、derive 比较折叠 | 构建期窥孔 + R 级变体规则；L 级机器窥孔 |
| 指令选择 | M | 后端 | — | 10.2 | 12.2 | SelectionDAG 选择与 DAGCombine | Machine IR 选择 + L 级窥孔（kfrc 后端） |
| 循环 | M | pre+post（展开只在 post） | 11/18 | 8.7 | 7 | 迭代器内联后的指针循环 | GCM + L 级 IV/PRE；rotate/unswitch/idiom 尚缺（R3）；展开推迟 |
| 内联器 | B | pre+post（CGSCC） | 2/2 | 7.1 | 7.2 | 压平闭包、drop/trait glue、Fn shim | R/S 级拼接已优化图；跨 crate 用共享 S 图 |
| GVN/EarlyCSE | A/通用 | pre+post | 3/2 | 6.7 | 6.3 | 冗余值与重复 load | GVN 哈希 + 分区 load 转发；L 级 PRE |
| JT/CVP/约束/SCCP | C | pre+post | 6/6 | 6.4 | 5.7 | 条件与范围传播、检查消除 | 按前驱拆分、Refine、Check 消除；乐观 SCCP 未写明（R3） |
| 过程间（IPSCCP/GlobalOpt/属性/参数） | B/G | pre+post | 8/9 | 6 | 8.1 | 过程间常量、属性推断、改签名 | 调用摘要承接属性；常量与改签名尚缺（R1、R2） |
| 机器级优化（CSE/LICM/sink/窥孔/布局…） | M | 后端 | — | 4.6 | 4.9 | 机器 CSE/LICM/sink、if-conversion、分支折叠、块布局 | SSA 段由 GVN+GCM 承接；布局只覆盖冷块（R5） |
| 寄存器分配与活跃性 | M | 后端 | — | 3.9 | 4.8 | Greedy 分配、合并、两地址 | 线性扫描（总览固定决策；质量见 R5） |
| SimplifyCFG | B/C | pre+post | 6/7 | 3.7 | 3.3 | 控制流化简、删空 landingpad、switch 查找表 | R 级 DCE、常量 Phi；switch 降低尚缺（R4） |
| 后端分析 | M | 后端 | — | 3.1 | 3.3 | 机器级支配树、BPI/BFI | L 级算一次；冷热靠结构标记 |
| IR 级后端准备（LSR/CGP…） | M | 后端 | — | 3.1 | 3 | LSR、地址模式下沉、memcmp 展开 | L 级强度削减与地址模式 |
| 分析：其他 | — | pre+post | — | 3 | 2.7 | LVI、SCEV、TTI 等 | L 级循环需要 SCEV 类信息 |
| 其他标量 | M | pre+post | 7/10 | 2.9 | 2.5 | 杂项标量变换 | 部分：L 级常量除法、if-conversion |
| DSE/MemCpyOpt | A | pre+post | 3/3 | 2.8 | 3.9 | move 的 memcpy 链、死 store | move 是值转发；分区死 store 删除（不需要大部分） |
| SROA/mem2reg | A | pre+post | 4/4 | 2.3 | 2.9 | 每个 MIR local 的 alloca 提升为 SSA | 聚合构图时即值；槽位提升（不需要大部分） |
| 分析：支配树/循环 | — | pre+post | — | 1.9 | 1.8 | 支配树、LoopInfo | placement 与 L 级一次 |
| 调试信息 | — | 后端 | — | 1.9 | 0.1 | 变量位置（DEBUG_VALUE） | 尚缺（R6） |
| 分析：别名 | A | pre+post | — | 1.4 | 0.9 | BasicAA、TBAA、ScopedNoAlias、GlobalsAA | 内存分区代替 |
| 向量化 | M | post（vector-combine 两段都有） | 1/4 | 1.4 | 1.3 | 循环与 SLP 向量化 | 推迟到 T38 之后 |
| 发射/帧 | M | 后端 | — | 1.4 | 0.7 | 序言/尾声、汇编与对象发射 | kfrc x86 后端（R5） |
| 分析：MemorySSA | A | pre+post | — | 1.3 | 1.7 | 内存依赖 | 内存分区代替 |
| 分析：BPI/BFI | — | pre+post | — | 1.2 | 1.1 | 分支概率与块频率 | 结构性冷标记，不用 profile |
| 指令调度 | M | 后端 | — | 0.8 | 1 | pre-RA 与 post-RA 调度 | 不做（总览固定决策） |
| 其他（PM 胶水、Coro/OpenMP 空转等） | — | pre+post | — | 0.7 | 0.7 | PM 胶水、协程/OpenMP 空转 | 不需要 |

## 文件

| 文件 | 内容 |
|---|---|
| [pass-inventory-o3.md](pass-inventory-o3.md) | rustc 自己加了什么；O3 pre-link 的结构与各 pass 次数；各组在重建什么 |
| [thinlto-postlink.md](thinlto-postlink.md) | ThinLocal 的步骤、屏障、重跑内容与代价 |
| [codegen-backend.md](codegen-backend.md) | 后端 pass 结构（aarch64 rustc、x86-64 llc）、时间、kfrc 承接面 |
| [opt-levels.md](opt-levels.md) | 选择规则、opt-level × CGU × LTO 矩阵、各级差异、cargo 实际参数 |
| [timeline-rustc.md](timeline-rustc.md) | rustc + cargo 的时间轴：串行/并行、关键路径、放错位置的优化 |
| [timeline-kfrc.md](timeline-kfrc.md) | kfrc 的时间轴：每类优化搬到哪里、好处与代价、新的串行点 |
| [research/llvm-mir/mapping-to-design.md](mapping-to-design.md) | 逐 pass 的 kfrc 承接状态 |
| [recommendations.md](recommendations.md) | 对 design 的建议清单 |
| [measurements.md](measurements.md) | 环境、开关、脚本、口径、已知结论的核对 |

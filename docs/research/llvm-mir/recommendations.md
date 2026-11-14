# 对 design 的建议

只提建议，不改 design。编号在本目录其他文件里引用。每条给出问题、证据、改法、目标文件；目标文件只写文件名。

## 覆盖面缺口

**R1 过程间常量传播与全局变量化简无人承接**
- 问题：IPSCCP（参数在所有调用点相同、返回值恒定）、GlobalOpt（未写入的可变 static 等）在 kfrc 里没有对应机制；`CallSummary` 只有效果，没有值。
- 证据：过程间族（IPSCCP、GlobalOpt、属性、参数）占 LLVM 计时 rg A 6.0%、cargo Z 8.1%；运行时价值没有数据。
- 改法：先量运行时价值。仿前期 `postlink_skip` 插件（`ShouldRunOptionalPassCallback`）做“按 pass 名跳过”的插件，分别关掉 `ipsccp`、`globalopt` 跑 rg/cargo 运行时负载。若有价值，在 `CallSummary` 加“返回常量/参数恒定”位，S 级用它折叠；否则写进“不做”。
- 目标：optimization-pipeline.md（调用摘要、不做），runtime.md。

**R2 内部实例的签名改写缺失**
- 问题：ArgPromotion（小 `&T` 改按值）、DeadArgElim 要改非导出、未取地址实例的签名；kfrc 的 PassMode 固定，`writes_indirect_args` 只省拷贝。
- 证据：rg A `ArgumentPromotionPass` 占 1.1%；它只在 O3 启用（实测 `diff-thinpre-O2-vs-O3.txt`）。
- 改法：L 级加“内部调用约定”：被调者不导出、不取地址时，按摘要改参数传递，答案键包含被调者的内部 ABI 答案。或明确不做并写理由。
- 目标：optimization-pipeline.md（L 级），generics-specialization.md（答案键）。

**R3 循环规范化与几类循环变换未写明**
- 问题：loop-rotate、loop-unswitch、loop-deletion、一般 loop-idiom（循环识别为 memset/memcpy），以及穿过循环 Phi 的乐观常量传播（SCCP）都没有写是否做。
- 证据：循环族占 LLVM 计时 rg A 8.7%（IndVarSimplify 1.8%、LICM 1.3%、LoopIdiom 0.9%）；展开、向量化已推迟到 T38，但这几项不是向量化。
- 改法：L 级“循环”逐项写：做（rotate 给 LICM/PRE 造前置块；deletion 需要终止性事实）、由 GCM 承接、或不做；共同机制里写明 worklist 是悲观还是乐观（Click 的组合分析可在 SoN 上做乐观 SCCP + GVN）。
- 目标：optimization-pipeline.md（共同机制、L 级）。

**R4 switch 降低没有设计**
- 问题：`VariantSwitch` 到机器码要在跳转表、位测试、查找表、比较树之间选择。
- 证据：LLVM 由 post-link 末尾的 `simplifycfg<switch-to-lookup>`（实测只出现在 ThinPost 的最后一个 `simplifycfg`）和 SelectionDAG 完成；Rust 的 `match` 大量依赖它。
- 改法：L 级“布局降低”加一行：switch 的降低策略与阈值；查找表进只读数据并按内容去重。
- 目标：optimization-pipeline.md（L 级），nodes.md。

**R5 后端承接面未列全**
- 问题：栈槽着色（value slot 也需要）、Shrink Wrapping、热路径块布局与尾复制、x86 修补（false deps、vzeroupper）、线性扫描对 Greedy 的质量差距。
- 证据：实测 x86 O3 后端 203 个 pass（[codegen-backend.md](codegen-backend.md)）；rg A 中寄存器分配族占 codegen 13.3%，指令选择 35.3%。
- 改法：不属于 design/ir 正文；在 design/README.md 给 kfrc x86 后端列“必须有 / 可以晚做 / 不做”清单，链接本目录 codegen-backend.md。
- 目标：design/README.md。

**R6 调试信息的变量位置没有设计**
- 问题：聚合是值、GVN 合并、placement 移动之后，变量的寄存器/栈位置要能写成 DWARF 位置表。
- 证据：rg 的 `debug = 1` 让 `Live DEBUG_VALUE analysis` 占 codegen 的 4%（前期 next-directions §2）；release 常开 `debug = 1`。
- 改法：写明变量位置作为节点旁路信息随降低传递，不进图哈希（红线 2 的延伸）。
- 目标：levels.md，README.md（红线）。

## 数字与口径

**R7 −28.5% 被当作实测**
- 问题：evidence.md §5 与 compile-time.md 写“签名优先比 bin pipelining 再降 28.5%”。
- 证据：这是 CPU 不设上限的模型；M3 在 8 vCPU 实测 rg −3.3%、cargo 项目无变化，原因是 CPU 地板（rg CPU/8 约 13 s，P1 之后调度空间约 3 s，前期 p2b-m2-m3）。
- 改法：写成“模型上限 −28.5%，8 vCPU 实测 −3.3%”，并说明 8 核上墙钟主要靠 C1/C2/C6 降低 CPU 总量。
- 目标：evidence.md §5、§6，compile-time.md“关键路径”。

**R8 `-Clto=off` 的数字口径不准**
- 问题：写成“省 27–29% 编译指令、运行慢 6–28%”。
- 证据：原表 O 臂（建在 B 臂上）相对基线 rg −31.61%、cargo −28.74%；运行时 rg +1.50%…+27.83%、cargo +5.79%…+8.71%。
- 改法：用原表数字并注明 O 臂含 B 臂补丁。
- 目标：evidence.md §5。

**R9 “未优化 IR 23.3%”来自单 CGU 构建**
- 问题：evidence.md §1 的数据用 `--emit=llvm-ir`，rustc 因此把 CGU 设为 1 并关闭 ThinLocal。
- 证据：实测 `matrix.tsv` 末两行；前期复盘：真实 16 CGU 构建的函数定义多 23.4%（LocalCopy）。
- 改法：§1 注明口径；kfrc 的对照测量禁止用 `--emit=llvm-ir` 取结构，改用 `-Csave-temps` 或 PipelineStart 计数插件。
- 目标：evidence.md §1，validation 的性能文档。

## 流水线位置与指标

**R10 内联决策用未优化 S 图的 `size_estimate`**
- 问题：优化会把函数大幅缩小，未优化估计偏大会导致少内联。
- 证据：ripgrep pre-link 前后函数 26,813 → 9,713、指令 1,175,161 → 540,833（前期 thinlto-postlink §2）。
- 改法：摘要另给“优化后大小”的廉价上界（扣掉 `NeedsDrop=false` 的 drop、空 cleanup），或被调者 S 图已优化时改用实际大小；A/D/B 量内联数与运行时。
- 目标：optimization-pipeline.md（调用摘要、内联策略）。

**R11 两个新的串行点没有指标**
- 问题：二进制 crate 的末端生成（C6）与跨进程“R 图集合”屏障可能成为新的关键路径。
- 证据：rg 最终 crate 5.7 s、平均只用 3.8 个 CPU；cargo 开跨 crate LTO 时库只出 bitcode、后端全在二进制（实测 `cargo-flags.sh`）；P2b 的提前开工被 CPU 争用吃掉。
- 改法：“关键路径构成”加两项：下游特化等待上游 R 图的时间、二进制 crate 末段的平均忙碌 CPU；R 图按下游请求优先优化。
- 目标：compile-time.md（指标与闸门）。

**R12 编译时间没有与 LLVM 可比的分项**
- 问题：无法逐项回答“kfrc 在哪一族工作上省了时间”。
- 证据：本目录 `census-passes.sh` 的族划分（InstCombine 类 13.6%、指令选择 10.2%……，rg A）。
- 改法：M5 的每 pass 指标按同一族名归类输出，与 LLVM 的族占比并列。
- 目标：compile-time.md（指标与闸门）。

## 优先级（推断）

1. R7、R8、R9：只改文字，避免后续取舍建立在错误数字上。
2. R1–R4：决定中端覆盖面，影响“运行时不退化”；先做 R1 的运行时测量。
3. R10–R12：M2–M5 的实验与指标。
4. R5、R6：归入 kfrc x86 后端计划。

# KFRC 对 LLVM：四库产物改进计划

本计划基于现有改动继续推进，范围为semver、base64、unicode-segmentation、humantime。专项subagent依次审阅各库，沿“实际产物模式 → crate调用路径 → KFRC实现 → 可验证改动”追溯。此次交付是诊断与实施计划；已有生产改动仍在当前工作树，归属同一PR #8，未另开PR、未合并。

## 当前结论

当前最值得移植的LLVM机制是：有界调用点特化及内联后的常量传播、SROA/内存转发、纯地址CSE、结果与溢出标志共享、较完整的x86地址与条件选择。它们直接对应现有产物中的重复工作。暂不引入外部solver，不把通用cost model、完整SCEV或向量化设为这些修复的前置条件。

transport在13项workload上的等权运行时间几何平均比baseline下降6.62%，仍为LLVM的3.12倍；四库编译wall几何平均增加2.09%。最新ranges进一步改善部分静态产物，但尚无同口径性能复测。这些数字覆盖选定库后端差分：驱动及部分泛型/std由LLVM生成，不代表整个程序均由KFRC生成。

| crate | transport运行耗时 / LLVM | transport编译wall变化 | 优先处理 |
|---|---|---:|---|
| semver | parse 3.75×；req 6.85×；identifier 1.69× | +10.8% | checked运算共享、Option/切片包装特化、聚合返回 |
| base64 | encode 8.25×；decode 6.82×；short 3.83× | +11.9% | 地址CSE、固定长度特化、小数组搬运、索引寻址 |
| unicode-segmentation | mixed 1.32×；ASCII 1.09×；reverse 3.73× | +5.1% | 反向UTF-8调用闭包、category查表与条件选择 |
| humantime | duration parse 8.66×；format 1.13×；timestamp parse 3.12×；format 1.71× | −16.6% | 单位匹配调用、聚合复制、L布尔事实与格式化回退 |

humantime timestamp format相对baseline回退3.51%，配对95%区间[3.35%,3.87%]。它必须单独解释，不能由duration parse收益抵消。semver identifier、Unicode reverse、timestamp parse的本轮改善区间跨1，不宣称稳定加速。完整区间见[evidence.md](evidence.md)。

humantime的56处memmove是静态调用点；抽查的连续48 B复制位于错误分支，不能按成功解析每次执行56次解释运行差距。timestamp format主fmt函数在三个KFRC版本中机器码相同，但其Result<&str, Utf8Error>::expect辅助函数由103 B增至136 B，显式栈预留由16 B增至48 B，成功路径新增快照搬运。这是优先做独立对照的回退假说，尚未证明它解释全部3.51%。

## 已有修复与剩余差距

已有独立ELF section、布尔diamond清理、布局后私有内存转发、两轮有界wrapper简化、枚举边界快照和KnownBits增强。这些不应重新列成未做的工作。

- 链接回收已在单根base64 consumer证明有效：保留代码13654B→31B，LLVM27B；stripped可执行文件376904B→350160B，LLVM350192B。只代表该consumer。
- base64 internal_encode从transport的5302B/1214指令/59个静态call进一步降到ranges的3595B/855/20，LLVM1376B/327/9。剩余热路径仍有重复地址和read_u64调用，不能把已删除边界检查再当首要缺口。
- semver小型Identifier方法已去掉多余分支/私有栈槽；数字循环的结果/overflow重复计算仍存在。
- humantime parse_unit已经从23799B降到5094B，但仍有4496B显式栈预留，LLVM为24B。显式预留不含所有push，不能直接视作完整frame，更不能把所有栈访问都叫spill。
- object/rlib反而增大。200个object的精确账本证明新增section headers和字符串表是主要增量，详见[evidence.md](evidence.md)。保留可回收性，单独研究元数据压缩，不回退成单一大section。

## 统一工作包与依赖

下表列的是实施顺序与交付边界；函数级证据、源码行和负例在各专项报告。每个工作包独立验证，然后累计到原PR，不将四库拆成互相漂移的编译器实现。

| 工作包 | LLVM参考机制 | 具体交付 | 主要受益库 | 前置 |
|---|---|---|---|---|
| W0 证据基线 | pass remarks、阶段计时、差分实验 | ranges等价复测；最新S/L/Machine dump；inline skip原因；driver调用边界；拆综合workload | 全部 | 无 |
| W1 地址去重 | EarlyCSE/GVN | L纯PtrOffset规范化与精确CSE，消除32份相同查表基址 | base64，其他库待测 | W0 |
| W2 checked运算共享 | with.overflow、多结果/flags选择 | 同一运算的value/overflow共享乘法或加法，分支复用flags | semver、humantime | W0 |
| W2b L布尔事实 | KnownBits、SimplifyCFG | 不只查询Compare：将任意布尔If条件的已知值用于折叠；清掉常量除数不可能溢出的路径 | humantime | W0；严格保留S/L契约 |
| W3 有界特化 | IPSCCP、callee简化、内联后SCCP | 固定slice长度/常量enum参数特化，按简化体计费，缓存摘要，接入inline意图 | 全部 | W0；与W1/W2独立 |
| W4 小数组与字段提升 | SROA、mem2reg、MemCpyOpt | 精确byte-range转发、整数carrier、私有lane Phi、sret destination forwarding | base64、semver、humantime | 先W3暴露证明；局部可独立 |
| W5 地址/条件选择 | SelectionDAG、机器指令选择 | base+index*scale+disp；纯标量SSA select→cmov；pointer Eq/Ne直接分支 | base64、Unicode、humantime | pointer分支可先做；其他在W1/W3后验证 |
| W6 分支/循环局部优化 | InstCombine、SimplifyCFG、LoopSimplify、unswitch | ASCII单区间规范化、小循环首轮剥离/纯不变量外提 | semver、humantime | W3/W4；循环克隆须有profile |
| W7 剩余栈与产物元数据 | stack coloring、机器清理、对象输出 | 区分aggregate槽/spill，按活性复用；审计section命名/字符串重复 | 全部 | W1–W5后重新定位 |
| C 编译成本轨道 | analysis invalidation、dirty worklist、有界pipeline | 摘要复用、少clone、局部清理、阶段缓存，必要时调整调度 | 全部 | W0起持续 |

### W0：先确保能回答“哪个改动有效”

补ranges完整21轮交错运行/编译；保持同一Rust/sysroot/target/flags/线程数。正式计时不与构建、采样并发。记录源码overlay、backend和object hash。单独运行perf计数，不把单次计数包装成21轮置信区间。

拆分semver req_parse/预构建req_match，base64预分配short encode/decode，Unicode forward/reverse各自实际调用闭包，humantime整数/小数/单位与timestamp格式分层。保留原综合workload供连续趋势比较。先审计驱动对象与最终链接relocation，判断哪些泛型/inline函数实际由LLVM实例化。

当前humantime dur-parse计时的10个输入不含小数，ts-format固定为nanos精度；corpus覆盖这些语义不等于已有对应性能数据。新增fraction及seconds/millis/micros/smart负载后分别验收，不能把现有综合倍率当作这些路径的性能结论。

新增pass保留实验开关与独立候选；用“baseline→单项→累计”比较。需要判断交互时只做相关两项组合，不枚举全部组合。已有`KFRC_PROFILE`和`prepare_wrappers_s`窗口可复用，新增细分项见[compile-budget.md](compile-budget.md)。

### W1/W2：先处理可以明确删除的重复工作

W1的键必须包含同一SSA base、类型/位宽、offset与语义标志；不把不同allocation、不同动态实例或来源不同但整数地址相等的pointer合并。不把地址CSE升级成未经alias证明的load CSE。

W2保留显式checked行为。先u64乘/加，再窄类型及signed；同一结果与标志只执行一次算术。测试只有一个结果被使用、两个结果跨block、flags被中间指令覆盖等情况。验收是每个数字迭代一次乘法/加法并保持溢出错误路径，不是简单删掉check。

W2b针对humantime timestamp fmt：现有L清理只分析Compare条件，`x & 0`等布尔值即使KnownBits可证明仍不能直接消除If。先在L层消费该事实，保持固定load/call/trapping操作原有控制路径；不要把S层要求保留poison的恒等式无条件改写。验收包含常量除数≠−1的signed MIN溢出分支消失，以及MIN/−1、除零、未定义值和多用户条件的负例。

这两项优先于大规模loop pass：因果清楚、独立性较强，可能同时减少运行与编译工作；实际收益仍由ablation确认。

### W3：先简化callee，再判断是否值得内联

从read_u64、decode_chunk_4/8、Option比较、UTF-8小包装开始，按调用图后序和有界常量事实准备callee。摘要至少区分graph revision、参数事实、目标布局及CGU可用符号；DataConflict、Missing、Size分别记录，不能统一解释为预算太小。

inline(always)只提供强意图，不绕过递归/ABI/数据身份/可链接性限制。匿名只读非TLS常量与有地址身份的data分开。一般内联预算不全局上调；按简化后的节点/call变化接受候选，并缓存失败结论直到依赖失效。

### W4：按访问范围消除临时对象

先做完整初始化、非逃逸、无别名的短直线路径，再做不同入边writer的lane Phi，最后处理循环局部快照。当前cycle guard是安全边界，不能直接删除。

read_u64理想成功路径是已证长度下的宽load+bswap；decode_chunk_8必须精确写6B，不能为省指令越界写8B。严格测试对象/页末、未对齐、重叠、padding、MaybeUninit/union、volatile/atomic。聚合返回直写sret须证明输出不覆盖尚未读取的输入，callee修改副本也不能改变原值。

### W5/W6/W7：以剩余产物决定范围

W5先实现当前查表确实需要的SIB寻址和select，验证x86寄存器编码限制、位移范围、flags活性、多用户和两地址约束。不能为了使用cmov提前执行可能越界的load。Unicode提供两项可单独验收的入口：`convert/intrinsics/select.rs:6`将select变成CFG和value-copy，标量实例需恢复SSA而aggregate/Drop语义须保留；`kfrc-x86/src/lower/branch.rs:73`排除pointer compare，可先只放开已降至地址标量的Eq/Ne、单use同block模式，不扩成任意pointer有序比较。

W6先u8范围规范化，全部256输入穷举；循环优化只在有规范preheader/latch/Phi、预算和收益证据后做有限peel/unswitch。区分S层poison观察和L层合法refinement，不让标量恒等式跨过guard改变trap/UB条件。

W7先标注栈槽来源，再谈stack coloring、rematerialization或寄存器分配。独立section产生的元数据可以尝试更短section命名/共享字符串表示，但须保持外部符号、重定位、GC边界、地址身份、工具兼容及对象确定性；不能用合并可回收单元掩盖object大小。

## 四库落地顺序

| crate | 第一轮 | 第二轮 | 后续条件 |
|---|---|---|---|
| semver | S1 checked算术；S6拆req；S7成本采样 | S2包装特化+S4字符区间，随后S3字段Phi/返回直写 | S5 peel/unswitch只在长identifier profile支持时做 |
| base64 | B1地址CSE；B7复测ranges | B2/B3先read_u64，再chunk_4/8；B4 SIB | B5聚合返回改善静态构造路径；B6只处理剩余spill/select |
| Unicode | U1短指针包装；U5 Eq/Ne分支；driver归属审计 | U2标量select；U3 category搜索闭包 | U4枚举/Result提升；U6新增word/sentence独立计时后排序 |
| humantime | H1格式化回退归因；H2单位匹配、H3边界复制、H5 L恒假分支独立候选 | H4 checked包装；H6两位数字闭包；临时对象与输出直写 | H7补小数/精度覆盖及编译预算；H3运行优先级由profile决定 |

先完成base64 read_u64一个闭包并不意味着其余库停滞：共用pass完成后，四库同时跑回归。这里“第一轮/第二轮”表示依赖和验收点，不是未经估计的工期承诺。每项实验提交可独立检查，最终累计到同一PR #8。

## 执行批次与验收

| 批次 | 内容 | 可交付结果 | 进入下一批的条件 |
|---|---|---|---|
| A | W0+C观测，处理timestamp format回退归因 | 固定ranges基线、复现脚本、当前阶段开销与调用闭包 | 版本/工作负载边界清楚，正确性差分通过 |
| B | W1、W2/W2b及W5的pointer分支独立候选 | 无重复table base、checked结果/flags共享、L恒假分支消失、pointer直接cmp+jcc | 模式消失；负例通过；独立运行/编译测量 |
| C | W3+W4逐闭包推进 | read_u64→chunk_4→chunk_8；semver op；Unicode UTF-8；humantime单位路径 | 每步有命中/拒绝原因、caller+callee闭包数据 |
| D | W5与W6局部规则 | SIB、select、区间归一；必要的有界循环特化 | 热路径cycles/分支改善，代码增长与编译成本受控 |
| E | W7残留、覆盖补齐、全矩阵回归 | 每库运行/编译/linked size验收表、同一PR内累计结果 | 不以平均值掩盖某库/某负载回退 |

建议将“接近LLVM”拆为两个验收里程碑：先使明显落后workload降至≤2×，再逐项争取≤1.25×；已经接近的路径不得稳定回退超过2%。这是工程目标，不是预测收益。所有倍率必须附输入范围和执行边界，不能用LLVM编译的泛型路径证明KFRC全链路接近。

编译预算先恢复新增成本：semver/base64/Unicode编译instructions回到本轮baseline±2%，保留humantime收益。50%wall目标另按固定历史基线和完整阶段占比验证；不以单pass或后端局部时间代替完整编译。新pass若收益未超过噪声且编译成本上升，不默认开启。

每项正确性至少覆盖对应语义负例、S/L verifier/解释器对照、native LLVM差分及四库语料。完整unit覆盖受fabsf64、atomic_xsub、atomic_load缺失阻塞，需独立补齐后重跑，不能记作当前全套通过。最终测量包括运行wall、cycles/instructions/branch、编译wall/CPU/RSS、符号code/object/rlib/linked consumer；不将这些指标合成一个未经验证的分数。

## 专项报告与证据

- [semver：S1–S7](semver.md)
- [base64：B1–B7](base64.md)
- [Unicode](unicode-segmentation.md)
- [humantime：H1–H7](humantime.md)
- [统一测量与ELF字节账本](evidence.md)
- [编译预算与50%目标](compile-budget.md)
- [完整函数体清单](function-inventory.csv)、[源码指纹](inputs.json)

LLVM机制参考来自`/Users/shom/Downloads/llvm-study`，资料固定21.1.0，实际对照为Rust1.94.1携带的LLVM21.1.8。这里根据最终产物定位可移植机制，尚未以逐pass ablation证明每条LLVM指令具体由哪一个pass生成。

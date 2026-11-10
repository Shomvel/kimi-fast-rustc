# 将 LLVM 优化 pass 迁入 KFRC

本轮聚焦优化 pass，暂不扩展成本模型。结论是先增强 **InstCombine → SCCP/SimplifyCFG → KnownBits/DemandedBits**，随后扩展已有的局部存储提升和内存优化，再处理后端模式与循环变换。每项独立落地和验收，不一次替换整条流水线。

**实施更新**：P1 的嵌套常量合并、共享 rewrite 和 GVN 交换律匹配已实现并验收，见 [InstCombine 结果](results/instcombine/validation.md)。普通操作数的全图排序因性能回退撤回；SCCP、KnownBits、cast 链及其余工作仍是计划。以下现状表和路线保留制定时的固定基线。

以下是实施前的移植计划与基线描述；新增实现和测量以上方更新为准。当前实现基线为 `7fb04aab2659e39dfa871b235fceb30bc0a5999b`（PR #8）；沿用已验收的早期优化。参考资料是本地 `llvm-study/README.md` 及 22 篇专题，研究源码固定为 LLVM 21.1.0 / `3623fe661ae35c6c80ac221f14d85be76aa870f1`。此前性能对照使用 LLVM 21.1.8，两者不混为同一实验版本。

## 1. 移植的边界

LLVM 的 pass 操作 `llvm::Function`、`BasicBlock`、`Instruction` 等对象，不能直接注册到 KFRC 的 Sea-of-Nodes 管线。应复用其算法、规则和证明条件，用 KFRC 的图、解释器和验证器重新实现。

| LLVM 概念 | KFRC 承接点 | 必须保留的条件 |
|---|---|---|
| SSA def-use | `NodeId`、`analysis::uses::Uses`、forwarding | 替换后更新使用关系；compact 会重编号 |
| BasicBlock / predecessor / Phi | 控制链、`Region`、`Phi`、`CfgEditor` | 一次事务同步修改前驱和所有 value/effect Phi |
| DominatorTree / LoopInfo | `ControlFlow`、`Dominators`、`analysis::loops` | CFG 改动后重新计算或明确维护 |
| 内存依赖与别名 | effect 链/Phi、`MemoryFacts`、参数权限 | 同地址不等于同内存版本；不能把指针数值相等当对象身份相同 |
| poison / flags / speculation | `Undefined`、`RequireDefined`、`ArithmeticFlags`、固定控制锚点 | 以 KFRC 的语义合同为准；不能机械照抄 LLVM 的 undef/refinement 规则 |
| 类型和布局 | R/S 级语义类型；L 级物理布局 | 逻辑 enum 字段、实际 tag 编码、ABI 传输分别处理 |
| target combine / MachineInstr | `LowerInstr`、`allocated`、`MachineInstr` | fixed/tied registers、调用 clobber、flags 和实际访存次数 |

例如 LLVM 可以在特定 refinement 条件下丢弃某些不确定行为，KFRC 的当前 `instcombine.rs` 则明确保留 poison 的可观察传播，刻意没有 `x * 0 → 0`、`x ^ x → 0`。迁移规则时不能仅核对普通整数的结果。

## 2. 现有实现与 LLVM pass 对照

以下代码链接固定在本轮基线，避免后续改动使现状描述漂移。

| LLVM pass / 机制 | KFRC 已有内容 | 真正应补的部分 | 次序 |
|---|---|---|---|
| InstCombine / InstSimplify | [instcombine.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/transforms/scalar/instcombine.rs)：常量折叠、简单恒等式；[seam.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/transforms/scalar/seam.rs)：变化点工作队列 | 常量位置与交换律归一化、嵌套表达式合并、cast 链化简；统一全图与 seam 的规则入口 | 第一批 |
| SCCP | 常量 If/VariantSwitch 删除、Phi 清理、范围事实和受控谓词推理分开运行 | 值格与可执行边联合传播，循环 Phi 的单调收敛 | 第一批 |
| SimplifyCFG | [simplify.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/transforms/control/simplify.rs) 删除已知分支和不可达图；[thread_variants.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/transforms/control/thread_variants.rs) 已处理部分 enum 路径 | 与 SCCP 对接；按具体缺口补空控制链、重复分支和退化 Phi 清理 | 第一批 |
| KnownBits / DemandedBits | [ranges.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/analysis/ranges.rs) 有区间、AND、逻辑右移推理；后端 [known_width.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-x86/src/lower/known_width.rs) 有窄值清理 | 稀疏已知零/一位、使用端需求位传播；不能把区间或字段需求当作完整位分析 | 第一批 |
| SROA / mem2reg | [promotion.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/transforms/memory/promotion.rs) 已提升不逃逸的逻辑存储，支持字段路径、Phi 和内联后重试 | 先统计未提升原因，再补固定数组元素/可分区访问；不是重新实现一个基本 mem2reg | 第二批 |
| EarlyCSE / GVN | [gvn.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/transforms/scalar/gvn.rs) 对浮动纯表达式做 exact numbering；内存读由独立 forward 处理 | 先用表达式归一化提高命中；随后考虑支配范围内 load 冗余，PRE 延后 | 第二批 |
| DSE | promotion 能删除被提升槽的 store；普通 DCE 保留固定内存操作 | 小范围、完全覆盖、无中间观察的死 store 删除；跨分支和部分字节覆盖以后做 | 第二批 |
| MemorySSA / AA | [forward.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/transforms/memory/forward.rs) 已沿 effect 链/Phi 有界查询，并使用 Frozen 权限 | 在现有 effect 图上增强 clobber/ModRef 查询，避免另建一套并行的全局内存 SSA | 随第二批扩展 |
| CodeGenPrepare / DAGCombine / Peephole | [address_mode.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-x86/src/lower/address_mode.rs) 已折叠常量 displacement；已有宽度清理、zero idiom、fall-through | 比例索引寻址、use-site 合并、flags 复用、复制链消除 | 第三批，可与第二批独立推进 |
| LICM / LoopSimplify / LCSSA | 有自然循环分析；[schedule.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-x86/src/lower/schedule.rs) 已将纯值放在支配路径中较浅循环层 | 变换用的循环规范形式；受保护 load 的合法外提、出口下沉、保留零迭代语义 | 第四批 |
| SCEV / IndVarSimplify / LoopStrengthReduce | 有循环结构和一般范围查询，未见通用递推分析 | 先支持单 latch、常量步长整数归纳变量，再做边界与地址递推 | 第四批 |
| RegisterCoalescer / InlineSpiller | [scan.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-x86/src/lower/allocated/scan.rs) 已有片段拆分、下一次使用驱逐、copy/回边偏好、spill 槽复用 | 有界重物化与热循环 spill 改进；不能误读旧 `machine/regalloc.rs` 后声称生产路径只做整区间分配 | 后端后续 |
| ADCE / aggressive CFG | 已有不可达路径和无用纯值删除 | 控制依赖驱动的活性传播尚需 postdom 等基础；保持不终止循环语义 | 延后 |
| LoopUnroll / LoopVectorize / SLP | 有部分向量传输及 intrinsic 降低，不能据此称已有通用自动向量器 | 循环规范化、依赖分析、合法化、尾部与运行时检查；先小固定循环完整展开 | 后续 |
| IPSCCP / CGSCC / LTO | 已缓存简化的 callee 图、收集调用引用，具备内联 | 纯函数返回常量摘要、受控的 callee-first 处理；跨 crate LTO 不作为首批前置 | 后续 |
| PGO / MachineBlockPlacement / BOLT | CFG 顺序与 fall-through 清理；对象每函数按 16 字节对齐并共享 `.text` | 分支/函数布局及分节独立实验；后链接优化另开工作包 | 独立后续 |

## 3. 第一批：三个可以直接开工的改动

### P1：补强 InstCombine，复用现有工作队列

LLVM 参考：`InstructionCombining.cpp`、`InstCombineAddSub.cpp`、`InstCombineAndOrXor.cpp`；学习资料专题 04、15、16。后两个具体规则文件可按需要补读固定版本源码，不把目录中的概述当作完整规则列表。

修改位置：`transforms/scalar/instcombine.rs`、`seam.rs`、`gvn.rs` 与对应测试。全图入口和内联 seam 调用同一套 rewrite，先解决规则能力，再以访问计数判断是否值得将全图最多四次扫描改成队列。

首批规则限定在普通、无溢出承诺的整数运算：

1. 交换律操作采用稳定 operand 顺序，常量放右侧，让 `x + y` 和 `y + x` 具有相同 GVN 键。不要按瞬时遍历顺序反复交换。
2. `((x + C1) + C2) → x + wrap(C1 + C2)`；`(x & C1) & C2 → x & (C1 & C2)`。结果按实际位宽计算，内外节点的 flags、类型和 poison 条件逐项核对。
3. 合法的截断/扩展链、重复掩码与嵌套位操作；第一步限 single-use 中间值，避免为了一个 use 复制表达式。
4. 把替换后的消费者重新入队，并维护新生成节点的 use 信息。规则方向固定；不增加全管线迭代到固定点的驱动。

验收：不同节点创建顺序下得到相同规范形式；8/16/32/64 位正常与回绕输入解释执行一致；带 flags、poison、跨类型和多 use 的负例不误改；再验证全管线是否改变内联和最终机器码。之前 early-CSE 曾通过节点预算改变内联，因此不能只验证这一 pass 的输出。

### P2：实现小而完整的 SCCP，并用现有 CfgEditor 提交结果

LLVM 参考：`Transforms/Scalar/SCCP.cpp`、`Transforms/Utils/SCCPSolver.cpp`、`ValueLattice`；专题 05。研究目录缓存只有部分入口，实现前需要补读核心求解器，不把薄 wrapper 当作整个算法。

拟新增 `analysis/sccp.rs` 和 `transforms/control/sccp.rs`，复用 `Uses`、整数求值规则、`CfgEditor`。首版只做 S 级具体整数/布尔值、Phi 和 If，不同时引入 IPSCCP。

- 为值维护 `Unseen / Constant / Overdefined` 的单调状态；`Unseen` 是分析状态，**不是** IR 的 `Undefined`。可能 poison 的值不能成为可删除观察的已知合法常量。
- 为控制边维护 executable 状态；维护 value worklist 和 edge worklist。
- Phi 只合并已经证明可执行的前驱；新边变为可执行时重新评价相应 Phi。循环信息只能单调放宽，不能在第一次迭代就把回边当永久不可达。
- 调用、未建模读取和其他无法推导的值保持 overdefined；求值器仍保留其控制和 effect。纯数值求解不决定一个调用能否删除。
- 分析收敛后批量替换值，使用 `CfgEditor` 同步裁剪 Region/value Phi/effect Phi，再交给现有 SimplifyCFG 和局部工作队列清理。遇到工作预算耗尽时保留原图或只提交已证明结论，不能将未访问路径当死路径。
- `VariantSwitch`、witness 和路径限定事实先保持现状；第二步再接入，明确维护证明作用域和动态循环实例。

典型增量是“只在可执行边上恒定的 Phi”，以及删边后继续传播的新常量。当前仅折叠 literal 分支并多次清理，并不等同于完整 SCCP。

验收：菱形 CFG、常量链跨 Phi、多层删边、零次/多次循环、循环 Phi 从 constant 升为 overdefined；不可达的 trap 可删除，可达的除零/RequireDefined/call 保留；前驱槽与 effect Phi 经验证器核对。与现有 `conditions`、`ranges` 的作用分开记录，防止把已有优化重新归功于 SCCP。

### P3：KnownBits 后接有限 DemandedBits

LLVM 参考：`ValueTracking`、`KnownBits`、`InstCombineSimplifyDemanded.cpp`；专题 21。KnownBits 是分析，DemandedBits 也是分析；实际改写接入 InstCombine，而非增加重复的独立重写链。

拟新增 `analysis/known_bits.rs`，首版覆盖 Const、AND/OR/XOR/NOT、合法常量移位和整数 cast。记录位宽、已知零位、已知一位，以及其成立的非 poison 条件；Phi 对已执行/可用输入取保守交集，不从原始未初始化内存推导有效值。

第二步做 use 方向的需求 mask：从截断、掩码以及明确的标量消费者倒推。多个 use 的需求取并集；未知 use、完整宽度返回/存储/比较保守要求全部位。先支持 single-use 局部链，再考虑跨 Phi 传播。

落地点是 base64 的 shift/mask/窄字节结果和解析器的字符分类。当前区间分析已能证明一部分表索引范围，不能将相同 bounds-check 消除再算一次收益。后端已有 `known_width`，语义层与机器层共享概念但各自维护所在表示的事实，不跨 lowering 缓存 NodeId 结果。

验收：一个值同时被低位截断和全宽比较使用时，高位仍正确；符号扩展、算术右移、超位宽移位遵守 KFRC 合同；改写不能仅因“高位无用”掩盖 flags 违约或 poison。检查最终 mask/cast/zero-extension 是否确实减少，而非只检查图节点数。

## 4. 第二批：扩展 SROA 与局部内存消除

### P4：在已有 promotion 上补 SROA 的访问分区能力

先对 `promotion::collect_slot` 的拒绝原因分类，优先收集真实热点中的固定索引、混合字段访问和整个聚合传输。已有的逻辑字段提升、enum whole-value 更新和内联后重试继续复用。

第一项可实现候选是：不逃逸、固定大小数组、所有索引已成为常量的局部槽，按实际访问元素形成分区再提升。动态索引、地址逃逸、根指针合流和部分 enum tag/payload 写入仍保留存储。内联或以后的小循环展开暴露常量索引后，再通过 dirty slot 集合重试。

LLVM SROA 不总是逐字段拆分。KFRC 的 `lower_layout/demand.rs` 已有字段需求，`owned_transport.rs` 已有 whole-value snapshot。必须同时保留“直接传输整值”的路径，避免把 Result/enum 的搬运扩大为所有变体的标量复制。humantime `parse_unit` 应分别检查 S 级字段操作、L 级 load/store、Phi copy 和实际 stack traffic，定位膨胀发生在哪一层。

验收：只读取一个字段不观察相邻 poison；整体复制保留原有初始化/字节语义；局部数组与 LLVM 执行差分；ABI 参数/返回传输正确；`parse_unit` 及其他负载同时比较机器码体积和执行时间。

### P5：先做局部 DSE，再增强 Memory GVN

新增局部 DSE 候选时，第一版仅处理同一直线控制/effect 段内、同一已证明的存储对象和同一完整访问范围的重复写入。后一写入必须覆盖前一写入所有字节，且中间不存在可观察旧值的 load、call、屏障或生命周期变化。指针观测、写入合法性和 `RequireDefined` 不能随旧 store 一起无条件删除。

首版不做跨分支必执行证明、部分字节 kill 或未知地址推理；这些需要额外的覆盖、postdom 或区间分析。已有局部提升已经消掉的 store 不计作 DSE 收益。

Memory GVN 优先扩展现有 effect walker 的查询复用、已证不别名写入穿越和受权限约束的调用穿越。复用已存在的 `WALK_BUDGET` 和验证机制。LLVM 各 pass 使用 MemDep/MemorySSA 的方式不同，没有必要为了名字一致同时维护另一套 MemorySSA。

## 5. 第三批：将后端 combine 搬到正确位置

首个独立工作包是比例索引寻址：`base + index * {1,2,4,8} + disp32` 折入实际 load/store。当前 `address_mode.rs` 仅沿常量 `PtrOffset` 累加 displacement，`MachineInstr::LoadIndirect/StoreIndirect` 也只有 base+offset，因而需要从地址选择、虚拟操作数、分配器约束到 SIB 编码一起扩展。

首版限单次访存、可表示 scale/disp、身份和偏移语义明确的地址；保留 base/index 的真实 use，不能让分配器漏算活跃值；不能越过 call、别名 store 或 effect 锚点移动 load。多 use 地址不强制全局共享，允许在访问处形成机器地址模式。

随后再加 flags 复用、复制链清理、合法的 load folding。load folding 并没有删除读取或其微操作；已有 `known_width`、zero idiom 与 fall-through 不重做。先保存每条规则的分配前后指令和可执行测试，再确认热循环收益。

地址共享和常量移位的旧候选已发生过跨负载回退，不能因 LLVM 有类似模式就原样恢复。晚期规则和 early-CSE 应作为不同候选验收，追踪是否改变了内联、spill 或最终布局。

## 6. 第四批：循环 pass 的依赖顺序

建议顺序是 **循环规范化 → 小范围归纳变量分析 → IndVarSimplify/LSR → 小固定循环完整展开 → 更一般的 LICM/向量化**。纯值循环外放置已部分存在，不以“添加 LICM 文件”作为成果。

1. 在 `CfgEditor` 上补受控的 preheader/边拆分与循环出口 Phi 维护，明确单 latch 和专用出口条件。首版拒绝不可约、多 latch 等未支持形态，不尝试修复所有 CFG。
2. 只识别一个整数 Phi、常量 start/step、可解释退出比较的归纳链。区分 backedge 次数与 body 次数；没有证据时不添加 nowrap。
3. 用递推事实减少重复边界推导或地址计算。x86 比例寻址已能承接的乘法，不机械改成新的指针循环变量。
4. 小固定循环完整展开必须先限制复制量，随后运行 SCCP、局部 SROA 和 InstCombine。验收包含循环零次、边界计数、wrap、提前退出和有副作用调用。
5. 对 load LICM，分别证明地址/内存不变及执行安全；不能在原本零次执行或条件读取的路径上新增访问。只有在 spill 证据支持时，再补 cheap address/constant 重物化；不假设后端自动补偿所有长活跃区间。

通用 LoopVectorize/SLP 在上述基础之后推进。需要别名/依赖证明、合法化、尾部和必要的运行时检查；现有资料不足以证明四库差距主要来自 SIMD，因此不排到第一批。

## 7. 管线接法与验收

接入点使用现有 [pipeline.rs](https://github.com/Shomvel/kimi-fast-rustc/blob/7fb04aa/crates/kfrc-son/src/pipeline.rs)，不照抄 LLVM 的 pass 名单和重复次数：

```text
已有的 S 图局部化简 / promotion
  → 增强 InstCombine（共享规则入口）
  → 现有范围与支配条件事实
  → SCCP → 原子 CFG 清理
  → 只重访受影响的标量/字段/槽
  → 现有内存转发、GVN；后续加入局部 DSE
  → 现有内联 → 对 splice seams 重做上述必要工作
  → layout lowering
  → 地址模式 / 机器级 combine
  → 现有调度、分配及机器清理
```

这是职责与依赖示意，具体插入顺序要逐项消融；不先重排整个默认管线。全图初始化和内联后的 seeded 路径分别验收，避免为了新 pass 丢掉现有的增量编译收益。

每个工作包交付三层证据：

- **pass 层**：正例与语义反例、解释执行、控制/effect/witness 验证；记录实际删除/替换和访问次数。
- **整管线层**：同输入经过完整优化与降低后，仍保留期望效果；核对内联选择、聚合展开和分配是否抵消或放大变化。
- **机器执行层**：Linux 语义差分、压力测试、四库正常/错误语料；正式计时固定基线、输入和构建配置，独立记录运行时间、动态指令、库对象/最终程序体积、spill/copy 和编译指令。

对新 pass 首先要求默认关闭时产物不变，再单独开启；单项通过后才组合。Verifier 只说明图结构有效，不能代替语义差分；单 pass 变少几条指令也不能代替运行性能验收。

**下一步最小可交付范围**：P1 的归一化/嵌套常量规则与共用 rewrite、P2 的整数/布尔 SCCP、P3 的 KnownBits 基础规则。先验收这三项，保留现有预算和默认基线，再进入内存和循环扩展。

## 参考材料

- 用户资料：`/Users/shom/Downloads/llvm-study/README.md`，重点专题 03–07、09–13、15–18、21；22 篇专题及 README 的指纹见 [llvm-study-inputs.json](llvm-study-inputs.json)。
- [LLVM 21.1.0 固定源码](https://github.com/llvm/llvm-project/tree/3623fe661ae35c6c80ac221f14d85be76aa870f1/llvm)。资料中的 `opt`/`llc` LLVM 21 示例没有全部实跑，Apple Clang 15 的观察不能当作本项目 LLVM 21 的 pass 消融结果。
- [现有差距与热点](report.md)、[已撤回候选的实验记录](implementation.md)。本计划据此选择验证对象，未将过往单例收益推广为新 pass 的预期加速。

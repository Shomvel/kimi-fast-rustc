# KFRC 成本模型与有限求解设计

状态：研究与后续设计。当前已按用户选择实现 [内建模型第一阶段](builtin-cost-model.md)，不引入外部求解器；下文求解器部分仅为备选研究。2026-11-10，基于 LLVM `llvmorg-21.1.8` 源码和本轮 baseline / candidate-v1 / candidate-v2 实验。当前两个候选均不能视为性能验收通过。

当前优先建立分层、带不确定性的内建成本模型。小规模整数求解器离线参照留待后续评估。不要把指令数当作运行时间，也不要在生产编译链中直接加入全函数、全阶段联合求解。求解器只能优化给定模型；如果漏掉代码布局影响，它会更精确地优化错误目标。

## 1. 当前实验暴露的三类问题

| 事实 | 说明 | 模型需要表达什么 |
|---|---|---|
| v1 base64 encode 约快 6%，`internal_encode` 6,095→4,042 B；重复表地址 39→1、CL 移位 43→0 | 合并冗余值确实能减少机器码和活跃值 | 机器操作、寄存器约束、共享后的活跃范围，而非仅 IR 节点数 |
| v1 semver requirement 慢约 5.4%，两个比较函数各增长约 500 B；`SplitInternal::next` 被展开 | 早期 CSE 改变内联阈值，局部收益影响后续决策 | 调用点特化后的净成本、阶段交互及明确的预算 |
| v2 修复上述内联变化，但 identifier 慢约 15%；v1/v2 identifier 路径规范化指令序列一致、retired instructions 基本相同 | 指令数模型不能解释；采样超过 91% 落在 `parse::identifier` | 布局敏感性与模型外残差；不能直接归因更多指令或 spill |

`parse::identifier` 本体 771 B；baseline / v1 / v2 链接地址模 64 分别为 32 / 32 / 0。这个相关性尚不能证明 cache line、分支预测器别名或其他前端机制是原因，必须用保持代码不变的布局干预实验验证。也不能因此硬编码某个函数的 padding。

## 2. LLVM 实际如何建模

### 2.1 IR 层：目标相关的多种成本

[`TargetTransformInfo::TargetCostKind`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/Analysis/TargetTransformInfo.h#L264) 区分 `TCK_RecipThroughput`、`TCK_Latency`、`TCK_CodeSize`、`TCK_SizeAndLatency`。成本可由目标定制，归一化也可能依目标而异，不能将所有返回值理解成精确 CPU cycles。

[`LoopVectorize`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Vectorize/LoopVectorize.cpp#L923) 按目标选择 cost kind，并在合法候选中评价不同 VF、标量化、控制流和访存成本。核心是先定义可行域，再比较目标相关成本；不是认为更少 IR 指令必然更快。

### 2.2 内联：结合调用上下文估计简化后的净收益

[`InlineCost.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/InlineCost.cpp) 跟踪常量传播、可简化指令、不可达路径、SROA 收益、调用点冷热与阈值。它并不简单数 callee 节点，也不保证预测到最终汇编。

该版本还有受条件控制的 cost-benefit 分析：[`isCostBenefitAnalysisEnabled`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/InlineCost.cpp#L895) 检查 profile summary、BFI、热点调用点及入口计数；不能因为命令行选项初值为 false 就概括成所有默认情形都禁用，也不能称所有内联都运行同一精确收益求解。

### 2.3 机器层：吞吐、关键路径和寄存器压力一起考虑

[`TargetSchedule.td`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/Target/TargetSchedule.td) 描述 issue width、微操作缓冲、operand latency、资源占用等。目标 CPU 的机器模型与 IR 层估计分开。

[`GenericScheduler::tryCandidate`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/MachineScheduler.cpp#L3940) 使用分层启发式，包含寄存器压力、stall、资源需求和 latency。源码明确说明采用层级判断便于效率、调试和统计分析；它不是将全部因素塞进一个固定线性加权和。

[`CalcSpillWeights`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/CalcSpillWeights.cpp#L276) 结合使用/定义、基本块频率、循环、可重物化性等估计 spill 权重。KFRC 当前主要依据下一次使用距离、固定寄存器和复制偏好；值得先补频率加权成本，而不是立即换整个 allocator。

### 2.4 LLVM 确实有求解器，但不是全局默认策略

[`RegAllocPBQP.cpp`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegAllocPBQP.cpp#L9) 将寄存器分配写成 Partitioned Boolean Quadratic Programming：变量选择寄存器或 spill，边编码冲突/复制成本。其求解器使用针对分配问题的启发式，不能据此宣称获得全局最优结果。

[`TargetPassConfig`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/TargetPassConfig.cpp#L1377) 的常规优化分配路径选 greedy，非优化路径选 fast；目标可覆盖。PBQP 是可选工具，并非 LLVM 常规优化链的统一求解基础。

[`MLGO`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/MLGO.rst) 将特定启发式交给模型，例如面向体积的内联、面向性能的 greedy eviction。合法性约束仍由编译器执行。我们只有四库、13 项负载，暂不足以训练可信的通用策略。

### 2.5 llvm-mca 的适用边界

[`llvm-mca`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/CommandGuide/llvm-mca.rst) 可分析给定 CPU 的机器依赖链、资源压力、吞吐和 timeline，很适合给 KFRC / LLVM 相同热点生成后端成本参照。

但它明确不建模 [fetch/decode 与分支预测](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/CommandGuide/llvm-mca.rst#L943)，也不了解 [cache hierarchy、实际 L1 命中及完整 store-to-load forwarding](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/CommandGuide/llvm-mca.rst#L1065)。本次相同指令、不同布局的性能变化，正属于它可能无法解释的范围。MCA 预测相同不等于机器实测应该相同。

## 3. 建议的模型结构

### 3.1 分离合法性、成本事实与策略

1. 合法性：保持 Rust poison、allocation identity、内存效果、控制依赖、整数溢出、ABI、fixed/tied registers 等硬约束。不用更低成本交换语义正确性。
2. 成本事实：返回多个维度、证据来源、置信范围、CPU 和模型版本，不直接替 pass 决定采用哪项优化。
3. 策略：按当前目标（低编译成本、运行性能、体积）选择候选，并遵守全局资源预算。
4. 事后核对：比较估计与实际降低/分配/测量，记录预测失败的原因，而非仅记录最终更快/更慢。

建议记录字段：

```text
context: target triple, CPU model, feature set, model version, phase,
         graph fingerprint, callsite/region identity, profile source
estimate: encoded bytes, uops, dependency/recurrence bound,
          resource-pressure bound, weighted memory operations,
          live-register pressure, fixed-register conflicts,
          spill/reload/copy costs, compiler work and peak memory
uncertainty: unknown costs, frequency confidence, residual range,
             layout sensitivity
choice: alternatives, rejected legality constraints, selected action,
        budget consumed, predicted deltas, observed deltas
```

没有 profile 时可用 CFG/循环结构估计相对频率，但必须标为估计，不能将循环深度直接当作真实执行次数。IR 层精度不足时保留 unknown，不能将其当零成本。

### 3.2 运行时间不能对指令 latency 简单求和

对一个基本块/循环区域，可先构建资源下界：

```text
backend_cycles_lower_bound = max(
    uops / issue_width,
    each_resource_demand / resource_capacity,
    dependency_or_loop_recurrence_bound
)
```

独立指令可重叠执行；循环携带依赖会限制迭代吞吐。load/store 端口、固定 CL 寄存器、长延迟除法都需要独立描述。上式是下界，不是实测总时间预测；调度模拟用于补充资源和依赖之间的交互。

全程序可先以基本块/区域执行频率加权得到后端近似，再用观测残差处理分支失误、cache 和前端限制。不得把能相互重叠的 stall 项直接相加两次。不同 CPU 的模型与残差分别校准。

### 3.3 CSE：省计算与延长活跃范围都入账

对候选共享点和其使用域计算：

```text
saving ≈ removed_dynamic_computations
         - added_spill_reload_copy_cost
         - added_dependency_cost
```

这里每项必须转换到相容成本单位，不能直接拿指令条数减 cycles。允许选择“全共享 / 按块共享 / 按循环共享 / 使用点重物化”，而非仅两个选项。低成本 LEA 在远处重新算一次，可能比跨调用保存指针更便宜。将同一指针合并后，不仅要看值的数量，还要看寄存器类别、存活区间和调用 clobber。

### 3.4 内联：替换裸节点阈值，保留编译预算

对调用点做有界的参数代入与局部简化估计，比较：被删除的 call/ABI 搬运、可折叠分支、切片长度/固定 tag 传播，与新增热路径工作、代码字节、活跃范围和后续编译工作。

callee 节点数仍可作防止爆炸的硬预算，不能单独当作收益。先提供 explain 日志，记录为何某个 CSE 导致一个 callee 跨过门槛；再引入新决策，避免两个 pass 通过同一阈值发生隐式耦合。

### 3.5 布局：单独处理模型外不确定性

保留函数/热点循环入口 mod16/32/64、分支目标、块顺序、指令编码长度，以及分支失误和前端相关硬件计数（事件可用性随 CPU/虚拟化而变）。不能用地址模值直接拟合可泛化的性能公式。

对本轮 identifier 做预先定义的多种布局扰动：保持热点指令和输入不变，改变函数/块的合法对齐与排序；测量多个布局而不是挑最优 padding。若性能随布局系统性变化，再选择通用布局策略。将候选在多布局上的中位/尾部回退作为稳健性指标。

## 4. Solver / LP 用在哪里

| 问题 | 变量与目标 | 建议 |
|---|---|---|
| 单调用点内联 | inline / keep，比较特化后的成本 | 先枚举两项，不需要求解器 |
| 多调用点预算分配 | 二元选择、总体积与编译预算 | 可用背包/MILP；第一版离线验证，成本交互需重新估计 |
| 小区域指令调度 | 整数开始时间、端口/资源选择 | CP-SAT 或 MILP 离线参照最有价值 |
| 小区域寄存器分配/复制 | register/spill/rematerialize 的离散选择 | PBQP 或有界 CP-SAT；先评估现有启发式距参照有多远 |
| 整函数联合选择+调度+分配+布局 | 大量二元变量，未知硬件效应 | 暂不做，模型误差和编译成本都过大 |
| 变换合法性 | 位向量、路径条件、内存模型 | SMT 可验证有限语义，但不是速度求解器，需处理 LLVM/Rust poison 与 UB 差异 |

以有限机器基本块为例，调度包含 `start[j] >= start[i] + latency(i,j)` 的依赖约束；每周期资源容量有限，memory effects 与调用屏障必须有边。指令不可分割，需整数变量。对固定干涉图，寄存器变量满足“每值选一个允许位置”和“冲突值不能占同一物理寄存器”；真实 x86 还要约束 register units、子寄存器、call clobber、tied operands、并行复制。若可选择 spill/split，必须计入新增指令与依赖，不能固定原图后把 spill 当免费选项。

因此普通连续 LP 通常只适合松弛下界或初筛，其分数解（例如半次内联、半个寄存器）不能直接执行。MILP/CP-SAT 能表达离散约束，但解的最优性只对建模问题成立；限时求解可能只给可行解与界，报告 optimality gap/timeout，不能将超时的最好解叫最优。

对于联合动作 `x_i`，若成本包含 `x_i*x_j`（例如 CSE 后才有利的内联），可引入辅助变量线性化；这不能修复遗漏的交互。先限制候选规模，求解后重新降低和估价，比一次求解全部 pass 更可控。

## 5. 实施次序与验收

| 步骤 | 改动位置与产物 | 验收 |
|---|---|---|
| M0：可解释账本 | `transforms/ipo`、x86 调度/分配输出 opt-in JSON 决策；关联源码、机器码和运行样本 | 默认关闭时对象不变；能完整解释本轮 early-CSE→inline 变化和布局残差 |
| M1：目标机器成本 | x86 虚拟指令层建立 bytes/uops/latency/resources 接口；离线导出固定热点供 llvm-mca 对照 | 使用真实 CPU 指纹或明确generic模型；未知操作显式unknown；正确预测立即数移位和固定CL差异 |
| M2：局部策略 | 调用点特化净成本、按频率的spill权重、CSE共享域/重物化选择分别实验 | 保留原算法回退；单项及组合验收；不让低收益候选消耗无限编译预算 |
| M3：求解器离线参照 | 导出小区域 dependency/resource/register 约束，用CP-SAT/MILP或PBQP方案求解 | 独立验证解的合法性；执行差分；报告模型值、实测、求解时间和gap，判断瓶颈在策略还是模型 |
| M4：决定是否在线 | 仅对已证明收益的小热点启用有界求解，超时回退 | 依据测量设置编译时间/RSS预算；优化收益足够覆盖成本后才作为可选模式 |

建议先做 M0，再做 M1/M2；不以集成 solver 包作为完成标志。第一版暂不引入 ML 模型，也不让外部 solver 成为编译器默认依赖。

总体验收采用多目标约束：在指定编译成本、体积和关键负载回退预算内优化运行性能，保留 Pareto 候选；不要用无单位的 `a*指令数+b*字节数+c*毫秒` 掩盖取舍。若需要综合分数，可使用有明确定义权重的对数比值，但必须同时报告逐负载和最坏退步。回退容限由目标与噪声实验决定，不在看到结果后临时放宽。

现有四库用于回归，不足以代表训练分布。至少扩展独立项目与输入分布，按整个 crate/程序划分训练、调参、保留评估集；同一函数的不同布局不能跨集合造成泄漏。校准时优先看候选排序准确性和误接受回退的比例，再看绝对 cycles 误差。布局应另作多样本稳健性评估。

编译成本保留三种口径：整个 rustc 进程、精确 codegen 窗口、单独模型/求解器开销。布局实验、profile 和正式计时仍串行分开。M0–M3 不宣称能消除所有性能波动；它们的价值是让未知成本可见，让每个策略选择可检验。

## 来源与复现

LLVM 文件均从 `llvmorg-21.1.8` 固定标签读取，URL 与 SHA-256 见 [llvm-cost-sources.json](llvm-cost-sources.json)。标签与本次 Rust 1.94.1 的 LLVM 版本一致；这不表示 Rust 工具链的全部构建补丁逐字节等于上游标签。

本轮实测限制和候选记录见 [implementation.md](implementation.md)。模型接口、求解预算和策略均为建议；本轮尚未安装 solver、运行 llvm-mca 或宣称模型预测准确。内建模型和实际内联策略的实现范围见独立实现说明。

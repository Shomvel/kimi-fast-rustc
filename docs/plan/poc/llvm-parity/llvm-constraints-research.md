# LLVM 21.1.8：指令约束、Phi 拷贝与临时寄存器

日期：2026-11-09。范围：为 LP2 的 SSA 线性扫描实现研究可复用机制，不改用 LLVM Greedy，不作独立 code review。已读取 `llvmorg-21.1.8` 的下列实际源码，并对照本仓库 `lower/allocated/{constraints,allocation,emit,moves,checker}.rs`。本轮没有编译或运行 LLVM；以下是源码事实与适配建议，不能当作候选性能结果。

## 1. 固定寄存器和时序是活跃区间的一部分

LLVM 用 [SlotIndexes.h 的 Slot 枚举](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/CodeGen/SlotIndexes.h#L65) 表示一条指令内的边界：

- `Slot_Block`：块边界以及 Phi 定义。
- `Slot_EarlyClobber`：提前写入；它与在普通 register slot 才被杀死的输入相干涉。绑定到 early-clobber 输出的输入也在这一点结束。
- `Slot_Register`：普通输入死亡和输出定义的时点。
- `Slot_Dead`：没有使用者的输出的结束点。

这使普通“输入最后一次读取后，输出可以复用该寄存器”和 early-clobber 的排斥关系可由半开区间表达，不必让所有输入与所有输出整个指令都相互排斥。

物理约束也不是发射阶段才临时补的动作。[LiveRegMatrix::checkInterference](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveRegMatrix.cpp#L194) 按顺序检查调用 `regmask`、固定物理寄存器 `regunit` 区间、已分配虚拟值之间的干涉；[checkRegUnitInterference](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveRegMatrix.cpp#L172) 还利用 coalescing pair 区分可合并的 copy 边界。

x86 的具体事实：

- [变量移位声明 `Uses = [CL]`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86InstrShiftRotate.td#L106)，不是任意 GPR。
- [64-bit DIV/IDIV 声明](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86InstrArithmetic.td#L150) 使用 RAX、RDX，定义 RAX、RDX、EFLAGS。kfrc 的 `Divide` 是包含 `cqo`／清零 RDX 的组合操作，所以除数不能在准备高半部分时被覆盖；不能仅照抄单条 `idiv` 的普通输入规则。
- [LiveIntervals::computeRegMasks](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveIntervals.cpp#L254) 把调用 mask 放在 register slot。调用前的参数使用、调用破坏、调用后的结果定义需要区分；返回值不属于跨调用保存的旧值。

**适配建议。** 保留当前约束接口和线性扫描，但让扫描器看到固定输入、固定输出与 clobber 的占用时段。优先引入明确的 use/def 子位置，而不是把 RCX 在整条移位指令上禁用给所有值。相同值已位于固定输入寄存器时应允许不搬运；一个无关值跨越该约束才需要换位置。调用输出从 clobber 之后开始，不能被这一调用本身排斥。单个整数位置下先加 pin 的保守实现可以正确，但仍可能在调用和移位两侧产生不必要的 reload；不要把它当作完整时序建模。

## 2. tied operands 不等于每次强制复制

[TwoAddressInstructionImpl::processTiedPairs](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/TwoAddressInstructionPass.cpp#L1554) 会把 `A = op B, C` 转成 `A = COPY B; A = op A, C`；如果绑定两端已经同一寄存器则无需 copy。它同步修正 live interval，并按普通或 early-clobber slot 结束输入片段。

在生成 copy 之前，[tryInstructionTransform](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/TwoAddressInstructionPass.cpp#L1314) 尝试交换可交换操作数，使被复用的输入在这里死亡；必要时转换成三地址形式（源码直接以 LEA 为例），或在合法范围内调整顺序。随后 RegisterCoalescer 继续消除 copy。

**适配建议。** 当前先做“将 dying source 分到输出位置”是合理的最小版本。下一步可在已知可交换的 Add/Mul/And/Or/Xor 上，把在此死亡的右操作数换到 `Reuse(0)` 一侧；Sub、移位、除法不得交换。必须同时更新输入次序、Reuse 索引和 checker 的期望值，不能只改物理寄存器。不要一开始引入 LLVM 的重调度；它需要额外的依赖、flags、内存和 trap 合法性分析。LEA 属于 LP4。

## 3. LLVM 的 Phi 消除不是分配后统一借物理 scratch 解环

[PHIEliminationImpl::LowerPHINode](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/PHIElimination.cpp#L341) 为一个 Phi 建立新的 **虚拟** `IncomingReg`，在前驱放 `IncomingReg = COPY source`，在目标块 Phi 区域之后放 `destination = COPY IncomingReg`。见 [前驱拷贝插入](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/PHIElimination.cpp#L541)。后续 coalescing 和寄存器分配消去不必要的副本。新虚拟值使循环 Phi 的旧值在所有应读取的位置仍存在；不是把互换的物理赋值随意顺排。

`undef` 输入不需要真实 COPY，LLVM 用 IMPLICIT_DEF 维持定义的控制流约束；整个 Phi 的输入都 undefined 时直接生成 IMPLICIT_DEF。kfrc 跳过 Undefined 的物理拷贝与这个方向一致，但仍要在 IR 与 checker 中保留“只对真实要求的位负责”的语义。

[SplitPHIEdges](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/PHIElimination.cpp#L757) 选择性拆关键边，以减少同一前驱上的 copy 延长源值生命周期、妨碍 coalescing 的情况。源码明确避免默认拆 loop backedge，因为小型 out-of-line 块对布局很差。kfrc 为条件边上的位置迁移生成 trampoline 是正确性机制，但热回边不应无条件多跳一次；应先减少边上的位置变化，再决定布局。

**适配建议。** kfrc 既然已经在分配后保留一组 simultaneous copies，就继续使用依赖排序和明确的 cycle 临时值，不必为了模仿 LLVM 恢复两层虚拟 copy。LLVM 的可复用部分是：边作用域、旧值快照、精确 liveness 和 coalescing；不能声称当前物理 move resolver 是 LLVM 同款实现。

## 4. coalescing 要验证值关系，不能只看一个端点

[RegisterCoalescer::joinVirtRegs](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegisterCoalescer.cpp#L3668) 先映射两边 value numbers，再双向解决冲突，成功后才合并 live ranges；复杂区间还有成本上限。由于相交范围上可能已经是同一个值，LLVM 不只是“两个 hull 不相交才合并”。反之，看到一个 copy 就直接 union 两个虚拟寄存器也不安全。

[compareMBBPriority](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegisterCoalescer.cpp#L3997) 优先处理深层循环，再考虑拆边块和 CFG 连接数；[joinAllIntervals](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegisterCoalescer.cpp#L4210) 按此顺序处理。可借鉴的是把有限的 coalescing 机会优先用于反复执行的边，不必完整移植 value-number coalescer。

**适配建议。** 第一阶段保留软 affinity：输入/输出的精确活跃片段不冲突，且固定约束允许时选择同位置；失败就继续普通分配，不破坏区间。Phi 有多个输入时给循环回边或已知热边更高权重，而不是让最后扫描到的一条边覆盖所有提示。对多定义的 Phi 虚拟值，应按片段/边检查，不能套用单定义 Copy alias 消除。两个 Phi 互换和一源多目标必须整体校验。

**片段结束不是值死亡。** 主实现者在 candidate17 的生成式 checker 失败中已定位：某个 source 的当前片段可能恰在下一条 call pin 前结束，但值仍跨调用活跃。不能以 `segment.end == position + 1` 单独证明 dying-source coalescing 合法；至少同时要求 source 不在当前指令的 live-after 中。LLVM 的 kill/value-number/liveness 判定也是值语义，不能用刚切出的片段端点代替。本研究不重复排查该故障，修复与回归由主实现者负责。

## 5. 临时寄存器先找可用者，紧急 spill 是后备

[RegScavenger::findSurvivorBackwards](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegisterScavenging.cpp#L120) 同时检查临时作用域内的 used/clobbered 单元与作用域外的 live-out，优先返回完全空闲寄存器。没有空闲者时才在有限窗口内找较远才需要的寄存器。[scavengeRegisterBackwards](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegisterScavenging.cpp#L295) 可通过 `AllowSpill=false` 表达“找不到就失败”，允许 spill 时才调用 [spill](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegisterScavenging.cpp#L221)，使用匹配 size/alignment 的 emergency slot 或目标专有保存方法。它不是每次先保存固定 scratch。

candidate17 的“materialize 临时值先选不必保存的 GPR”与这个原则一致；尚不能据源码推出运行收益，需等测量。当前 `copy` 的 stack→stack／imm→stack 固定借 R11 并保存恢复，可以按同一原则改进。

**可直接实施的最小改法：**

1. `parallel(copies, protected)` 明确传入这一次搬运边界仍要保存的寄存器。整组 copy 的所有 src/dst 寄存器一律排除，先采用保守规则，不尝试在组内中途回收。
2. 从剩余 caller-saved GPR 中选临时寄存器；若没有，再考虑已经保存的 callee-saved，或者采用原有临时槽。新使用 callee-saved 也要计入保存／恢复，不能把新增栈帧当作“免费 scratch”。
3. `moves::resolve` 的临时值类型可从 `StackSlot` 改为已证明可用的 `Location`；循环仍采用 `temp = old source`，把所有 pending 源的该位置换成 temp，然后继续依赖排序。
4. 入口拷贝必须保护所有未消费的 ABI 输入；指令前拷贝保护 live-through 以及所有原始输入；指令后拷贝保护 live-through、结果与已准备的 restore 值；CFG 边保护目标 live-in 在源边的位置。不能在所有场景复用一个粗略 `before[position]` 结论。
5. cycle 临时寄存器持有旧值时，嵌套的 stack→stack lowering 不能将其作为无保存 scratch。统一保留集合或借用作用域；实在没有空闲寄存器就保留现有 R11 保存/恢复后备。临时栈槽与 R11 保存槽也不能别名。
6. copy sequence 必须保持 flags，尤其位于 cmp 和条件跳转之间。普通 MOV/LOAD/STORE 满足这一点，不能为了写 0 替换成 XOR。

这些是从 LLVM 的空闲性证明和后备策略推导的 kfrc 方案，不是 LLVM generic post-RA parallel-copy resolver 的直接移植。

## 6. 建议随实现一起验证的边界

| 改动 | 最小结构与行为验证 |
|---|---|
| dead scratch 优先 | 长期活跃参数跨几十次 immediate 运算且低压力，spill slots=0，最后仍读取原参数；已有 candidate17 测试覆盖这一类 |
| fixed input 时序 | 一个无关长期值跨多次 CL 移位，另一个计数值在 RCX 中继续使用；再加入除余的 RAX/RDX、调用双返回值；同时检查正确性和保存/恢复数量 |
| tied commute | `a+b` 中 a 继续使用、b 在此死亡，输出应可复用 b；与 Sub 作不能交换的对照；相同输入 `a+a` 和输入仍活跃都要覆盖 |
| register cycle temp | 2/3/16 值旋转、一源多目标、两独立环、R11 本身是环成员；有 free GPR 时不引入临时槽，无 free GPR 时结果仍正确 |
| memory copy scratch | 同组混合 reg→stack、stack→reg、stack→stack、imm→stack，包含长活跃 R11 与 cycle 临时值，证明临时槽不互相覆盖 |
| 边与 flags | 同一前驱两后继各需不同搬运；Phi 源还用于分支条件；循环回边；cmp/jcc 中间插入 copy 后分支结果不变 |
| coalescing | 相邻但不冲突的片段合并；源在 Phi 后仍活跃则保持独立；六个跨调用活跃值及调用前 Reuse，防止把 call pin 截断误判为值死亡；checker 验证整组同时赋值，不只检查 mov 数 |

结构检查不能代替真实执行。按现有流程继续 Linux 压力三臂、语义语料和 semver；正式比较保留原始分支、mov、rbp 访问、spill 来源与编译阶段数据。并行拷贝少了几条并不自动意味着 runtime 改善。

## 7. candidate21 的具体接口设计：只改变 scratch 选择

本节是基于当前 `emit.rs` 的实施设计，未修改业务源码。candidate20 保持字节透明编译优化；本节改变搬运指令，需另行冻结、验证和测量。

### 接口与不变量

建议 `parallel(copies, protected: RegMask)`。`protected` 的含义是“不能被额外 scratch 写坏的现有值”，**不是**禁止本组合法 copy 写这个寄存器。不要把 scratch 禁用集合混入 copy 本身的合法性判定。

```text
parallel(copies, protected):
    // 必须在移除 identity copies 之前做。r→r 表示已经到位但仍要保留的值。
    touched = regs_of_all_src_and_dst(copies)
    forbidden = protected | touched
    cycle_reg = choose_free_scratch(forbidden)
    cycle_temp = cycle_reg.map(Reg).unwrap_or(existing_cycle_or_next_slot)

    // resolve 依旧检测非法/重复目的地、去 identity、拓扑顺排及断环。
    sequence = resolve(copies, cycle_temp)
    if sequence actually writes proposed cycle stack slot:
        reserve cycle slot before emitting any sequence instruction

    // 全序列保守保护 cycle_temp，无须追踪哪一步临时值死亡。
    copy_forbidden = forbidden | reg_if_any(cycle_temp)
    memory_reg = choose_free_scratch(copy_forbidden)
    for copy in sequence:
        emit_copy(copy, memory_reg)  // 没有 free register 时保留现有安全后备
```

`choose_free_scratch` 首版只选择未在 mask 中的 caller-saved GPR；可次选函数已经使用并将保存的 callee-saved。不要仅为一个 cycle 首次引入 callee-saved：新增保存／恢复可能比临时槽更贵。RSP/RBP 永远排除。此处没有调用，caller-saved 只表示不额外增加函数 ABI 保存义务，并不表示“当前值可随意丢弃”。

`moves::resolve` 将参数 `temporary: StackSlot` 改为 `temporary: Location`，只允许 Register 或独立 StackSlot。确认临时位置不出现在原始任何 src/dst；这一点由调用者证明，也可在 resolver 入口显式拒绝，免得未来调用点忘记。duplicate destination 验证仍保留；合并 post 后应在删除 identity 之前验证原始目的地，避免 `RAX→RAX` 被删掉后掩盖另一条不同值的 `saved→RAX`。调用点已将重复且来源一致的 operand assignments 去重，不需要在 resolver 默默吞掉冲突。

```text
emit_copy(copy, free_memory_reg):
    Reg→Reg / Stack→Reg / Imm→Reg / Reg→Stack: 直接发射原有单条指令
    Stack→Stack / Imm→Stack:
        if free_memory_reg exists:
            materialize source into free_memory_reg
            store free_memory_reg into destination
        else:
            // 和原实现一致，作用域仅包住这一条copy。
            store R11 into dedicated scratch_save_slot
            materialize source into R11
            store R11 into destination
            load R11 from scratch_save_slot
```

`cycle_slot`、`scratch_save_slot`、每个物理寄存器的 `preserve_slot` 必须是互不别名的槽。若 `cycle_temp == R11` 且没有第二个 free register，后备仍可保存 R11、执行一个 memory copy、恢复 R11；这会保持 cycle 临时值。不要改成“既然是 scratch 就跳过保存”。简化版本也可以从 cycle 候选中排除 R11，但不是正确性必需。后备 materialize 和 store 都是基础指令，不再递归调用可能申请同一 scratch 的通用 parallel。

### 每个调用点的 protected 来源

记 `H(v,p)` 为当前位置的 home；`R(locs)` 只取其中的 GPR；`T(p)` 为当前指令所有 live-through 值，排除当前 definitions；`D` 为原始当前并行组的定义集合。所有集合都来自真实 liveness，不能从片段 end 猜测死亡。

| 调用点 | 传入 protected | 说明 |
|---|---|---|
| `Entry(args)` | 首版保守用全部六个 ABI GPR 参数寄存器 | copy 内的真实 ABI 输入和目的 home 也由 touched 自动保护。unused ABI 参数可以不保护，但首版不必为省这点成本削弱入口规则。R10/R11/RAX 通常可作为 free scratch。 |
| `Operation::Parallel` | `R(H(v,p), v∈T(p))`，其中 definitions 取原始 Phi/copy 组目的虚拟寄存器 | 所有源和最终目的地由 touched 保护；必须保留原始 identity 的寄存器。其他 live-through 值没有出现在 copy 中也不能被借用。 |
| `edge(source,target)` | 对目标全部 live-in，取 `R(H(v,source)) ∪ R(H(v,target))` | `Allocation::edge` 已经过滤 source==destination，故不能仅由返回的 Copy 列表推 protected。相同位置的 live-in 恰恰是容易被漏掉的值。分支边各自计算，不需保护只属于另一路的死值。 |
| 指令前 `before` | `R(H(v,p), v∈T(p))` | 所有实际 inputs 的源 home 与选定 operand GPR 来自**未删 identity**的 before 列表；touched 自动保护。首版保留所有 live-through home，即使已存进 preserve slot，也不把旧 home 临时回收，减少状态推理。 |
| 合并后的 `after + restores` | `R(H(v,p), v∈T(p))` | 结果物理寄存器、结果home、恢复home、保存槽都在该组 copy 中；touched 保护所有寄存器端点。没被覆盖、因而没有restore的 live-through home 由 protected 保护。 |
| `restores` 若暂时不合并 | 额外保护全部结果home，以及未恢复的/不需恢复的 live-through home | 单独恢复组不再包含 after 的目的地；若只看 restores，已经到位的结果可能被当作 free scratch。保持两组也可正确，但调用点更易漏保护。 |

preservation stores 在当前代码中全是 `Reg→Stack`，继续用不借 scratch 的直接发射，不必引入新 parallel API。所有该类 stores 必须在 `before` 可能覆盖任何 source 之前完成；不要改为“边发现 live value 边开始 before moves”。若未来合并 preservation 与 before，需要在同一 simultaneous-copy resolver 中同时表达，不能简单拼接并默认寄存器值没变。

### 推荐合并 after 和 restores

当前顺序是 `parallel(after); parallel(restores)`。推荐改为：

```text
// 先完成 operation，outputs[] 此时持有所有新结果。
post = every (Reg(outputs[i]) → H(inst.outputs[i].value,p))
post.extend(every (preserve_slot[r] → Reg(r)))
parallel(post, R(H(v,p), v∈T(p)))
```

这个合并在当前契约下安全：

1. 所有新输出值和 live-through 旧值最终必须占不同 home；否则分配/checker 本来就应拒绝。
2. `preserve_slot[r]` 是 emitter 新分配的私有槽，不与输出 home、spill home、cycle/scratch 槽重叠，operation 不写这些槽。
3. operation 完成时，所有 `post` 源都已经存在：新结果在物理输出寄存器，旧值在保存槽。不存在“先执行一条 after 才产生另一条 restore 的源”的必要串行依赖。
4. 合并后拓扑排序能处理 `RAX(new)→R8; saved→RAX`：必须先消费新结果再恢复旧 RAX。也能处理双返回值对应 home 互换。
5. 保留 duplicate-destination 拒绝；不要为了合并把两条不同来源、相同目的地悄悄去重。出现这种情况说明分配或约束违反了最终home不重叠契约。

若 `after` 有 identity `RAX→RAX`，它仍需要参与 touched mask。它和 `saved→RAX` 同时存在且要求不同值，是无效分配，不是 resolver 可以修复的冲突。现有 checker 对 `after` 切片的验证同时包含 restores，因此两组合一不要求改变 checker 的语义。

### cycle 与嵌套 memory copy 的完整作用域

循环临时寄存器的生存期是从 resolver 插入 `temp←old_source` 到最后一个读取该 temp 的 copy。在首版实现里整个序列都排除 temp，虽保守但省去一套维护中的物理 liveness。若循环包含栈位置，比如 `r0←slot0; slot0←r0`，用 free `r1` 断环后都是基础 copy，无须第二个 scratch；若另一条独立 copy 是 `slot2←slot1`，它可使用另一个 free GPR，否则保留 R11 的逐copy保存后备。

保护集合不得在执行第一条 copy 后按“源已消费”随意缩小：该寄存器可能已经成为另一条 copy 的最终结果，或者还被 fan-out 源引用。完整 src/dst union 加 live-through mask 是最小改动里最容易证明的版本。进一步缩小借用范围可以后续用独立 move-plan 活跃性证明完成。

### candidate21 最有价值的定向验证

1. Phi 三值环加一个完全不参与拷贝的 live-through R11；有空闲 GPR 时 cycle slots=0，R11 保持。
2. edge 中一个 source==destination 的长期值，以及另一组 stack→stack；这能抓到 `Allocation::edge` 过滤 identity 后漏保护。
3. 新结果已在最终home（identity after），同时另一个输出需要存栈和旧值需要恢复；最终结果不得被 scratch 覆盖。
4. 物理输出 RAX 被新结果占用，但旧 live-through RAX 从保存槽恢复，结果home在另一GPR/栈；验证合并 post 的顺序。
5. 全 GPR 压力、R11 本身为cycle临时或原始源，混合两独立环、fan-out、stack→stack；验证后备保存槽与cycle槽隔离。
6. cmp 与 jcc 之间仅执行搬运，用已知 flags 的真实分支结果验证零立即数仍用 MOV、无 XOR。

checker 应继续默认开启。candidate21 的优化量另外记录为“cycle register vs stack 次数”“free memory scratch vs saved scratch 次数”，便于解释 mov/访存变化；这些计数可以只在诊断模式开启，避免影响正式编译指令口径。

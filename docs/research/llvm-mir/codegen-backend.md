# 后端：从 LLVM IR 到目标文件

每个模块在 post-link（ThinLocal）或 pre-link（无 LTO）之后，由同一个 worker 调 `codegen`（[write.rs#L939](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/write.rs#L939)）→ `LLVMRustWriteOutputFile` → `TargetMachine::addPassesToEmitFile`（[PassWrapper.cpp#L461](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_llvm/llvm-wrapper/PassWrapper.cpp#L461)）。后端仍是 legacy PM，rustc 不改它的 pass 序列，只给 TargetMachine 选项（function/data sections、TrapUnreachable、PIC 等，#L299）和 codegen 级别：O0→None、O1→Less、O2/s/z→Default、O3→Aggressive（[write.rs#L142](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/write.rs#L142)）。

## 结构（实测，`codegen-passes.sh`）

| config | blocks | passes | regalloc | sched |
|---|---:|---:|---|---|
| llc-x86-O0 | 1 | 74 | Fast Register Allocator |  |
| llc-x86-O1 | 1 | 203 | Greedy Register Allocator | Machine Instruction Scheduler;Post RA top-down list latency scheduler |
| llc-x86-O2 | 1 | 203 | Greedy Register Allocator | Machine Instruction Scheduler;Post RA top-down list latency scheduler |
| llc-x86-O3 | 1 | 203 | Greedy Register Allocator | Machine Instruction Scheduler;Post RA top-down list latency scheduler |
| rustc-O0 | 2 | 74 | Fast Register Allocator |  |
| rustc-O1 | 5 | 211 | Greedy Register Allocator | Machine Instruction Scheduler;PostRA Machine Instruction Scheduler |
| rustc-O2 | 5 | 212 | Greedy Register Allocator | Machine Instruction Scheduler;PostRA Machine Instruction Scheduler |
| rustc-O3 | 5 | 222 | Greedy Register Allocator | Machine Instruction Scheduler;PostRA Machine Instruction Scheduler |

- `passes` 是最大一段结构里的 pass 数（含分析）。rustc 每次编译多打印几段：分配器 shim 模块的 TargetMachine 用 `OptLevel::No` 创建（[lib.rs#L429](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/lib.rs#L429)），所以那一段总是 O0 结构，另有 3 段只含 domtree/SCEV 的小段，所以 rustc O1+ 为 5 段。
- **指令选择：** aarch64 在 O0 用 GlobalISel（IRTranslator → Legalizer → RegBankSelect → InstructionSelect，失败回落 SelectionDAG），O1+ 用 SelectionDAG；x86-64 各级都是 SelectionDAG（`X86 DAG->DAG Instruction Selection`；推断：O0 时其内部启用 FastISel）。
- **寄存器分配：** O0 `Fast Register Allocator`，O1+ `Greedy Register Allocator`；O1/O2/O3 的 x86 后端结构相同（203 个），差别只在 pass 内部的阈值。
- aarch64 O3 另有 `Machine Outliner`，x86 O3 没有。

x86-64 O3 的顺序（去掉分析与簿记 pass，名字取自 `llc-x86-O3.passes.txt`）：

| 段 | pass |
|---|---|
| IR 级准备 | Expand large div/rem、Expand Atomic、Canonicalize Freeze in Loops、**Loop Strength Reduction**、Merge contiguous icmps → memcmp、Expand memcmp、Constant Hoisting、Partially inline libcalls、Interleaved Access、**CodeGen Prepare**（地址模式下沉、select 处理）、Exception handling preparation、stack protector |
| 指令选择 | **X86 DAG->DAG Instruction Selection**（含 DAGCombine、合法化、switch 降低为跳转表/位测试/二叉树）、Finalize ISel |
| SSA 机器级 | Early Tail Duplication、Optimize PHIs、Merge disjoint stack slots（栈着色）、Early If-Conversion、Machine InstCombiner、X86 cmov Conversion、Early Machine LICM、Machine CSE、Machine code sinking、Peephole、X86 LEA Optimize、X86 Optimize Call Frame |
| 寄存器分配 | Eliminate PHI、Two-Address、Register Coalescer、**Machine Instruction Scheduler**、**Greedy Register Allocator**、Virtual Register Rewriter、Stack Slot Coloring |
| RA 之后 | Machine Copy Propagation、Machine LICM、PostRA Machine Sink、Shrink Wrapping、Prologue/Epilogue、Control Flow Optimizer（分支折叠）、Tail Duplication、Post RA scheduler、Branch Probability Basic Block Placement |
| x86 修补与发射 | Execution Dependency Fix、BreakFalseDeps、vzeroupper inserter、LEA Fixup、Fixup Inst Tuning、Live DEBUG_VALUE analysis、X86 Assembly Printer |

## 时间（前期 E2B，rg A 臂 `llc -O3` 重放，`census-passes.sh` 汇总）

| codegen family | codegen_s | share_of_codegen_% |
|---|---:|---:|
| 指令选择 | 7.36 | 35.3 |
| 机器级优化（CSE/LICM/sink/窥孔/布局…） | 3.34 | 16 |
| 寄存器分配与活跃性 | 2.78 | 13.3 |
| IR 级后端准备（LSR/CGP…） | 2.21 | 10.6 |
| 后端分析 | 2.2 | 10.5 |
| 调试信息 | 1.36 | 6.5 |
| 发射/帧 | 0.98 | 4.7 |
| 指令调度 | 0.61 | 2.9 |

- 单个最大的是 `X86 DAG->DAG Instruction Selection`，占全部 LLVM 计时的 10.2%（rg A）、12.2%（cargo Z），与 InstCombine 相当（[measurements.md](measurements.md)）。
- 前期 cargo 主库 aarch64 trace：AArch64 Instruction Selection 20.17 s，占后端 RunPass 累计的 29.2%；其次 Greedy Register Allocator 5.68 s、Machine Instruction Scheduler 3.96 s（前期 trace-hotspots §2）。
- rg 的 `debug = 1` 让 `Live DEBUG_VALUE analysis` 占 codegen 的 4%；cargo crate 无 debuginfo 时只有 0.2 s（前期 next-directions §2）。
- **codegen 与最终代码量同比：** Z 臂把 codegen 只降 6%，与 after-pm 指令 −7.5% 同步；Z 之后 codegen 成了 LLVM 最大的阶段（rg 39%，同上）。所以后端的杠杆是“少生成代码”（可达性、去重、冷代码），不是后端本身。

## 位置

- 后端紧跟在每个模块的最后一次优化之后，在同一个 worker 里，**并行度 = CGU 数与 jobserver token 的较小者**。ThinLocal 下它在 post-link 之后，所以也在 ThinLTO 屏障之后。
- `-Clto=fat` 时全部代码一个模块，后端单线程（实测：fat 只有 1 条 LTO 流水线，见 [opt-levels.md](opt-levels.md)）。
- 链接器随后丢弃 22% 的已编译代码（库 crate 31%，evidence.md §4）：这部分后端时间完全浪费。

## kfrc 的承接面

design 的 L 级到 placement 为止；以下由 kfrc 自写 x86 后端承接，逐项写明是否需要：

| LLVM 后端工作 | kfrc | 说明 |
|---|---|---|
| LSR、CodeGenPrepare 的地址模式 | L 级“强度削减与地址模式” | 已列 |
| DAG 指令选择、DAGCombine | Machine IR 选择 + L 级窥孔 | SoN 已做 GVN/窥孔，选择器可以是树模式匹配 |
| switch 降低（跳转表、位测试、查找表） | **尚缺** | design 只写“enum 按 tag 值分支”，见 [recommendations.md](recommendations.md) R4 |
| Machine LICM/CSE/sink | placement（GCM）+ GVN | SoN 的固有能力，不单独做 |
| Early if-conversion / cmov | L 级 if-conversion | 已列 |
| Greedy RA、合并、拆分 | 线性扫描（总览固定决策） | 质量差距需对拍，见 R5 |
| 指令调度（pre-RA MachineScheduler、post-RA） | 不做（总览固定决策） | 时间只占 codegen 2.9% |
| 栈着色、Shrink Wrapping、Prologue/Epilogue | **未写明** | value slot 也需要着色；见 R5 |
| 分支折叠、尾复制、块布局 | L 级“冷区域布局”只覆盖冷块 | 热路径布局见 R5 |
| x86 修补（false deps、vzeroupper） | **未写明** | 用到 AVX 时 vzeroupper 关乎性能 |
| 调试值位置 | **尚缺** | 值形式聚合 + GVN 后的变量位置，见 R6 |

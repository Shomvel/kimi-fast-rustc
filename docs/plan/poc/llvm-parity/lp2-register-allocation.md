# LP2 寄存器分配、Phi 消除与栈帧

## 现状

- 函数只要有 Load／Store／StackAddress 就走内存路径 `compile_memory_function`，几乎覆盖所有真实函数：
  - 可分配池只有 rbx、r12–r15（`regalloc/memory.rs`）；
  - 操作数先装入固定 scratch，运算后再写回；
  - 进入函数时 6 个参数寄存器全部存栈。
- 标量路径 `compile_virtual_function` 有 10 个寄存器，但真实函数基本用不上。
- 两条路径都是不拆分的线性扫描：一个虚拟寄存器在整个区间只有一个位置。
- Phi 在每条入边上先搬到临时寄存器、再搬到 Phi（两次 Mov）；`Undefined` 输入物化为 `mov 0`。
- `canonicalize` 让每个运算结果多一次 Mov。

结果：`from_str` 有 7,241 条 mov，其中 3,751 条访问栈帧。

## 目标（与 design 一致）

[goals/runtime.md](../../../design/goals/runtime.md) 和 [backend.md](../../../design/backend.md) 已定：

- 算法：SSA 线性扫描（Wimmer & Franz 2010）+ 区间拆分 + 远者优先 spill；
- 约束接口（Early/Late × Fixed/Reuse、并行 move）和分配后的符号 checker 参考 regalloc2，算法不照搬；
- 可分配 GPR 14 个，rax/rdx 改用固定区间。

本任务按这个设计交付：

1. **一条发射路径**：内存指令与标量指令共用一个分配器，删除 5 寄存器的内存路径。
2. **寄存器池**：14 个 GPR 全部可分配。
   - 调用 clobber 由 SysV caller-saved 集合给出；跨调用活跃的值优先放 callee-saved，不够时拆分。
   - 参数、返回值、`div` 的 rax/rdx、变量移位的 cl 用固定区间。
3. **拆分与 spill**：在调用、循环边界和使用点拆分；spill 放在定义点，reload 放在使用点前；冷区域（panic 路径）优先 spill。
4. **Phi 消除**：
   - 每条边生成一组并行拷贝，按依赖排序（成环时用一个临时寄存器打破）；
   - 来源与目标同一位置的拷贝不生成；
   - 来源为 `Undefined` 的不生成。
5. **合并（coalescing）**：Phi 与其输入、Mov 的两端，区间不冲突时分到同一位置。
6. **参数与帧**：
   - 参数寄存器只在确实被覆盖且之后还要用时才存栈；
   - callee-saved 只保存用到的；
   - spill 槽按区间着色复用。
7. **canonicalize**：窄整数的规范化只在需要高位干净时做，见 LP4 的宽度规则。

## LLVM 参考

LLVM 用的是 Greedy 分配器，这里只借它的拆分和合并思路，不改用 Greedy：

- [RegisterCoalescer.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegisterCoalescer.cpp)：合并判定；
- [SplitKit.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SplitKit.cpp)、[RegAllocGreedy.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegAllocGreedy.cpp)：按区域拆分、spill 代价；
- [PHIElimination.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/PHIElimination.cpp)、[MachineCopyPropagation.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/MachineCopyPropagation.cpp)；
- [StackColoring.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/StackColoring.cpp)、[ShrinkWrap.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/ShrinkWrap.cpp)：栈槽复用和保存/恢复下沉（ShrinkWrap 可以晚做）。

## 验收

- 符号 checker：每次分配后核对每个使用点读到的是该虚拟寄存器的最新值。所有测试和生成式对拍默认开启；正式计时关闭。
- 结构：给定小函数（无调用、一次调用、循环含调用），断言 spill 数和 mov 数的上限。
  “合法 enum 数据流增加冗余要求不增加指令”这类已有断言继续保留。
- 生成式：`crates/rustc_codegen_kfrc/tests/p0-scalar/generated` 加入高寄存器压力和多 Phi 环的程序。
- semver：报告 `from_str` 的 mov、以 rbp 为基址的 mov 和访存指令数。目标：机器访存数与 L 层 Load/Store 加真实 spill 同量级。


## candidate14 接续基线

LP1 后 `from_str` 仍有 7,129 条 mov（其中 rbp mov 3,733）、
显式内存读／写 1,902／2,171 条，而 L 层 Load／Store 仍仅 180／80。
2／16／64 变体的简单判别值 fixture 都是 30 条指令，其中 25 条 mov；LLVM 为 2 条。
下一轮从这份冻结候选开始，优先验证固定约束、调用 clobber 与 Phi 并行拷贝；
不能把增加寄存器数量但仍保留两条发射路径当作 LP2 完成。
原始计数见 [candidate14 报告](../../../../evidence/poc-llvm-parity-lp1-20261109/comparison.md)。

## candidate15：统一分配路径（2026-11-09）

生产 `lower_function` 已统一走 `lower/allocated`；旧的内存分配与发射路径已删除。
共享路径使用全部 14 个 GPR，记录 Early/Late、Fixed/Reuse、调用 clobber，
在压力和固定寄存器边界拆分区间，并对实际 spill 片段的保守 hull 着色复用槽。
Phi 入边改为并行拷贝，跳过 Undefined 与同位置拷贝；依赖成环时复用一个临时槽。
单定义 Copy alias 在活跃分析前消除，部分 dying-source Reuse 已合并。
参数使用入口并行拷贝，callee-saved 仅保存实际用到的寄存器。

符号 checker 默认开启，覆盖指令约束、活跃值保留、入口参数、Phi 并行拷贝和
CFG 边上的位置迁移。正式性能臂关闭，并单独核对开启／关闭生成的对象一致。
checker 验证分配与搬运；真实指令语义仍由 Linux E2E 和 LLVM/C 对拍验证。

冻结源码 SHA-256：`3f1d99a30f9c5a4460709ef81a692394165211c33e30a1420deff3a3d4b60d46`。
本机 585 passed、1 ignored；Linux 614 passed、1 ignored；压力三臂各 2,048 行，
包括真实清空 caller-saved 的 C 外调、24 值跨调用、16 值 Phi 旋转、除余／变量移位和栈参数。
另通过 semantic 466 行、semver 34 项、7 套旧语料、优化关闭／开启各 200×6 生成式对拍。
原始回执位于 `evidence/poc-llvm-parity-lp2-20261109/`。[同沙箱正式性能](../../../../evidence/poc-llvm-parity-lp2-20261109/candidate15-comparison.md)
未验收：text −6.1%，identifier runtime +72.9%，总编译指令 +102.2%；
`from_str` 的 rbp mov 从 3,733 增至 4,494。x86 阶段从 0.938 G 增至 5.972 G。
静态反汇编表明 `bytes_for_varint` 中大量无必要的临时 rax 保存／恢复，
分配器还让 spill 值逐条指令重新排队。后续候选修正两处策略与查询成本。

candidate16 的叶函数帧省略已通过本机与 Linux（615 passed、1 ignored）及完整差分；
candidate17 在生成式 optimize=0／seed5／f10 中被 checker 拒绝，未运行性能。
其触发点是 coalescing 把调用 pin 截断的片段末端当作源值死亡。
candidate18 加上真实 live-after 检查及六值跨调用回归后再验证，失败证据保留。

**LP2 尚未完整验收。** 仍需基于性能数据处理保守 hull 的空洞、循环边界与冷路径
spill 策略、更多 Phi 合并及编译开销。当前 parallel-cycle 使用临时槽而非空闲寄存器；
无栈叶函数帧省略作为后续候选独立验证。不得把以上正确性通过当作 LLVM 性能门槛达标。


## LLVM 21.1.8 源码研究

按用户要求由两个研究 subagent 分工完成，未启动独立 code review：

- [区间、拆分、循环与活跃分析](llvm-allocation-research.md)：多段 lifetime ranges、
  CFG 边位置迁移、分配成本预算、循环／冷路径的频率与定义身份约束。
- [指令约束、Phi 与临时寄存器](llvm-constraints-research.md)：use/def 子位置、
  真正死亡源的合并、Phi 虚拟 IncomingReg 与后续 coalescing、空闲 scratch 证明。

当前采纳顺序：先验证真实死亡修复及临时寄存器选择；再验证精确 lifetime holes，
并把活跃快照改成紧凑排序数组。相邻指令边的搬运可按位置变化事件构建，
避免每次遍历所有 live 值；这类仅减少编译工作的改动须核对对象字节不变。
循环外移 spill、冷边加权和跨定义重物化均需要额外身份／支配／频率证据，尚未实现。


## candidate18：修复回退后的测量

完整 Linux E2E（617 passed、1 ignored）及原 seed5 回归通过；新结构回归在
candidate17 独立副本上重现相同的分配重叠错误，candidate18 修复后通过。
[同沙箱报告](../../../../evidence/poc-llvm-parity-lp2-20261109/candidate18-comparison.md)
显示，相对 candidate14，text 461,920 → 311,451 B；parse 131.10 → 91.41 ms，
req 138.16 → 104.38 ms。identifier 123.95 → 137.94 ms 仍回退，编译总指令
4.928 G → 7.524 G 仍回退，LP2 未验收。

`from_str` mov 7,129 → 3,297，rbp mov 3,733 → 2,190，显式访存 4,073 → 2,530；
`bytes_for_varint` rbp mov 从 candidate15 的 880 降至 124，但仍高于基线的 17。
这些结果支持修正临时寄存器策略，不能证明保守区间及固定约束的剩余开销已经解决。

candidate19 开始采用精确 lifetime ranges，next-use 不穿越当前片段的 end，
片段重新进入时保留旧寄存器作为软偏好。互斥分支布局测试要求无 spill，
并执行两臂校验；初版因无必要的边拷贝环产生临时槽，加入软偏好后断言通过。
冻结 SHA-256 `5a32f0c80eb9dba787c9788bdcb7d85c115617df10ea8644e0178d2eb8ad52ab`，
本机 589 passed、1 ignored，E2B 验证中。

## candidate19／20：生命周期空洞与编译工作量

candidate19 完整正确性通过；[同沙箱报告](../../../../evidence/poc-llvm-parity-lp2-20261109/candidate19-comparison.md)
记录相对 LP1 的 text −23.7%，parse −24.7%，req −20.1%，identifier −14.8%；
三项 runtime 的 candidate/baseline 单侧 95% 上界均低于 1。
但它相对 candidate18 的 text、parse、req 有回退，identifier 改善，不能只保留获益项。

candidate20 将相邻 CFG 边搬运按片段边界事件缓存，块级活跃分析改为 predecessor
worklist；debug／release 两组各五套语义／压力／semver 对象均与 candidate19
逐字节相同，完整 E2E 通过。runtime 引用字节相同的 candidate19，没有重新采样。
总编译指令从 candidate19 的 7.076 G 降至 6.346 G（−10.31%），仍高于 LP1 约 28.8%。
严格 Body 后口径与 LLVM pass-family 对照仍缺，LP0／LP2 均未宣告完整验收。

下一候选由约束研究 subagent 负责落地 protected scratch：完整保护 live-through
和所有原始拷贝端点，优先借空闲寄存器，无空闲时保留独立槽保存／恢复后备。

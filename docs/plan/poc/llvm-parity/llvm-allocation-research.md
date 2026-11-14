# LP2：LLVM 21.1.8 的活跃区间与 spill 拆分

研究日期：2026-11-09。只读核对 `llvmorg-21.1.8` 源码；未改业务代码，未运行编译或压测。
此文区分 LLVM 的实际实现和适配 kfrc 线性扫描的建议，不把后者称为 LLVM 算法。

最值得先采用的是**保留 lifetime holes，以及按新增搬运的执行频率选择拆分边界**。
LLVM 的 Greedy、区域搜索与 spill placement 不宜整套移植；当前编译工作量已经回退，
应先消除不真实的区间干涉，再增加有明确收益的策略。

## 1. LLVM 实际做什么

### 多个 segment，而非一个最早定义到最晚使用的 hull

[`LiveInterval.h:9`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/CodeGen/LiveInterval.h#L9)
明确举例：一个 live range 可以是 `[1,20), [50,65), [1000,1001)`。
[`LiveRange::Segment`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/CodeGen/LiveInterval.h#L153)
记录有序半开区间与 `VNInfo`。`VNInfo` 区分同一虚拟寄存器中的不同定义及 CFG 合流值；
这不只是将一对 start/end 改成数组。
[`liveAt`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/CodeGen/LiveInterval.h#L403)
先找 segment，再确认查询位置不在洞里。`endIndex()` 仍可返回全体 segment 最后端点，
但它不能替代干涉查询。

[`SplitAnalysis::analyzeUses`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SplitKit.cpp#L167)
从 def/use 收集使用位置，排除 undef 使用，对同一指令去重。
[`calcLiveBlockInfo`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SplitKit.cpp#L199)
区分有真实使用的块、只穿过而不使用的块，以及有 lifetime gap 的块。
同一块存在 gap 时，建立独立的 live-in 与 live-out 描述，而非把 gap 填满。

LLVM 还有指令内位置：
[`SlotIndexes.h:67`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/CodeGen/SlotIndexes.h#L67)
区分 block、early-clobber、register 和 dead slot。
普通最后一次 use、early-clobber 和新 def 的干涉关系不能仅凭“同一条指令”判断。

### 拆分按块与边界约束决策，搬运成本按频率计

[`RAGreedy::trySplit`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegAllocGreedy.cpp#L1934)
对单块区间先尝试 local split，然后 instruction split；多块区间先尝试 region split，
进展不足时退到 block split。它并非见到每个 call 或 block 边界就无条件切开。

[`addSplitConstraints`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegAllocGreedy.cpp#L735)
为块入口与出口记录 `PrefReg`、`PrefSpill`、`MustSpill` 等约束，
检查拆分点是否合法，并把新增 spill 指令数乘上块执行频率累计。
[`calcGlobalSplitCost`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegAllocGreedy.cpp#L998)
还计算区域边界的 register/stack 状态变化：同一块内 spill 后 reload 是两次成本。

[`SpillPlacement.cpp:9`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SpillPlacement.cpp#L9)
在 CFG 的 edge bundles 上求 register/stack 偏好，以块频率加权约束和连接，
目标是降低预期 spill 代码执行次数；源码明确其收敛保证是局部最小。
没有使用的 through block 可以允许值保持在寄存器或栈上，避免无意义切换。

### 冷路径来自频率；循环状态受更高保护

[`LiveIntervals::getSpillWeight`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveIntervals.cpp#L908)
在速度模式下用 `(isDef + isUse) × block_frequency_relative_to_entry`；
按体积优化时只考虑静态读写成本。
[`weightCalcHelper`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/CalcSpillWeights.cpp#L266)
对 loop-exiting block 中定义且 live-out 的疑似归纳变量更新再乘 3，
并在末尾按区间大小归一化。可重物化的值降低权重。
这些是在已知 CFG/频率上的具体成本，不等同于“名字含 panic 的变量优先 spill”。

[`SplitEditor::findShallowDominator`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SplitKit.cpp#L940)
在原定义支配的范围内找更浅的循环层级；
[`hoistCopies`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SplitKit.cpp#L1049)
只合并同一 parent value 的 back-copy，利用共同支配点，并在速度模式下拒绝
把多条冷 copy 上提到执行次数更高的热支配块。
因此“把 spill 一律搬到定义后”不是 LLVM 的通用规则。

### 拆分必须有进展界限

[`tryLocalSplit:1763`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegAllocGreedy.cpp#L1763)
说明新增 copy 会使一次合法拆分的总指令数反而上升，不能简单要求每轮总数减少；
其 stage 机制只允许有限次无严格缩短的拆分，之后要求子区间包含更少指令。
[`GrowRegionComplexityBudget`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/RegAllocGreedy.cpp#L116)
还显式限制 region 搜索的工作量。
kfrc 当前按 start 单向推进的拆分较简单，应保留这种可解释的终止与复杂度保证。

## 2. 适配当前 kfrc 的最小方案

以下是对 `analysis/liveness.rs`、`lower/allocated/{scan,allocation,emit,checker}.rs`
的适配设计。它没有照搬 LLVM Greedy，也不声称已经完成完整 Wimmer active/inactive 算法。

### 第一步：精确片段，保留现有 CFG 边搬运

当前已经有逐指令 `before`、`after` 以及 `definitions`。先在指令粒度构造：

```text
occupied(v, p) = v ∈ before[p] ∪ after[p] ∪ definitions[p]
range(v) = occupied 为真的最大连续半开片段集合
```

- 必须包含 dead definition：发射器仍会查询它在定义指令处的 home。
- `after[p]` 必须是所有实际 successor 的 `before` 的并集，包含 backward edge。
  单靠线性文本的前一次定义和下一次 use 不能恢复循环活跃性。
- 每个片段可以独立以 `(start, vreg)` 放进现有 pending 队列。片段之间的洞不占寄存器，
  不插入跨洞“线性 reload”。这先获得真实干涉的收益。
- 所有实际 CFG 边继续对 live-in 值执行 `home(v, source) → home(v, target)`。
  即使目标的文本位置相距很远或是回边，也必须做。已有 taken-edge trampoline 与
  fallthrough copy 可继续承接；不能只在相邻指令 home 改变时搬运。
- 每个 vreg 的最终片段保持有序、不重叠；`home` 的二分查询必须拒绝洞内查询。
- next-use 优先级仅看该 active/current 片段 `end` 之前的 use。
  不能穿过 lifetime hole，用另一个 CFG 分量中的 use 制造虚假优先级。
- 本轮保留每 vreg 的 spill-fragment hull 槽着色，暂不同时改变栈槽生命周期规则。
  它可能多占槽，但能把寄存器洞和槽洞的正确性风险分开。

这比保留一个 hull 更准确，但与完整 Wimmer 仍有区别：独立片段可能换寄存器，
因此在 CFG 边增加 copy。后续再引入 inactive 集合，保留跨洞寄存器偏好，并用
“与未来片段的第一次真实交点”限制 `free_until`，以降低边搬运。
不应仅把 inactive 的最终 end 当作整个洞期间都被占用。

第一步可继续使用现在的指令内 Early/Late/Reuse 约束。
`before ∪ after ∪ defs` 在同一指令内仍保守；dying source 与新 output 的同位复用
仍需依赖明确的 Reuse 和 checker，不能因为两个 vreg 同时覆盖 p 就批量消除干涉。

### 第二步：只在真实 spill 热点上增加块／循环成本

先记录每类新增机器搬运来源：真实 L Load/Store、spill/reload、固定约束保存、
Phi/边拷贝、并行环临时、callee-save。总 mov 或 rbp mov 不能单独指明 spill 策略是否变好。

在已有 CFG 上计算 dominator 与自然循环，保留循环深度及入口／出口边。
自然循环回边必须由“目标支配来源”判定，不能把所有文本向后跳转当作循环；
不可约 CFG 保守保留通用边搬运。

对确实需要 spill 的区间，比较少量合法候选边界：当前位置、当前块使用区间边界、
受原定义支配的循环外入口位置。成本按新增 load/store/copy 的执行次数估计，
并优先保留循环中重复使用的值。缺少实测频率时可以使用明确记录的静态 loop/cold
估计；不要把这种估计写成已测频率，也不要直接复制 LLVM 的常数 3 充当已验证阈值。

LLVM 的冷路径收益依赖块频率，不要求照搬其 Hopfield 求解器。
针对明确 cold/noreturn 属性的边，在冷侧保留 spill、在热侧保留寄存器，是可单独验证的
小范围策略；仅凭符号字符串推断 cold 不够可靠。

### 第三步：上提存栈与重物化前，先确定值的身份

当前 Phi 被消解为多处对同一 vreg 的定义，不能假设 `vreg == 一个永不改变的 SSA 值`。
把 spill store 从某个分支上提到共享支配点，必须证明它保存的是同一次定义，
且支配所有对应 reload；loop-carried Phi 每次迭代更新时也必须更新栈副本。
在没有 LLVM `VNInfo` 等价信息前，保留边上的搬运和局部存栈，比跨定义上提更稳妥。

纯常数可在使用处重物化；普通 load 不可因“地址相同”就重物化，别名写入可能改变结果。
地址表达式也需确认其依赖值跨调用、循环和别名写入的稳定性。

## 3. 验证与采纳标准

| fixture | 要证明的事实 |
|---|---|
| diamond，两臂各自短生命周期且寄存器需求不超过 14 | 互斥路径不因文本 hull 而 spill；两臂都执行对拍 |
| 多条跳过中间块的边，前后 home 不同 | 生命周期洞不丢值，CFG 边搬运正确；布局置换输出不变 |
| self-loop 与两个 backedge，循环 Phi 旋转 | 固定点活跃性覆盖回边；入口／回边并行 copy 不串行覆盖 |
| 冷边含 caller-saved 全清空外调，热边为循环 | 热边保存策略不被冷外调污染；冷边也保持正确 |
| 同一 vreg 在不同入边重定义，之后跨调用 | 栈副本对应当前定义，不能复用前一条路径／前一轮迭代的值 |
| 在最后 use 同指令产生 Reuse output；div/call 固定 output | 指令内时序保持正确，不能用普通位置区间替代固定约束 |
| 稀疏 use 的长压力区间 | 分配事件随真实 use/约束数增长，不随每条中间无关指令重新排队 |

沿用默认开启的 allocation checker，再跑现有三臂压力、LLVM/C 参考、semantic、
MaybeUninit/快照、semver、生成式测试。正确性通过后，同沙箱对照 candidate14 和新候选：
报告编译阶段指令、动态 runtime、`.text`、上述搬运来源，以及
`from_str`／`parse::identifier`／`bytes_for_varint` 的机器计数。
生命周期洞改善不保证 runtime 获益，新增 edge copy 也必须计入，不能只报告减少的 interval hull 长度。

## 4. 证据边界

本研究实际下载并阅读了固定 tag 的 `LiveInterval.h`、`LiveIntervals.cpp`、
`SplitKit.{h,cpp}`、`RegAllocGreedy.cpp`、`CalcSpillWeights.{h,cpp}`、
`SpillPlacement.cpp` 与 `SlotIndexes.h`。核心文件 SHA-256：

- `LiveInterval.h`：`090539edeca814701ef6440edf74e24c4b967b2aadc7740ee98878c7ba4a9a9c`
- `SplitKit.cpp`：`a2f10c4adbbdffe5d22b54dd79edac37942225d6e68b69e0d268e876813d6dcc`
- `RegAllocGreedy.cpp`：`c1905eae0464f4f71f95122d34e2790e8209e5ee15c357548b363a4c3107dcf8`

candidate15 的回退数值由主任务既有 E2B 回执提供；本研究没有重新采样。
本报告不包含新的性能结论，不构成对当前候选的 code review 或验收。

## 5. 编译开销：先换数据访问，再换分配策略

主任务补充：candidate18 正在修复“被 call pin 截断的片段，被合并逻辑误认为真实死亡”。
该正确性修复应独立完成；精确 ranges 不与它混在同一冻结候选。

补查 LLVM 的实际 liveness 构建后，更适合当前下一步的顺序如下。

| 优先级 | 最小改动 | 输出要求 |
|---|---|---|
| 0 | 完成 call-pin 与真实死亡的区分，开启 checker 跑 E2E | 正确性全部通过 |
| 1 | 普通相邻位置的 edge reconciliation 只处理 home 真正改变的值；缓存每条实际 CFG 边的 copy 清单 | 同一源码输入生成对象字节不变 |
| 2 | 块级 liveness 改 predecessor worklist；将重复使用的 facts／live-through 数据只计算一次 | 对象字节不变，逐位置集合与旧版相等 |
| 3 | 去掉每条指令两份 HashSet 克隆，改块内反向扫描产生生命周期端点／紧凑快照 | 对象字节不变，包含 loop、redef、dead def |
| 4 | 在以上费用减下来后，接第 2 节的 lifetime-hole 分配 | 对象允许变化，E2E＋性能复验 |
| 5 | 根据 spill 来源和热点，增加循环／冷边拆分成本 | 有可测 runtime／搬运收益才采纳 |

### LLVM 的构建数据流

[`LiveIntervalCalc::calculate`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveIntervalCalc.cpp#L41)
先为每个定义建立最小 segment，再沿实际 use 延伸。
[`extendToUses`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveIntervalCalc.cpp#L134)
遍历寄存器的 operand use-def 链；Phi operand 的 use 放在对应 predecessor 末尾，
而不是 Phi 所在块内。kfrc 已转换成入边并行 copy，不必重新造一套 LLVM Phi 表示。

[`LiveRangeCalc::extend`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveRangeCalc.cpp#L86)
先尝试同块延伸；成功即返回。
[`findReachingDefs`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/LiveRangeCalc.cpp#L190)
只对需要的 predecessor 做 BFS，用 Seen bit vector 和 block-indexed live-out cache
避免重复遍历。找到唯一 reaching value 时批量加入块 segment，只有多 reaching value
才进入 SSA 更新。其
[`LiveRangeCalc.h:53`](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/include/llvm/CodeGen/LiveRangeCalc.h#L53)
明确说明 Seen 独立存为 bit vector 是为了换 range 时快速清空。
这条路径不需要为每条机器指令保存完整 live-in/live-out HashSet 副本。

### 当前代码可直接采用的访问优化

**边搬运。** 当前 `edge(source,target)` 先收集、排序 target 的完整 live 集合，再对每值
二分查询两个 home，即使一个 home 都没变。对 `target == source + 1`，可以预计算
在该边界结束且下段位置不同的 segment 变化事件；只检查这些 vreg 是否 live-in target，
生成同样的拷贝。无变化事件的边直接返回空。非相邻跳转和回边仍使用完整目标 live-in
集合，一次计算后缓存。不能简单省掉所有 fallthrough copy：压力拆分、call clobber
及同一指令后的重新分配都可能令它承担真实迁移。taken branch 的“是否为空”判断与
实际发射应复用同一 copy 清单，避免两次完整扫描。

**块 worklist。** 保留现有数据流方程，先预计算 predecessor 列表。只有某块 live-in
改变时，才把 predecessor 加进队列；位图记录已排队状态。每次重新计算 out 和 in
使用可复用缓冲区。相比全块反向扫描直到不变，这属于计算调度优化；它不是直接复制
LLVM 的 per-vreg reaching-definition 算法。队列顺序固定，结果按原来的 vreg 顺序
输出，避免机器码变化来自容器迭代顺序。

**逐指令数据。** 最低风险过渡是保留块边界 live-in/out，而每个块只维护一个反向扫描
的 live set；每步按 `uses ∪ (after − defs)` 更新。用每个 vreg 的开启／关闭事件输出
ranges，可避免 `2 × 指令数 × 活跃值数` 个 HashSet entry。
如果发射器暂时仍需要逐位置 live-through，可先用 dense bitset 或排序 vreg 向量代替
HashSet 快照，并只计算一次 `before ∩ after − definitions` 供 registers、emitter 和
checker 共用；但该过渡仍有逐位置存储成本，不能宣称已经变为稀疏分析。

更彻底的方案是为 allocated segments 建 start/end 事件，按顺序发射时维护当前物理
寄存器占用表；约束临时寄存器选择只查询 14 个 GPR 的活跃 occupant，而非遍历全部活跃
vreg。checker 保持单独的完整符号验证路径。该改动应在边搬运和块 worklist 的收益确认后
再做，尤其要区分同位置旧值、新值及 Reuse alias，避免复发 call-pin 的时序错误。

以上 1–3 项先要求 `object_bytes_unchanged`，再测 x86 阶段编译指令；不要把存储结构
变化与新的分配策略同时提交为一个性能样本。LLVM 的分段表示和 use-def 驱动构建提供
设计依据，但收益仍需本仓库的冻结候选测量。

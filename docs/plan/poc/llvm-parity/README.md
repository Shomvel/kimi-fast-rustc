# 追平 LLVM：交接与任务（LP0–LP9）

最新已验收实现与基准结果：[2026-11-10 E2B acceptance](results/20261110/README.md)。体积、运行和 3× 编译成本目标仍未全部达到；具体支持缺口见报告。

基于上述验收的 [LLVM 差距分析](llvm-gap-20261110/report.md) 与 [具体优化计划](llvm-gap-20261110/optimization-plan.md)。新增 [LLVM 成本模型与有限求解设计](llvm-gap-20261110/cost-model.md)。

日期：2026-11-09。起点：main `37cea66`（PR #3，语义 IR 成为唯一生产链，
ObserveValue／Refine 已按 [requirements-migration](../requirements-migration.md) 拆分）。
六项门槛不变，见 [metrics.md](../metrics.md)；本目录只写下一阶段做什么、按什么顺序、怎么验收。

## 起点：candidate13 交接摘要

- 正确性：candidate13 通过 Linux 608 项、semver 34 项原版测试、7 套完整语料、
  200 个生成函数 × 6 组输入的 LLVM 对拍。
- 性能：离门槛很远，数字和出处见 [evidence.md](evidence.md)。`.text` 约 53×，
  runtime 15–160×，编译指令是 LLVM 的 3.34×（门槛是少 3×）。
- Observe／Refine 重设计在结构上达标（冗余要求不增加机器指令），
  但 `from_str` 的分支 150、Load/Store 180/80 没变，**剩余差距不在语义要求上**。
- 证据显示的三个主因：
  1. 判别值不连续的直接 tag 枚举（`core::ptr::Alignment`）走 O(n) select 链；受影响的 23 个 alloc/Layout 符号占 `.text` 的 45.6%（LP1）；
  2. 带内存操作的函数只用 5 个寄存器，`from_str` 的 82% 指令是 mov（LP2）；
  3. S→L 后每个聚合叶子在每个合流点各建一个 Phi（LP3）。
- 在这三项之外，指令选择、循环和块布局都还没开始，这些也是 runtime ≤ 1.10 的前提（LP4–LP7）。

## LP1 里程碑：candidate14（2026-11-09）

LP1 直接 tag 数据流已落地并通过本机、Linux 与三臂差分。
同沙箱测得 `.text` 缩小 53.0%，req 中位耗时降低 60.3%，编译总指令数降低 11.4%。
[更新后的证据与边界](evidence.md#candidate14lp1-直接-tag-数据流) 保留原始回执与完整表格。

LP1 的 O(n) 映射已从普通判别值、Phi、复制与分支路径移除；
2／16／64 变体 fixture 均为 30 条机器指令，≤ 4 条目标仍未达。
显式动态序号查询的查表降低留给 LP5；LP0 的阶段／热点测量已有数据，严格 Body 后口径仍缺。

## 当前进展：转入 LP3（2026-11-09）

统一 14 GPR 分配、精确生命周期空洞、约束时序、并行拷贝、spill 槽复用和叶函数
帧省略已落地。candidate20 通过本机 591 项、Linux 620 项测试及完整差分，
各 1 项按原配置忽略；debug／release 各五组对象与 candidate19 逐字节一致。

相对 LP1，candidate19 的 text 减少 23.7%，parse／req／identifier 中位耗时分别
减少 24.7%／20.1%／14.8%。candidate20 保持这些对象字节，编译总指令较 candidate19
降低 10.31%，但仍比 LP1 高约 28.8%。[证据与回退记录](evidence.md#lp2-迭代证据candidate1520)
保留所有失败候选，不把中间获益外推为 LLVM 门槛达标。

LP2 已收束到 candidate22 作为 LP3 对照：三项 runtime 均优于 LP1，编译指令回退
及循环／冷路径等缺口仍保留，未宣告完整验收。用户明确要求转向 Rust 语义机会更大的 LP3。

candidate23 已实现字段需求 worklist、按需叶子 Phi／读取、内联 Effect Phi 的变体穿线修复，
以及连续同类型同地址读取的 owned 快照复用。本机 610 passed、1 ignored，E2B 全套正确性通过。
相对 candidate22，`.text` 减少 3.39%、编译总指令减少 3.70%；parse 与 identifier
中位耗时分别上升 3.77%、4.28%，尚无运行时收益结论。`from_str` L Phi 从 695 降至 571。
未知消费、完整返回及 MaybeUninit 保守处理。candidate24 私有槽通过完整正确性，
但逐边复制引入 28 次 memmove，运行时间回退，不接受为性能优化。
candidate25 已冻结纯分支快照转发；candidate26／27 消除完整传递链逐边复制与返回中间槽。
candidate28 保留只搬运的调用结果子对象表示，完整正确性通过，`.text` 和编译总指令
相对固定 22 分别减少约 16.6%／16.3%；req 改善，但 identifier 仍回退，parse 尚无稳定
收益结论。下一阶段按用户指定的 [RS3 计划与分工](../rust-semantics/rs3-execution.md) 并行实施。
详见 [LP3 实施方向](lp3-layout-phi.md) 与 [Rust 语义机会](rust-semantic-opportunities.md)。
具体泛型类型的 [inhabited 分支研究](llvm-inhabited-research.md) 也已核对实际图：
当前 `Result<Infallible, …>` 没有直接 tag query/switch 消费者，暂不据此新增 pass。

用户要求的 LLVM 21.1.8 源码研究由 subagent 分担，另有
[区间与活跃分析](llvm-allocation-research.md)、[约束、Phi 与 scratch](llvm-constraints-research.md)。
研究 subagent 也负责落地相邻边搬运缓存与 protected scratch；正式编译／测评由验证
subagent 在同一任务 E2B 沙箱串行执行。

## 任务

| ID | 文件 | 内容 | 依赖 | 主要影响 |
|---|---|---|---|---|
| LP0 | [lp0-measurement.md](lp0-measurement.md) | runtime 热点、分阶段编译指令、按 metrics 边界计数 | — | 所有任务的归因 |
| LP1 | [lp1-enum-tag-domain.md](lp1-enum-tag-domain.md) | 直接 tag 枚举在 tag 值域上运算 | LP0（可并行） | `.text`、runtime |
| LP2 | [lp2-register-allocation.md](lp2-register-allocation.md) | 统一分配器、拆分、Phi 并行拷贝、帧与参数 | — | mov、栈访问 |
| LP3 | [lp3-layout-phi.md](lp3-layout-phi.md) | 按需建叶子 Phi、活跃变体合并、大值留槽 | LP0 | L 节点、mov |
| LP4 | [lp4-operands-and-flags.md](lp4-operands-and-flags.md) | 地址模式、LEA、立即数/内存操作数、cmp+jcc、窄宽度 | LP2 的指令接口 | 指令数、体积 |
| LP5 | [lp5-select-and-switch.md](lp5-select-and-switch.md) | `Select` 节点与 cmov、switch 降低 | LP1、LP4 | 分支、体积 |
| LP6 | [lp6-loops.md](lp6-loops.md) | 循环分析、rotate、LICM、归纳变量/LSR、边界检查 | LP4 | 热循环 runtime |
| LP7 | [lp7-block-layout-peephole.md](lp7-block-layout-peephole.md) | 块布局、冷路径、分支折叠、拷贝传播 | LP2 | 跳转、体积 |
| LP8 | [lp8-compile-time.md](lp8-compile-time.md) | worklist 驱动、verify 开关、少编译死函数 | LP0 | 主张 2 |
| LP9 | [lp9-acceptance.md](lp9-acceptance.md) | 每项的结构验收、差分、semver 指标、LLVM 形状对照 | — | 全部 |

建议顺序：LP0 → LP1 → LP2 → LP3 → LP4 → LP5 → LP6/LP7 → LP8。

- LP1 改动只在 `lower_layout/variants*.rs`，收益最确定，应最先落地。
- LP2 是最大的一块，可与 LP1 并行，单列 owner。
- LP4–LP7 依赖 LP2 的新指令接口，先把接口定下来再分工。

## 与现有工作包的对应

[optimization-modules.md](../optimization-modules.md) 的 owner 表继续有效：

- LP1、LP3、LP5 中属于 L 级的部分 → D；
- LP2 → I；
- LP4–LP7 的机器部分 → D（`x86/lower/`）与 I；
- LP6 的 SoN 循环分析 → A（analysis）+ G；
- LP0、LP9 → F；
- LP8 → 整合者 + G。

同一文件只有一个 owner。

## 红线（集中列出）

1. 门槛冻结，不在看到数据后放宽；未达项如实报告归因。
2. 正式计时在 E2B，同一沙箱内 LLVM 与候选串行对照；profile 与正式计时分开。冻结候选为准，不把未冻结改动混进旧结果。
3. 动态验证首次暴露一类错误时，停下重复运行，静态排查同类实例，批量补回归并修复后再恢复。
4. 不把删掉的节点数当作性能收益；收益以机器码、编译指令、runtime 实测为准。
5. 不引入未定义行为：
   - cmov／投机执行只用于不会 trap、不会读非法地址的值；
   - LICM 外提可能 trap 的操作必须有守卫；
   - Rust 没有 `mustprogress`，删除循环需要终止性证明。
6. 默认不开独立 code review，用真实 E2E 和差分发现问题。
7. LLVM 只作参考，不照搬 pass 组合。参考版本固定为 LLVM 21.1.8（与 rustc 1.94.1 一致）。

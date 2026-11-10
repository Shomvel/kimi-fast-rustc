# 编译预算与50%目标

这次审阅以产物差距为入口，但不能以无界增加编译工作换取运行收益。transport相对本轮baseline的四库wall几何平均为1.02095，即增加2.09%；semver/base64/Unicode分别增加10.8%/11.9%/5.1%，humantime减少16.6%。这组数据没有达到编译时间减半。

## 先固定分母

本轮baseline是bc718cc，已经包含前几轮并行等优化。旧`../compile50/summary.json`对更早后端测得并行候选wall几何平均0.7830，属于另一组后端hash、实验时间与比较分母。不能把两个比值连乘当作本轮端到端实测，也不能把旧semver backend单阶段约52%的下降当作四库完整rustc调用减半。

后续保留两条曲线：相对固定的用户目标基线；相对上一候选的回归。若原目标基线不能通过source/backend hash重建，则先重建再报告进度，不能不断改分母。

## 复用已有观测能力

现有`crates/rustc_codegen_kfrc/src/profile.rs:17`已经提供按stage/per-function计时与Linux user instructions，`lib.rs:194`有`prepare_wrappers_s`窗口，`codegen.rs:188`起有inline、post-inline、layout、x86等阶段。不是从零再做一套计时框架。

新增观测只补当前不可归因部分：wrapper候选数、clone节点/字节、成功与失败原因、缓存命中率；layout cleanup每轮改变的节点/边、KnownBits查询数/预算耗尽数、private-memory搜索长度；CFG/支配关系重建次数；x86 lowering、schedule、regalloc、encode各自开销。记录总工作与关键路径，不把并行各线程elapsed相加当作wall；嵌套perf窗口不能重复累加。

旧`../compile50/compile50-profile-summary.json`曾显示humantime parse_unit的x86窗口占较大比例，但产物已由23799B降至5094B，这份旧profile只能提示下一次采样入口，不能当作当前优化的成本归因。

## 执行项

| 编号 | 实现入口 | 具体工作 | 验收 |
|---|---|---|---|
| C0 | profile.rs、lib.rs、codegen.rs | 对baseline/transport/ranges用同一阶段口径重新采样；补wrapper和L清理细分；正式benchmark关闭详细观测 | 分阶段CPU工作可解释回退，观测开销独立测量，样本附后端/源码/对象hash |
| C1 | codegen.rs:62 `simplify_wrappers` | 先用廉价摘要筛候选，再clone；缓存简化body与特化结果；以callee依赖变化决定第二轮候选 | 无命中的函数不重复clone，缓存正确失效；候选+失败路径受fuel限制；减少编译指令而保持产物语义 |
| C2 | lower_layout/cleanup.rs:48 | 每个pass报告changed集合，未改变不再做下一轮；让局部seam触发相关清理，减少全图scan/compact | 在图/CFG变化时失效旧事实；循环与Phi仍有查询上限；不能用跳过必需清理换编译收益 |
| C3 | 图分析与后端阶段，按C0决定 | 缓存CFG/支配/循环分析直到相关变更；对热点函数减少中间复制与全图重建 | 相同输入确定性一致；缓存开关下语义/对象对照；记录分析复用率 |
| C4 | parallel.rs、lib.rs任务调度 | 只有在C0显示关键路径仍受可并行任务限制时调整粒度/排序 | 1/2/4/8线程对比wall、CPU、RSS；小crate不因线程开销回退；保持对象确定性 |

不能把“缓存”作为无条件改进：摘要必须包含graph revision、目标/布局、有效权限、常量参数、CGU可用符号集合；出现地址身份、私有符号或数据对象冲突时重新验证。特别是跨CGU inline，命中旧摘要不意味着该callee在新的目标CGU可链接。

## 默认启用门槛（建议）

1. 先将semver/base64/Unicode新增编译指令成本恢复到本轮baseline的±2%以内，保留humantime收益。这是短期预算，不是已完成结果。
2. 新pass单项默认不得使任何crate的完整编译wall稳定回退超过2%。确有运行收益但超过门槛的候选单独保留，展示收益/成本后再决定策略；不要混进默认pipeline掩盖回退。
3. 配对21轮，串行运行各arm，固定CPU/线程/优化级别/缓存口径；报告wall、user+sys、RSS、instructions、对象hash。边界结果在新一轮实验复核，不将一次bootstrap视为跨机器保证。
4. 50%目标必须检查完整构建的阶段占比。若可优化部分占比为f、该部分加速s倍，总比值为`(1-f)+f/s`；当f≤0.5时，仅优化该部分无法达到0.5。先测前端/MIR转换/后端/链接分解，再决定是否扩大优化范围。

运行性能与编译耗时分别设门槛。此次不引入solver，也不先建立新的通用cost model；先凭具体产物模式、已知合法性和单项ablation决定哪些LLVM机制值得移植。

# 四库产物系统审查

审查对象按用户确认是 LLVM。按 semver → base64 → unicode-segmentation → humantime 顺序，由专项 subagent 完成产物盘点、热点深读与源码追溯。此次交付为改进计划，不代表已实施计划中的所有优化。

四份专项报告与统一计划均已完成。接手复核已重现全部统计，并完成完整证据归档的哈希核验；详细范围见[证据说明](evidence.md)。下一项实施工作是计划W0/H1：固定ranges性能基线、隔离timestamp格式化回退，再按独立候选推进共用优化。

后续集中实现与新的E2B验收记录见[focused-opt](../focused-opt/README.md)。本目录保持诊断时的候选和测量口径；后续结果使用独立冻结的start基线，不与旧实验绝对时间混合。

## 版本与证据边界

- `baseline`：提交 `bc718cc` 的 KFRC 后端，已经包含前几轮算术和并行编译改动。
- `transport`：本轮候选；有 21 轮交替运行、4 倍工作量，以及 21 对完整编译测量。
- `ranges`：在 transport 上扩展 KnownBits 的 Phi 与 `(x + c) - x` 规则；四库语料/checksum 和 Linux workspace 已通过，尚无等价的运行/编译性能复测。
- 本地工作树比 ranges 只多一项 KnownBits 循环负例测试（以及验证脚本、证据报告）。以上描述对应诊断时快照；后续已将接手基线保存为本地提交 `abc59a0`，集中优化保存为 `add2206`，尚未推送。
- LLVM：Rust 1.94.1 携带的 LLVM 21.1.8；共用 MIR0 sysroot。库 O3/CGU16，公共驱动由 LLVM 构建。它不是“所有执行指令都由 KFRC 生成”的对照。

每份报告区分全量自动盘点与人工深入检查。使用 ELF 符号长度界定函数，排除对齐填充与重定位记录；同名函数的内联边界可能不同，因此不以单个函数指令数直接推断运行时间。调用次数是静态 call site 数，动态指令来自独立 perf 运行。

`inputs.json` 保留基线、候选补丁哈希和范围说明。`compile-transport.json` 为完整编译原始样本；`selected-transport.json` 为已选函数的精确符号范围。该旧统计中的 `niche_constants` 匹配方式不适用于实际十六进制打印，不作为分析证据。

## 阅读路径

1. [semver 专项](semver.md)
2. [base64 专项](base64.md)
3. [Unicode 专项](unicode-segmentation.md)
4. [humantime 专项](humantime.md)
5. [统一改进计划](plan.md)
6. [统一数据与ELF字节账本](evidence.md)
7. [编译预算与50%目标](compile-budget.md)

## 验收原则

每项计划必须有具体产物模式、可定位的实现入口、可证伪的因果假说与可执行验收。实现时保留单项开关或独立候选，测量真实调用闭包；不把总库静态体积、运行动态指令、运行时间和编译时间混为一项指标。已观察到的编译回退必须列入预算，不得仅报告热点运行加速。

# LP0 测量：先确认热点，再动手

## 要回答的问题

1. runtime：parse／req／identifier 的时间花在哪些函数上？alloc 路径（`finish_grow`、
   `Layout`、`Alignment`）占多少？
2. 编译：5.56 G 条指令在 build／optimize_s／inline_s／layout／verify／x86 各占多少？
   c12→c13 的 +10.8% 来自哪一段？
3. 主张 2 的口径：metrics.md 规定 kfrc 从 Body 转换器出口开始计。现在的总窗口含
   collector 和 MIR 转换，要另出一个符合口径的数字。

## 做法

- **runtime profile**：在 E2B 沙箱里对 kfrc 臂和 LLVM 臂的 semver 负载分别做
  `perf record -e instructions:u`（perf 在 E2B 可用），只做 profile，不与正式计时混跑。
  输出按符号聚合的表，以及 kfrc 前 20 个热点函数的 perf annotate。
- **分阶段编译指令**：`crates/rustc_codegen_kfrc/src/profile.rs` 已有 `KFRC_PROFILE`
  窗口（build、verify_s_input、optimize_s、identity_s、inline_s、post_inline_*、
  verify_s_output、layout、verify_l、x86）。近期候选只设了 `KFRC_PROFILE_TOTAL`，
  需要恢复逐窗口采集，按阶段、按函数汇总；同时报告这些窗口之和占总窗口的比例，
  差额即 collector、MIR 转换和 object 写出。
- **LLVM 对照分项**：沿用 [llvm-mir/measurements.md](../../../research/llvm-mir/measurements.md)
  的族划分（指令选择、寄存器分配、机器级优化、InstCombine 类等），让每项 kfrc
  阶段有一个可比的 LLVM 数字（研究建议 R12）。
- **机器形状统计**：`scripts/poc-validation/machine-attribution.py` 已能按符号统计
  mov、跳转、访存。增加两项：每个函数的 spill 槽数、Phi 搬运产生的 mov 数
  （在发射时打标签统计，不靠反汇编猜）。

## 产出

- `evidence/<候选>/lp0/`：perf 原始数据、符号聚合表、阶段分项 JSON、生成表格的脚本。
  表格由脚本从原始数据生成，不手抄数字。
- 在 [evidence.md](evidence.md) 末尾追加一节“LP0 结论”，写明哪些推断被证实或否定，
  并据此调整 LP1–LP8 的顺序。

## 完成标准

- kfrc 前 20 个 runtime 热点都有归属（属于 LP1–LP7 中的哪一项，或另列新项）。
- 阶段分项之和与总窗口的差额 < 5%，或差额已逐项说明。
- 出一个符合 metrics.md 口径的主张 2 比值，旧口径数字并列保留。

# 验收

本文规定每个步骤的证据包与 issue #5 的关闭条件。六项 PoC 门槛与测量协议以
[metrics](../metrics.md) 为准；阶段出口的通用证据要求以 [plan/README](../../README.md#验收) 为准。

## 每个步骤的证据包

| 类别 | 内容 |
|---|---|
| 身份 | 实现 PR；源码归档 SHA-256；rustc/kfrc 工具链版本；输入（crate 版本、lockfile、flags、sysroot）哈希；E2B 沙箱指纹 |
| 正确性 | 解释器支持范围内的 S→L→原生差分；对定义良好输入与 rustc/LLVM 三臂对拍（LLVM、kfrc 优化关、kfrc 优化开）；生成式差分；本步骤负例全部通过 |
| 优化有效性 | 最小 fixture 的结构证据：目标节点、检查、内存访问或拷贝在 S/L 图与机器码中消失；真实库（semver，必要时 regex-syntax）的端到端计数 |
| 成本 | 分阶段编译指令（`KFRC_PROFILE`）、峰值内存、R/S/L 节点、Phi、机器指令、spill、帧、`.text`、相关 runtime |
| 归因 | 未达到预期的项写明属于设计、实现未完成，还是 PoC 范围之外 |

规则：

1. 正式计时在 E2B，原版与候选在同一沙箱串行对照；profile 与正式计时分开；不同宿主的绝对耗时分开报告。
2. 表格由脚本从运行日志生成。
3. 结构计数用于解释收益；收益以机器码、编译指令与 runtime 实测为准。
4. 源 UB 输入只用于确认解释器的 UB 判定；正确性结论来自有定义输入。
5. 动态验证首次暴露一类错误时暂停重复运行，静态排查同类实现，批量补回归测试并修复后再恢复。

## 负例编号

负例按 `N<方向>.<序号>` 编号，定义在各方向文件中。issue #5 点名的负例类别与编号的对应：

| 类别 | 编号 |
|---|---|
| UnsafeCell / Cell / RefCell | N4.4、N4.5、N5.1–N5.3 |
| 原始指针逃逸与 ptr→int | N2.6、N4.1–N4.3、N5.5、N5.7 |
| 循环动态实例 | N3.5、N4.13、N7.2、N7.3 |
| wrapping 运算 | N3.1 |
| 零输入 | N1.1 |
| 部分初始化 | N2.3 |
| Move 后源存储变化 | N2.1、N2.2 |
| ZST 与 Drop 次序 | N6.2、N7.6、N7.7 |
| MaybeUninit 与 union | N2.8、N3.3 |
| 同大小不同语义 | N6.1–N6.8 |

## 共享 fixture 位置

| 层 | 位置 |
|---|---|
| 图级单测 | `crates/kfrc-son/tests/semantic_*.rs`，每个方向新增文件（如 `semantic_permissions.rs`、`semantic_ranges.rs`） |
| 真实三臂差分 | `crates/rustc_codegen_kfrc/tests/semantic-ir/`，每个方向新增 `support_*.rs` |
| 真实库 | `crates/rustc_codegen_kfrc/tests/p3-semver/`；`scripts/poc-validation/semver-perf.py` |
| walkthrough 程序 | [research/walkthroughs](../../../research/walkthroughs/README.md) W01、W02、W04–W07、W09、W10 |

## issue #5 的关闭条件

1. 七个方向各有明确处置：已落地的步骤关联实现 PR 与上述证据包；延期或不采用的步骤在
   [README](README.md#延后项) 的延后表中记录理由、证据与后续跟踪入口。
2. 每个落地的事实在 [facts.md](facts.md) 中五项齐全，且消费者已合入。
3. 只增加元数据、只增加 pass 名称或只有设计文档的步骤保持 `planned` 状态。

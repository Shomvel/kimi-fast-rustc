# LP9 验收与报告

每个 LP 任务按同一套流程验收，结果写进新的冻结候选（candidate14 起）。

## 流程

1. **本机**：`cargo test` 覆盖相关 crate；结构测试与解释器差分先过。
2. **冻结**：`scripts/poc-validation/snapshot.py` 打包源码，记录 commit、补丁和哈希。之后的改动不进入该候选。
3. **E2B 正确性**（HK 主机，`sandbox_runner.py`，沿用现有入口和模板，不启动 SWE benchmark）：
   - `run-linux.sh <repo> <results> all`，再跑 `semantic`；
   - 三臂语义语料（LLVM／kfrc 关优化／kfrc 开优化）；
   - 7 套 legacy 语料；
   - 生成式对拍（200 个函数 × 6 组输入，LP2／LP4–LP6 各加专门的生成器）；
   - semver 34 项原版测试；W01–W10 与 regex-syntax 按候选说明是否重跑。
4. **E2B 性能**（与正确性分开、同一沙箱内串行）：
   - `semver-perf.py`：编译 5 轮、runtime 7 轮；
   - `total-profile.py`：编译指令 3 轮；
   - LP0 的阶段分项与 runtime profile；
   - `machine-attribution.py`：重点函数的机器形状。
5. **出现第一类错误**：停止重复运行，静态排查同类，批量补回归和修复后，作为新候选重来。

## 每项必须报告的数字

| 类别 | 内容 |
|---|---|
| 门槛 | `.text` 比值、三个 runtime 比值（中位数和单侧 95% 上界）、编译指令比值（两种口径） |
| 结构 | `from_str`、`finish_grow`、`RawVec::drop`、`Alignment::as_usize` 的 S/L 节点、Phi、Undefined、Load/Store |
| 机器 | 同上函数的指令数、mov、以 rbp 为基址的 mov、访存、条件／无条件跳转、call、体积 |
| 编译 | 分阶段指令数与上一个候选的差 |

表格由脚本从原始数据生成并原样粘贴，不手写数字。回退与改进都写进 `candidate<N>-acceptance.json` 的 `interpretation`。

## 与 LLVM 的形状对照

每个 LP 都维护一组小 Rust 函数 fixture，同时用 LLVM 臂和 kfrc 臂编译，objdump 后统计指令。用它来判断“某类代码离 LLVM 还差多少”，比只看 semver 总数更容易归因。

- fixture 来源：walkthrough 的例子，以及从 `llvm/test/CodeGen/X86/` 中对应主题（地址模式、cmov、switch、循环）挑出、改写成 Rust 的小例子；
- 对照结果按 fixture 生成表，放在 `evidence/<候选>/shapes/`。

## 完成定义

LP 任务完成 = 结构验收通过 + 正确性全过 + 性能数字已报告。

性能不要求单项达到门槛，但不得出现未解释的回退。最终六项门槛按 [metrics.md](../metrics.md) 统一判断，未达项写明属于设计问题、实现未完成还是 PoC 范围外。

# 批 0：冻结、错误与共享契约（第 1–2 天）

任务：[T00](../roadmap/m0-baseline.md#t00)、[T01](../roadmap/m0-baseline.md#t01)、[T02](../roadmap/m0-baseline.md#t02)、[T03](../roadmap/m0-baseline.md#t03)。

## 做什么

集成人冻结：干净 commit、正式参考源码、oracle 二进制、初始 target/edition/cfg、工具链构建模式。已有的脏工作区和旧二进制只作历史证据。版本选择见 [sources/](../sources/README.md)。

可并行：

- A/B：阅读要移植的实际闭包和上游回归。
- C/D：梳理现有图和 ABI 对新表示的要求。
- E：建立输入/命令/产物指纹和真实执行 runner。

这一批的输出是来源和接口约束，不是各自的实现。

## 必须收敛

- **最小输入语言：** 整数/布尔、绑定/赋值、函数、if/loop、return。具体边界类型、普通模块是否纳入首轮逐项确认。
- **最小契约：** 共享身份、类型、Body/CFG、控制流合并、目标 ABI、错误分类、产物来源与状态。
- **fixture：** 一个成功用例、一个预期拒绝用例，以及 [已知 if 错编译](../validation/regression-if-expression.md) 的干净重放。冻结构建不复现时保留结果并追查差异，不修改输入去凑同一个数字。
- **调用关系：** 每条链会调用哪些外部程序；目标函数对象必须由候选生成。

## 出口

一份下一执行者能直接用的派工包：输入 revision、配置、公共类型定义或精确草案、fixture 清单、原始结果位置、每个角色可改的文件。接口字段明确之后，依赖它的任务才进入 `in_progress`。

# 内建 IR 成本模型与内联决策

本轮交付实现成本分析接口、内联成本策略、决策记录和回归测试，不引入外部求解器或新依赖。成本信息是只读分析快照，图修改后重新计算；不在 IR 节点中保存容易过期的权重。此前出现性能回退的地址共享及立即数移位候选已从默认代码撤回，其实验记录保留用于解释建模需求。

## IR 分析接口

`crates/kfrc-son/src/analysis/cost.rs` 提供：

- `CostKind::{CodeSize, ReciprocalThroughput, Latency}`：参照 LLVM TTI 分开查询，不混用量纲。
- `InstructionCost::{Known, Unknown}`：unknown 不能排序成免费操作；累加溢出也使总成本未知。
- `CostModel`：只读、可替换的模型接口；当前内建 `GenericCostModel`，模型名固定为 `generic-ir-v1`。
- `GraphCost`：保留已知成本与未知节点数量，另记录节点数、输入边数作为编译工作量。图级只汇总 code size 和静态 throughput work，不把各指令 latency 相加冒充执行时间。

内建模型以标量 add 为 1 个相对单位，对乘除法、动态/常量移位、访存、地址和控制节点作区分。编译期证明节点的机器成本为零，但仍计入编译工作量；call 的指令边界有体积成本，整个 callee 的动态耗时未知。聚合值传输、未建模操作保留 unknown。

这些是明确标注的初始启发式权重，不是实测 CPU cycles、精确机器码字节或最终 uop 数。尤其在当前后端的中间表示和机器选择尚有冗余时，估计不能代替最终产物。模型接口允许后续接入 x86/具体 CPU 成本，但本轮不宣称已经做到。

## 实际决策入口

`inline_calls_with_cost` 接受 `InlineCostPolicy` 与 `CostModel`，原 `inline_calls` 保持兼容。

| 模式 | 排序及决策 | 成本记录 |
|---|---|---|
| `Legacy`（默认） | 原来的节点预算与顺序 | 不扫描成本、不生成记录 |
| `Observe` | 与 Legacy 完全相同 | 估价并记录实际接受/拒绝结果 |
| `CodeSize` | 按已知 code-size 成本排序，以成本阈值替换 callee 节点阈值 | 记录模型、估计、阈值、预算、结果；unknown 保留调用 |

深度、调用数、总节点增长、递归、ABI、类型、匿名数据身份等约束仍由原内联实现处理。成本更低不授予绕过合法性检查的权限。sole-call-site 的消除收益沿用原策略，但不能绕过未知成本和硬增长预算。

每个 caller 内的 callee 成本只计算一次；累计估价的活节点数受 `InlineBudget.max_nodes` 限制。预算不足产生 unavailable 估计而非免费成本，不建立全局易失效缓存。候选优先级同成本时仍按原发现顺序稳定排序。

模型目前估计的是 callee 的静态成本，尚未模拟调用点常量替换、分支裁剪、聚合字段需求或估计净内联收益。也未加入块频率、寄存器压力、spill、关键路径或布局风险。这些是后续扩展项，不能用本轮接口存在来声称已具备 LLVM 的完整成本判断。

## 后端使用方式

同一后端支持以下显式环境选项：

```sh
KFRC_INLINE_COST_MODEL=legacy   # 默认；已验收策略
KFRC_INLINE_COST_MODEL=observe  # 记录，不改变优化决策
KFRC_INLINE_COST_MODEL=size     # 启用相对 code-size 策略
```

`size` 的初始 `max_units=64`、`free_units=16` 是可追溯的实验阈值，不是已经校准的最优参数；库 API 可传不同阈值。原有 `KFRC_INLINE=0` 仍关闭内联。无效模式会报配置错误，不静默回退。

Observe / size 模式在 stderr 输出前缀为 `kfrc-cost ` 的 JSON 记录。每条包括 caller/callee/call NodeId、深度、模型版本、策略、已知/未知成本、原结构预算及已消耗预算、sole-call-site、free、实际结果和每个 caller 的分析节点数。NodeId 是该次内联图内的位置，compaction 后不可跨图复用。

从构建日志取出记录：

```python
import json
with open('compile.stderr') as stream:
    decisions = [json.loads(line[len('kfrc-cost '):])
                 for line in stream if line.startswith('kfrc-cost ')]
```

`estimate: null` 表示缺少 source 或成本分析预算不足；`size_unknown > 0` 表示图内包含未建模体积成本。两者均不能用 `size_units` 作完整成本。`outcome` 区分 `inlined`、`Cost`、`UnknownCost` 与原有 `Budget`、`Recursive`、`AbiMismatch` 等。

默认保留 Legacy 是为了先建立可信比较基线，避免把尚未校准的数值当作已验证的性能策略。Size 模式已经真实参与排序、free 判定和接受门槛，并非仅打印日志；其性能仍需后续按多负载校准。

## 验证

新增六项测试覆盖：Observe 与 Legacy 图和决策一致、成本阈值与节点阈值分离、硬增长预算、未知成本拒绝、重复调用的摘要缓存、不同成本维度、编译结构节点与机器成本分离、未知调用耗时、累计溢出、模型排序与节点排序不同。接受的内联仍通过解释执行与验证器。

修复后本机 workspace 798 passed、1 ignored。Linux workspace 831 passed、1 ignored。四个 crate 的默认与 observe 对象均逐字节等于冻结基线。完整数据见 [验证报告](results/builtin-cost-model/validation.md) 和 [机器可读摘要](results/builtin-cost-model/summary.json)。设计依据及后续任务见 [LLVM 成本模型研究](cost-model.md)。外部求解器仅保留为历史研究选项，不在当前实施范围。


### 改变内联决策后暴露的布局错误

首次以 `size` 编译 humantime 时，`RawVecInner::try_allocate_in` 的 aggregate call-result snapshot 未支配后续 store。原因是 `lower_layout::decode_boundary` 重接控制/效果边时比较原始 NodeId；此前删除无用 `LoadValue` 所转发的 continuation 尚未规范化，导致其消费者漏接在新 snapshot load 之后。

修复在这两处比较前解析 forwarding，保持验证器和原有语义约束。新增 `call_snapshot_precedes_users_of_a_forwarded_load_continuation`：修前稳定触发 dominance violation；修后通过布局验证器和解释器，返回预期的 42。相关 18 项 memory 测试通过，原失败的 Linux humantime size 编译也已通过。


### 最终验收范围与限制

- 显式 size 模式：四库语料及 13 项 workload checksum 均与 LLVM 一致；semver 34、base64 13、unicode-segmentation 8 项受支持集成测试通过。TLS、atomic_xsub、u128 导致的既有上游 unit 支持缺口单列在摘要中。
- size 语义差分三臂各 824 行一致，寄存器压力差分三臂各 2,048 向量一致；分别以 1,118 与 121 条 size 日志确认真实模式，避免测试脚本清理环境后误跑默认模式。
- Observe 5,707 条和 size 4,319 条记录均通过 schema/budget 检查。size 中 3,515 条因 UnknownCost 保留调用，277 条接受内联；真实四库中 Cost 拒绝为 0，阈值拒绝由定向单测覆盖。这说明模型覆盖率仍不足，不能把默认切换到 size。
- 同沙箱交替三轮测量，默认编译指令开销为 +0.27%～+0.89%；每次测量的对象仍逐字节等于基线。Wall/RSS 仅记录为三轮含噪声观测，不据此宣称加速。没有测量 size 模式的运行速度。
- 源码 11 文件、后端、原始日志及产物均保存 SHA-256；新旧归档分别核验 1,516 与 6,009 个成员。E2B 沙箱已释放，见 [回执](results/builtin-cost-model/release-receipt.json)。

后续优先扩大聚合值和常见投影的成本覆盖，再用调用点特化后的净收益替换静态 callee 总量，并以机器产物校准权重。块频率、寄存器压力和布局影响需分别建模；保持当前 legacy/observe 基线，逐项验证后再决定默认策略。

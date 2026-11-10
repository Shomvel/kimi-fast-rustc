# 重复地址与常量移位实验（尚未通过性能验收）

**当前状态：v1 与 v2 均出现跨负载回退，尚未采用为正式优化。工作树保留 v2 实验候选；后续转向 [系统成本建模](cost-model.md)，不得将下列初版收益当作最终交付结果。**

本轮基于 `b254941179c6f052bc2b732814c61ed299e39a3b`，即已经合入 PR #7 的全部前序优化。实现覆盖两个在新基线产物中确认的差异；其余工作见 [优化计划](optimization-plan.md)。[基线分析](report.md) 中的历史数据与本轮测量分开记录。

## 变更及语义边界

内联后 `internal_encode` 中，同一输入指针反复经过 `PointerCast`、`FieldAddr[0]` 得到编码表地址。例如 `n51=PointerCast(n3), n52=FieldAddr[0](n51)` 与 `n75=PointerCast(n3), n76=FieldAddr[0](n75)` 的类型和操作数完全相同。基线汇编整个函数有 39 次 `lea [rdi+3]`，其中热循环准备段连续 32 次；栈空间为 `0x178` 字节。

内联后的专用 GVN 扫描现在可共享浮动的 `FieldAddr`、`IndexAddr`、`PointerCast`。表达式键包含 opcode、完整结果类型、payload 和原始 SSA 输入的代表节点。共享保持相同基指针的 allocation identity、offset 和 poison；不依据地址数值相等替换指针，不合并分配、load、TLS、控制锚定值或带变体证明的地址。内联后增加一次仅处理这三种地址表达式的扫描，避免重新执行完整 GVN 和 witness 分析。扫描按创建顺序运行，不要求求出所有等价式的闭包。

基线 `internal_encode` 还有 43 处通过 `CL` 执行的移位。未带 `AMOUNT_IN_RANGE` 的常量移位原先先生成临时寄存器掩码，再走 `CL`。现在在降低时按整数宽度计算 `count & (width - 1)`，将结果保留为立即数，复用已有立即数指令选择和编码。动态移位以及带契约的移位保留原来的处理。

这两项变更均不删除输出切片检查。基线范围分析已经消除了 32 个编码表索引检查，不需要重复实现这项能力。

## 与 LLVM 的具体差异

新建的 LLVM 诊断产物中，编码热循环是标量 `load u64 → bswap → immediate shift → table load → byte store`，直接把 `+3` 编码表偏移折入内存地址。主循环每轮先检查输入/输出切片，再写入固定位置；热循环没有 `read_u64` 调用，也没有 `CL` 移位。这是后续切片长度传递、小 helper 内联和地址模式优化的具体参照，不能把当前差距统称为缺少 SIMD。

该 LLVM IR/汇编诊断使用额外 `--emit=llvm-ir,link`，改变了对象拆分，因此只用于指令形态分析。正式体积和性能比较使用 suite 原始 O3/CGU16/MIR0 配置的产物。

## 正确性证据

地址用例覆盖同一地址共享、不同对象、不同字段，以及写入前后读取不可混淆；优化前后解释执行结果一致。常量移位用例覆盖三种方向、8/16/32/64 位宽、零、边界和超位宽计数；Linux 将生成的对象与 C 驱动链接，执行 360 组输入并核对 wrapping 语义。另有结构检查确认常量移位不再经 `CL`。

当前 v2 本机 workspace 重新运行：90 个测试套件，793 passed、1 ignored；v1 同样通过该范围。报告生成器：8 项回归通过。基线分析复算：59 个输入哈希、13 项负载、全部精确 codegen 窗口核对通过。

## 初版候选的单项与组合比较（未采用）

同一沙箱、相同基线，在完成候选回归测试后执行 11 轮四方案串行交错计时。以下耗时比值为逐轮配对比值中位数，均相对原基线；原始比值见 [base64-paired.json](results/ablation/base64-paired.json)。

| 方案 | 库 `.text` | encode 耗时比 | decode 耗时比 | short 耗时比 |
|---|---:|---:|---:|---:|
| 原基线 | 29,651 B | 1 | 1 | 1 |
| 仅常量移位 | 29,011 B | 0.9870 | 1.0322 | 1.0004 |
| 地址共享 + 常量移位 | 27,427 B | 0.9394 | 1.0005 | 0.9432 |
| LLVM（体积参照） | 8,648 B | — | — | — |

组合版 `.text` 减少 7.50%，编码耗时下降 6.06%，短输入下降 5.68%；解码持平。仅移位方案在本组解码负载退步 3.22%，因此不能仅凭代码缩短宣称所有负载提速。完整正式四库对照另行验收组合方案。

正式对象中，`internal_encode` 从 6,095 B 缩小到 4,042 B（−33.68%）；仅移位版为 5,629 B，LLVM 为 1,376 B。组合版仍为 LLVM 的 2.94 倍。

组合版 `internal_encode` 的 39 次相同表地址 LEA 被一次入口 `add rdi,3` 替代；43 处 `CL` 移位降为 0。主栈空间从 376 B 降为 80 B，另需 8 B 对齐；均不含保存寄存器的 push。静态结构改善不能直接换算成动态 spill 或周期占比。

## 初版候选 Linux 全量验收

同一候选源通过 Linux 827 项单测，1 项按原配置 ignored。三方案语义差分各 824 行一致；scalar 565 行、5×40 generated cases、pressure、semver 34 项上游集成测试、七套 legacy 语料、CLI 与 feature probes 全部通过。

额外执行的旧 `p1-memory` 阶段在 `std::intrinsics::select_unpredictable::<select::Word>` 暴露 union/SIMD 支持缺口；冻结原基线和组合候选均在同一位置拒绝。该额外阶段单独记为 unsupported，不计入通过项。原有上游完整测试支持缺口仍保留。

实验首次解包候选时保留了早于缓存产物的源码 mtime，Cargo 复用了陈旧库，导致地址结构断言失败。核对五文件 SHA-256 后，清理相关缓存并刷新 mtime，重新构建两候选，全部回归通过。旧失败不进入测量；正式测量使用新构建且独立冻结的后端。

## 回退诊断与 v2 候选

初版还在早期通用 GVN 共享地址。这改变了按节点计数的内联预算：semver `Prerelease::cmp` / `BuildMetadata::cmp` 从 1,085 / 1,130 B 膨胀到 1,582 / 1,626 B。汇编证实前者原先调用 `SplitInternal::next`，初版将其展开成 `CharSearcher::next_match`、`get_unchecked`、`get_end` 等调用。正式 11 轮对照发现 semver requirement 解析退步 5.47%；独立隔离序列再次测得 5.40%，故不采用初版。

v2 候选将共享限定在内联完成之后，保持内联决策前的图与预算不变。验证结果单独记录，不把初版通过或收益直接移用到 v2 候选。


v2 仅在内联后共享地址，恢复两比较函数原大小，base64 收益仍在；独立11轮中 semver requirement 约改善2.1%，但identifier约退步15.3%。两版identifier相关规范化指令序列一致，动态指令数基本相同；性能原因仍需布局干预验证。v2同样未通过性能验收。

原始样本与汇总：初版 [正式运行](results/candidate-v1/runtime-summary.json)、[体积](results/candidate-v1/size-summary.json)、[编译成本](results/candidate-v1/compile-cost.json)；[semver隔离](results/semver-diagnosis/paired.json)、[v2隔离](results/v2-diagnosis/paired.json)、[当前源码指纹](results/source.json)。

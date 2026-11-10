# SCCP 与 KnownBits 首批实现

基线为 PR #8 的 `f321983`（包含前一批 InstCombine/GVN）；先独立验证 SCCP，再验证 SCCP + KnownBits。用户随后明确要求编译墙钟时间降低约 50%，**这批改动不满足该目标**：它们主要改善生成代码，编译指令工作量反而增加约 0.46%。后续编译时间优化以此版本为冻结基线，不将这批结果计作编译加速。

## 实现

SCCP 用 Unknown / Constant / Overdefined 三态联合传播整数/布尔值和可执行控制，Phi 只合并可执行前驱。新增回边会重访 Phi；常量遇到不同值单调升级为 Overdefined。Unknown 不是 IR Undefined，poison、读取、调用、未建模值为 Overdefined。收敛后将残留 Unknown 放宽并再次求解，随后才替换值。原有 simplify/CfgEditor 负责原子裁剪 Region/value/effect Phi，保留可达副作用和不终止循环。

共享 InstCombine 的位宽归一化、整数求值、移位与 flags 检查。首版不传播 Char、指针、浮点、聚合或 VariantSwitch 的分支事实；不做 IPSCCP。没有标量 Phi 时跳过求解；分析预算为 32N + 8E，按访问输入与用户计费，耗尽时不提交任何常量替换。工作队列在全图入口以及内联 seam 收尾各运行一次，没有增加全管线固定点循环。

KnownBits 使用独立 zero/one 位掩码，支持 Const、AND/OR/XOR/NOT、常量移位和整数扩展/截断，每次查询最多访问 32 个节点。事实只保证“值有定义时这些位成立”，不证明值不是 poison。InstCombine 据此删除冗余 AND/OR，始终保留原输入及其 flags/控制依赖；不将值折为常量、不做比较消除，不修改带 DISJOINT 的外层 OR。Phi、内存和未建模运算保守未知。DemandedBits 尚未实现。

参考 LLVM 21.1.0 固定源码的 SCCPSolver::markEdgeExecutable、visitPHINode、solve、resolvedUndef，以及 KnownBits 的 zero/one 和扩展规则。引用地址及哈希见 llvm-references.json。LLVM 的范围格、跨过程传播和 undef 规则没有机械移植到 KFRC。

## 结果

SCCP 五个定向函数的函数体总计 **131→87 字节（-33.6%）**；KnownBits 四个独立样例 **76→57 字节（-25%）**。动态合流不误折叠，循环变化量保留，仅常量/不变量减少。每组每臂都用共同 LLVM 驱动验证 4096 组输入。函数体字节不含对齐，不等于运行时间。

真实库 `.text`：semver 34148→34148；base64 29651→29635；unicode-segmentation 18618→18602；humantime 57477→57461。总共减少 **48 字节**，对象和 rlib 总大小同样减少 48 字节。semver 函数体少 5 字节，被对齐抵消。完整库、最终驱动节大小和后端文件大小分别在 summary.json 与 artifact-sizes.json，不把库节大小当成最终程序大小。

同一 AMD EPYC 8 CPU / 16 GiB 沙箱，Rust 1.94.1 / LLVM 21.1.8，共用 MIR0 sysroot；四臂（LLVM、基线、SCCP、SCCP+KnownBits）预热后 11 轮顺序交替运行，perf 单独采样。组合版/基线的配对耗时比几何平均 **0.99736（约 -0.26%）**，整体基本持平；候选/LLVM 的中位耗时比几何平均仍为 **3.483×**。没有单项达到 >3% 回退门槛。

| 负载 | 配对运行时间变化 | 单次 perf 动态指令变化 |
|---|---:|---:|
| semver / parse | +0.25% | -0.0000% |
| semver / req | +0.63% | -0.0000% |
| semver / identifier | -0.03% | -0.0000% |
| base64 / encode | -0.75% | -0.3635% |
| base64 / decode | -2.54% | -0.0000% |
| base64 / short | -0.94% | -0.0544% |
| unicode-segmentation / graphemes-mixed | -0.78% | -0.0000% |
| unicode-segmentation / graphemes-ascii | +0.08% | -0.0001% |
| unicode-segmentation / graphemes-rev | +0.43% | -0.0000% |
| humantime / dur-parse | -0.89% | -0.0000% |
| humantime / dur-format | +0.77% | -0.0001% |
| humantime / ts-parse | +0.37% | -0.0000% |
| humantime / ts-format | +0.02% | -0.0002% |

base64 decode 独立 21 轮复测时间比 **0.97417（约 -2.58%）**，输出校验一致；它的动态指令数基本不变，因此不能归因于热路径删除指令，布局/微架构原因尚未证实，也不外推至其他输入或机器。base64 encode 的动态指令减少约 0.36%。humantime dur-format 在首轮/复测中方向不同，按波动处理，不宣称稳定收益。

三轮交替编译（checker=0，逐次对象与本臂参考一致）：组合版编译指令增量 semver +0.609%、base64 +0.433%、unicode +0.433%、humantime +0.379%，几何平均 **+0.463%**。KnownBits 相对 SCCP 的编译指令增量几何平均约 +0.0008%，主要增加来自 SCCP。这些是指令计数，不代表编译墙钟时间加速。

SCCP 独立阶段：运行配对几何平均 +0.37%，编译指令约 +0.4%～+0.6%，unicode/humantime 各少 16 字节；原始结果单列 sccp-only.json。最终四库诊断累计 1902 条优化调用记录中，SCCP 替换 11 个节点、访问 85202 次、预算退出 0 次。统计包含初始和内联后多次调用，不是唯一函数/常量计数。诊断打开后对象仍与正式参考一致。

## 正确性与证据

最终本机 **815 passed / 1 ignored**，Linux **848 passed / 1 ignored**。新增 6 组 SCCP 测试及 5 组 KnownBits 测试，覆盖长常量链、late backedge、零次/多次循环、不终止循环、effect Phi、poison、flags、8/16/32/64 位移位、符号扩展和共享使用者。旧 flags 测试改为要求标记和语义保留，允许安全删除无标记的冗余外层掩码；PoisonObserved 的节点编号不作为语义结果。

三臂语义差分各 824 行、压力差分各 2048 向量；scalar 565 行、生成 5×40 函数；四库 corpus 和 13 负载 checksum 与 LLVM 一致。受支持集成测试 semver 34 / base64 13 / unicode 8 通过。checker 开关不改变各臂对象。regex-syntax union/SIMD 及单测 TLS、atomic_xsub、u128 既有缺口仍保留，未记为通过。

source-manifest.json 对应最终 8 文件，sccp-source-manifest.json 对应消融阶段。最终后端 SHA256 `2867f89c8e5030b114ab97140b42287c174dca07335e3f9237e77e248ae3ab0b`，SCCP `90149dae473bc2098ac5e71d998b671d3cd38cd5b5983f5d08e6cb66df46ad4a`，基线 `53a87ea4948de415e347eb99721b9e1ed1a717de444a7fcb1ad38d4fc42451a2`。

证据包含源归档、各臂后端、正式对象/rlib/driver、完整原始样本、测试日志和压缩图诊断，排除 Cargo 构建缓存。4325 个文件逐项哈希核验完成，见 evidence-receipt.json。沙箱继续用于用户新指定的编译时间优化；后续实验单独记录，不覆盖本轮归档。

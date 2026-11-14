# InstCombine / GVN：第一批 LLVM pass 迁移

本轮在 `e3c6295` 基线上实现嵌套常量合并和交换律表达式去重，保持原有内联策略与成本模型。最终采用 v2；初版 v1 因运行回退撤回，原始结果保留。SCCP、KnownBits、cast 链和循环 pass 尚未实现。

## 实现及语义边界

- 全图 InstCombine 与内联 seam 复用同一个 `rewrite`。沿用已有四次扫描上限和 seeded worklist，不新增全管线固定点循环。
- 对无 flags、无控制锚点、同类型的整数/布尔 Add、Mul、AND、OR、XOR，将 `(x op C1) op C2` 合为 `x op C`，按实际位宽回绕；支持常量在任意一侧，常量规范到右侧。
- 保留可能携带 poison 的 `x`；即使组合常量为零也不把 `x * 0`、`x & 0` 折成常量。保留有其他使用者的内层表达式；不复制 x 的计算。不重排带 NUW、NSW、DISJOINT 或控制锚点的节点，不处理 Char/指针/浮点。
- GVN 仅规范比较键中的交换律输入，命中重复表达式才替换节点，保留代表节点的真实操作数顺序。初版在 InstCombine 中按 NodeId 重排所有普通操作数；该规则已删除。

参考 [LLVM 21.1.0 InstructionCombining.cpp](https://github.com/llvm/llvm-project/blob/3623fe661ae35c6c80ac221f14d85be76aa870f1/llvm/lib/Transforms/InstCombine/InstructionCombining.cpp#L429) 的 `SimplifyAssociativeOrCommutative`：常量位置规范化、结合律化简和 flags 证明。LLVM 按表达式复杂度排序；NodeId 排序是本项目初版尝试，不是照搬 LLVM 的规则。

## 实测收益及局限

六个完整编译、链接并执行的定向函数：

| 函数 | 基线函数体字节 | v2 | LLVM |
|---|---:|---:|---:|
| nested_add | 12 | 8 | 5 |
| nested_mul | 12 | 8 | 9 |
| nested_and | 18 | 11 | 9 |
| nested_or | 18 | 11 | 10 |
| nested_xor | 18 | 11 | 10 |
| reversed_add | 13 | 10 | 3 |
| 合计 | 91 | 59 | 46 |

函数体合计减少 **35.2%**，前五项由两条算术/位运算合为一条，最后一项由两个加法合为一个。三臂各执行 4096 输入 × 6 函数，均与显式 wrapping 参考结果一致。这里统计 symbol size，不含函数间对齐填充，不把字节数当作速度。LLVM 还能把 reversed_add 完全折为零；KFRC 当前的 poison 合同要求更保守，不能直接照抄该化简。

真实四库的 `.text`、对象文件和 rlib 总大小均未变；semver 的函数符号大小总和减少 4 字节（32795 → 32791），被对齐填充抵消。热负载动态指令数基本不变，单次 perf 采样差异均小于 0.0002%。没有据此宣称消除了现有主要热点。

同一 AMD EPYC E2B 沙箱，Rust 1.94.1 / LLVM 21.1.8，共用 MIR0 sysroot。13 个可对照负载先预热，再 11 轮顺序交替运行；perf 单独采样。v2/基线的配对时间比几何平均为 **0.9971**（约 -0.29%），视作基本持平，不认定为确定加速。各负载保留如下，负号表示候选耗时更少：

| 负载 | 配对耗时中位数变化 |
|---|---:|
| semver / parse | -0.56% |
| semver / req | -0.22% |
| semver / identifier | +0.24% |
| base64 / encode | -0.06% |
| base64 / decode | +1.00% |
| base64 / short | +0.09% |
| unicode-segmentation / graphemes-mixed | -0.85% |
| unicode-segmentation / graphemes-ascii | -0.32% |
| unicode-segmentation / graphemes-rev | +0.61% |
| humantime / dur-parse | +2.17% |
| humantime / dur-format | -5.16% |
| humantime / ts-parse | -0.50% |
| humantime / ts-format | -0.08% |

最大变慢为 +2.17%，未触发预先设定的 >3% 复测门槛；这不意味着小于 3% 的变化统计上等于零。dur-format 的 -5.16% 未经独立确认，且指令数未变，不作为稳定收益宣称。候选/LLVM 的各负载中位耗时比几何平均仍为 **3.497×**（同期基线 3.509×）；本轮没有显著缩小整体差距。

三轮交替编译，checker=0，每次对象匹配该臂正式参考对象：编译指令增量 semver +0.086%、base64 +0.073%、unicode-segmentation +0.071%、humantime +0.035%，几何平均 +0.066%。wall、cycles、RSS 原始值在 summary.json 中，不将该微小指令增量外推为墙钟时间结论。

## 撤回候选与消融

v1 在 InstCombine 中将两个普通操作数按 NodeId 排序。首轮 humantime dur-parse +3.88%，独立 21 轮反向交替起始次序复测 +3.63%；尽管 ts-parse 约 -8.14%，整体几何平均 +0.07%，仍按预设门槛拒绝 v1。

v1 的 dur-parse 动态指令基本不变，parse_duration 函数位置与大小均相同；其他指令操作数/寄存器和布局可能变化，因此没有证明单一微架构原因。只移除普通操作数排序的 constants 消融降低了 dur-parse 回退，同时消失了 ts-parse 的大幅收益。最终把交换律匹配移入 GVN 键，重新完成全套验证。不能将 v1 的 base64 -32 字节、humantime -16 字节或 ts-parse 加速记作最终成果。

## 正确性、复现与证据

- 本机 workspace：804 passed / 1 ignored；Linux：837 passed / 1 ignored。
- 六个新增语义测试覆盖 8/16/32/64 位、有符号/无符号回绕、交换顺序、poison、flags、多使用者、幂等和 seam 路径。
- Linux 语义差分三臂各 824 行、压力差分三臂各 2048 向量；scalar 565 行；生成测试 5 × 40 函数全部一致。
- 四库 corpus、13 workload checksum 与 LLVM 一致；受支持上游集成测试 semver 34 / base64 13 / unicode-segmentation 8 通过。checker=1 和 checker=0 对象哈希逐臂一致。
- 既有缺口仍为 regex-syntax 的 union/SIMD；base64 单测 TLS、unicode 单测 atomic_xsub、humantime 单测 u128。没有把未执行测试记为通过。
- 源码基线与最终三文件指纹见 source-manifest.json；所有机器结果来自该指纹对应后端。编译器后端基线 SHA256 `fcb4dc20d17e2ab3004a858c8b0e25ad98f73da311605bea26581fcb2fa341ad`，v2 `53a87ea4948de415e347eb99721b9e1ed1a717de444a7fcb1ad38d4fc42451a2`。
- run-kernels.sh 固定沙箱布局；kernels.rs 和 driver.rs 是定向输入，正式四库使用仓库 tools/llvm-parity-suite。外部控制器/运行脚本、冻结源码和本机日志保存在 `/Users/shom/Downloads/kfrc-instcombine-20261110`。
- 首次 apt HTTP 更新卡住后主动终止，改 HTTPS 完成 perf 安装；所有正式计时发生在安装和构建结束后。effective-suite.json 保存实际 perf 路径。此环境准备失败不计为编译器测试失败。

机器证据归档包含基线/v1/v2 后端、正式对象/rlib/driver、反汇编、输入、完整原始计时、编译样本和验证日志，排除上游 Cargo 构建缓存。归档哈希与沙箱释放回执见 release-receipt.json。仓库另存全部正式运行样本 runtime-samples.json、失败候选的汇总与复测样本、三臂定向反汇编，便于不下载机器产物也能复核结论。

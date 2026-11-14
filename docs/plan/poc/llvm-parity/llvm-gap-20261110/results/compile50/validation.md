# 编译时间优化：并行构图、函数编译与批量活跃值分析

用户目标是完整编译墙钟时间降低约 50%。相对已包含 InstCombine/GVN/SCCP/KnownBits 的 `09cb657`，本轮四个真实库的配对耗时比几何平均为 **0.7830，即降低 21.7%**。**50% 目标尚未达到**。所有前期改动继续保留在 PR #8。

## 实现

- 将 rustc 查询、权限和静态数据转换与纯 Body/graph 编译分开。工作线程只借用不可变的拥有完整数据的图，持有自己要修改的函数图；`TyCtxt`、`Instance` 不跨线程。
- 并行构建、验证和优化初始 S 图。跨函数内联仍读取已完成初始优化的同一份图；没有减少验证或改变 pass/内联预算。
- 从原有根符号出发，按编译后机器码和数据重定位发现下一批函数，批内并行。每个 CGU 保留自身 available 集合和内部符号约束；不编译优化已经消除的内部源函数。最终重放原有有序可达性遍历，保持函数顺序、数据合并顺序和 emission 错误顺序。
- 默认线程上限为可用 CPU 数，额外线程必须持有 rustc/Cargo jobserver 令牌；`KFRC_CODEGEN_THREADS=1` 保留串行路径。调用线程在等待额外令牌期间继续工作；作用域结束会 join 全部线程并归还令牌。
- 活跃值分析按块向前扫描已定义集合，复用 block-entry bitset，替代每个 live-before/live-after 值的重复定义位置查询。独立 HashSet 数据流作为差分参照；不改变活跃区间、分配决策或生成机器码。

这轮参考 LLVM 的函数级编译边界和数据流分析组织方式，重点减少编译关键路径；没有扩展 cost model 或引入 solver。参考研究入口为 `llvm-study/topics/02-pass-manager.md`、`11-register-allocation.md`、`16-phase-ordering-budget.md`。线程令牌接口直接核对 Rust 1.94.1 的 `rustc_data_structures/src/jobserver.rs` 及 jobserver 0.1.34 实现。

## 正式计时

同一 AMD EPYC、8 vCPU / 16 GiB E2B 实例，Rust 1.94.1 / LLVM 21.1.8。固定 MIR0 sysroot、O3、CGU16、panic=abort、无调试信息；没有改 rustc 前端参数。baseline、候选串行、候选并行三臂轮换并反转顺序，1 轮预热 + 21 轮测量。每次都启动 rustc 完整生成 rlib，warm filesystem cache，无增量命中。计时包含进程启动、前端、后端及归档。

计时关闭可选寄存器分配 checker，三臂一致；正确性阶段开启它并核对产物。计时没有 profiler、dump 或 `-Ztime-passes`，使用阻塞 `wait4` 记录墙钟、CPU 和 RSS，避免旧 runner 的 5 ms 轮询误差。构建、采样、正确性检查均与正式计时分开。

| 库 | 基线中位时间 | 并行候选中位时间 | 配对时间降低 | 配对比率 bootstrap 95% 区间 | 峰值 RSS 增幅 |
|---|---:|---:|---:|---:|---:|
| semver 1.0.28 | 343.0 ms | 224.5 ms | 35.5% | 0.636–0.653 | 9.5% |
| base64 0.22.1 | 286.6 ms | 240.4 ms | 15.5% | 0.827–0.871 | 5.1% |
| unicode-segmentation 1.13.3 | 331.3 ms | 289.2 ms | 12.4% | 0.867–0.898 | 4.7% |
| humantime 2.4.0 | 335.2 ms | 265.8 ms | 21.2% | 0.776–0.797 | 3.9% |

配对中位比率与两个独立中位数之比不同，因此不能直接用表中两列时间复算配对百分比。区间仅反映本实例这批重复样本，不代表跨机器置信范围。

四库几何平均降低 **21.7%**；同一候选限制为单线程时，几何平均降低 **2.0%**，其中 humantime 配对降低 **4.0%**。主要收益来自并行执行。完整编译的 CPU 时间中位数增加约 **13.9%–20.2%**，峰值内存增加约 **6–14 MiB**。因此这是墙钟与 CPU/内存的取舍；没有证明多 crate 饱和 Cargo 构建的总吞吐会同幅改善。jobserver 会限制竞争时的并行度。

## 分阶段诊断与 50% 的限制

另做三次带 `KFRC_PROFILE_TOTAL` 的诊断，计数覆盖后端入口至 join 的初始线程及子线程。这些数字不能替代正式完整编译结果。

| 库 | 后端基线中位时间 | 后端候选中位时间 | 后端用户指令变化 |
|---|---:|---:|---:|
| semver | 216.7 ms | 103.3 ms | +0.91% |
| base64 | 142.9 ms | 97.9 ms | +1.00% |
| unicode-segmentation | 117.9 ms | 79.4 ms | +1.51% |
| humantime | 243.9 ms | 184.6 ms | -8.47% |

semver 的后端时间约减半，但完整编译只改善约 35.5%，不能据此宣称完成用户的 50% 目标。前期 metadata-only 诊断约 0.10–0.20 秒，unicode 的类型检查/借用检查等前端工作已超过原完整时间的一半；它的 metadata-only 路径并非严格数学下界，但足以说明仅继续减少后端工作很难使完整编译减半。

下一步应先对 `prepare_instance_input` 内 MIR 转换和类型/布局记录单独采样，验证跨实例重复查询与记录的占比，再决定是否引入有失效边界的共享记录。对前端占主导的库，需要另测 rustc 前端并行或 Cargo/incremental 路径，并保持独立基线；不能把改变构建方式后的结果混入本表。对大函数继续分解 layout、指令选择和分配；本次 `parse_unit` 仍是主要长尾。

## 消融与撤回

[ablations.json](ablations.json) 保留每版 13 轮探索数据，使用旧 runner，精度弱于正式 21 轮：

1. `parallel-cgu` 非阻塞 jobserver 接口返回 Unsupported，实际没有工作线程；四库比率 1.0065，已撤回这条调度实现。
2. `parallel-prep` 改用异步令牌接口，并行初始图验证/优化与 CGU：0.8909。
3. `parallel-roots` 并行确定输出的根函数：0.8396。
4. `parallel-live` 增加批量活跃值处理：0.8427。跨批噪声使完整时间看不出单独增益，不把它解释成确定回退或收益。
5. `parallel-reachable` 按实际引用批量并行内部函数：0.8054。
6. 最终 `parallel-body` 把 Body→SoN 构建也移入线程，正式结果 0.7830。

不同探索批次的比率不能连乘；最终效果只采用相同实例、相同时间段的正式 A/B/C 对照。

## 正确性

- 本地 workspace：816 passed / 1 ignored；Linux workspace：849 passed / 1 ignored；后端调度与 emission 单测：6 passed。
- 三臂语义差分各 824 行；压力差分各 2048 向量；scalar 565 行；生成测试 5×40 函数全部一致。
- 四库 corpus 和 13 个 workload checksum 与 LLVM 一致。最终 checker=1、checker=0 的四库 50 个目标文件均与 `09cb657` 基线逐字节一致，因此本轮没有改变这些样本的机器码、对象大小或代码布局。没有重跑运行速度计时，也不声称程序运行更快。
- 真实继承匿名管道 jobserver，额外令牌分别为 0/1/3：均正常完成、令牌全部归还、产物一致。单 CGU、4 CGU 对照 semver 产物一致；非法线程数 0 被明确拒绝。
- 测试覆盖零令牌不阻塞、单令牌活动线程上限、乱序完成后恢复顺序、每个任务仅处理一次、panic 后 join/归还令牌、循环引用、数据重定位、不可达内部函数省略及错误顺序。
- 四个 entry/allocator 可执行程序与 LLVM 的输出和退出状态一致。
- 扩展 p1-memory 测试在基线与候选都拒绝 `select_unpredictable::<select::Word>` 的 union/SIMD，未记为通过。其余已知 regex-syntax、TLS、atomic_xsub、u128 支持边界沿用前轮报告；本轮没有重跑全部上游测试。

## 证据

- [summary.json](summary.json)、[raw.json](raw.json)：正式命令、原始时间/CPU/RSS、配对比率、区间、二进制和目标文件哈希。
- [phase-summary.json](phase-summary.json)、[compile50-profile-summary.json](compile50-profile-summary.json)：分阶段诊断。
- [validation.json](validation.json)、[corpus.json](corpus.json)、[scheduling.json](scheduling.json)：验证摘要。
- [parallel-body-source.json](parallel-body-source.json)：冻结源码指纹；基线 backend SHA-256 为 `2867f89c8e5030b114ab97140b42287c174dca07335e3f9237e77e248ae3ab0b`，最终 backend 为 `d403e72ed5e4c61b50b33955159d586c40f6b653eab1d5767ab803897b18d5ea`。
- [compile50-final-bench.py](compile50-final-bench.py)、[compile50-phase.py](compile50-phase.py)：使用已有 suite 语料和路径的计时脚本。
- 原始日志、perf 样本、源码包、各版后端和可执行产物见 [evidence-receipt.json](evidence-receipt.json)。

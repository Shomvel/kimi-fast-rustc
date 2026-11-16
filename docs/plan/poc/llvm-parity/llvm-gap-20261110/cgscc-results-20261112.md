# kfrc CGSCC 后序简化：v6 实现与实测

2026-11-12。按 review 的 S1 落地可达调用图 SCC 后序简化，并区分共享内联源和本函数发射候选。最终保留 v6 默认开启；`KFRC_CGSCC=0` 可精确回到本轮前的 wrapper 路径。

相对上一轮 InstCombine v4 基线，13 项固定负载的耗时比中位数等权几何平均为 **0.953985（耗时减少 4.60%）**，动态指令比为 **0.962671（减少 3.73%）**。四库编译耗时比等权几何平均 **1.028439（增加 2.84%）**，编译指令比 **0.973541（减少 2.65%）**。这是同环境、固定语料的阶段结果，尚未达到 LLVM parity。

本报告的“基线”是上一轮 InstCombine v4，不含本轮 CGSCC；“检查点 v4”是本轮此前已验证的单视图 CGSCC，二者不是同一个版本。

## 实现与边界

先从 CGU 根、adapter 与数据重定位收集可达闭包，再用确定性的迭代 SCC 调度按被调函数在前的顺序准备。递归 SCC 保留原发射路径。发生实际 identity/inline 替换后才再次执行完整 S 简化，缩小的图，或不超过 64 节点且调用减少的图，才发布为父层可用的共享源；拒绝候选时原图仍保持可用并保留真实 CGU 重试。

已完整简化、但因共享价格或私有依赖而不发布的图移入独立发射缓存。真实 CGU 仅在全部私有函数、地址和完整数据表重定位目标都可见时选它，否则选择共享图。发射候选不进入父层 provider，因此不会改变共享内联定价。有残留 Call 的候选保留普通 identity/inline/seam 重试；无调用候选可以跳过此重试。

现有 same-ABI 函数属性证明应用于两种图。候选不成为可达根；最终 machine/data relocation walk 决定死函数移除。address-taken 取并集，同一 caller/callee 在两种视图中的调用次数取最大值，避免把互斥候选相加。

普通调用预算仍为 depth4/free24/callee64/calls32/4096nodes。每 crate 准备有 1,000,000 逻辑工作账户，缓存上限为 1,000,000 个存储节点；这不是总内存字节上限或严格 CPU 指令上限。缓存图可在保留已有替换后花费一份新的 residual retry 预算，不能声称总增长等于单次旧发射预算。当前四库均未触发 work/cache budget。

实现入口：[后序准备](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen/cgscc.rs:22)、[实际 CGU 起点选择](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/codegen.rs:214)、[worker 与重定位闭包](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/rustc_codegen_kfrc/src/emission.rs:100)。

## 正确性和绑定

- Workspace 1,285 通过、0 失败、1 项原有 timing 测试忽略；另行 backend 26 通过，其中 15 项 CGSCC。
- 原 13 个 native 入口，加 InstCombine 和 CGSCC 入口，共 15 组通过。新夹具七层 wrapper 各从 152 S 节点、33 calls 降到 11–23 节点、0 calls；chain 终码 calls 1→0，diamond 指令 25→10。10,006 组输入、160,096 个有序观察以及递归/noinline/跨 ABI 边界差分通过。
- 四库 corpus 与 13 个 workload 校验和匹配 LLVM。关闭 CGSCC 的 50 个 whole objects 和 4 个 driver 与同沙箱重建基线逐字节一致。默认 1/8 线程、diagnostic/normal 对象与 driver 精确一致。
- 24 个 LLVM caller 对象在 7 个 arm 中精确一致；本轮 v4 使用独立冻结的实际二进制，而非用名称或摘要替代。
- 四库上游 cargo tests 沿用已记录的 legacy intrinsic 阻断而未运行；上述 corpus/native/workload 差分是本轮的第三方库验收范围。

所有目标编译、native、PMU 和计时由 subagent 在通过 HK 0820 实际契约核实的同一个 E2B 执行；没有在本机编译或 profile。Rust 1.94.1、LLVM 21.1.8、8 vCPU/16 GiB；运行固定 CPU7，编译使用全部 8 CPU/8 codegen threads。

## 真实终码与 CGU 选择

以下为对象中解析到的函数体指令字节与静态调用点合计，不是动态执行次数。

| 库 | 基线 / v6 函数体字节 | 基线 / v6 静态调用点 | v6 / 基线字节比 |
| --- | ---: | ---: | ---: |
| semver | 30,272 / 30,184 | 409 / 393 | 0.997093 |
| base64 | 23,220 / 22,186 | 151 / 141 | 0.955469 |
| unicode-segmentation | 17,565 / 17,503 | 175 / 165 | 0.996470 |
| humantime | 37,572 / 36,273 | 293 / 272 | 0.965426 |

Base64 `decode_helper` 在实际 CGU 选中候选，0 个缺失私有依赖；终码从 6,140B/1,390 指令/26 calls 降为 5,344B/1,224 指令/20 calls，MOV 824→664、stack 操作 357→279。encode 仍为基线的 2,043B/486 指令/19 calls，前版新增的 12 个 helper 回退全部消失。

`Parser::parse` 从 11,952B/2,389 指令/32 calls 降为 10,737B/2,334 指令/19 calls。`parse_unit` 候选因私有 `to_vec::<Global>` 数据表重定位在 CGU 中缺失而回退共享图，终码仍为 6,230B/1,517 指令/86 calls；不能把这个未选中的机会计入收益。`Unit::from_str` 静态指令 410→523，但主 duration 解析负载最终有动态净收益。

342 个缓存候选对应 179 次真实 candidate 起点、28 次 shared 回退；0 依赖选择违规。candidate 起点中 51 次 complete、128 次保留 retry。四库 shared 发布记录与检查点 v4 一致，说明收益没有通过改变父层共享价格取得。

## 同环境运行对照

先独立 4 轮 PMU（208 样本），再 24 轮正式 wall 加 2 轮 warmup（1,352 样本）。四个实际 arm 为基线、检查点 v4、v6 和 LLVM；每 arm 每位置有 6 个正式样本。正式 wall 不附带 PMU，所有阶段均与编译、上传、下载和归档分离。

比值为 v6 / 比较对象，越低越好。Q1–Q3 是这次配对样本的经验四分位，不是置信区间。

| 库 / 负载 | 动态指令 / 基线 | 耗时 / 基线中位数（Q1–Q3） | 耗时 / 检查点 v4 | 耗时 / LLVM |
| --- | ---: | ---: | ---: | ---: |
| semver / parse | 1.000000 | 0.957684（0.953575–0.962441） | 0.950683 | 3.176444 |
| semver / req | 0.991193 | 0.956715（0.946716–0.961977） | 1.025304 | 6.181071 |
| semver / identifier | 0.999638 | 0.997474（0.995489–1.006899） | 0.997903 | 1.676021 |
| base64 / encode | 0.999999 | 0.998773（0.997895–1.000008） | 1.000314 | 1.131165 |
| base64 / decode | 0.817077 | 0.951254（0.948980–0.951948） | 0.951735 | 2.114720 |
| base64 / short | 0.812909 | 0.749332（0.747065–0.750219） | 0.754420 | 1.705535 |
| unicode-segmentation / graphemes-mixed | 0.999999 | 0.998309（0.987265–1.001784） | 0.999199 | 1.338759 |
| unicode-segmentation / graphemes-ascii | 1.000000 | 1.001085（0.996499–1.003212） | 0.997176 | 1.093135 |
| unicode-segmentation / graphemes-rev | 0.996612 | 0.963131（0.961813–0.965716） | 0.969054 | 2.823876 |
| humantime / dur-parse | 0.930516 | 0.815294（0.812342–0.817791） | 0.813420 | 6.106042 |
| humantime / dur-format | 1.000000 | 1.028123（1.016397–1.037765） | 1.055318 | 1.132737 |
| humantime / ts-parse | 0.999207 | 1.029085（1.023985–1.032079） | 1.041082 | 1.738409 |
| humantime / ts-format | 1.000000 | 1.003376（0.999124–1.005890） | 0.998839 | 1.623474 |

13 项负载中位比值的等权几何平均：v6/基线耗时 0.953985、指令 0.962671；v6/检查点 v4 耗时 0.961730、指令 0.963465。这不等于按真实应用调用占比加权的提速。

主要净收益为短输入解码耗时减少 25.07%、长解码减少 4.87%、duration 解析减少 18.47%；相应动态指令减少 18.71%、18.29%、6.95%。semver parse 的耗时下降没有对应指令下降，不将其解释为减少执行工作。

负结果全部保留：duration 格式化比基线慢 2.81%，时间戳解析慢 2.91%；相对检查点 v4 分别慢 5.53%、4.11%，semver req 慢 2.53%。这些负载对检查点的指令数基本不变。保存终码核对确认这两项负载的主函数和已检查 helper 与检查点 v4 归一化相同；指令和 branches 没有回退。地址、GOT 和对齐有移动，布局影响只是未证实假设，原因仍未知；本轮没有为消掉回退重复计时。见 [逐函数归因](/Users/shom/Downloads/kfrc-cgscc-20261112/research/v6-wall-attribution.md)。

## 编译成本

测量对象是编译四个目标 crate，未计编译器自身构建。独立 4 轮 PMU，随后对三组 pair 各执行 1 warmup +6 正式 ABBA block，共 400 样本。每次计时编译都在计时退出后核对完整输出对象，全部等于对应 normal arm。

| 库 | 编译 wall / 基线中位数（Q1–Q3） | 编译指令 / 基线 | 编译 wall / 检查点 v4 | 编译指令 / 检查点 v4 | RSS / 基线 |
| --- | ---: | ---: | ---: | ---: | ---: |
| semver | 1.109553（1.094941–1.112973） | 1.004064 | 0.989556 | 0.925061 | 1.037527 |
| base64 | 0.931594（0.909837–0.941128） | 0.894861 | 0.870821 | 0.810397 | 1.014133 |
| unicode-segmentation | 1.029502（1.002957–1.040356） | 0.978580 | 0.984292 | 0.928951 | 1.015797 |
| humantime | 1.051264（1.042930–1.062575） | 1.021651 | 0.997335 | 0.935502 | 1.032699 |

四库等权编译 wall 比为 1.028439，指令比为 0.973541；相对检查点 v4 分别为 0.959033 和 0.898414。关闭配置的编译指令几何平均为基线的 1.000340。缓存复用了已完成的准备结果，但本轮没有逐阶段 profile，因此不能将全部编译成本变化归因于某一种重试或缓存。semver wall +10.96%、humantime +5.13% 的编译成本回退均保留。

后续成本改进的有限清单为：在实际 CGU 上跳过没有任何可准备来源的 retry；按不可变 source generation 复用 live-count/identity 元数据且保持原逻辑收费；独立实验 SCC full-S 与 seam。第三项可能改变共享定价和终码，须作为另一个冻结候选验证，本轮未修改。

## 负结果、复现与剩余差距

v2/v3 的 encode 动态指令曾回退到 2.539 倍，已完整封存。原因是跨 free24 的增长保护拒绝了原 wrapper 可接受的 15→58 节点、2→0 calls 简化，继而使五层包装触发 depth4；v4 恢复既有发布谓词修复。v5 的两项真实 CGU 测试首次暴露 rustc jobserver 未初始化后立即停测；核实官方 1.94.1 合同后仅以测试 helper 的 Once 初始化修复为 v6，生产逻辑与 v5 相同。v5 没有机器码或性能 arm。

本轮 v6/LLVM 的 13 项耗时中位比等权几何平均仍为 2.043414，req 与 duration 解析仍约为 6.18、6.11 倍。S1 已取得部分热点收益，未达到 review 的整个 M1 或最终 parity 退出线。`parse_unit` 的私有依赖回退、聚合/enum 跨边界物化与 L 层布局仍是已定位的大缺口；后续以 review S3 的实际访问分区和 L 清理为依据，不能靠增加局部算术规则数量代替。

最终工作树：`/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108`；HEAD `7fb99c5b057e7ce7f8481f8ce9cd2e05f2d599e6`。保留继承未提交改动，本轮未 commit/push。1,378 个源码/夹具输入与冻结 tar 逐文件一致，相对本轮基线有 12 个输入变化、无删除。

- source tree：`8697ab4c7f887feeedbc4c3dc544c99be4d3cdf1817b61511d7fa6f54ee035f9`
- source tar：`59529e662dc1dd2bb7884684b9457768168dfd9107a8ab91519b4454c837c24c`
- backend：`e66569c7d6c1213c37efb4c6e4a63e35c8a901984489ea84acd417402a43e4c0`

原始 review：[S1](/Users/shom/Downloads/kfrc-llvm-parity-review-20261112/02-bottom-up-simplification.md)、[S3](/Users/shom/Downloads/kfrc-llvm-parity-review-20261112/03-layout-representation.md)。

证据：[最终选择](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/final-selection.json)、[源码 manifest](/Users/shom/Downloads/kfrc-cgscc-20261112/source/candidate-cgscc-v6/manifest.json)、[最终源码核验](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/final-source-root-check.json)、[四库完整测量](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/v6-measurement-report.md)、[机器码与实际 CGU](/Users/shom/Downloads/kfrc-cgscc-20261112/research/v6-machine-gate.md)、[原始样本与产物回执](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/v6-artifact-receipts.json)。

独立复算：[208 PMU](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/candidate-cgscc-v6-checkpoint-screen-raw-root-check.json)、[1,352 wall](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/candidate-cgscc-v6-checkpoint-wall-raw-root-check.json)、[400 compile](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/candidate-cgscc-v6-checkpoint-compile-raw-root-check.json)、[阶段绑定和串行次序](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/final-measurement-bindings-root-check.json)。

持久原生验证入口：[cgscc.sh](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/scripts/poc-validation/cgscc.sh)。测试沙箱已释放，E2B API 返回 SandboxNotFoundException 确认不存在，见 [释放回执](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/release-receipt.json) 和 [progress-final.json](/Users/shom/Downloads/kfrc-cgscc-20261112/validation/progress-final.json)。最终归档包含 8,288 个 raw 文件及 1,464 个 binary/source 文件，所有成员已核对 SHA。

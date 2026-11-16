# kfrc Layout 调用实参常量传播：2026-11-13

最终选择：保留 `candidate-call-constants-v1` 默认开启，`KFRC_LAYOUT_CALL_CONSTANTS=0` 可关闭本轮规则。源码、正确性、机器码、运行与编译成本均已完成验收；原始证据封存和沙箱释放状态见文末。

本轮沿 review 的 S3 方向，补齐固定调用实参的支配常量传播。`Unit::from_str` 的 21 个短字符串 `memcmp` 全部被既有精确宽度比较替代，duration 解析动态指令减少 **1.36%**、耗时减少 **1.68%**。13 项负载的指令比等权几何平均为 **0.998947**，耗时比为 **0.988188**。编译指令增加 **0.277%**、完整编译耗时增加 **0.254%**。结果范围为四库固定语料，尚未扩展保留集，也未达到 LLVM parity。

## 实现和 LLVM 对应

在 Layout 清理中，若原固定 `Call` 控制点被普通整数的 `Eq true` / `Ne false` 分支支配，把同一 SSA 值的该次调用实参替换为同类型常量，再运行现有 InstCombine。典型形式为 `if len == 5 { memcmp(a, b, len) == 0 }`：把调用点长度精化为 5，已有认证 `MemcmpZero` 和 x86 小比较即可生效。

LLVM 21.1.8 的直接对应为 `GVNPass::propagateEquality` 及 `replaceDominatedUsesWithIf`；InstCombine 本身还传播分支 Boolean 条件。已实际阅读 LLVM 源码，详见 [源码对应](/Users/shom/Downloads/kfrc-layout-20261113/research/llvm-scoped-equality.md)。CVP 的 `processCallSite` 没有一般整数实参替换，不把这次规则误归因给它。

本轮只处理 Layout 直接调用、没有 scalar validity 限制的普通整数，位宽最多 64。`PredicateFacts` 在原使用点检查支配和同一循环动态实例；只改该操作数，保留原分支、固定 producer、effect、ABI、目标和其他观察。先收集不可变 receipt，在 debug 构建中用独立新建的 CFG/facts 重验，然后批量改写。4096 次事实查询预算耗尽只放弃后续优化；没有候选时不构造 CFG。

同时补齐 x86 最终展开点的认证检查：小比较必须持有仍与名字和 ABI 一致的 `RuntimeComparison::Bcmp`。同名用户调用不取得展开权限，整数事实不授予任何指针有效性或内存读取权限。

实现：[call_constants.rs](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/call_constants.rs)、[清理入口](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/cleanup.rs:87)、[事实证明](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/analysis/refinement.rs:77)、[x86 认证](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-x86/src/lower/abi.rs:146)。`KFRC_LAYOUT_CALL_CONSTANTS=0` 在分析前关闭规则；`KFRC_LAYOUT_CALL_CONSTANTS_STATS=1` 输出实际改写和查询统计。

## 正确性与终码

- Workspace 1302 项通过，0 失败，1 项原有忽略；backend 26 项通过。新增 15 项作用域/观察/循环/receipt 测试和 2 项 x86 认证测试全部通过。
- 原有 15 组 native，加新规则 on/off，共 17 组通过。新夹具每臂 117306 个检查，含 5284 对字节输入、512 个循环序列和保护页。固定长度覆盖 0–8、15/16/17、31/32/33 及大长度回退，每个字节翻转、重叠、顺序比较、未走 guard 的不可读地址和循环长度切换均纳入差分。
- 四库 corpus 与 13 个 workload 校验和匹配同沙箱 LLVM；所有模式的 24 个 LLVM caller 对象相同。关闭规则时，50 个 whole objects 和 4 个 driver 精确复现 v6；默认 1/8 线程与 diagnostic/normal 产物也精确一致。
- 四库上游 cargo tests 仍有已记录的 legacy intrinsic 阻断，未运行；不将 native/corpus 差分写成上游全套测试通过。

| 函数 | v6 字节 / 指令 / 调用点 | 本轮字节 / 指令 / 调用点 | LLVM 字节 / 指令 / 调用点 |
| --- | ---: | ---: | ---: |
| Unit::from_str | 1744 / 523 / 37 | 2028 / 618 / 16 | 675 / 187 / 0 |
| Parser::parse | 10737 / 2334 / 19 | 10737 / 2334 / 19 | — |
| Parser::parse_unit | 6230 / 1517 / 86 | 6230 / 1517 / 86 | 1093 / 293 / 9 |

这些是静态函数体指标。Unit 体积增加 284 字节、静态指令增加 95 条；21 个 `memcmp` 消失，剩 16 个切片比较 helper。最大 `sub rsp` 仍为 16。humantime 函数体字节合计 36273→36557（+0.78%），base64 22186→22185，其余两库不变；四库 `.rodata` 不变。

诊断的 24 次 refinement 和 24 次 `MemcmpZero` 是 pass 事件，其中 3 条为长度 1 比较的不可达 runtime arm，随后被 CFG 清理删除；终码机会为 21 处。新增 native 中，0–32 范围内的 14 种固定长度全部无调用，按 8/4/2/1 字节精确覆盖；33 字节保留 runtime call。Unit 的 37 个显式输入读取均落在对应长度 guard 内，16 个外部 helper 内部读取不在这份静态证明范围。完整图和逐指令证据见 [机器码归因](/Users/shom/Downloads/kfrc-layout-20261113/research/call-constants-machine-v1/machine-attribution.md)。

base64 的两个实际改写是 `Alphabet::new` 的 panic 参数，构造函数少两条指令；最终 executable 与 v6 完全相同。semver、unicode 的 executable 也完全相同。humantime 只有 Unit 对象函数体改变；35 个其他函数的 linked 地址统一后移 288 字节，解析 GOT 后归一化函数体全部相同。

## 同沙箱运行对照

v6、候选和 LLVM 均在本轮拥有的同一 E2B 中重新构建。v6 与旧验收的 50 个对象、4 个 driver、24 个 LLVM caller 精确一致；本轮 LLVM 也精确复现固定产物。采用 Rust 1.94.1、LLVM 21.1.8、8 vCPU/16 GiB、AMD EPYC family25/model17；运行固定 CPU7。

先做独立 6 轮 PMU，共 234 样本；再做 24 轮正式 wall 加 2 轮 warmup，共 1014 样本。三个实际 executable 均独立采样，包括 SHA 相同的控制臂。正反 Latin 顺序使每臂每位置有 8 个正式样本；墙钟阶段不附带 PMU，编译、传输、归档与采样分离。

比值均为候选 / 比较对象，越低越好。每负载先计算同轮配对比的中位数，再对 13 负载取等权几何平均；不是按实际应用频率加权。Q1–Q3 为本次经验四分位，非置信区间。

| 库 / 负载 | 指令 / v6 | 耗时 / v6（Q1–Q3） | 耗时 / LLVM |
| --- | ---: | ---: | ---: |
| semver / parse | 1.000000 | 1.009350（0.999945–1.043362） | 3.175431 |
| semver / req | 1.000000 | 0.981564（0.978555–0.987001） | 6.058758 |
| semver / identifier | 1.000000 | 0.998196（0.992708–1.009901） | 1.675107 |
| base64 / encode | 1.000000 | 1.000350（0.998024–1.003483） | 1.131423 |
| base64 / decode | 1.000000 | 1.000316（0.998658–1.000761） | 2.110283 |
| base64 / short | 1.000000 | 1.000814（0.996729–1.003419） | 1.708984 |
| unicode-segmentation / graphemes-mixed | 0.999999 | 0.997252（0.991799–1.003386） | 1.337254 |
| unicode-segmentation / graphemes-ascii | 1.000000 | 1.001714（0.997843–1.005570） | 1.095498 |
| unicode-segmentation / graphemes-rev | 1.000000 | 1.001414（0.998425–1.002894） | 2.814498 |
| humantime / dur-parse | 0.986395 | 0.983182（0.981106–0.990220） | 5.970957 |
| humantime / dur-format | 0.999999 | 0.900823（0.894962–0.916597） | 1.042653 |
| humantime / ts-parse | 1.000000 | 0.969168（0.967543–0.972406） | 1.689663 |
| humantime / ts-format | 0.999999 | 1.007543（1.004862–1.008830） | 1.629055 |

13 项等权平均：候选/v6 指令 **0.998947**，耗时 **0.988188**；候选/LLVM 耗时 **2.019467**。每 crate 的候选/LLVM 耗时几何平均分别为 semver **3.1823**、base64 **1.5980**、unicode **1.6035**、humantime **2.0346**；尚未达到 review 的 parity 退出线。

duration 解析是有终码变化且动态指令下降的直接收益：指令 -1.36%、耗时 -1.68%。duration 格式化 -9.92%、时间戳解析 -3.08% 的耗时变化没有相应指令或函数体变化；保留地址/对齐漂移证据，原因未证实，不把它们写成减少执行工作的收益。时间戳格式化 +0.75% 的回退全部保留。完全相同 executable 的 semver parse +0.94%、req -1.84% 也展示了本轮耗时对照的变动，未为消掉这些结果重复计时。

## 编译成本

测量对象为完整编译四个目标库，未计编译器自身构建。先独立做 4 轮 AB/BA PMU，共 32 样本，再单独做 1 warmup +6 正式 ABBA block，共 112 次有效墙钟/资源编译；编译固定 CPU0–7、8 codegen threads，关闭诊断。每次计时退出后核对完整输出对象，全部等于对应 normal 的 50 个对象引用。Layout 规则不在 metadata 路径执行，本轮没有 metadata 样本。

| 库 | 编译指令 / v6 | 修正后编译耗时 / v6（Q1–Q3） | 峰值 RSS / v6 |
| --- | ---: | ---: | ---: |
| semver | 1.002773 | 0.998583（0.985523–1.009049） | 0.999307 |
| base64 | 1.002277 | 1.010341（0.997224–1.031659） | 0.999718 |
| unicode-segmentation | 1.001809 | 1.000007（0.975697–1.016875） | 1.000864 |
| humantime | 1.004227 | 1.001269（0.995540–1.009308） | 1.000512 |

四库等权几何平均：编译指令 **1.002771（+0.277%）**，编译耗时 **1.002539（+0.254%）**，峰值 RSS **1.000100**。base64 编译耗时中位比 +1.03% 保留；本轮成本结果不表明编译更快。

编译墙钟首批 112 个样本发现仪表问题：Python 3.12.3 的 `subprocess.run(timeout=1200)` 用 `waitpid(WNOHANG)` 加最多 50ms backoff，外层耗时量化至约 0.3645/0.4146/0.4646s。静态排查所有当前 harness 和实际 stdlib 后，保留原始 144 个编译样本并明确作废其中 parent wall/derived CPU 字段；PMU、GNU time 的 user/sys/RSS 及对象 hash 保持有效。只重测了编译 wall 的 112 个样本，采用阻塞等待、外部整批 deadline 和高分辨率 monotonic 计时，并用 GNU time 的独立粗粒度 elapsed 交叉核对。未重跑 PMU 或运行负载，未改源码。运行负载正式值来自 Rust driver 的 `Instant`，不受这个问题影响。详见 [计时静态排查](/Users/shom/Downloads/kfrc-layout-20261113/validation/timing-static-audit.json) 和 [修正后编译复算](/Users/shom/Downloads/kfrc-layout-20261113/validation/candidate-call-constants-v1-compile-final-root-check.json)。

## 下一步和证据

本轮保持当前源码冻结。Unit 的 16 个余留 helper 已定位到 day/week/month/year 正常输入，并非全部冷路径；它们受内联预算拒绝。但直接提高 `max_calls=32` 或只放开小 callee preparation gate 都不能保证收益：调用实参事实尚未穿过已执行的 slice 转换，S 清理也未把分支中的动态长度事实精化到其后比较。下一步应先补可证明的上下文事实与简化，再按实际缩小结果定价。精确的配额账本、Rust/LLVM 源码对照和实现边界见 [余留调用分析](/Users/shom/Downloads/kfrc-layout-20261113/research/remaining-slice-equality.md)。

`parse_unit` 的 52 次 48 字节 memmove 是 26 对错误路径快照，不能按静态数量预计正常解析收益。后续若沿 review S3 处理，需保留原捕获时点、未初始化/指针字节、Undefined inactive arm 与快照不可变性；增大 copy 展开阈值不会消除这些重复物化。

工作树 `/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108`，HEAD `7fb99c5b057e7ce7f8481f8ce9cd2e05f2d599e6`。保留继承修改，本轮未 commit/push。1386 个输入相对 v6 有 13 个新增/变化，无删除，工作树与冻结输入逐文件一致。

| 指纹 | SHA-256 |
| --- | --- |
| v6 source tree | 8697ab4c7f887feeedbc4c3dc544c99be4d3cdf1817b61511d7fa6f54ee035f9 |
| candidate source tree | 0414327697ad2d7627be7c06ac82fcd6607f88d7d12bd8bb2c34d6b787c84de3 |
| candidate source tar | e706937c3a5172557ef1dc8c2ed70fb551a384ce9e8d58ead4c4b3cdfd8de47e |
| candidate backend | b175c3b81e76fd922b19efc410f8986778d92d53c7ee7a7b337860b0cb02764f |
| fresh v6 backend | 15a9bcdf32c16b29d489f04e954ef32127cf4ef1a1990814bef516e04938629b |

证据入口：[最终选择](/Users/shom/Downloads/kfrc-layout-20261113/validation/final-selection.json)、[本轮独立 diff](/Users/shom/Downloads/kfrc-layout-20261113/source/candidate-call-constants-v1/baseline-delta.patch)、[源码清单](/Users/shom/Downloads/kfrc-layout-20261113/source/candidate-call-constants-v1/manifest.json)、[正确性](/Users/shom/Downloads/kfrc-layout-20261113/validation/candidate-v1-correctness-root-receipt.json)、[234 PMU 复算](/Users/shom/Downloads/kfrc-layout-20261113/validation/candidate-call-constants-v1-runtime-screen-root-check.json)、[1014 wall 复算](/Users/shom/Downloads/kfrc-layout-20261113/validation/candidate-call-constants-v1-runtime-root-check.json)、[最终源码核验](/Users/shom/Downloads/kfrc-layout-20261113/validation/final-source-root-check.json)。原生验证入口为 [layout-call-constants.sh](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/scripts/poc-validation/layout-call-constants.sh)。

最终原始证据共 **8117 个成员**，已在本地逐项核对大小和 SHA，并从原始 stdout/command/perf/time 重建 234 PMU、1014 runtime wall 和 256 个编译样本（含 112 个作废旧墙钟样本）。正常二进制、两版冻结源码、完整 shared std sysroot 与测试日志均保留；重复编译产物在逐对象一致后去重，保留每次完整 hash 清单。见 [全 raw 独立复核](/Users/shom/Downloads/kfrc-layout-20261113/validation/final-raw-root-check.json)、[归档回执](/Users/shom/Downloads/kfrc-layout-20261113/validation/candidate-call-constants-v1-final-evidence.tar.gz.local-check.json) 及 [串行阶段绑定](/Users/shom/Downloads/kfrc-layout-20261113/validation/final-measurement-bindings-root-check.json)。

本任务 E2B 沙箱已释放；API 复查返回 `SandboxNotFoundException` 确认不存在，见 [释放回执](/Users/shom/Downloads/kfrc-layout-20261113/validation/release-receipt.json)。本地 control-plane 封存另外保留 launcher、上传下载回执、复算脚本、研究材料与本报告。

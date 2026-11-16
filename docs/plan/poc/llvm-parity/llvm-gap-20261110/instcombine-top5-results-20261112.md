# kfrc InstCombine 前五项：实现与最终测量

2026-11-12。已在现有工作树落地五条规则，最终 v4 完成 E2B 正确性、机器码、独立微基准、四库 workload 和编译成本验证。微基准中部分调用路径耗时降低约 11%–30%；当前四库整体没有普遍提速，编译指令数增加约 0.75%–1.32%。尚未达到 LLVM 整体水平。

本轮使用三个 subagent 分别负责规则实现、终码归因和统一 E2B 执行。编译、profile、正式计时按顺序执行，正式计时期间没有编译、上传或归档竞争；未启动独立 code review agent。

## 最终实现范围

| 开关 ID | 规则 | v4 默认启用边界 |
| --- | --- | --- |
| `mul-pow2` | `x * 2^k → x << k` | 普通整数 ≤64 位、wrapping、无旧 flags、k>0；保留与有观察用途的 unsigned64 MulOverflow 可共享的低半乘积。 |
| `trunc-binop` | 截断穿过位运算 | 仅 And/Or/Xor，K<W≤64，producer 单用；无新增 cast，或新增一次 trunc 能让原 64 位寄存器常量变成直接可编码的窄立即数。 |
| `opposite-shifts` | 等量反向逻辑移位 → mask | 两个常量先各自按原宽度取模，再比较；inner 单用，无旧 flags。 |
| `mul-shift-factor` | `x * (1 << z) → x << z` | factor 单用、普通同宽整数、无旧 flags；保留 x/z 和 masked-shift 语义。 |
| `memcmp-zero` | 仅判零相等的 memcmp → bcmp，接通短比较内联 | 仅常量长度 ≤32；intrinsic 认证 runtime 身份与整个范围可读的契约；所有传递用途经有界无环 Phi 闭包后均为 Eq/Ne 0。 |

前四条拒绝 bool/char、128 位和 restricted ScalarValidity。被复用的原输入仍携带自身 poison/flags；不把原算术承诺丢掉后继续改写。新规则只接 batch pipeline，不在每个 splice seed 重建全图分析。每轮先筛相关表达式，只有存在 scalar 候选才构造 Uses；最多三轮，新节点留待下一轮。

`memcmp-zero` 只换原 Call 的目标，保留参数、projection、control/effect；零长度也保留参数观察。认证作为 `Callee::RuntimeComparison` 从 intrinsic 转换进入 graph，普通同名用户函数不能获得认证；clone/layout/splice 保留认证，ABI/符号被修改则认证失效。`compare_bytes` 要求全范围有效和初始化，短比较展开不能依靠首个差异来放宽读取范围。

实现：[入口](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/instcombine.rs), [strength.rs](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/instcombine/strength.rs), [narrow.rs](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/instcombine/narrow.rs), [calls.rs](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/instcombine/calls.rs), [规则配置](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/transforms/scalar/instcombine/rules.rs)。

`KFRC_INSTCOMBINE_DISABLE` 接受逗号分隔的上述 ID，`all` 关闭五条规则；`KFRC_INSTCOMBINE_STATS=1` 记录成功事件。配置每进程读取一次，所有正式计时关闭诊断。全关闭配置与 v5 的 50 个正常对象和 4 个 driver 逐字节一致。

## 正确性与终码

- Workspace **1,285 通过、0 失败、1 项原有 timing 测试忽略**；31 个新增定向测试覆盖 strength 13、narrow 10、calls 8。Backend 11 项另行通过。
- 原 13 个 native 入口通过；新增入口在 v5、all-off、default、五个 only-rule 及正常/诊断构建上对拍。每个 arm 均通过 **85,706 scalar +636 byte cases**，checksum `355b491adf462af0`。driver 使用独立低位公式、u128 checked-product Option oracle 与逐字节 ordering 参考。
- 页边界的 5 个 arm 各通过 **132 个动态长度 case +30 个常量检查**，覆盖前后保护页、相等/首差/尾差。常量 0/7/32 确实触发优化，normal 与 diagnostic 对象及 driver 精确绑定。
- 七个 gate 配置的四库 corpus 与 13 个 workload 校验全部匹配 LLVM；28 组四库 diagnostic 对象绑定 normal。

| 规则 | native 成功事件 / 改变函数 | 终码结果 |
| --- | --- | --- |
| mul-pow2 | 4 /4 | IMUL→SHL；sign-bit 例同时移除 movabs，4→3 条、18→8B。 |
| trunc-binop | 3 /3 | 大常量 XOR/AND/OR 各 4→3 条、17→11B。 |
| opposite-shifts | 3 /3 | u32/u8 各少 1/2 条；u64 mask 仍需 movabs，4→4 条、12→17B。 |
| mul-shift-factor | 3 /3 | u8/i16/u64 均 9→6 条；去除 factor materialization 和 IMUL。 |
| memcmp-zero | 2 /2 | 常量 7/32 去除 libc call，转为范围内精确加载与 XOR/OR；动态比较保留 memcmp。 |

四库最终只存活 **9 个函数、13 处 IMUL→SHL**，指令/MOV/call/stack 数量均未增加。semver default 的全部对象与 driver 精确等于 v5；另外三库 default 精确等于 only-mul-pow2。其余四个 only-rule 的四库对象与 driver 全部精确等于 v5，不能把 synthetic 命中算作四库热点收益。

终码归因见 [逐函数机器码报告](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/research/machine-differences.md)，含 603 份 SHA 回执、解析后的 relocation、负例和 LLVM 对照。原始 relocation 字节会受 symbol index 变化影响，不能单独作为调用目标的证据。

## 独立微基准

同一份 LLVM caller 编译一次，产生的 3 个 caller 对象精确重链接 8 个 arm；关闭 LTO，每轮保留一个外部 noinline fixture 调用。1024、1031 与正式 iteration 数在所有 arm 对拍成功后，CPU 7 串行执行 4 轮 PMU 与 12 轮平衡墙钟测量。保存 pilot；短样本校准到约 150ms。

以下均为 default/all-off 的比值，越低越好；四分位为经验分布，不是置信区间。
| 用例 | wall 中位数 | wall Q1–Q3 | instructions 比值 | default/LLVM wall |
| --- | --- | --- | --- | --- |
| pow2-sign | 0.99995 | 0.99942–1.00094 | 0.94738 | 1.000 |
| trunc-xor | 0.99996 | 0.99817–1.00090 | 0.95001 | 1.001 |
| trunc-and | 0.99939 | 0.99446–1.00007 | 0.95001 | 0.998 |
| trunc-or | 1.00016 | 0.99970–1.00284 | 0.95001 | 1.001 |
| opposite-right | 1.00001 | 0.99967–1.00042 | 0.95239 | 1.000 |
| opposite-u8 | 0.88813 | 0.88600–0.88877 | 0.90478 | 1.041 |
| mul-shift-u64 | 0.99934 | 0.99670–0.99993 | 0.88890 | 1.000 |
| mul-shift-i16 | 0.86556 | 0.86474–0.86668 | 0.89287 | 1.097 |
| mul-shift-u8 | 0.86505 | 0.86353–0.86532 | 0.89287 | 1.096 |
| memcmp-7 | 0.70114 | 0.70014–0.70202 | 0.84268 | 1.385 |
| memcmp-32 | 0.75432 | 0.75252–0.77718 | 0.92487 | 1.424 |
| memcmp-dynamic | 0.99977 | 0.99839–1.00029 | 1.00000 | 1.103 |
| checked-mul | 0.99926 | 0.99580–1.00467 | 1.00000 | 1.761 |

反向 u8 移位约快 11.2%；dynamic-factor i16/u8 约快 13.4%/13.5%；7/32 字节比较约快 29.9%/24.6%，同时有 PMU 指令下降。常量乘法、三种大常量位运算、u32 反向移位和 u64 dynamic-factor 虽少执行指令，在这个 caller 中墙钟基本持平。

这些结果包含 caller、call、black_box、校验和与循环开销；内存输入多驻留缓存，重复的相等/差异模式可被预测。它们只支持所测调用路径。7/32 字节比较相对 LLVM 的 wall 仍为约 1.39/1.42，checked multiply 约 1.76，不能宣称达到 LLVM。

## 四库 workload

semver 最终可执行文件与 v5 完全一致，因此用精确绑定代替重复计时。default 与 only-mul-pow2 的四库 driver 全部相同，未把它们当独立候选重复测。对三个确有终码变化的库测 v5/default/LLVM：CPU 7、先独立 3 轮 PMU，再 2 轮 warmup +24 轮平衡 wall，共 780 个 wall 样本（含 warmup）和 90 个 PMU 样本。

以下为 default/v5；LLVM 比值来自同一轮输入和环境。
| 库/用例 | wall 中位数 | wall Q1–Q3 | instructions 比值 | default/LLVM wall |
| --- | --- | --- | --- | --- |
| base64/encode | 0.99847 | 0.99764–1.00104 | 0.9999996 | 1.132 |
| base64/decode | 1.00169 | 0.99894–1.00284 | 0.9999995 | 2.224 |
| base64/short | 0.99320 | 0.99020–0.99747 | 1.0000000 | 2.269 |
| unicode-segmentation/graphemes-mixed | 0.99800 | 0.98952–1.00091 | 1.0000013 | 1.341 |
| unicode-segmentation/graphemes-ascii | 1.00057 | 0.99708–1.01217 | 1.0000014 | 1.095 |
| unicode-segmentation/graphemes-rev | 0.99438 | 0.99183–0.99741 | 1.0000004 | 2.922 |
| humantime/dur-parse | 1.00559 | 1.00286–1.00868 | 0.9999998 | 7.542 |
| humantime/dur-format | 0.98341 | 0.97995–0.99219 | 1.0000002 | 1.123 |
| humantime/ts-parse | 1.00097 | 0.99775–1.00578 | 1.0000001 | 1.692 |
| humantime/ts-format | 0.99941 | 0.99549–1.00343 | 0.9999993 | 1.625 |

10 个 workload 的动态指令数均接近 1，未建立普遍减少执行工作的证据。dur-format 墙钟约快 1.66%，base64 short 与 unicode reverse 也有小幅下降，但这不等于整个库都受益。dur-parse 墙钟慢约 **0.56%**，Q1–Q3 为 1.00286–1.00868；指令数基本不变，cycles 分布跨 1，原因尚不能确定，报告保留这一回退。首版 ts-parse 约 1.82% 的回退在最终版本为 +0.10%，Q1–Q3 跨 1。

## 编译成本

测的是编译这四个目标 crate，不是编译编译器。8 个 codegen threads、全部 8 个 sandbox CPU；每对先独立 4 个 PMU cycle，再 1 个 warmup ABBA +6 个正式 ABBA block，每 arm 12 个正式样本。每个计时编译生成的 whole objects 均与 normal 精确相同。
| 库 | 配置/v5 | wall 中位数 | wall Q1–Q3 | 编译 instructions 比值 |
| --- | --- | --- | --- | --- |
| semver | default_vs_v5 | 1.01259 | 0.99615–1.02357 | 1.01149 |
| semver | alloff_vs_v5 | 1.02009 | 0.99251–1.03463 | 1.00392 |
| base64 | default_vs_v5 | 0.97688 | 0.97076–1.00297 | 1.01321 |
| base64 | alloff_vs_v5 | 1.00695 | 1.00506–1.02616 | 1.00787 |
| unicode-segmentation | default_vs_v5 | 1.00196 | 0.97714–1.04839 | 1.00754 |
| unicode-segmentation | alloff_vs_v5 | 1.00966 | 0.99544–1.01533 | 1.00223 |
| humantime | default_vs_v5 | 0.99874 | 0.99788–1.00210 | 1.00936 |
| humantime | alloff_vs_v5 | 1.01112 | 0.99904–1.02151 | 1.00339 |

最终 default 编译指令增加 **0.75%–1.32%**，首版为 1.47%–2.59%。收紧规则并加入候选预筛、延迟构造 Uses 后，总成本下降；这轮未单独拆分三者的贡献，也没有消除新增工作。wall 更波动，不把 base64 wall 的下降解释成编译器少做工作。全关闭配置也有 0.22%–0.79% 的编译指令增量，应与新增规则扫描分开看，不能把全部 default 成本归因于实际改写。

## 根据反例收紧的边界

1. 原 unsigned64 Mul 与 MulOverflow 能共享低半乘积；首版把 value 改成 shift 后，base64 `encoded_len` 从 47→49 条。最终 guard 以有界局部 Uses 查当前仍被观察的共享 overflow，支持交换操作数和等值不同常量节点。native checked_mul、`encoded_len` 均恢复原码。
2. 算术窄化在 native extended-add/const-mul 增加类型规范化指令；semver 两份 UTF-8 helper 的小常量逻辑各增加一条。最终延后 Add/Sub/Mul，需要新增 trunc 的逻辑形态必须实际省掉宽常量寄存器装载；unsigned u32 bit31=1 仍不可直接编码的常量也拒绝。两份 UTF-8 helper 恢复 39 条/147B。
3. 首版动态 memcmp→bcmp 只改变调用符号与链接布局。实测 glibc 2.39 两个 IFUNC 在同进程解析为相同执行地址；ts-parse 函数体相同、入口前移 32B。最终仅接通常量 ≤32 的 inline equality，动态 Eq/Ne/Phi 的目标保持 memcmp。

首版负证据见 [v1 完整测量](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/validation/validation-report.md)、[机器码历史章节](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/research/machine-differences.md)。历史数据保留，最终结论均使用 v4。

## 复现、范围与下一步

工作树 `/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108`，HEAD `7fb99c5b057e7ce7f8481f8ce9cd2e05f2d599e6`，有大量继承的未提交改动；本轮未 commit/push。最终源码 1,370 文件，相对冻结 v5 的增量为 17 文件，无删除。工作树全部构建输入、manifest 与 tar 内容已逐文件核对。

- v5 tree：`99302efa761c6ff781cca35cd098f6e5fe8741984b78cda478ed571c79d56d54`
- v4 tree：`a8501a9ca63d05ff6e712374ffec3e8a37200947ef44f6d42179db9058e8e2cf`
- v4 tar SHA：`b6325ee447c2df54aed079ce8521dd29624436cb8b4dd40c041e6a9403f2a1b6`
- backend SHA：`81b53a6f40ed3716a5464981e2e4ff6162c5fb8f002a5917c242538df55cfe17`

[最终源码与 manifest](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/source/candidate-instcombine-top5-v4/manifest.json)；[最终测量汇总](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/validation/final-measurement-summary.json)；[原始样本](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/validation/final-raw)；[源码绑定核验](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/source/final-worktree-check.json)。

持久 native 验证入口为 [instcombine-top5.sh](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/scripts/poc-validation/instcombine-top5.sh)。微基准 [源码与契约](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/validation/native-rule-bench.md)、guard-page 来源和运行脚本均保存。

本任务通过 HK 核对实际 0820 入口、SDK 与可用模板后，在独立 E2B `iq6w5fuar082wyynhawot`（8 vCPU/16GiB）使用 Rust 1.94.1/LLVM 21.1.8/rustc-dev 执行。基线在同一 sandbox 重建，与 v5 的 50 个对象和 4 个 driver 绑定。凭证仅作为客户端选项，未上传。本任务沙箱已释放，API 查询确认 SandboxNotFoundException；状态与回执见 [最终执行状态](/Users/shom/Downloads/kfrc-instcombine-impl-20261112/validation/progress-final.json)。

下一步应按真实热点继续选择 Top20 中的规则，并处理已经定位的后端缺口：checked pow2 的专门 lowering、LEA/窄位指令选择、移位 amount mask、短比较双重 Boolean normalization 与 SIMD。当前四库剩余大差距仍包含跨调用边的信息与物化成本，不能仅靠扩大局部代数规则的数量解决。

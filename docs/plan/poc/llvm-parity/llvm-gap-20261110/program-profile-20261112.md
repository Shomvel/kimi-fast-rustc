# 从程序 profile 定位通用 pass 缺口

本轮已完成真实程序的 PMU、调用栈采样和最终 S/L 图归因，并实现、验证了两个通用 pass 候选。**两者改变了目标机器码，但没有证明整组耗时收益，已从默认路径撤回。** 独立补丁、失败基线、正确性与性能结果均保留；后续优先处理大头热点的数据流和控制流缺口，不继续围绕小 Phi 调成本阈值。

## 输入与测量

- 源码基于 HEAD `7fb99c5b057e7ce7f8481f8ce9cd2e05f2d599e6` 加冻结 overlay；完整 source tree SHA-256 为 `dd3727e1902263ac8a16081baf38768001118597213af41e8dacf093aede4094`。
- KFRC backend SHA-256：`b061596b669df85845a26ce4fff0d8d953972a6ee9c9917a111ef3776ff7fccf`。对照为 rustc 1.94.1 / LLVM 21.1.8。
- 同一 E2B 实例，8 CPU / 16 GiB，运行时固定 CPU 7。四库、13 个冻结负载；没有改变程序源码、输入、编译 flags 或内联预算。
- 78 份 PMU 计数：每个负载双方交替 3 轮；26 份 cycles profile，499 Hz，DWARF 16384；共 37,025 个样本，36,692 个带 caller。PMU 事件 running 均为 100%。
- 快臂至少约 200 ms 的计数、1.5 s 的采样使用双方相同的放大迭代数，仅用于诊断。正式运行比分数仍使用原迭代配置。
- 74 份热函数注释、四库完整 S/L/寄存器分配日志。所有 diagnostic objects 与正常测量 objects 逐字节一致。

此前 partial SROA r4 通过 19 个针对性测试、1188 个 workspace 测试（另有 1 个既有 ignored）、11 个 backend 测试、8 组原生差分和四库 corpus/13 个负载校验，但四库产物与 parent 完全一致。正式 9 轮的 candidate/parent 几何均值 0.99835 是同一 binary 的噪声；candidate/LLVM 为 2.192768。六个独立机制家族中只有 nested copy 发生一次 partition。r4 没有在这些程序中兑现实际机器码收益，因此暂停扩展其 Phi 闭包。r4 全部结果已下载验封，归档 SHA-256 `79ccdec128ed04cceca302a8d3f4a9bcec06b9ad49b8d5024c0bd54c0ab68b7e`。

## 程序实际多做了什么

下表为每 iteration 中位数的 KFRC/LLVM 比值；cycles 比值来自诊断 PMU，不能替代正式 wall-time 比值。

| 负载 | instructions | cycles |
|---|---:|---:|
| humantime dur-parse | 6.508 | 7.528 |
| semver req | 5.315 | 6.190 |
| semver parse | 3.262 | 3.174 |
| unicode graphemes-rev | 3.059 | 2.932 |
| base64 decode | 2.453 | 2.420 |
| humantime ts-parse | 2.132 | 1.688 |
| base64 short | 2.080 | 2.253 |
| humantime ts-format | 1.779 | 1.638 |
| semver identifier | 1.643 | 2.220 |
| unicode mixed | 1.288 | 1.357 |
| base64 encode | 1.160 | 1.132 |
| humantime dur-format | 1.150 | 1.114 |
| unicode ascii | 1.045 | 1.102 |

将双方各自的 PMU 中位数折回原迭代配置，dur-parse、req 分别多执行约 548.57M、325.63M cycles，占 13 个配置批次全部额外 cycles 的 55.35%。加上 reverse、identifier、decode 后占 79.06%。这是这组配置的总工作排序，不是生产流量权重；[派生数据](results/program-profile-20261112/baseline-profile-summary.json)保留完整计数及 binary 指纹。

大头首先是指令膨胀。若把 dur-parse、req 全部差距归给额外 branch miss，每次分别需要解释约 1453、1093 cycles；计数不支持这种解释。identifier 仅多约 3309 次 miss，却多约 126.22M cycles。当前证据也不足以优先归咎 cache 或 spill。

跨编译器内联改变了符号归属，不能用两边某个函数 self share 相减预测优化收益。short、dur-format 的 unknown leaf 约 15–31%，其精确归因受限。

## 已定位到 pass 的机制

### A：小 ScalarPair enum Phi 被表示成本误判为落栈

humantime 的 UTF-8 `next_code_point`、unicode reverse 的 `next_code_point_reverse` 在最终 S 返回 `Phi<Option<u32>>`。L 却为三个 constructor 分别建立私有槽，合并地址后重新加载两路返回值。

根因在 [owned_transport.rs](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/crates/kfrc-son/src/lower_layout/owned_transport.rs)：原规则只比较 logical leaves 与 `ceil(size/8)`。`Option<u32>` 的两个 scalar leaves 因总大小 8 字节，被拿来与一个 word 比较；producer 已记录的 ScalarPair 两路 ABI 没有参与成本基准。

独立候选 A 只将 physical transport 基准修正为 `max(ceil(size/8), recorded ABI lanes)`，复用已有 scalar Phi lowering，不改变内联。它仅是运输成本基准修正，还不是完整寄存器压力模型。多 variant 重叠仍可能更适合 storage；当前默认规则保持原状。

回归用例通过真实 verify(S) → lower_layout → verify(L)，比较 S 的 variant/payload 与 L `ReturnPair`。基线已真实复现：两个小 pair 用例在 `owned_transport_phis == 0` 断言失败，实际为 1；多 payload variant 对照通过。候选 A 的三个用例全部通过。正常程序中，humantime helper 的快照槽从 3 减为 1，unicode reverse helper 的总槽数从 19 减为 17；返回 ABI 仍物化一次。它只将 dur-parse、req 的动态指令数分别减少约 0.63%、0.20%，没有兑现明显运行时收益。

### B：布局降低新造的 tag Phi 没消费已有 SCCP

base64 decode 最终 L 中，一个内层 Phi 的两个不同 Const 节点都表示 tag 1；它经另一个 Region 的 Phi 进入外层 success/error 合流。现有 threading 对当前 Region 的入边求值，无法把跨 Region 的等值 Phi 当常量，成功路径仍测试 Result tag 并搬运 inactive error payload。

`lower_layout/cleanup.rs` 已运行 private memory、InstCombine、条件和 CFG 简化，但没有 SCCP。现有 scalar SCCP 能按 bits 合并这些 Phi，并保持固定 load/call/trap 的控制路径。候选 B 在 L cleanup 的 InstCombine 后接入它，再交给现有 threading；该接入目前保留为实验补丁。

回归用例使用可观测 leaf calls 保留真实控制流，覆盖嵌套同 tag、混合 variant、固定 poison 观察及两种前驱顺序。原始 r4 中正例真实失败、两个负例通过；AB 的全部六项针对性测试通过。AB 中 base64 decode 动态指令减少约 8.27%，运行耗时却没有改善，不能把该指令降幅记为加速。

### C：私有小 copy 被拆成逐字节输出

base64 decode 每 32 输入字节的成功 CFG 静态路径，KFRC 约 542 条指令，LLVM 约 224 条；这是静态路径计数，不是动态 trace。四次 6 字节输出在 KFRC 变为 24 次 byte store，LLVM 为 8 次宽 store。每块 extraction/store 约 21 条对 3 条，共额外约 72 条指令。

S 中是 `ByteSwap → TransmuteValue<[u8;8]> → CopyNonOverlapping(6)`。L 的 `private_memory_copy.rs` 明确按 byte lanes 输出；whole-pack 能处理 array→integer，尚未覆盖 inverse pack 的 partial copy。这是通用 copy 表示缺口，可独立试验 4+2 字节分解。需要保留初始化、读写边界、重叠限制和快照时序，不能只将 byte stores 无条件合并。

### D：仅供观察的 Aggregate 使 iterator 被判逃逸

base64 的局部 iterator 地址经 FieldAddr/PointerCast 被包装进 closure Aggregate，唯一消费者是 RequireDefined；没有 call 收到该地址。MemoryFacts 与 promotion 的 use collector 仍将这种包装判为逃逸，iterator 的指针、长度、chunk size、计数留在内存，循环中反复加载已知 chunk size 32。

后续可将 closed Aggregate 的整值定义性观察归一化为原位置的组成义务，再让现有 promotion 工作。不能直接放宽任意 Aggregate 的逃逸规则，也不应由此扩展 partial SROA Phi 闭包。

### E：Transmute 的分量等价关系没有传给 S 级事实查询

semver `into_searcher` 的 scratch 已正确命中常量字符 32，encode_utf8 已写入 byte 32，长度是 Const 1；事实丢在宽指针经过 TransmuteValue 后的 `Field[1]`。因此 TryFrom<u8>/unwrap 冷路径仍存活，prepared graph 133 节点超过现有 64 节点阈值。这不能归因为常量没传入，也不应用提高阈值处理。

固定 rustc 1.94.1 的 codegen 对 wide→wide PtrToPtr 直接运输 data/meta；LLVM 的对应 requirement parse 已直接比较空格。此证据证明准备路径被 LLVM 消除，不能证明仅补本规则就能消除全部运行时差距。

合法传播需要共享的表示分量 certificate：真实完整宽指针、两侧相同字段 TyId/实际 offset/宽度、范围不重叠且在对象内。保留原 fixed transmute 的控制、effect、整值 poison 观察和目标解码，使用原 SSA snapshot。总大小相同或 ScalarPair 标签本身不足。缺布局、偏移交换/重叠、vtable pointer↔usize、MaybeUninit、union、padding 必须拒绝；poison、目标有效性失败、未执行分支、load 后覆盖、循环/Phi 的动态实例需单独验收。当前未实现该能力。

### F：较大但尚未实现的控制/表示缺口

humantime 的单位匹配包含 40 个 literal，经 32 个 SlicePartialEq 与 8 个 str wrapper 串行调用；LLVM 先按长度分派、再比较短整数。候选通用能力是 callee 条件常量返回/无效果摘要，用它先提升错长 guard；之后再评估 bytes equality 或判定树合并。直接扩大 40 次调用的内联预算不是当前方向。

unicode `GraphemeState::eq` 对一个 byte enum 重复 niche decode/读 payload，实际约 45 条指令，reverse 中 self 约 7.1%。若能证明无 padding、全初始化、表示 injective，可以将这种派生 equality 降为 raw-byte equality；不能推广到自定义 PartialEq、float 或含 undef padding 的产品类型。

## 候选的完整验收与取舍

A 使用源码树 `91aaf72b59c014be9e723304658761af34feb2e82affd83393f29a97b36b619b`、backend `b8ae2cf63142ed57a63651f690bf2d6c2738c8b0b321d3a23bc3e8e64e47c756`；AB 使用源码树 `626e600b5ba8115e7b205f9897c27cd7d53209781a51571919a30aec1400ab10`、backend `85fcfa2df747ffcabd147488dbf4ed54d3a6e3c84eff7ed7b572742fda872c54`。

- A：1191 个 workspace 测试通过，1 个既有 ignored；AB：1194 通过，同一 ignored。
- 双方各有 11 个 backend 测试、8 组原生差分、四库 corpus 和 13 个负载校验通过。既有 whole-pack 与 layout-threading 测试包含在 workspace 中。
- 最终性能对照使用**同一基线依赖库源码绝对路径**，四臂为基线、A、AB、LLVM。8 组诊断构建对象与正常对象一致；64 次完整编译也逐次通过正常对象一致性校验。
- 运行时 9 轮加 1 轮 warmup，共 520 个样本；PMU 3 轮，共 156 个样本；编译成本 3 轮加 1 轮 warmup，含 full/metadata，共 128 个样本。编译、上传及解压没有与运行计时并行。

| 指标 | A/基线 | AB/基线 |
|---|---:|---:|
| 13 负载运行时间，配对中位数的几何均值 | 1.0196 | 1.0077 |
| 四库完整编译 instructions，中位数比的几何均值 | 0.9998 | 1.0265 |
| 四库完整编译 wall，中位数比的几何均值 | 1.0030 | 1.0284 |

同一轮基线/LLVM 为 2.1986，AB/LLVM 为 2.2145。这里报告观测值，不将约 1% 的变化包装成稳定差异，也没有测 B 单独臂；AB/A 的 0.9890 只表示在 A 条件下的增量。base64 完整编译 instructions 增加约 8.35%，说明 L 上重跑分析也有成本。缺口存在、规则合法、机器指令减少，都不足以证明此处值得默认接入。

最初的 `profile-phi-fixes-v1-formal` 使用了不同依赖库源码路径，编译对象校验暴露这一差异。该轮和后续只补编译的结果完整保留，但不参与最终取舍；正式结论来自路径对齐后的 `profile-phi-fixes-v2-formal`。磁盘余量检查的一次失败发生在编译前；清理本任务已下载验封的旧构建缓存后恢复，没有改动输入或放宽验证条件。

本轮仅撤回自己新增的两处生产改动和两个实验测试文件，继承的工作树改动保留。实验测试与生产改动均在[可重新应用的补丁](results/program-profile-20261112/profile-phi-fixes-ablation.patch)中，`git apply --check` 已通过。[输入指纹](results/program-profile-20261112/ablation-inputs.json)和[完整派生结果](results/program-profile-20261112/ablation-results.json)固定补丁、源码、backend、运行和编译结果。

## 下一轮的优先顺序

| 顺序 | 热程序中的明确问题 | 通用能力与验收 |
|---|---|---|
| 1 | semver 的字符已知，但宽指针 transmute 后长度 1 事实丢失 | 表示分量 certificate 与受控字段等价传播；保留 fixed transmute，证明冷检查和准备调用真正消失 |
| 2 | humantime 的 literal 匹配反复调用比较 helper，错长判断未提升 | callee 条件常量返回/无效果摘要与判定树简化；先按长度排除，再保留必要比较 |
| 3 | base64 私有整数经 byte array 输出成逐 byte copy，局部 iterator 因观察包装假逃逸 | inverse pack 的 partial copy 合并、closed Aggregate 观察归一化；按宽 store 与循环实际 load 数验收 |

这三项来自真实热点及可指认的额外机器工作。收益仍需独立实验确认；不会以修改节点阈值、命中特定符号或只减少 IR 节点数作为完成标准。

## 证据保全与范围

- [文本、注释、完整诊断和测量 binary](/Users/shom/Downloads/kfrc-llvm-one-x-20261111/validation/program-profile-v1-thin-v1-evidence)：1264 个成员，归档 SHA-256 `825f643f4d66599ea31d83684c262786979b06a00edb1eec30ac3087cd93f33a`，已逐成员验封。
- [26 份原始 perf.data](/Users/shom/Downloads/kfrc-llvm-one-x-20261111/validation/program-profile-v1-raw-v1-evidence)：29 个成员，归档 SHA-256 `2584bdbf4a88a125864a92d120af848d2dced28f12af335627a2be5477b0dc9b`，已逐成员验封，含环境指纹。
- graph 文本以 `hot-graphs-v2.json` 为准；旧提取器误识别格式而生成的空 `hot-graphs.json` 不作为证据。
- llvm-study 固定 LLVM 21.1.0，测量对照为 LLVM 21.1.8，两者不混用。
- 四库 corpus 与冻结 workloads 是语义/性能回归，不等于 upstream 全套；已有 intrinsic blocker 仍未解决。

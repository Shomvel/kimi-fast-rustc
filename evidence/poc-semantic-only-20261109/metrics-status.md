# 指标状态与已验证边界

- 独立 E2B `ibneicdujhnbif39umsgi`，Rust 1.94.1、LLVM 21.1.8，8 vCPU / 16 GiB。最终资源与源码指纹以各候选 receipt 为准。
- 共享 LLVM std/alloc/core 已用 O3、CGU16、MIR opt-level=0、panic=abort 构建，两编译器通过同一 sysroot 对照。日志与库哈希在 `metrics-environment/shared-std-records/`。

## 已完成的 semver 对照

candidate08a 已通过真实 semver 34 项上游测试和工作负载。对照 candidate04，固定同一 E2B、原版 semver 1.0.28、release 后端、共享 MIR0 std；交替编译 5 轮、预热后运行 7 轮，校验和一致：

| 指标 | candidate04 kfrc | candidate08a kfrc | 同轮 LLVM |
| --- | ---: | ---: | ---: |
| 编译 wall 中位数 | 3.08 s | 0.79 s | 0.26 s |
| 库对象 text | 3,931,990 B | 1,142,645 B | 18,515 B |
| parse 中位数 | 362.417532 ms | 170.882535 ms | 2.827172 ms |
| req 中位数 | 600.099142 ms | 377.502153 ms | 2.306392 ms |
| identifier 中位数 | 229.515312 ms | 154.476644 ms | 9.543893 ms |

改善已实测，但仍远未满足性能目标。candidate08a 的其他正确性扩展仍有未完成项，详见其 acceptance receipt；不能把 semver 验收推广为全 Rust 支持。

`Version::from_str` 相同源码符号的布局节点从 34,517 降到 12,689，If 从 1,871 降到 607，Store 从 686 降到 158，Load 仍为 258；该函数 text 从 262,604 B 降到 82,658 B（LLVM 917 B）。优化后的 S 节点反而从 1,067 增至 1,129，说明此次主要收益确实来自降低 L 层膨胀。全库 text 符号数从 1,013 降到 250（LLVM 81）。

原始样本在 `results/candidate04/semver-performance-02/` 和 `results/candidate08a/semver-performance/`；原始图、反汇编和汇总在各候选 `graph-attribution/`，总对照 `candidate04-to-08a-comparison.json`。

## 指令计数机制

- 旧 `KFRC_PROFILE` 是每线程 pinned instructions:u 分阶段窗口，已通过 170 个窗口机制检查；用于归因，但阶段总和不覆盖完整编译后端。
- Rust `-Zself-profile-counter=instructions:u` 只覆盖创建线程，实际 LLVM worker 事件为零，即使单 rustc 线程或 no-parallel-backend 也无效。保留原 .mm_profdata/JSON，不能把这些零当作实测 LLVM 成本。
- 新 LLVM wrapper 委托真实 LLVM 后端，统计 `codegen_crate` 入口到 `join_codegen` 返回。既有线程和新建/孙线程探针均通过，`enabled=running`。在 candidate04 semver 上连续三轮所有对象 SHA 与未包装 LLVM 完全一致，计数约 1.668B。证据在 `results/llvm-wrapper-probe/` 与 `results/llvm-wrapper-transparency/`。
- kfrc `KFRC_PROFILE_TOTAL` 使用同一 counter 源码，关闭嵌套分阶段窗口，覆盖同一入口到 join 边界，包含 collector 和 MIR 转换，不包含链接。因此这是完整 codegen 窗口，不能冒充排除 MIR 转换的指标。
- candidate07 已通过的 arithmetic 语料上，release 后端与 wrapper 交替 3 轮，所有计数 `enabled=running` 且对象字节与各自无计数版本一致。中位数 LLVM 39,337,032、kfrc 117,561,291（2.989 倍）。它是 O0 arithmetic 计数机制探针，不是 semver 或完整 PoC 性能验收。证据在 `results/candidate07/arithmetic-total-probe/`。

正式计时与 profile 独立执行；没有用 self-profile 时间占比推算指令，也没有混合不同宿主机的绝对耗时。

- candidate08a semver 正式配置已完成相同完整窗口的 3 轮 profile：LLVM 1,668,127,787、kfrc 6,819,119,404 指令中位数（4.088 倍）。所有窗口 enabled=running，各 arm 的对象与自身无 profile 版本逐字节相同。证据 `results/candidate08a/total-profile/`；仍含 collector/MIR 转换，不是排除 MIR 的指标。

## candidate09 增量

相同配置 5/7 轮且 semver 原 34 tests/arm 与 workload 通过。kfrc 编译 0.76 s、text 1,134,533 B，parse 167.356461 ms、req 376.671748 ms、identifier 154.788157 ms。相对 08a 的改进很小（text -0.71%，parse -2.06%，req -0.22%，identifier +0.20%），不能称为显著整体收益。

`from_str` S 优化后仍为 1,129 nodes，L 12,473 nodes，Refine 531→280，但 If607、Load258、Store158 未变；text 80,781 B。完整 codegen 窗口中位数 kfrc 6,785,016,333 / LLVM 1,668,383,450（4.067 倍），计数有效且机器码透明。MU on 另有 partition/libcall 正确性失败，详见 candidate09 acceptance；这不改变独立 semver 样本，但不能宣称该候选全面验收。

## candidate10 增量

全部机制测试、strict generic/nominal/MU/发射三臂、semantic三臂、CLI、semver原34tests/arm、7suite旧语料通过。W01–05/08/09三臂通过；W06浮点与W07/W10原子intrinsic边界、regex caller-location适配失败保留，未声明全W或regex验收。

相同5/7配置kfrc编译0.65s，text1,057,726B，parse141.190757ms、req352.154773ms、identifier141.969767ms。完整窗口kfrc5,192,455,564/LLVM1,668,287,251（3.112倍），对09下降23.5%；counter有效且对象透明。

from_str S1129→851（Observe73→18、PointerCast75→18、WidePtr24→0），L12473→7753（If607→267、Observe545→31），text80781→63455B；Load258/Store158未变。S观察去重确实减少下游分支，剩余主要负担仍需继续解释和验证。

## candidate11：旧协议冻结基线

源码归档 SHA `b69fb91feeaf0db7776a0da5b21fd5325123aca5903ec6dc1000feed7216d356`。这是 ObserveValue/Refine 重设计之前的 identity call folding 候选，不能计为新要求／证明契约的验收。

Linux 63 个测试套件共 579 pass、1 ignored；strict generic/nominal/MU/发射三臂、semantic 381 行三臂、CLI、semver 原 34 tests/arm 与 workload 通过。candidate11 未重跑完整旧语料、W01–10 和 regex；这些范围的最新证据仍是 candidate10，不能继承为 candidate11 已通过。

同一 E2B、相同共享 MIR0 std、O3/CGU16/ThinLocal、交替编译 5 轮、预热后运行 7 轮，校验和一致：

| 指标 | candidate10 kfrc | candidate11 kfrc | candidate11 LLVM |
| --- | ---: | ---: | ---: |
| 编译 wall 中位数 | 0.65 s | 0.64 s | 0.25 s |
| 库对象 text | 1,057,726 B | 1,030,574 B | 18,515 B |
| parse 中位数 | 141.190757 ms | 150.178035 ms | 2.804150 ms |
| req 中位数 | 352.154773 ms | 351.744843 ms | 2.200874 ms |
| identifier 中位数 | 141.969767 ms | 142.567721 ms | 9.413092 ms |

text 减少 2.57%，但 parse 慢 6.37%，其他模式近似持平，不能称为整体运行时收益。独立 3 轮完整 codegen 窗口：kfrc 5,223,361,663 / LLVM 1,668,150,398（3.131 倍）；kfrc 比 candidate10 增加 0.60%。所有根计数 enabled=running、总计数为正，各 arm 对象与自身未计数版本逐字节一致。窗口仍包含 collector/MIR 转换，不是排除 MIR 的正式指标。

from_str S532→845、Call37→31、Observe18→24；L7753→6133、If267→213、Load258→180、Store158→80、Observe31→109，Refine 仍为 280。函数 text63455→51116 B（LLVM917 B），库 text 符号256→252（LLVM81）。减少 ABI 物化同时增加观察工作，节点减少不能替代运行时间证据。

完整回执 `candidate11-acceptance.json`；原始结果 `results/candidate11/`，包括 graph-attribution、semver-performance 的反汇编和 total-profile。归档 SHA 已写入 `artifact-hashes.json`；历史候选和失败记录保留。

## candidate12：要求擦除第一批

冻结 SHA `e69122714140141678153b4eb1b2133bb5d9e20b3834990d6f28c4212e1fb970`，包含源码与设计文档。第一批实现 RequireDefined 无业务结果、R/S Ignore 保留源码值，以及布局需求分析前擦除要求；第二批 Witness/Assume 尚未计入该候选。

本机 563 tests，Linux 66 suites、592 tests、1 ignored；feature 三臂、semantic 381 行三臂、CLI、原 semver 34 tests/arm、全部 7 suite 394 fixtures/1295 vectors/19 traps 三臂通过。完整旧语料源码哈希与 candidate10 一致。W/regex 本轮未重跑，其最新证据仍为 candidate10，已知浮点、原子和 caller-location 边界未宣称修复。

相同 5/7 配置，kfrc wall 0.64 s、text 998,290 B、parse 145.098092 ms、req 340.290392 ms、identifier 139.921680 ms。LLVM 同轮为 0.26 s、18,515 B、2.801145/2.141516/9.459812 ms。对11 text -3.13%、三种 runtime 分别 -3.38%/-3.26%/-1.86%；parse 仍比10慢2.77%。独立3轮完整窗口 kfrc5,021,183,569/LLVM1,667,852,731（3.01057倍），kfrc比11减少3.87%；计数有效且机器码透明，仍含collector/MIR转换。

from_str S仍532→845，24个RequireDefined；L6133→5216，If213→150，观察/要求节点归零，Load180/Store80不变，**Refine280尚未迁移**。函数text51,116→48,229 B，库text符号252→250。第一批有实测收益，不能算完整新设计或性能目标验收。

同一 from_str ELF 符号的静态机器计数如下。按真实符号字节范围计数，排除末尾对齐；显式内存读写不含call/ret/push隐式栈访问。rbp mov只是栈帧访问代理，不能直接称为spill。

| 机器指标 | candidate11 | candidate12 | LLVM12 |
| --- | ---: | ---: | ---: |
| 总指令 | 9,947 | 9,314 | 232 |
| mov（不含movzx等） | 8,055 | 7,745 | 62 |
| mov系列 | 8,362 | 7,989 | 78 |
| 含rbp内存操作数的mov | 4,297 | 4,103 | 0 |
| 条件分支 | 213 | 150 | 25 |
| 无条件jmp | 142 | 142 | 20 |
| 显式内存读指令 | 2,211 | 2,080 | 29 |
| 显式内存写指令 | 2,426 | 2,363 | 30 |

原始机器计数及反汇编哈希在 `results/candidate{11,12}/semver-performance/from-str-machine*.json`。计数脚本 `scripts/poc-validation/machine-attribution.py` 对未知内存操作报错，不猜测访问方向，并核对解码字节之和与ELF符号大小一致。完整回执 `candidate12-acceptance.json`，归档及派生计数哈希见 `artifact-hashes.json`。

## candidate13：凭据与 Assume 验收收尾

冻结 SHA `5316798289d820c4aa9443043197095f592bcc4e4791544b5e894511071067ca`。源码实现无 ObserveValue/通用 Refine/pin_tree；VariantWitness 是证明，Assume 是固定源要求。真实 MIR、intrinsic 和 generic definition 入口均纳入冻结版本。该候选是本轮收尾验收，不继续追逐旧优化目标。

本机 579 tests，Linux 67 suites、608 tests、1 ignored；包括 requirements_release 的精确机器序列不增断言和 semantic_refinement 的真实空指针分支放置测试。三臂 feature、semantic 381 行、CLI、原 semver 34 tests/arm、完整7suite语料全部通过。真实 assert_unchecked/direct intrinsic/generic Assume fixture 只执行满足契约的输入，检查 R/S 保留 Assume，L 无要求/证明节点。

生成式入口 `crates/rustc_codegen_kfrc/tests/p0-scalar/generated/run.sh` 运行种子1–5，每种子40函数、每函数6输入，共200函数/1200向量。优化关闭与开启分别同LLVM对拍；各臂生成的源码哈希一致、每种子240行输出完全一致。生成器限制移位范围、循环次数、排除除法，关闭溢出检查；不把native UB当作一致性验收。原始源码/输出在 `results/candidate13/generated/`，逐种子哈希见其 `summary.json`。

固定同一E2B和共享MIR0 std，编译5轮、预热后运行7轮。下表LLVM列为13同轮结果；完整编译指令另跑3轮，所有根enabled=running且对象与无profile版本一致。窗口包含collector/MIR转换、排除链接，**不是排除MIR转换的正式指标**。

| 指标 | candidate11 | candidate12 | candidate13 | LLVM13 |
| --- | ---: | ---: | ---: | ---: |
| 编译wall中位数/s | 0.64 | 0.64 | 0.67 | 0.26 |
| 库text/B | 1,030,574 | 998,290 | 983,122 | 18,515 |
| 编译窗口指令 | 5,223,361,663 | 5,021,183,569 | 5,563,263,785 | 1,667,909,650 |
| parse/ms | 150.178035 | 145.098092 | 144.057114 | 2.841176 |
| req/ms | 351.744843 | 340.290392 | 354.594890 | 2.221866 |
| identifier/ms | 142.567721 | 139.921680 | 138.492920 | 9.531582 |

13对12库text减少1.52%，parse减少0.72%、identifier减少1.02%，但req增加4.20%，编译wall0.64→0.67s、完整窗口指令增加10.80%。parse仍比10慢约2.03%。正确性和表示结构已验收，**不是全面性能改善，也不宣称正式性能门槛达成**。

同一from_str符号如下；机器统计仍是静态计数，显式内存不含隐式栈流量，rbp mov不等于已证明spill。

| from_str指标 | candidate11 | candidate12 | candidate13 | LLVM13 |
| --- | ---: | ---: | ---: | ---: |
| S输入/优化后节点 | 532/845 | 532/845 | 532/845 | — |
| L节点 | 6,133 | 5,216 | 4,859 | — |
| L If | 213 | 150 | 150 | — |
| L Load/Store | 180/80 | 180/80 | 180/80 | — |
| L Observe或RequireDefined | 109 | 0 | 0 | — |
| L Refine | 280 | 280 | 0 | — |
| 函数text/B | 51,116 | 48,229 | 45,111 | 917 |
| 机器指令 | 9,947 | 9,314 | 8,810 | 232 |
| mov | 8,055 | 7,745 | 7,241 | 62 |
| rbp内存mov | 4,297 | 4,103 | 3,751 | 0 |
| 条件分支/无条件jmp | 213/142 | 150/142 | 150/142 | 25/20 |
| 显式内存读/写指令 | 2,211/2,426 | 2,080/2,363 | 1,911/2,180 | 29/30 |

13的S含35个VariantWitness和24个RequireDefined；L无Witness/Assume/RequireDefined。证明不再复制整树确实减少搬运，编译验证/分析开销仍有回归，二者同时保留。

完整回执 `candidate13-acceptance.json`，归档与派生证据哈希在 `artifact-hashes.json`。W01–10和regex本轮未重跑，candidate10已知浮点/原子/caller-location边界保留。所有原始归档已取回并核验，不改写此前候选。

本任务E2B沙箱 `ibneicdujhnbif39umsgi` 已在2026-11-09 06:52:22 UTC释放；本机和HK归档保留。释放回执见 `sandbox-release.json`。

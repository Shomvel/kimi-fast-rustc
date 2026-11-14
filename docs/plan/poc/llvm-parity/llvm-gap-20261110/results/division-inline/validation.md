# 常量算术与按引用导入内联数据

2026-11-10。基线 `ae096c7`，继续 PR #8；没有提高全局内联预算，没有更改 cost model，没有外部 solver。完整源码指纹见 [source-manifest.json](source-manifest.json)。

## 实现与合法性

1. **常量移位**：先按 Rust IR 位宽对常量计数取模，再保留立即数到机器选择；动态计数和 `AMOUNT_IN_RANGE` 合同路径保留。8/16/32/64 位三种移位、超范围计数均做差分。
2. **常量商余数**：在固定 Div/Rem 原来的调度位置选择乘积高半部、修正加法和移位；按 64 位规范化输入计算，输出按 Rust 类型规范化。无符号非零除数、有符号正除数受到支持。负有符号除数、除零和动态除数保留硬件除法；EXACT 仍走原路径。负 numerator 用无符号绝对值及符号恢复，`MIN / 正数` 不溢出。没有在 S/L 层删除 poison 观察合同。
3. **商/余数共享**：仅在同一基本块、相同 SSA numerator、常量 divisor 和 signedness 下复用商。进入新块清空，避免跨兄弟分支和回边复用；余数按 `n-q*d` 生成。它仍不是完整机器 CSE 或 SCEV。
4. **乘积高半部**：新增虚拟与机器操作，实际分配器显式约束 RAX 输入、RDX 输出和 RAX/RDX clobber。补齐直接编码器、汇编、解释器、旧 emitter 和参数保存；六个 live 参数、同值双输入有负例覆盖。
5. **内联数据引用闭包**：原来预检与 splice 遍历整个 callee 数据表，已死的 panic location 等本地对象仍可触发 `DataConflict`。现在从被克隆节点中的符号地址、调用目标、Const memory partition 出发，再沿重定位闭包导入；导出定义仍保留。活的本地身份数据、可写数据、通过匿名常量间接引用的可写数据仍不能复制；不启用通用 anonymous constant copying。

参考用户 LLVM study 的指令选择、合法性及内联章节，并核对 LLVM 21.1.0 [DivisionByConstantInfo.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.0/llvm/lib/Support/DivisionByConstantInfo.cpp)。源文件 SHA-256 `71d69f3ae0b6eb631eac5cdf51b012d92f24a587a39c2c78d7d612adb67dfde0`。这里独立实现同样的 unsigned multiply-high/add/shift 形式，以 u128 直接计算倒数；没有逐行复制 APInt 递推，也未实现 LLVM 的全部 signed magic、known-leading-zero、even-divisor 策略。

## 实验设计

同一 HK E2B Linux x86-64 实例，8 vCPU / 16 GiB，Rust 1.94.1 / LLVM 21.1.8，共用重新构建的 MIR0 sysroot，O3 / CGU16 / panic abort。库后端变化，共同驱动和 std 由 LLVM 编译。所有 crate 包按 suite.json 的 SHA-256 校验；既有四库 13 负载输入保持一致。

消融臂：`baseline` 旧 KFRC、`shift` 仅常量移位、`candidate` 加常量除法、`shared` 加商余数共享、`data` 再加内联数据引用闭包、`llvm` 对照。先做 checker=1 的 library/corpus/checksum，后做串行交替计时；编译测试与运行计时不并行。

先后三组每负载预热后 21 轮。最后因部分小差异/波动，再将运行迭代数扩大 4 倍，预热后 31 轮交替 `llvm/baseline/shared/data`。报告配对耗时比中位数和 10,000 次 percentile bootstrap 区间，seed=1110；总体为各负载配对比率的几何平均。单个机器上的区间不能代表跨硬件泛化。

## 最终运行结果

31 轮、4 倍工作量。比率 <1 为加速；CI 为本次配对 bootstrap 95% 区间。最终 `data` 相对旧 KFRC 几何平均 0.95146，耗时降低 **4.85%**；相对 LLVM 为 **3.327 倍**。没有相对旧基线稳定慢 >3% 的负载。

| crate | 负载 | 最终/旧 KFRC | 95% CI | 最终/LLVM |
|---|---|---:|---|---:|
| semver | parse | 0.9205 | [0.8893, 0.9239] | 4.055 |
| semver | req | 0.9794 | [0.9759, 0.9846] | 7.438 |
| semver | identifier | 0.8646 | [0.8614, 0.8691] | 1.699 |
| base64 | encode | 0.9846 | [0.9807, 0.9902] | 9.710 |
| base64 | decode | 0.9794 | [0.9680, 0.9893] | 8.135 |
| base64 | short | 0.9803 | [0.9778, 0.9826] | 4.436 |
| unicode-segmentation | graphemes-mixed | 0.9929 | [0.9895, 1.0056] | 1.340 |
| unicode-segmentation | graphemes-ascii | 0.9964 | [0.9846, 1.0032] | 1.091 |
| unicode-segmentation | graphemes-rev | 0.9768 | [0.9657, 0.9786] | 3.760 |
| humantime | dur-parse | 0.9843 | [0.9780, 0.9872] | 10.245 |
| humantime | dur-format | 1.0059 | [0.9916, 1.0091] | 1.164 |
| humantime | ts-parse | 0.9872 | [0.9785, 0.9972] | 3.145 |
| humantime | ts-format | 0.7524 | [0.7517, 0.7540] | 1.655 |

新增内联数据规则相对 `shared` 有明确取舍：semver identifier 约改善 13.4%，而 duration format 从约 104.6 ms 变为 108.7 ms（中位耗时之比约慢 3.9%；逐轮配对耗时比中位数为 1.04777，95% bootstrap 区间 [1.04363, 1.05251]，约慢 4.8%）。最终相对旧基线为 1.0059，区间覆盖 1。保留此通用修复，但不声称每个增量都单调变快；已证实核心 fmt 机器字节相同，尚未完成链接布局/动态事件归因。semver req 相对 `shared` 也失去部分收益，最终仍比旧基线约快 2.1%。

| 库 `.text`（B） | LLVM | 旧 KFRC | 最终 | 最终/旧 |
|---|---:|---:|---:|---:|
| semver | 18,515 | 34,148 | 34,052 | 0.9972 |
| base64 | 8,648 | 29,635 | 28,778 | 0.9711 |
| unicode-segmentation | 8,398 | 18,602 | 18,490 | 0.9940 |
| humantime | 10,426 | 57,461 | 58,709 | 1.0217 |

## 机器码与取舍

- base64 `internal_encode`：6,079 → 5,629 B，1,398 → 1,287 静态指令，43 → 0 个 CL 移位；其 bounds-panic/调用链仍未充分消除。LLVM 为 1,376 B / 327 指令。
- base64 `decode_chunk_8`：1,604 → 1,542 B，392 → 374 静态指令，8 → 0 个 CL 移位。
- humantime timestamp fmt：3,226 → 4,265 B，808 → 1,027 静态指令，61 → 0 个硬件除法。静态指令数增多但运行约快四分之一，说明条数无法替代运行成本。LLVM 为 1,759 B / 431 指令。
- `shared` 相比不共享商的 `candidate`，humantime `.text` 减少 320 B；但仍比旧基线更大。最终代码体积见结果表。
- duration format 的核心 1,086 B 函数在 `shared/data` 间字节及调用目标相同，SHA-256 `5d563e3aaad7ace745725112e4eba5d3cd135a8101e276b0196bbd281fb8b6cb`。若运行差异持续，不把它解释为核心指令变化；链接布局与动态采样尚需进一步区分。

## 编译、链接产物与正确性

最终 21 轮完整 rustc→rlib 交替对照，预热一轮，warm filesystem cache，8 个 codegen worker、checker=0；首轮对象逐字节核对 checker=1 的计时产物。相对本轮旧 KFRC 的墙钟比率几何平均 **0.99012**，约降低 0.99%，不能据此宣称达到 50%。此前 21.7% 的并行收益是另一批实验，不与本轮相乘。

| crate | 最终/旧墙钟 | 95% CI |
|---|---:|---|
| semver | 0.9881 | [0.9727, 1.0038] |
| base64 | 0.9837 | [0.9704, 1.0176] |
| unicode-segmentation | 0.9806 | [0.9645, 0.9956] |
| humantime | 1.0083 | [0.9997, 1.0230] |

同批 LLVM/旧 KFRC 几何平均 1.00284。humantime 最终编译仍约慢于 LLVM 20%，不能靠其他 crate 平均抵消单库门槛。CPU、sys 和峰值 RSS 原始统计见 `compile-summary.json`；没有冷缓存、Cargo 整体吞吐或 50% 编译收益结论。

相同 GNU strip 对共同驱动处理，最终字节数 / LLVM：semver 423,320 / 396,416（1.068×），base64 416,408 / 390,952（1.065×），Unicode 528,632 / 499,720（1.058×），humantime 454,576 / 391,552（1.161×）。完整驱动含共同 std，不能替代库 `.text` 或小 API 消费者；后者尚未新增。

- 本机 workspace **825 passed / 1 ignored**，Linux 最终源码 workspace **859 passed / 1 ignored**。Linux 补测的原生编码商余数与 C 对照、随机除数、同值乘法输入、跨兄弟分支复用和活跃参数均通过。
- 8 位普通商余数穷举，16/32/64 位边界及随机机器解释差分；20,000 个随机 u64 除数的倒数算法差分；原生 C 驱动每个函数 4,096 个输入。保留除零、动态和负 signed 除数路径检查。
- semantic-ir 三臂各 **824 行**一致，跨 ABI aggregate、enum/niche、借用/索引及 O1 权限 guard；scalar **565 行**一致。
- 四库 corpus 和 **13 workload checksum** 与 LLVM 一致。所有正式库构建开启寄存器分配 checker；最终源码关闭 checker 后生成的 50 个对象与被计时 data 臂完全一致。
- 最终上游集成测试：semver **34**、base64 **13**、Unicode **8**，全部通过。base64 unit 被 fabsf64、Unicode unit 被 atomic_xsub、humantime unit 被 atomic_load 阻断；相同阻断已在基线复现。LLVM 对照完整通过 34 + 13 + 179 + 8 + 15 + 38 项。未把阻断当作通过。
- 一次 Linux 全套中的 PGO mock rustc 遇到 `ETXTBSY`；日志保留，确认属于脚本执行时文件忙后，以单线程测试重跑完整 workspace 通过。计时进程并未与这些测试重叠。

最终后端 SHA-256 `9a7d841e2fdb1564560ce0b476c2545b61039e0a3fd90fa7f46274933b94cf1b`；运行计时后端 `599780b0f72ac44199f09bca3ff0779cb5df6a7ac5e66ab27403645172115308`。差异为最终源码的空数据表 fast path 和测试/格式整理；四库全部对象一致检查是二者运行数据可复用的依据。

## 仍未完成的差距

这批是 B1/H3 和一项有证据的 IPO 修复，不是四库收敛路线图全部完成。`parse_unit` 仍为 23,799 B / 5,674 指令；正式观察日志确认其 28 个 `Try::branch`、17 个 `ok_or`、9 个 OverflowOp::div 都因 budget 留下，两个 add_current 因 size 留下。Unit::from_str 的字符串比较在 1,998/2,000 已添加节点后耗尽预算；slice equality 68 节点超过 64 节点门槛。详见 [inline-hot-rejections.json](inline-hot-rejections.json)。

下一项应为有界 callee-first 包装化简和 seam 上的 enum/aggregate 消除，保留 drop、effect、错误 payload 与 poison；不以全局扩大预算代替设计。base64 的范围证明、布局后地址清理，Unicode reverse 的调用链消除仍未完成。运行接近 LLVM、默认 release/保留输入集验收和完整编译时间降低 50% 都仍未达到。上述延迟收益在同一机器、既有 13 个负载上成立，不替代独立保留输入集。

## 证据与复算

仓库保存 [31 轮运行汇总](runtime-summary.json)、[运行原始数据](runtime-raw/)、[21 轮编译汇总](compile-summary.json)、[编译原始数据](compile-raw.json)、[库体积](library-sizes.json)、[链接产物体积](artifact-sizes.json)、[上游测试结果](upstream-tests.json)、[旧基线测试阻断](baseline-test-limits.json)及 [workspace 测试汇总](workspace-tests.json)。`runtime-summary.json` 沿用实验脚本中的 `candidate_over_llvm` 字段名，此处 candidate 指最终 `data` 臂。静态函数数据见 [static-hotspots.json](static-hotspots.json)。

在仓库根目录运行以下命令即可从提交的原始数据复算两份汇总；本次已核对复算 JSON 与归档汇总完全一致：

```sh
python3 docs/plan/poc/llvm-parity/llvm-gap-20261110/results/division-inline/scripts/summarize-runtime.py docs/plan/poc/llvm-parity/llvm-gap-20261110/results/division-inline /tmp/kfrc-runtime-summary.json
python3 docs/plan/poc/llvm-parity/llvm-gap-20261110/results/division-inline/scripts/summarize-compile.py docs/plan/poc/llvm-parity/llvm-gap-20261110/results/division-inline /tmp/kfrc-compile-summary.json
```

`scripts/compile-data.py` 保留实际 Linux 编译实验入口，`scripts/check-objects.py` 保留对象比对入口；二者依赖证据包中的 suite、源码及 `/workspace` 实验布局，不能作为本机开箱即用脚本。运行配置及四倍工作量见 [runtime-config.json](runtime-config.json)，原始 suite、输入补丁、全部消融与测试日志保存在完整证据包中。

完整证据包：`/Users/shom/Downloads/kfrc-divshift-20261110/evidence.tar.gz`，83,325,308 B，SHA-256 `69420198a6eaf347213f632598bf81582be97a112c3430d225692867f37f5ad7`。安全解包后按 `results/SHA256SUMS` 核验全部 6,090 个文件通过。另保存本机测试日志 `/Users/shom/Downloads/kfrc-divshift-20261110/macos-final-workspace.log`。大体积二进制归档不提交 Git。

本任务 E2B 沙箱 `icnn5xkfphq8fb8podxfc` 已于 2026-11-10 08:26:29 UTC 释放；回执见 [evidence-receipt.json](evidence-receipt.json)。

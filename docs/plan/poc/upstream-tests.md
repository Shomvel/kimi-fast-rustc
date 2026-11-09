# rustc 原版测试批量接入

最新 candidate19 已完成固定 v3 清单的第六轮验证：**144/144 份源文件、
145 个官方 revisions 全部通过**，包含函数 9 项和控制流/循环 25 项。
同一 427 候选池的 90 BLOCKED、193 EXCLUDED 保持独立记录；没有失败或
暂缓。各轮原始结果分别保留，最新通过数来自完整重跑。

固定源码与工具版本均为 rustc 1.94.1 commit
`e408947bfd200af42db322daf0fadfe7e26d3bd1`。测试源码、辅助 crate、mod/include
依赖及预期输出保留上游路径和原始字节。目录为
`crates/rustc_codegen_kfrc/tests/rustc-upstream/`；`manifest.json` 记录来源、
归档 SHA-256、逐文件 SHA-256、原始指令和选择原因。

首批候选池是在清单列出的语义目录内，全部明确标注 run-pass、build-pass、
run-fail 的 427 份主测试。最初选择 160 份，静态能力阻塞 73 份，批次配额外或
依赖当前不支持协议的 194 份单独列出。测试相关文件共 467 份，另有 98 份
官方执行器初始化资源；这些数量是迁入
和选择数量，不能当作通过数。完整 rustc 测试树的其他目录尚未迁入。

## 执行方式

使用同一固定版本的官方 standalone compiletest。它不要求 bootstrap
驱动，但需要提供原 bootstrap 通常填写的配置。`compiletest.py` 仅准备
原工具源码、构建和填写配置，不修改工具的 Rust 源码。edition、revisions、
auxiliary crate、目标限制、运行参数、环境变量、预期输出和测试判定由
官方执行器处理。`protocol.py` 用于清单规划和显示阻塞原因，不是其替代品。

先检查来源和执行计划，无需编译 Rust：

```sh
python3 scripts/rustc-upstream/run.py --out /tmp/rustc-plan --plan-only
```

官方工具的准备、构建入口见 `scripts/rustc-upstream/README.md`。在原生
x86_64 Linux、配有 rustc-dev 的 Rust 1.94.1 环境中，使用新结果目录运行：

```sh
python3 scripts/rustc-upstream/run.py \
  --compiletest /absolute/compiletest \
  --backend /absolute/librustc_codegen_kfrc.so \
  --out /absolute/fresh-results
```

`--family integer` 可限定能力组，`--limit 10` 可限制实际排入的源文件数量。
未请求、暂缓、阻塞和官方 ignored 的项目都进入报告。一次官方调用覆盖
一份原始文件的全部 revisions，不把文件过滤器误当作 revision 过滤器。

外层分别执行 LLVM 和 kfrc。运行包装器原样转发程序的 stdout、stderr 和
退出状态/信号，并保存原始字节；差分只归一化适配器控制的构建路径和二进制
路径，不删除 PID、地址或任意数字。kfrc 一组要求所有 codegen 对象带 `FILE kfrc`
来源标记，包含经过同一后端编译的 auxiliary。rustc 链接器另外生成的
`rustc*/symbols.o` 只有在逐个 ELF section 确认没有非空可执行内容后才归为
链接元数据，保留原始符号表与 section 表；不把它计作 kfrc 生成的代码。
std 仍使用预编译的 LLVM 代码。
运行前后都核对导入文件哈希。

## 范围与结果

两组额外编译配置均明确记录：panic=abort、prefer-dynamic=no、save-temps、一个 codegen unit、
无 debuginfo、MIR opt level 0。原测试的参数仍由 compiletest 解释；与产物
捕获或当前后端要求冲突的参数显式阻塞，不改写测试来规避。

当前后端仅支持 abort。固定版本官方 UI run-fail 在可 unwind 的目标上要求
非 crash 的失败状态，不能把 abort 信号当作该协议下的通过。因此这批 run-fail
原样迁入，单独记录 profile 阻塞；已有 PoC panic 差分另有明确的 abort 契约。
浮点、SIMD、原子、unwind 等也保留具体阻塞原因。官方 backend capability
枚举不能表示 kfrc，依赖 ignore-backends/needs-backends 的测试暂不执行。
官方 Linux 默认把普通 auxiliary 编为 dylib；与当前 abort/static profile 冲突
的依赖按原始指令递归判断并明确阻塞。只有原辅助源码本身要求的静态模式才
按该契约执行，不注入测试专用指令。

每个能力组第一次失败后，后续同组标记 `DEFERRED_FAMILY`，先排查同类实现，
补回归并修复，再重新执行该组。LLVM 基线失败、kfrc 编译/运行失败、行为差异、
工具失败和来源检查失败分别记录。`summary.json`、`results.jsonl` 和每项原始
compiletest JSON、命令、输出、对象哈希共同构成证据。按源文件及官方 revision
分别统计实际通过数；blocked/excluded/ignored/deferred 均不算通过。

## 首轮实际结果

2026-11-09，在 HK 控制的 E2B 沙箱中复用 candidate12 后端和原版官方
compiletest，完成首轮 151 个排入项。工具 SHA-256 为
`c499f1e7d5c65d949c13842ab9de67151c16d0acc483859be0caf03d49d00303`。

| 状态 | 源文件数 |
|---|---:|
| 双后端执行、差分与 codegen 对象来源全部通过 | 22 |
| kfrc 编译或运行失败 | 5 |
| LLVM 基线失败 | 2 |
| 同组首错后暂缓 | 122 |
| 当前能力或协议/profile 阻塞 | 82 |
| 本批次选择外 | 194 |
| 完整候选池 | 427 |

实际通过的官方 revision 数为 22。首轮没有 HARNESS_FAIL 或 PROVENANCE_FAIL；
5 个 kfrc 首错和 2 个 LLVM 首错另行归因，122 个未启动项不计为通过。
27 项 Python 协议/执行包装器测试也已通过。原始命令、JSON 事件、运行输出、
状态、对象和双锁证据保存在 `evidence/poc-20261109/`，以对应快照记录为准。
首轮摘要为
`evidence/poc-20261109/upstream-batch01-results/results/rustc-upstream-batch01/summary.json`。

首轮之后修正了整数形式浮点后缀的能力分类（如 `3f32`），原 160 份成员保留，
其中 1 份改为 blocked，不补名额；清单保留原批次指纹和分类变更记录。
同时按官方辅助 crate 的实际规则补全 profile 检查。该阶段的完整计划为
**145 ready、88 blocked、194 excluded**，共 427；它属于下一批次计划，
不重写首轮数字。Python 协议、包装器和分类回归目前为 41 项通过。

## 第二轮实际结果

candidate14 完整稳定源码构建及 workspace 验证后，按上述新计划运行：

| 状态 | 源文件数 |
|---|---:|
| 全部检查通过 | 61 |
| kfrc 首次失败 | 6 |
| 同组暂缓 | 78 |
| 明确阻塞 | 88 |
| 批次外 | 194 |
| 完整候选池 | 427 |

实际通过 62 个官方 revisions；LLVM、工具和来源检查均无失败。引用组
20 份通过，包含原 mutable-static 首错；聚合组 17 份全部通过。
后端 workspace 同轮为 79 个 suite、758 passed、5 ignored，包括 union
与可写静态数据的真实机器回归。P1 新增 boxed FnOnce 用例仍暴露动态
by-value receiver 边界，不能据上游通过数声称所有 PoC 用例通过。

分类器 v3 又补全了 128 位整数类型/字面量识别：1 份初选源及 1 份批次外源
改为明确阻塞，仍未新增替补项目。最新计划为 **144 ready、90 blocked、
193 excluded**；原始 160 份成员与每轮清单指纹保留。当前 Python 回归 45 项。
第二轮摘要保存在
`evidence/poc-20261109/batch02-candidate14-results/results/rustc-upstream-batch02/summary.json`。

## 第三轮实际结果

candidate15 patch1：**121 份源文件通过，122 个官方 revisions 通过**；
2 个 kfrc 首错，21 个同组暂缓，90 blocked、193 excluded，总数仍 427。
LLVM、工具和来源检查均无失败，上一轮已过项目全部保留。

| 能力组 | 实际通过源文件 |
|---|---:|
| 整数 | 23（全部排入项） |
| 引用 | 22（全部排入项） |
| 聚合 | 17（全部排入项） |
| drop | 30（全部排入项） |
| enum | 18（全部排入项） |

完整 P1 同一后端为 1070 行与 LLVM 一致；入口 4 项和独立 Debug 探针也通过。
原版上游残留首错集中在 dyn Fn 对齐和 HashMap 的下一能力边界，未启动的
21 项保持显式暂缓。该轮 semver 推进到 `is_val_statically_known` intrinsic，
该轮尚未通过。

第三轮的独立历史摘要为
[`batch03/summary.json`](../../../evidence/poc-20261109/batch03-candidate15-results/results/rustc-upstream-batch03/summary.json)，
SHA-256 `f032523f3fee52c9a722b0bf82c49ff1b7d678ad57133f598f45b4148f70e3bc`。
后续分组复验不改变该轮 121/2/21 的统计。HashMap 的真实首错是
`std::hash::RandomState::new::KEYS::{constant#0}::{closure#1}` 在 bb0 的
`thread-local reference`；TLS 已进入实现工作。原测试仍保持 v3 清单的
selected 状态，以后续真实执行验收，没有将其改成永久能力阻塞。

随后 candidate15 patch2 的函数组独立复验 **9/9 原版源测试通过**，包含此前
失败的 `dyn-fn-alignment`。该次报告把其他 135 个可执行项记为 NOT_REQUESTED，
不把独立分组与历史轮次拼成新的全量通过数。相同候选的 semver 1.0.28 原版
34 项集成测试也全部通过，工作负载输出和对象来源核验完成。

函数组结果见
[`call15/summary.json`](../../../evidence/poc-20261109/patch02-call15-results/results/rustc-upstream-call15/summary.json)，
SHA-256 `a8bfabd7b79922a814cb7623c4e12ba828f2ec44c45dcb9362a139de20d1dee3`；
它记录 9 份源文件、9 个官方 revisions 通过，135 NOT_REQUESTED、90 BLOCKED、
193 EXCLUDED，仍覆盖整个 427 候选池。
[`binaries.json`](../../../evidence/poc-20261109/patch02-call15-results/results/rustc-upstream-call15/binaries.json)
记录后端 SHA-256
`c8160ed11d59b123d71afdb750285651ae345ca8777ad0e825dccb783ba4a03f`，
原版 compiletest SHA-256
`c499f1e7d5c65d949c13842ab9de67151c16d0acc483859be0caf03d49d00303`。
同候选编译器源码归档 SHA-256 为
`fc4437efc03a4e732a1a45eb87fbac14b17b316b1362cc39f6cef522ceaf129b`。
semver 的 34 项结果及 LLVM harness/std 边界单列在
[`semver-frontier.md`](semver-frontier.md)，不计入 427 上游 UI 候选池。

## 第三轮后的控制流准备记录

第三轮控制流组有 11 份通过、1 份 HashMap 首错和 13 份暂缓；因此还有
14 份需按原源码验证。HashMap 用例通过默认 RandomState 初始化 TLS，
随后只插入三对整数、遍历并比较键和值的总和，不依赖随机迭代顺序，也不
创建线程。其余 13 份原源码没有 TLS、随机源或线程调用。

| 原始用例 | 尚需真实执行覆盖的语义 |
|---|---|
| `foreach-external-iterators`、`-loop`、`-nested` | 100 元素 repeat 数组、slice iterator、enumerate、continue 与双层循环 |
| `foreach-nested` | 两层 FnMut 捕获可变 Vec/计数器、动态索引写 |
| `foreach-put-structured`、`foreach-simple-outer-slot` | FnMut 的 tuple/scalar 参数、外层捕获更新和可见 println |
| `issue-2216`、`while-label`、`loop-break-cont-1` | 嵌套 label 的 break/continue 与退出目标 |
| `loop-scope`、`while-with-break` | 借用 Vec 迭代，以及 break 离开局部作用域时的 Vec 析构 |
| `while-cont`、`while-loop-constraints-2` | continue 后条件重检、循环赋值与可见输出 |

上述记录来自当时的静态准备；实际通过状态由后续独立执行确认。

## candidate17 的函数与循环验收及全量第四轮

candidate17 用同一后端完成用户要求的两组原始测试：函数 **9/9**、
控制流/循环 **25/25**，共 34 份源文件、34 个官方 revisions，包含真实
RandomState/TLS 路径的 HashMap 用例。其余 110 份可执行测试记录为
NOT_REQUESTED，没有与旧轮次拼接统计。
[`functions-loops17/summary.json`](../../../evidence/poc-20261109/candidate17-results/results/rustc-upstream-functions-loops17/summary.json)
的 SHA-256 为
`87bb77ae819d6e5499d1743de903f591e4076f3b7a30bd0e2ab9e9e3883b7380`。

随后完整第四轮按固定 v3 清单独立运行，**144/144 份源文件、145 个官方
revisions 全部通过**。90 BLOCKED、193 EXCLUDED 保持显式记录；没有
LLVM/kfrc/HARNESS/PROVENANCE 失败或 DEFERRED_FAMILY。
[`batch04/summary.json`](../../../evidence/poc-20261109/candidate17-results/results/rustc-upstream-batch04/summary.json)
的 SHA-256 为
`78b4d5366bae65e06978fa60b06db2f77becac69953432a55c99e4ab3459b520`。
分组与全量两次运行的后端 SHA-256 均为
`7ffe97bfb77918cca85b228a0cd1113732d23629beba2b643644c7eab3b1e588`。
源码归档为 `63353d4f350902c085eb22df1818ab4f1f90aeb85d66a620c5a47cf12b5ffcb9`，
863 个输入文件复核无差异；同后端 semver 34 项也复验通过。

## candidate18 的全量第五轮复验

candidate18 再次独立执行完整清单：**144/144 源文件、145 个官方
revisions 全部通过**，90 BLOCKED、193 EXCLUDED；无失败或暂缓。
[`batch05/summary.json`](../../../evidence/poc-20261109/candidate18-results/results/rustc-upstream-batch05/summary.json)
内容与第四轮相同，SHA-256 仍为
`78b4d5366bae65e06978fa60b06db2f77becac69953432a55c99e4ab3459b520`；
两轮的原始命令、JSON、运行输出和二进制证据分别保留。
第五轮后端 SHA-256 为
`5a7803dbaa2ae0c1eb2710d5f2b71a356691209fda14f0332b8d19e4e45ec796`，
源码归档为 `9ff6ef30b678f327413b1e3a5e8c735dadbbc215a30af47d61488444884df61e`，
866 个输入文件复核无差异。semver 原版 34 项在同候选也复验通过。

## candidate19 的全量第六轮与最终范围

第六轮在 candidate19 后端独立完成 **144/144 源文件、145 个官方 revisions**
验收；函数组为 9/9、控制流/循环组为 25/25。所有双后端行为差分和对象
来源校验通过，无 LLVM_FAIL、KFRC_FAIL、HARNESS_FAIL、PROVENANCE_FAIL
或 DEFERRED_FAMILY。固定 v3 manifest 和 427 个候选身份保持不变。

最终原始摘要为
[`batch06/summary.json`](../../../evidence/poc-20261109/candidate19-results/results/rustc-upstream-batch06/summary.json)，
SHA-256 `78b4d5366bae65e06978fa60b06db2f77becac69953432a55c99e4ab3459b520`。
它与第四、第五轮摘要内容相同，各轮 JSON、命令、对象和运行输出分别保存。
[`binaries.json`](../../../evidence/poc-20261109/candidate19-results/results/rustc-upstream-batch06/binaries.json)
记录最终后端 SHA-256
`b16007417002cdea3a55e465e0d0cb0febddcc7485ebc38e9a86bd95a814ecb4`。
[`final-provenance.json`](../../../evidence/poc-20261109/candidate19-results/results/candidate-19/final-provenance.json)
确认测试后源码归档
`0357d1d76dff113acd8ff67ab8e817ea64b8152b0f4acaac46f04fd268b0a492`
的 867 个文件仍全部一致，后端与官方工具哈希未变。最终证据归档 SHA-256 为
`1fd1cbf991786b1f07d954a3d936aa4cbadc51db27f9b333c996347e5f1992c2`，
其中 5010 个文件下载后逐项核对哈希通过。

| 独立轮次 | 通过源文件 | 通过 revisions | kfrc 失败 | LLVM 失败 | 暂缓 | blocked | excluded |
|---|---:|---:|---:|---:|---:|---:|---:|
| batch01 / candidate12 | 22 | 22 | 5 | 2 | 122 | 82 | 194 |
| batch02 / candidate14 | 61 | 62 | 6 | 0 | 78 | 88 | 194 |
| batch03 / candidate15 patch1 | 121 | 122 | 2 | 0 | 21 | 90 | 193 |
| batch04 / candidate17 | 144 | 145 | 0 | 0 | 0 | 90 | 193 |
| batch05 / candidate18 | 144 | 145 | 0 | 0 | 0 | 90 | 193 |
| batch06 / candidate19 | 144 | 145 | 0 | 0 | 0 | 90 | 193 |

90 个阻塞项仍受能力或官方协议/profile 契约限制，193 项仍为初选范围外；
它们都未计入已通过分母。candidate19 同后端另通过 workspace 781 项
（5 ignored）、P1 1155 行差分、semver 原版 34 项及 regex-syntax 0.8.8
public API smoke。这些独立验收不计入上述 427 UI 候选池。

regex 完整内部测试仍有明确限制：原版 LLVM 基线 147/147 通过，kfrc 在
构造 uninhabited enum 时编译阻塞，尚未执行该 147 项。public API smoke
通过不能替代完整内部测试结果。具体原始入口和边界见
[`semver-frontier.md`](semver-frontier.md)。

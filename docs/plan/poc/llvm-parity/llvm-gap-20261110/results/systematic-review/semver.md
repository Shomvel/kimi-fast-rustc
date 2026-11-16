# semver 1.0.28：LLVM 差距专项审阅

本报告由 semver 专项 subagent 完成，范围是产物、调用关系与编译器实现诊断，不包含编译器修改。优先处理 checked arithmetic 的重复计算、parser 的聚合临时对象和包装调用，再研究循环特化。现有证据不支持把 req 的全部差距归因于 parser，也不支持用静态指令数直接预测运行时间。

## 1. 输入、口径与覆盖

- 基线 `bc718cca4a4c0e26af6e3695f13d1878e61d3bd1`；transport 为现有未提交优化合集，ranges 是其上的 KnownBits Phi / 差值事实增量，见同目录 `inputs.json`。
- transport patch SHA-256：`42f1f2f4364d54e40c2dedd46d93b89e01e7953315d35135910017a172851366`；ranges 增量 SHA-256：`6b467de60b44809e4a8b43bd99874b8dd31242f3b74822a31a1c8977347abcbd`。不能用 HEAD 单独表示候选。
- runtime 来自同机21轮交错实验；compile 来自21轮 baseline/transport 配对实验。ranges 尚无性能实验，不得把 transport 测量写成 ranges 的收益。
- 静态统计按 ELF `st_size` 截断函数范围，指令计数排除 relocation 行和区间外对齐。函数内部 padding 仍在函数范围；函数 size 之和与整个 `.text` section 总量不同。
- 自动盘点涵盖全部 semver 提取 object 的函数范围。人工深读集中在 numeric_identifier、identifier、op、comparator、Version::from_str、matches_impl 与 Identifier 小函数；不声称逐条人工通读所有 fmt/debug/分配函数。
- 深读 baseline/LLVM 汇编，并已补核最新 ranges 的 numeric_identifier、identifier、op；这三个函数所在 cgu.01 的 transport/ranges object SHA-256 相同：`38ec3bd1e93b4a01d2e826b1bb354816da37c24c574c200fa183012a596c2a70`。
- 最新全量函数表与汇编已持久保存到 `/Users/shom/Downloads/kfrc-artifact-fixes-20261110/systematic-review-evidence/evidence/results/static-review`，原始测量与元数据在相邻 `suite`；以下为该证据的汇总。旧完整证据目录 `/Users/shom/Downloads/kfrc-divshift-20261110`；新实验原始资料 `/Users/shom/Downloads/kfrc-artifact-fixes-20261110`。

驱动 `tools/llvm-parity-suite/drivers/semver.rs:9` 的实际工作负载：

| workload | 实际执行 | 归因限制 |
|---|---|---|
| parse | 六种版本串解析，含前导零错误、u64最大值、pre/build，之后取字段和释放 | 不是纯数字循环 |
| req | requirement 解析后立即 matches 固定 Version | 需拆 req_parse/req_match |
| identifier | 1024字节 `a` 的 Prerelease 构造、clone、as_str、相等比较、drop | 小型 ptr_eq 改善不保证整体改善 |

公共 driver 由 LLVM 编译，部分跨 crate 泛型/std 也由 LLVM 实例化；结果是所选 crate 后端差分，不是整个程序完全更换后端。

## 2. 已测进展与剩余距离

| arm | 函数范围数 | 函数字节合计 | 指令数 | call数 | branch数 |
|---|---:|---:|---:|---:|---:|
| LLVM | 75 | 18515 | 5061 | 189 | 831 |
| baseline | 194 | 32705 | 9500 | 512 | 1390 |
| transport | 171 | 31114 | 9021 | 459 | 1312 |
| ranges | 171 | 31089 | 9014 | 459 | 1310 |

ranges 相对 transport 只再减少25 B、7指令、2分支。它对 semver 的静态边际收益很小。ranges 仍为 LLVM 函数字节的1.68倍、call总数的2.43倍，但这不能推导动态调用次数或运行倍率。

21轮日志中的运行时间中位数（ms）；下表候选/LLVM是中位数相除，仅描述量级，不冒充配对比值或置信区间：

| workload | LLVM | baseline | transport | transport / LLVM |
|---|---:|---:|---:|---:|
| parse | 46.609 | 188.675 | 174.254 | 3.74× |
| req | 69.121 | 525.161 | 477.280 | 6.91× |
| identifier | 130.580 | 221.686 | 220.708 | 1.69× |

transport 编译wall配对比值中位数1.10805、instructions 1.07203、user time 1.02091、RSS 0.99486。即本轮以约10.8%的wall编译成本增长换取部分运行时间/体积改善。尚无阶段profile支持把开销归于某个pass；用户希望编译时间降低约50%，本轮尚未达到。

Identifier::ptr_eq 从30 B/11指令/2分支降到16 B/6指令/0分支，LLVM为10 B/4指令；Identifier::is_empty 从58 B/20指令/16 B显式栈预留降到20 B/6指令/无显式栈预留，LLVM为8 B/3指令（`selected-transport.json`）。布局清理有效，但 identifier workload 几乎持平，应转向循环及调用链。

最新 ranges 重点函数如下；LLVM内联边界不同，不应逐函数做运行速度排名：

| 函数 | ranges B/指令/call | LLVM B/指令/call |
|---|---|---|
| parse::comparator | 2096 / 580 / 30 | 1824 / 459 / 16 |
| Version::from_str | 1889 / 524 / 25 | 917 / 232 / 9 |
| parse::version_req | 1282 / 350 / 17 | 1228 / 306 / 8 |
| parse::op | 1116 / 292 / 28 | 无独立符号，逻辑被并入caller |
| parse::identifier | 759 / 212 / 7 | 605 / 177 / 2 |
| parse::numeric_identifier | 729 / 215 / 9 | 337 / 103 / 1 |
| BuildMetadata::cmp | 1076 / 286 / 16 | 1177 / 306 / 7 |
| Prerelease::cmp | 1063 / 281 / 16 | 657 / 173 / 6 |


21轮逐round配对比值中位数及配对bootstrap 95%区间（10000次重采样、seed=0，排除round=-1暖机）：

| workload | transport / baseline | transport / LLVM |
|---|---|---|
| parse | 0.9270 [0.9176, 0.9360] | 3.7548 [3.6919, 3.7587] |
| req | 0.9113 [0.8987, 0.9233] | 6.8508 [6.8023, 6.9417] |
| identifier | 0.9989 [0.9947, 1.0083] | 1.6925 [1.5509, 1.7044] |

identifier候选/基线区间跨1，不能宣称稳定加速。采样轮次交错可减小漂移，但不能消除共享机器噪声。perf单独计数中，parse动态指令从2.818B降至2.485B，LLVM0.692B；req从6.627B降至5.899B，LLVM1.048B；identifier从3.672B到3.657B，LLVM2.220B。perf不是每轮计数，不为这些计数构造置信区间。

完整产物元数据揭示另外一个tradeoff：

| arm | .text B | rodata B | 14个object合计B | rlib B |
|---|---:|---:|---:|---:|
| LLVM | 18515 | 2008 | 85240 | 238558 |
| baseline | 34052 | 11795 | 149960 | 302638 |
| transport | 31114 | 11201 | 238464 | 391142 |
| ranges | 31089 | 11201 | 238448 | 391126 |

候选机器码变小但object/rlib变大；不能把.text减少表述为整个编译产物减少。父agent随后对全部ELF对象逐项核算：transport相对baseline增加88504B，其中ELF/section headers增加45760B、字符串表增加47063B，code减少2938B，relocation减少1440B，symbol table减少888B，其他差额见[evidence.md](evidence.md)。这些是文件布局的精确归因；semver最终消费者仍需补测。LLVM元数据有81个函数symbol，而disassembly有75个函数体，差异包含同地址别名；表中“函数范围数”是实际反汇编body，不把alias重复计为机器码。四arm corpus均87行、SHA-256为`96c9f6c125b42251fee60f392b8ecfbdbae5691d6651db27947d32f3b5e4d56a`；这是所选语料一致，不是全输入正确性的证明。

构建为rustc1.94.1 x86_64 Linux，opt-level3、CGU16、panic=abort、debuginfo0、overflow-checks=off、debug-assertions=off、mir-opt-level0，std使用shared-std-mir0。Rust checked_mul的显式检查不因overflow-checks=off而消失。backend hash：baseline `f1fe2b139a363669ae519a2bc610b9eec43f75b857c818beef4aa61ab0596910`、transport `d04a0c96fff22749c4724cb31f57e23ffd56faaedc6df941bd445e570dfcb6d7`、ranges `a06f9f1fcbb248f5b64ae0f7328ab78355e4bbd3d6803faf676e8ddc6eb997dd`。

## 3. 根因与具体移植方案

### S1：checked arithmetic 结果与溢出重复计算（P0）

**证据。** baseline numeric_identifier 每位先执行 mul 检查高位，成功后又 imul乘10；checked_add 也先add/compare，成功路径再次add。LLVM为一条mul+jo、一条add+jb。这是每位重复工作，不只是冷错误路径体积。已确认最新 ranges 保留相同重复：numeric_identifier +0x21b为mul，+0x23c再次imul；+0x263为add，+0x275再次add。

**源码。** `crates/kfrc-son/src/build/semantic/values.rs:311` 建立运算结果，`:319` 另外建立仅返回bool的MulOverflow；`crates/kfrc-x86/src/lower.rs:511` 单独lower；`crates/kfrc-x86/src/machine/x86_encode.rs:797` 计算乘积后只保留overflow。结构根因可信度高，性能贡献需ablation。

**机制。** 参考LLVM with.overflow多结果语义及flags选择，让(value, overflow)保留同一运算身份。机器lowering合并同操作数、位宽、符号性的计算，相邻溢出分支直接消费flags。先做u64 checked_mul/add，再扩到窄整数和signed；不可直接删除溢出错误路径。

**验收。** 数字热循环每位仅一次乘法和一次加法，溢出跳转仍正确。覆盖0、00、u64::MAX、MAX+1、19/20/21位串、溢出后接ASCII/UTF-8；独立负例覆盖signed MIN/-1、窄整数边界、只用结果/只用标志、flags被中间指令破坏。配对测parse/req与compile成本。

### S2：包装调用阻断常量与边界推理（P0）

**证据。** 最新numeric_identifier已去掉metadata调用，但仍有is_char_boundary、RangeFrom::get_unchecked等9 call；identifier保留两次Option<&u8>::eq等7 call。最新op的28 call明确分成7次Option equality、7次is_char_boundary、7次get_unchecked、7次slice_error_fail，而Rust源码`parse.rs:262`的op只是首/次字节比较与切片。LLVM无独立op，不能记作零成本，需比较comparator内对应逻辑。

**源码。** `rustc_codegen_kfrc/src/codegen.rs:62` 已有两轮wrapper preparation，源图≤128、深度2、4调用、callee≤64、总fuel100000；`:125`仅接纳图缩小或小图调用减少。`kfrc-son/src/transforms/ipo/mod.rs:218`在调用点特化前检查callee大小，`codegen.rs:148`还限制CGU私有依赖。已有 wrappers 阶段日志 `semver-wrappers-graphs.txt:308` 记录comparator对op/is_char_boundary/get_unchecked因DataConflict拒绝，对numeric_identifier因Size拒绝，strip_prefix因Missing拒绝；这不是最新ranges的日志，最终策略前需更新。Missing还可能来自provider屏蔽，不能直接解释为没有MIR。

**机制。** 按LLVM内联后SROA/SCCP/InstCombine思路，对有常量参数及纯metadata/Option包装先做有界特化，再按简化图计费，缓存摘要。先输出skip reason和简化前后节点数；不全局提高内联预算、不按semver函数名特判。CGU私有引用仍须满足可链接性。

**验收。** 消除op/comparator和numeric_identifier中可折叠的metadata/Option equality包装，保留必要边界错误。记录每个保留调用原因、caller+callee闭包体积和编译指令数。负例：空串、单字节operator、>=/<=、多字节首字符、冷panic、匿名常量relocation、跨CGU私有callee。

### S3：Result/Option临时对象与Phi汇合栈往返（P1，依赖S2）

**证据。** 最新numeric_identifier显式栈预留152 B（baseline168 B），+0x2bb起仍三字复制到sret；identifier为208+8 B，+0x293起从split_at结果读四字，先写另一临时槽，再于+0x2bc起复制到sret。op同样在+0x430构造临时tuple，再复制到sret。LLVM直接写结果成员，无同等大临时栈帧。显式sub rsp不含所有push/save，不能视为完整frame，也不能将所有栈访问判成寄存器spill。

**源码。** `lower_layout/private_memory.rs:71` writer对Phi仅接受所有入边同值；`:141`搜索预算256，循环保守失败；`:155`要求exact type。`boundary_transport.rs:137`对含回边函数整体拒绝，因此parser循环无法直接享受目前快照方案。这是有意的安全约束，不能直接删除。

**机制。** 移植LLVM SROA/mem2reg的受限版本：不逃逸、字段范围已知的对象提升为SSA lane，不同入边值建立正确Phi；仅在所有路径完成初始化、输出不别名输入时做sret destination forwarding。循环内snapshot需证明动态实例生存期，先支持出口局部传输，再逐步放开。

**验收。** 消除可证明多余的临时copy，记录StackSlot/load/store/frame变化。负例包含部分初始化、不同variant、padding、union/MaybeUninit、重叠访问、地址逃逸、callee修改输入、跨迭代保留快照；新增Phi须过verifier及语义/布局解释器对照。

### S4：ASCII双边界检查缺少单区间归一（P1）

**证据。** 最新identifier在+0x93/+0x99比较A/Z、+0xa4/+0xaa比较a/z、+0xbf/+0xc5比较0/9；LLVM减去下界再做一次unsigned比较。numeric_identifier也有同类模式。减少分支可能增加算术指令，需测cycles/branch，不能只数全函数指令。

**源码。** `transforms/scalar/instcombine.rs:51`为局部重写入口；`transforms/control/conditions.rs:90`主要利用支配谓词证明后续If，非合并区间规范化。ranges的KnownBits增强不等同完整ConstantRange，不能假定已覆盖。

**机制。** 识别同SSA值的lo<=u x && x<=u hi，变成(x-lo)<=u(hi-lo)，保持位宽与wrapping。合并控制流仅跨纯且不会trap的计算，不得提前执行短路保护的load。

**验收。** u8全256值穷举；负例含signed比较、跨零区间、窄类型提升、不同SSA/load动态实例、guarded load。以每字节热路径compare/branch和cycles验收。

### S5：循环不变量与首轮特例仍在热路径（P2，依赖S2–S4）

**证据。** LLVM identifier入口按Position::Pre分出两个loop；最新ranges仍保留nondigit/leading-zero等状态，Position比较在+0x15c后才进行。LLVM numeric_identifier区分首字节和后续数字。对1024字节identifier这可能比ptr_eq更重要，尚无profile证明占比。

**源码。** `analysis/loops.rs:66`已有自然循环/不可约区域分析；未发现transforms使用它实现相同loop unswitch/rotation。

**机制。** 先规范化小自然循环preheader/latch/exit/Phi，再有界unswitch一个纯不变量或peel首轮，随后SCCP/CFG清理。限制克隆节点/层数，按简化后结果决定保留。不把完整SCEV或向量化作为此crate前置工程。

**验收。** Position guard移出循环或常量化，长identifier配对改善且短串不显著回退；覆盖零/一迭代、多出口、pre/build区别、前导零、非ASCII、不可约拒绝、克隆后Phi/支配验证，记录代码增长。

### S6：matches必须独立测量（P1诊断）

baseline matches_impl593 B/186指令/11call，LLVM1377 B/373指令/12call。LLVM更大是内联边界不同：合入匹配逻辑并使用Op jump table，KFRC保留greater/less/caret/tilde helpers。req的最大差距含解析、Vec构造和matches，不能全算parser。

新增benchmark-only req_parse、预构建req_match及各Op分层输入，保留综合req指标。对完整调用闭包分析dispatch、minor/patch Option、prerelease比较、Vec/drop；拿到分支频率和callee采样后决定dispatch或特化。诊断优先于进一步大规模parser pass投资。

### S7：新增pass必须受编译成本预算约束（P0贯穿）

transport编译增长已有实测。`codegen.rs:62`两轮wrapper clone/inline及`lower_layout/cleanup.rs:48`两轮清理是待测成本来源，不是已证实热点。借鉴LLVM分析失效与有界pipeline，为阶段计时、graph visits、clone bytes、CFG/dominator构建次数和命中率留证据；只重跑dirty函数/seam，缓存未失效摘要。

每项优化保留关闭开关用于ablation；同机21轮交错编译，校验对象hash一致。建议先将semver compile instructions恢复baseline±2%，再追求更大的wall下降。±2%是建议门槛，不是已有结果；wall须报告配对区间和噪声。

## 4. 执行顺序与验收

1. 固定ranges对象/hash，补ranges runtime/compile，拆req_parse/req_match，记录inline skip和阶段成本；补section/relocation/string table与最终链接产物大小，定位object/rlib增长。
2. S1结果/flags共享：直接去除每位重复计算，独立验证。
3. S2有界包装特化与S4区间规范化，每项独立开关、对照。
4. S3私有lane Phi提升与出口destination forwarding；ABI/初始化负例通过后再扩大循环支持。
5. S5有界peeling/unswitch，仅在profile支持且代码增长可接受时启用。
6. 按req_match独立数据决定dispatch/调用闭包优化；S7贯穿所有阶段。

停止条件：语义差分失败立即停；性能收益未越过噪声而编译成本上涨则不默认启用；单函数变小但闭包变大需重新评估。“接近LLVM”应按parse/req_parse/req_match/identifier分别定义倍率和置信区间，不能平均掩盖落后项，也不能承诺这组pass必达50%编译加速。

## 5. LLVM资料对应

`/Users/shom/Downloads/llvm-study/topics/03-sroa-mem2reg.md`对应S3；04-instcombine、05-sccp-cfg-dce对应S2/S4；07-loop-licm对应S5的规范化与执行条件；09-ipo-lto对应S2；10-instruction-selection对应S1；14-experiments和16-phase-ordering-budget对应测量与S7。资料基线LLVM21.1.0仅用于机制参考。本次未逐个运行LLVM pass证明某最终指令一定来自某个单独pass。

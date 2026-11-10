# 四个 crate 接近 LLVM：正式产物诊断与实施计划

后续实施更新（2026-11-10）：常量移位、普通正常量除法、块内商余数共享及按引用闭包导入内联数据已实现，见 [实现与验证报告](results/division-inline/validation.md)。最终 31 轮运行耗时相对该轮起点降低 4.85%，相对 LLVM 仍为 3.327 倍；新增优化的编译墙钟约降低 0.99%。以下保留实施前的诊断与验收线，其中“未实现”描述对应当时基线。

日期：2026-11-10。实现基线 `ab530534da62991a8ce0e4977aa5b0582cf7c724`，继续沿用 PR #8。

本轮新增工作是对保存的正式 LLVM / SCCP+KnownBits 目标文件重新反汇编、按 ELF 函数范围统计并检查调用点，再对照当前源码定位。**没有新增运行计时，没有实现以下优化，也没有达到“接近 LLVM”的验收目标。** 当前版本与该 KFRC 产物的四库 50 个对象已逐字节核对一致，见 [compile50 验证](results/compile50/validation.md)。

结论：先解决降低阶段丢失常量、简单调用未消除、聚合 ABI 往返展开和降低后的清理缺口。它们在四库均有具体表现。通用向量化、完整 SCEV、外部 solver 和全面更换寄存器分配器不作为本阶段前置。

## 1. 对照范围与验收目标

LLVM 21.1.8、Rust 1.94.1，Linux x86-64、O3、CGU16、MIR0、panic=abort、相同 sysroot；此前计时在同一 AMD EPYC 8 vCPU 实例完成。LLVM 学习资料固定于 21.1.0，是算法参考，不是测量工具链。共同驱动和标准库由 LLVM 编译，这里衡量四个库的后端代码质量。

将“接近”先定义为以下工程验收线，而不是预测一定可达到的收益：

| 指标 | 收敛验收线 |
|---|---|
| 13 个现有运行负载 | 每个 crate 内耗时比几何平均 ≤1.20；每一个负载 ≤1.50；总体几何平均 ≤1.20 |
| 泛化 | 独立保留输入集也满足上述线；不能只针对已有 checksum 或字符串常量优化 |
| 库代码体积 | 各库 `.text` ≤LLVM 的 1.50 倍；逐函数统计辅助定位，不要求函数边界一模一样 |
| 链接产物 | 同样 strip/链接选项下驱动整体 ≤1.10 倍；另测小 API 消费者及其 `.text`，避免共同 std 掩盖差距 |
| 编译时间 | 新同批 LLVM/KFRC A/B；完整墙钟几何平均 ≤1.10，单库 ≤1.20；保留单线程 CPU/RSS 对照 |
| 正确性 | 现有语义、压力、corpus、checker、可运行上游测试全部通过；阻断上游测试的 TLS/atomic_xsub/u128 路径须另行补齐后才称四库完整验证 |
| 改动回退 | 单个候选若某负载稳定慢 >3%，先解释/修正或撤回；不能靠平均值吞掉回退 |

运行门槛采用交替顺序、预热后至少 21 轮的配对耗时比，并给 bootstrap 区间；临界结果要求区间上界也通过，噪声过大时判定不确定。性能分析和正式计时分开。上述相对 LLVM 的编译门槛不替代用户原来的“相对 KFRC 基线完整编译降低 50%”目标；后者单列，当前仅降低 21.7%。

保留 MIR0 作为连续基线，同时增加默认 release 的独立对照组，双方配置完全一致。不能单独打开 KFRC 的 MIR 优化，或改变 CGU、CPU feature、LTO、panic 配置后声称后端改善。

## 2. 当前距离

运行列来自 [SCCP summary](results/sccp/summary.json)，为各臂运行中位数之比，不能与后续配对统计混算。静态汇编统计是本轮新增。

| crate | 负载：当前耗时 / LLVM |
|---|---|
| semver | parse 4.17；req 7.51；identifier 1.95 |
| base64 | encode 9.82；decode 8.10；short 4.60 |
| unicode-segmentation | mixed 1.37；ASCII 1.09；reverse 3.48 |
| humantime | duration parse 10.62；duration format 1.19；timestamp parse 3.44；timestamp format 2.22 |

总体 3.483 倍，库 `.text` 比率几何平均 2.964 倍。要达到总体 1.20 倍，需将当前整体耗时约降低 65.5%；base64 encode 和 duration parse 若要到 1.20 倍，需要约 88% 的下降。这里是目标所需变化的算术，不是预计收益。因此不能靠一批百分之几的小规则声称接近。

## 3. 四个可证伪的诊断假设

在检查新汇编前按以下顺序提出假设，并用正式产物检验：

1. **聚合边界往返放大 humantime**：若是主因，`ok_or → Try::branch → ?` 的调用、标签解码/重编码、栈对象会大量出现；将它们在 S 图中消除，应同步缩小 L 图、分支和编译工作量。
2. **base64 降低没有保住常量与地址模式**：若成立，源码中的常量移位会变为 CL 移位，同一地址会多次物化；修复选择后，原内联决策应保持不变，而 RCX 约束、栈流量下降。
3. **semver 的抽象调用链阻碍优化**：若成立，字符/切片/迭代器和比较包装残留；有界内联与摘要折叠后，相应调用和聚合传输消失。若只减少调用但不减少动态指令，应检查代码膨胀与重复解析。
4. **Unicode reverse 的额外工作在函数边界之外**：若成立，单个 `prev_boundary` 虽小，外置的 UTF-8 解码、状态判断和查表链仍重；应比较调用链与每个 grapheme 的动态成本。

以下证据确认这些代码模式存在，但尚不能定量分摊运行耗时。没有当前热点 call graph / cycles 样本，也没有本轮变换消融，不将假设标为已证明的完整性能归因。

## 4. humantime：聚合表示、调用边界和除法是三个独立问题

### 4.1 `Parser::parse_unit` 的膨胀仍然存在

| 函数体指标 | LLVM | KFRC |
|---|---:|---:|
| 字节 | 1,093 | 23,799 |
| 静态指令 | 293 | 5,674 |
| 静态跳转（含无条件） | 80 | 1,436 |
| 静态 call 点 | 9 | 64 |
| 带内存操作数的指令，不含 lea | 47 | 1,450 |
| 序言主要 `sub rsp` | 24 | 6,448 |

栈数字不是完整栈帧（不含 push、调用临时调整），内存操作数不是 spill 数量：许多栈地址先放入普通寄存器，必须追踪来源才能分类。

KFRC 保留 17 个 `Option<u64>::ok_or<Error>`、26 个 `Result<u64, Error>::Try::branch`、2 个 `Result<(), Error>::Try::branch`、9 个 `OverflowOp::div` 调用点。LLVM 的对应函数没有这些调用点。

例如对象内偏移 `0x230f` 调用 `ok_or` 后，从 `0x2314` 起出现多组 niche tag 比较、条件字段读取；随后在 `0x23d7` 起物化新的对象并重编码。全函数含 719 条 `movabs`，其中大量为 enum niche 常量。已确认的问题比“copy 太多”更具体：**嵌套 enum 在调用边界反复解码/编码，生成控制流和临时存储。** 各组展开是否完全冗余须由 S/L 图和 effect 事实证明。

源码承接：`lower_layout/variants.rs::decode_variant_with_transport` 遍历需求字段，对不能共享的 variant 构建 guard、字段读取和 Phi；`owned_transport` 已有受限整值存储传输，不能声称尚未实现聚合优化。当前 `codegen.rs` 在 layout 后验证并进入 x86；layout 内会 compact 和重建 memory partition，没有通用的降低后标量/CFG/局部内存清理管线。

**改进 H1：优先消除中间 `Result/ControlFlow`，再优化剩余布局。**

- 在热点调用点输出准确的内联拒绝原因和调用深度。先区分预算、callee 过大、不可用图、ABI/type/data 冲突；目前 `Missing` 混合了 provider 拒绝的多种原因。
- 为便宜包装函数实现有界、语义结构驱动的摘要/内联后清理。以调用图的 callee-first 顺序预简化非递归小函数，保留递归 SCC 和代码增长上限。
- 在 S 图融合已知构造、`Try::branch` 和随后的 variant 分支；正确保留错误对象所有权、drop、副作用和 poison 观察。
- 对无法消除的整体 ABI 传输，维持物理表示到真正读取字段处，再按需解码；源可变或跨 effect 时用必要快照，不能把原值替换成可能被后续写入的裸地址。
- 在 L 图新增有界清理：常量/地址归一化、支配条件下的重复 tag 判断、同内存版本的 store→load 转发、无用纯值。不能直接把依赖 S 级权限的 pass 原样搬到 L 级；需要重新取得内存和 CFG 事实。

机制验收：最小 `checked_mul()?.checked_add()?` 风格的实际 ABI fixture 中，成功路径不再往返构造完整错误 payload；真实函数目标先从 23.8 KB 降到 ≤4 KB，再向 ≤2 KB 收敛。字节目标只作为阶段门槛，必须同时验证 duration parse，不承诺由此单项达到 1.2 倍。

### 4.2 单位字符串识别缺少调用后的常量化简

KFRC `Unit::from_str` 1,294 B，40 个调用点（18 个 slice equality、22 个 str equality）；LLVM 675 B、零调用点，是长度/字节判断。已有小常量 bcmp 降低能力，但上层调用边界没有充分暴露常量长度。

**H2**：先让 equality 包装和长度常量到达已有 bcmp 规则；再补按已证明长度读取固定字节的 compare combine。禁止越界宽读，保留短串、非 ASCII、未知单位及错误对象内容。测试所有单位别名和接近匹配的反例，单独测 `Unit::from_str` 与端到端解析。

### 4.3 时间戳格式化有 61 个硬件除法点

KFRC `Rfc3339Timestamp::fmt`：46 条 `div` +15 条 `idiv`，LLVM：两者均为 0。例：同一值除以 86400 的商和余数在 `0xed1`、`0xee2` 分别执行除法；十进制位生成也反复除以 10/60/3600。

`lower/arithmetic.rs` 已优化带 EXACT 合同的常量除法，但缺少一般常量除法的完整降低。

**H3**：参考 LLVM 常量除法 magic-number 算法，先无符号、后有符号，贯通 multiply-high、修正加法、移位和商余数共用。保持窄位宽、有符号向零取整、MIN/-1、除零和现有 flags 合同。小位宽穷举、64 位边界及随机差分；非零常数除法不再降成硬件 div，除数动态时不强制替换。算法实现前补读固定 LLVM 版本 `DivisionByConstantInfo.cpp` 和目标 lowering；当前资料缓存未提供该核心文件。

该任务范围清楚、可独立验收，适合作为第一批实现；也覆盖 base64 的常量商余数。

## 5. base64：先修 encode，再打通 decode 调用链

### 5.1 encode 的正式机器码证据

| 指标 | LLVM | KFRC |
|---|---:|---:|
| `internal_encode` 字节 | 1,376 | 6,079 |
| 静态指令 | 327 | 1,398 |
| 跳转 | 17 | 165 |
| CL 移位 | 0 | 43 |
| bounds-panic 调用点 | 5 | 56 |
| 相同 `[rdi+3]` 的 lea | 0 | 39 |

KFRC 序言后把多个相同地址提前物化并写入大量栈槽；存在反复保存/恢复同一 spill 槽的序列。LLVM 的实现是标量宽读、bswap、立即数移位、查表和字节存储，SIMD 不是这条路径的前置条件。

**B1：在降低处保留常量移位。** `lower.rs` 的 shift 分支：没有 `AMOUNT_IN_RANGE` 时无条件生成 `And` 临时寄存器，即使输入是立即数。后面的 `constraints.rs::shift` 只识别 `VirtualValue::Imm`，于是落到 RCX。修复为先按 IR 位宽计算常量掩码，再传 Imm；不要只硬编码机器的 63，也不要改变动态移位规则。验收 8/16/32/64 位、超范围计数和 flags；该 encode 中 43 个 CL 移位应消失，S 图/内联决策不变。

**B2：在布局以后处理地址表达式。** 对同一 base、偏移、控制合法范围的纯地址做有界 CSE，随后按 use-site 折叠或重物化；不改变前面的内联节点预算。观察 39 个相同地址来源及寄存器活跃长度，不能仅以 lea 数量最少为目标。先独立验证 B1/B2，再组合；此前 early-CSE 候选发生过 semver 回退，不能原样恢复。

**B3：打通 `read_u64` 并证明索引边界。** 目前 encode 有 4 个 read_u64 调用点；callee 397 B、7 个调用点，保留 slice len/metadata/as_ptr、array cast 和 unwrap。让这些包装被简化，再按访问合法性合并宽读。区分三种 bounds check：固定 64 项表索引、循环块输入/输出范围、尾部输入。先消除 `&63` 等已可证明的检查，块循环再用其入口条件与不溢出的索引事实；尾部和失败输入保留必要检查。

### 5.2 decode 不能只比入口函数大小

LLVM `internal_decode` 为 2,205 B，内部无 decode_chunk_8 调用；KFRC 入口仅 267 B，但调用 3,316 B 的 decode_helper，后者有 4 个 decode_chunk_8 调用点。decode_chunk_8 为 1,604 B，保留 16 个 bounds-panic 调用点和 8 个 CL 移位。

**B4**：callee-first 清理 chunk 函数，有限内联到 helper，使输入块长度、256 项解码表的 byte 索引、输出写入长度可见；结合 B1/H3 清理移位和除法。对成功路径与非法字符路径分别验收，不能删除坏字符检查。低层采用通用规则，不按 base64 函数名字特殊处理。

输入矩阵：长度 0/1/2/3，8/24/26/32/64 附近边界，长数据；标准/URL-safe alphabet、有无 padding、非法字符各位置、尾部非规范位、输出缓冲区不足。长输入按 cycles/byte，短输入按 ns/call，避免吞吐掩盖启动开销。

## 6. semver：解析链和匹配链要分别定位

| 函数 | LLVM / KFRC 字节 | LLVM / KFRC 指令 | LLVM / KFRC call 点 |
|---|---:|---:|---:|
| Version::from_str | 917 / 1,934 | 232 / 538 | 9 / 25 |
| parse::numeric_identifier | 337 / 804 | 103 / 236 | 1 / 11 |
| parse::comparator | 1,824 / 2,156 | 459 / 597 | 16 / 32 |
| Prerelease::cmp | 657 / 1,085 | 173 / 287 | 6 / 18 |

numeric_identifier 保留 metadata、is_char_boundary、get_unchecked、Iter::new、next_code_point、from_u32_unchecked。Prerelease::cmp 保留 SplitInternal、copied/all/try_fold、闭包和切片比较链。部分包装最终函数仅 3～14 B，仍然成为调用；现有 identity folding 只处理严格的参数原样返回，不能据此假设所有这种调用都已消除。

**S1**：在复用 H1 的有界摘要/内联机制后，检查字符迭代状态和 Option/Result 是否被提升到标量，调用边界是否阻碍 ASCII 范围事实。避免针对 semver 改写源码算法。

**S2**：从[实际驱动](../../../../../tools/llvm-parity-suite/drivers/semver.rs)可确认，req 每轮先做 `VersionReq::parse`，再做 `matches`；先用动态采样确定两段贡献，再分别处理 parse::op/comparator 和 matches_impl/Prerelease::cmp，不能用 comparator 缩小就解释整个 7.51 倍差距。

**S3**：内联后只重访新增 seam，消除已知字节比较和身份/空串/长度包装；跨 CGU 图不可用则修复导入与依赖所有权，不简单放开 provider 限制。全局提高 max_depth/max_nodes 只是消融实验，不是默认解法。

验证：短/长版本号、u64 边界和溢出、前导零、长 prerelease/build、不同分段数量、空输入/非法 UTF-8 边界相关合法 str、复杂范围和匹配失败。热点拆分测量，外加现有 parse/req/identifier 端到端 gate。

## 7. Unicode：保住 ASCII，先收敛 reverse

ASCII 已为 1.09 倍、mixed 1.37 倍，reverse 3.48 倍。[实际计时驱动](../../../../../tools/llvm-parity-suite/drivers/unicode_segmentation.rs)中的 reverse 是 grapheme 反向遍历；`AsciiWordBoundIter::next_back` 虽在静态对象中膨胀，只有 corpus 使用，不能列作这项计时的原因。`prev_boundary` 在 KFRC 只有 1,326 B /363 指令，比 LLVM 的 2,906 B /761 指令小，但它调用独立的 is_boundary、UTF-8 reverse decoder、category 包装等。

KFRC is_boundary 为 2,111 B /577 指令 /37 call 点；next_code_point_reverse 为 496 B，含 4 个 `NonNull<u8>::sub`、3 个 unreachable_unchecked 调用点及 3 个 CL 移位。NonNull::sub 自身只有 12 B。LLVM 的 prev_boundary 未保留这些 reverse decoder/char 包装调用。

**U1**：复用有界包装消除，内联 UTF-8 指针调整和状态枚举比较；让分支条件在相同路径上复用，不能把未初始化 payload 提前读取。

**U2**：检查 reverse 与 is_boundary 之间 cursor 状态是否反复落栈/重读，依据字段别名和 effect 事实做局部提升和转发。category 查表的 closure/bsearch 边界同样列入跟踪，但没有动态占比前不重写整个 Unicode 算法。

**U3**：只有调用链清理后仍有明显差距，才推进对应循环的归纳变量/寻址优化。保住当前 ASCII 路径；不因反向遍历改善就接受 mixed/ASCII 回退。

验证 CRLF、组合符、emoji ZWJ、区域指示符、Indic conjunct、长组合序列、空串、chunk 边界与多次 provide_context；保留 Unicode 官方/上游语料。反向输出边界与正向对应关系做差分。

## 8. 实施顺序、阶段证据与停止条件

| 批次 | 改动及责任模块 | 必须产生的证据 |
|---|---|---|
| 0 | 固定源码、重新建立 Linux 两臂；驱动拆分与热点采样；`profile.rs`/suite 记录函数阶段规模 | 当前运行/编译/大小基线，成功/错误输入分别采样；每函数 S/L 节点、Region/Phi/load/store/call、virtual 指令、copy/spill 计数 |
| 1 | B1 常量移位；H3 常量除法；各自独立提交与消融 | 移位常量保留到 selector；除法语义差分；最终机器模式改变；四库无稳定回退 |
| 2 | 有界包装摘要与 callee-first 简化：`transforms/ipo`、`codegen.rs`；优先 H1/S1/U1/B3 | 每个热点残留调用的拒绝原因可解释；无名称白名单；S 图中的中间聚合和包装调用减少 |
| 3 | H1 聚合传输和 L 级清理：`lower_layout/{variants,memory,demand,boundary_transport,owned_transport}`；新增 L 级清理入口 | 对每个减少的 guard/copy 给出合法性事实；parse_unit 成功/失败路径、别名/循环/poison 差分；编译和运行同时验收 |
| 4 | B2/B4 地址与范围证明；H2/S2/U2 完成各调用链 | base64 长短输入都改善；semver 与 Unicode 逐负载接近门槛；不靠扩大整体预算 |
| 5 | 针对残差再做 SROA/DSE、有限循环改进、寄存器压力优化；最后分节/常量合并 | 先有剩余热点证据再选 pass；动态指令已接近但 cycles 落后时才重点追 IPC/分支/缓存/调度 |
| 6 | 全输入矩阵和保留集、默认 release、上游测试支持补齐 | 每库、每负载 gate 通过后才声明接近；保持所有改动在 PR #8，不合并 PR |

批次 0 的 stage dump 要限制到热点函数，正式计时关闭全部 profiler/dump。结合 per-function hash 固定消融输入，保留 IR 与机器码原始证据。预优化 callee 图的变更需正确处理递归、内部符号、匿名数据、地址身份和确定性并行读取。

编译时间方面，在运行代码质量的关键路径上优先缩小大图；分析缓存仅在重复分析占比得到验证后引入，使用 CFG/value/memory/节点重编号的明确失效规则。性能 pass 增加编译工作时要按收益单独决策，不能把新增并行掩盖 CPU 成本。

若单项实验没有消除预测的机器模式，先停止调阈值，沿 S→L→virtual→allocated 找丢失点。若模式消失但运行没有改善，核对动态热度和输入覆盖；不要连续叠加更多 pass。任何静态指令数、函数字节或调用点下降都不能替代运行验收。

## 9. 可复核证据与本轮边界

- 归档：`/Users/shom/Downloads/kfrc-sccp-20261110/evidence.tar.gz`，SHA-256 `a16660cd9fadaa8af642accc343a34c48751212809231fa7890437f3876141ab`。
- 从 `results/formal-suite/{llvm,bits}/{crate}/ref/objects/*.o` 提取 100 个对象；另外提取 8 个符号清单和 8 个驱动，未执行 Linux 二进制。
- 用 Apple LLVM 15 objdump 解码 x86-64 ELF；这是反汇编工具版本，目标文件依然来自正式 LLVM 21.1.8/KFRC 构建。
- [analyze_hotspots.py](analyze_hotspots.py) 保存每个输入 SHA-256、函数大小、别名、操作码、静态调用目标和计数。[hotspot-evidence.json](results/diagnosis/hotspot-evidence.json) 是可复算的热点子集。符号同地址同节别名归为一个函数体，不把别名重复算为额外代码。
- 完整反汇编和全部函数 JSON：`/Users/shom/Downloads/kfrc-sccp-20261110/gap-diagnosis/analysis/`。输入对象在相邻 `results/formal-suite/`。
- 本轮用 ELF 函数 extent 排除函数间对齐字节，核对符号/别名均被覆盖；选取关键汇编片段人工复核计数与调用模式。静态分支包含冷错误路径，call-target 列表基于 call/重定位，**不是运行调用图**；内存指令也不是缓存事件。
- 本轮完成诊断与计划，不宣称已经优化运行性能。后续动态实验仍需 Linux 环境；之前 E2B 已释放，不能引用其历史状态称当前仍有可用沙箱。

复算：

```sh
python3 docs/plan/poc/llvm-parity/llvm-gap-20261110/analyze_hotspots.py \
  /Users/shom/Downloads/kfrc-sccp-20261110/gap-diagnosis/results/formal-suite \
  --output /tmp/kfrc-four-crate-diagnosis
```

参考用户资料的 SROA/mem2reg、InstCombine、IPO、指令选择、KnownBits/DemandedBits、阶段顺序专题。迁移算法与证明条件，不移植 LLVM pass 名单或固定执行次数。

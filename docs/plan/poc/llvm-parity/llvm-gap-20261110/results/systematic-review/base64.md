# base64 0.22.1：LLVM 产物差距与移植计划

base64 的优先工作是消除布局阶段重新生成的重复地址、在调用点传播固定 slice 长度、合并小数组的字节搬运。最新产物已经去掉大量编码边界检查，但热循环仍反复调用 read_u64、decode_chunk_8，并为相同查表基址保留大量独立值。LLVM 对照的主体是标量查表代码；当前证据不支持把 SIMD 当成第一项工程。

## 1. 证据和覆盖边界

- baseline 为 `bc718cc`；transport 是已有未提交改动合集；ranges 再加入 KnownBits Phi / 差值事实。具体补丁哈希见 `inputs.json`。本报告只写诊断和计划，未修改生产代码。
- Rust 1.94.1 / LLVM 21.1.8、x86_64 Linux、O3、CGU16、panic=abort、MIR opt-level0、共享 MIR0 sysroot。driver 由 LLVM 构建；四 arm 的 driver object 哈希一致，测试的是库后端差分。
- 运行与编译性能只测到 transport。ranges 有最新汇编与语料一致性，尚未等价复测性能。不能把 transport 的倍率用于声称 ranges 的性能。
- 原始证据持久目录：`/Users/shom/Downloads/kfrc-artifact-fixes-20261110/systematic-review-evidence/evidence/results`。以下 `static-review`、`suite` 路径均相对此目录。
- 全量自动盘点为四 arm 共 269 个函数体记录、每 arm 14 个 object；函数以 ELF 符号长度界定，排除区间外对齐和重定位行。可按同目录 `function-inventory.csv` 筛选 base64 复核。`niche_constants` 旧字段不可靠，不使用。
- 人工深读：ranges/LLVM internal_encode、internal_decode 的调用闭包，read_u64、decode_helper、decode_chunk_8/4、decode_suffix，及 Alphabet::try_from 的聚合复制；其余函数完成符号、尺寸、调用类别盘点，不声称逐条读完所有格式化和分配路径。
- S/L 证据 `base64-wrappers-full.txt`、`encode-layout.txt` 是较早 wrappers arm；用于定位机制，不能当成最新 ranges 的 IR 或最新 inline skip 日志。最新汇编对残留模式做了复核。

全量 CGU 盘点如下，覆盖未进入性能热点的符号，避免只看 internal_encode。数字为 LLVM → ranges 的函数体数 / 字节合计。

| CGU | 主体 | 函数数 | 字节 |
|---|---|---:|---:|
| 00 | engine 编解码、构造、read_u64 及包装 | 5 → 15 | 4354 → 10143 |
| 01 | RawVec/Global 分配和扩容 | 2 → 10 | 364 → 2021 |
| 02 | decode_suffix、迭代器包装 | 1 → 4 | 1229 → 1448 |
| 03 | Vec extend/append、reserve 包装 | 1 → 7 | 101 → 282 |
| 04 | Alphabet 构造/转换及错误格式化 | 7 → 9 | 1074 → 7627 |
| 05 | DecodeError/DecodeSliceError 格式化与长度估计 | 4 → 5 | 414 → 839 |
| 06 | encoded_len、padding、编码错误格式化 | 3 → 5 | 188 → 518 |
| 07 | copy_from_slice_impl | 1 → 1 | 36 → 51 |
| 08 | complete_quads_len | 1 → 1 | 124 → 237 |
| 09 | StringSink 及 UTF-8 包装 | 3 → 4 | 312 → 480 |
| 10 | FormatterSink 及格式化包装 | 3 → 4 | 231 → 494 |
| 11 | Debug/LowerHex | 2 → 4 | 47 → 164 |
| 12 | Option Debug | 1 → 2 | 73 → 127 |
| 13 | StrConsumer::consume | 1 → 1 | 101 → 13 |

函数之间有内联和尾调用边界差异。最后一项变小不能直接解读为运行加速；CGU04 很大也不能直接归因于三个 runtime workload。

## 2. 性能与产物差距

| arm | 函数体 | 函数字节 | 指令 | 静态 call site | 分支 |
|---|---:|---:|---:|---:|---:|
| LLVM | 35 | 8648 | 2168 | 53 | 277 |
| baseline | 90 | 28175 | 6860 | 279 | 778 |
| transport | 72 | 26196 | 6336 | 219 | 698 |
| ranges | 72 | 24444 | 5961 | 180 | 614 |

transport 的同轮配对中位数与 bootstrap 95% 区间，计算方法及原始数据见 `measurement-summary.json`：

| workload | transport / baseline | transport / LLVM | 动态 instructions / LLVM |
|---|---|---|---:|
| encode | 0.8501 [0.8398, 0.8569] | 8.2492 [7.7773, 8.3286] | 4.3550 |
| decode | 0.8464 [0.8301, 0.8507] | 6.8233 [6.8023, 6.8768] | 4.1620 |
| short | 0.8575 [0.8496, 0.8646] | 3.8299 [3.8257, 3.8387] | 2.7857 |

encode 为 4096 字节输入、预分配输出；decode 解码同规模标准编码串；short 覆盖 0–63 字节，包含 encode/decode 的分配及释放。`tools/llvm-parity-suite/drivers/base64.rs:14` 起可复核。short 不能代表纯尾部编解码开销。运行时间差距大于动态指令差距，提示依赖链、调用/栈搬运和微架构因素仍需 cycles/采样证明，不能把倍率相除当成已测 IPC 根因。

transport 编译 wall 为 baseline 的 **1.1190 [1.1142, 1.1368]**，CPU 1.0577，instructions 1.1234，RSS 0.9997。已有运行改善附带编译回退，离降低编译时间约 50% 的目标仍有距离。

| arm | .text B | rodata B | 14 个 object 合计 B | rlib B |
|---|---:|---:|---:|---:|
| LLVM | 8648 | 1926 | 53632 | 411664 |
| baseline | 28778 | 10890 | 128416 | 486266 |
| transport | 26196 | 10302 | 204744 | 562594 |
| ranges | 24444 | 10302 | 200176 | 558026 |

机器码缩小，但 object/rlib 增大。统一 ELF 核算 `object-sections.json` 已把 transport 相对 baseline 的 +76328 B 分解：headers +40064、strings +42229、code −2582、relocations −2208、symbols −1200，其余项见原表。这是独立 function/data section 的元数据代价，不应归咎于代码生成体积增长。

相反，单根 `internal_decoded_len_estimate` 的 GC 保留代码从 13654 B 降到 31 B，LLVM 为 27 B；真实 stripped consumer 从 376904 B 降到 350160 B，LLVM 350192 B。该 consumer 的 .text 已同为 257563 B。这证明链接可裁剪性修复有效，仅代表这个 consumer，不能外推完整编解码程序已接近 LLVM。

四 arm 的 491 行 corpus SHA 均为 `6b6bc7c96dea2202adfbf8faebfef7f2b583b409619cb0a019c695e4802ff24a`，三个 workload checksum 一致。transport 上游 integration tests 13 个通过；unit suite 构建仍受未实现的 `fabsf64` 阻塞。不能写作上游全套测试通过。

## 3. 热点产物的具体差异

| 函数/闭包 | baseline B/指令/call | transport | ranges | LLVM |
|---|---|---|---|---|
| internal_encode | 5629/1287/68 | 5302/1214/59 | 3595/855/20 | 1376/327/9 |
| read_u64 | 397/108/7 | 233/57/2 | 233/57/2 | 已合入 encode |
| internal_decode | 独立入口+helper | 267/77/1 | 267/77/1 | 2205/494/7 |
| decode_helper | 3295/801/29 | 3310/807/24 | 同 transport | 已合入 decode |
| decode_chunk_8 | 1542/374/25 | 1013/252/10 | 同 transport | 已合入 decode |
| decode_chunk_4 | 908/231/17 | 553/146/6 | 同 transport | 已合入 decode |
| decode_suffix | 1495/412/6 | 1392/385/5 | 1369/376/5 | 1229/318/2 |

1. **编码范围传播已有显著静态进展。** ranges 相对 transport 的 internal_encode 减少 1707 B、359 指令、39 call site、80 分支。KnownBits 在布局生成的 `[Undefined, (i+32)-i]` 路径上恢复 32 字节事实，消除了重复输出索引检查。仍有 20 call site：4 次热路径 read_u64、8 次 slice_index_fail、8 次 panic_bounds_check。LLVM 的 9 个 call site 全在错误路径；保留公共入口必要检查是合理的，不以“零 panic call”为目标。
2. **同一 encode_table 基址被复制。** 最新 ranges 在进入 fast loop 前有 **32 条 `lea ..., [rdi+3]`**，全函数有 39 条；+0x53、+0x57、+0x5b 等相同计算被放进不同虚拟值，许多落到栈。热路径 +0x4d6 开始出现 `load stack; mov; add index; load byte`。LLVM 使用如 +0x58 的 `[rdi+r14+3]` 单条寻址。KFRC 显式预留 352+8 B，LLVM 无 sub rsp 但有 push；这里不能把所有栈访问都称为 aggregate copy 或全部称为 spill。
3. **read_u64 仍将 8 字节展开又重组。** 最新 +0x12..+0x34 分别加载8字节，+0x3d..+0x59写入 Result 临时对象，再 +0x6b调用 unwrap。返回 qword 在 +0x74写栈，之后8个 byte load、8个 byte store，最终 +0xc1重新加载 qword、+0xc4 bswap。LLVM encode +0x4a/+0x4e 是直接 qword load + bswap。瓶颈不是缺少 bswap 指令，现有后端已经发出了它。
4. **解码调用边界保留了固定长度检查。** 最新 decode_helper 每32字节循环于 +0x378/+0x439/+0x512/+0x5e0 调用 decode_chunk_8。后者有8次逐字节 input bounds check、1次输出切片检查、1次 copy_from_slice_impl；该包装又调用 memcpy。源 `decode.rs:173` 是 `#[inline(always)]`。LLVM 将 chunk/helper 合并到 internal_decode，保留字母表 INVALID_VALUE 检查及错误索引，但不保留这组重复长度检查。
5. **解码 byte packing 和 Result 表示仍绕栈。** decode_chunk_8 +0x33e bswap 后 qword store，随即拆成8个byte，再写到第二个数组，+0x3ad调用长度为6的 copy_from_slice_impl。成功返回在 +0x3b2构造 tag=4 的临时 Result，然后复制两字到sret。输出只允许6字节，不能粗暴改成8字节 store。
6. **尾部有小包装残留。** decode_suffix 的5次调用包括 Iter::new、Iter::next、Step::forward_unchecked，加两个错误路径。LLVM 只有两个错误调用；short 工作量需独立拆成预分配的 tail encode/decode，才可测量它们的贡献。
7. **非热点聚合膨胀仍突出。** Alphabet::try_from 3667 B/618指令，LLVM 217 B/70指令；KFRC 调用 Alphabet::new 后，把64字节结果逐byte加载、保留/重写再复制到返回地址，显式预留992+8 B。源码只委托构造，LLVM合并后直接返回。这里可检验通用表示修复，但不能拿它解释当前 encode/decode 性能。

## 4. 可执行改进项

### B1：L 阶段纯地址公共子表达式消除（P0，独立于内联）

**入口与根因。** `crates/kfrc-son/src/lower_layout/cleanup.rs:48` 只做 private_memory、InstCombine、条件/CFG；`transforms/scalar/gvn.rs:1` 明确排除地址，且仅接受 Rust value type。旧 L dump 同一 n3 派生大量重复 `(base+0)+3`；最新汇编确认结果仍在。仅重跑现有 GVN 不会解决。

**方案。** 参考 LLVM EarlyCSE/GVN，给 L 的无副作用 PtrOffset 建精确键：同一SSA base、同类型、同 displacement、同语义标志，先做常量偏移归一，再合并重复地址。限于布局阶段已确定的地址运算；不同来源但运行地址偶然相等的指针不合并。不要借此跨越 load、调用或生存期扩大观察范围。

**验收。** 32个相同基址只保留一个SSA表达式，记录 L nodes、spill slot、frame和动态 loads/stores。负例含不同 allocation/provenance、不同类型、同地址不同生存期、整数溢出/不同位宽、不同控制锚点；S/L verifier和解释器对照。先测 B1 单独收益，确认生成值变少是否同时降低寄存器分配和编译成本。

### B2：固定 slice 长度的调用点特化与有界内联（P0）

**入口与根因。** `rustc_codegen_kfrc/src/lib.rs:148` 只记录 inline(never)，没有把 always 的意图传入策略；`codegen.rs:62` 两轮 wrapper preparation 仅处理≤128源节点、callee≤64、4次计费调用。`kfrc-son/src/transforms/ipo/mod.rs:218` 在调用点特化前按原始大小拒绝；`ipo/plan.rs:30` 检查本地data identity，普通内联默认不复制匿名常量。

较旧 wrappers 日志记录 read_u64 被 DataConflict拒绝，decode_chunk_8 被Size拒绝，decode_chunk_4 被DataConflict拒绝。最新机器码确认调用仍在，但实施前应补最新skip日志，不把旧原因视为当前运行的直接证据。

**方案。** 先按调用图后序准备小callee；导入调用点的固定 slice metadata，对 read_u64 的至少8字节、chunk_8的8/6、chunk_4的4/3传播事实，折叠Result/panic后再按简化体收费。缓存“callee版本+参数事实”结果，保留总clone/node预算。接入 inline(always) 为有界优先意图，不能绕过递归、ABI、数据identity及链接约束。允许复制的常量仍限 unnamed、readonly、非TLS；有地址身份的本地data继续拒绝或经明确共享符号机制处理。

**验收。** fast encode中4次read_u64消失或成为已特化的直接load；fast decode中固定长度不再逐byte重复验证，InvalidByte顺序和索引准确。比较 caller+helper 闭包，不要求把所有decode逻辑塞进单个大函数。负例：0..8短slice、输出不足、跨CGU私有callee/data、同符号不同data、递归、错误分支常量和冷panic；禁止按base64函数名特判。

### B3：小数组整数表示、精确 byte-range forwarding（P0，与B2互相促进）

**入口与根因。** `lower_layout/array_transport.rs:9` 的storage表示只处理≥32个窄叶片且排除元素观察/Phi，不能优化[u8;8]。`private_memory.rs:71`仅接受精确相同range/type的writer，且所有Phi入边同值；因此qword store→byte loads不转发。`lower_layout/semantic_memory.rs:229`把copy转成memcpy；`kfrc-x86/src/lower/abi.rs:87`已有≤32字节常量copy展开，但copy_from_slice_impl包装隐藏了常量6。

**方案。** 参考SROA/MemCpyOpt，不是把所有数组再拆小：对已初始化、非逃逸[u8;4/8]按访问范围用整数carrier；从qword写入转发窄读取为shift/truncate，或对相邻load组做合法宽load；inline/specialize copy wrapper后让已有small-copy生效。对于to_be_bytes保留字节序关系，消除没有观察价值的中间数组。

**验收。** read_u64在已证长度成功路径为load+bswap；chunk_8结果写入为精确6字节的4+2等组合，chunk_4精确3字节。负例覆盖非对齐地址、页末/对象末尾、不足长度、volatile/atomic、别名重叠、未初始化padding、MaybeUninit、大小端语义及caller修改输入快照。禁止扩大读取/写入字节范围；memmove需保留先读后写。对比copy/load/store数和runtime，不只看栈帧。

### B4：完整 base+index+scale+displacement 地址选择（P1，B1之后）

**入口。** `kfrc-x86/src/lower/address_mode.rs:10`仅沿常量PtrOffset折叠为(base, i32)，`lower.rs:773`/`:802`据此发load/store；当前没有动态index的SIB模式。最新查表中的独立mov/add是直接证据。

**方案。** 参考SelectionDAG地址选择，扩展内部地址描述和编码路径，先base+index+disp，再scale1/2/4/8。消费纯地址链且每个用户均可覆盖时，避免额外物化临时寄存器。保持非地址用户和多次使用的实际值。

**验收。** 查表成为单条带index的byte load，减少临时值；测试rsp/r12作为index限制、rbp/r13零位移、负/溢出displacement、寄存器别名、只用于地址/也作为普通值两类。目标是减少依赖和压力，不通过删除边界语义降低指令数。

### B5：聚合返回destination forwarding及受限Phi提升（P1）

**入口。** `boundary_transport.rs:128`对整个循环函数保守拒绝；`private_memory.rs:71`不同writer不建SSA Phi；数组/enum字段观察会阻止整体transport。结果是decode Result和Alphabet::try_from仍出现临时拼装再复制。

**方案。** 先做非循环、完整初始化、不与输入别名的sret destination forwarding；对局部可分字段建立Phi，处理成功tag与错误payload的不同活性。只在生命周期证明完成后支持循环内快照，不直接删除当前cycle guard。

**验收。** Alphabet::try_from不再为64byte结果物化64个独立长寿命值；decode success不需构造完整未使用payload。负例涵盖variant切换、部分初始化、callee修改原存储、跨迭代快照、union/padding、输出输入别名。即便非热点收益很大，也须分别报告库text/compile与三个runtime结果。

### B6：后端小型选择与spill清理（P2，先完成B1–B4再定位）

最新encode入口的saturating_sub生成setb、movzx、neg、xor、and、xor链，LLVM为sub+cmovb；有些spill还出现同一槽load后原样store。前者可做局部选择，后者应先看B1去冗余后是否仍存在。

参考LLVM instruction selection/机器清理：只对已定义整数、确切mask-select模式选cmov，保留flags依赖，不能推测执行本被guard保护的load。为剩余spill记录来源、live range、两地址约束及call-clobber，再决定rematerialization/coalescing，不能见大栈帧就重写整个寄存器分配器。分别测试flags被破坏、多用户mask、内存operand以及长/短输入。

### B7：编译预算、测试覆盖与工作负载拆分（P0贯穿）

先补ranges同口径21轮编译/运行、pass时间、节点访问、clone次数和inline skip原因；分别记录B1/B2/B3/B4开关，新增dirty区域增量清理而非全pipeline固定点。当前base64编译指令已经增加12.34%，不能继续只用运行速度决定默认开启。

补预分配的short encode/decode、边界长度23–27/31–33及0–8、合法/非法字节各位置、padding模式与custom alphabet。完整unit suite需另行实现fabsf64后重跑；这属于覆盖缺口，不是已证明的库编译错误。

## 5. 顺序、阶段门槛与LLVM对应

1. **固定ranges基线**：完整复测，最新IR/skip/phase成本，补短串拆分。产物改善只在这一阶段确认是否转化为时间收益。
2. **B1地址CSE**：工程边界小且独立，先消除32份table base；用数据决定是否继续B4。
3. **B2+B3逐个闭包推进**：先read_u64，再chunk_4，再chunk_8与helper；每一步保留可独立回退的候选。先暴露常量，再按访问范围消除临时，随后重新估计内联体。
4. **B4地址选择与B5聚合返回**：分别测量，不将多个根因合成一个无法归因的大补丁。
5. **B6仅处理残留**，在上述标量差距压下后再评估循环重排/向量化。

建议阶段门槛（计划目标，不是已测结果）：先把每个encode/decode/short workload压到LLVM的≤2×，再到≤1.25×；不得用平均值掩盖某一项。每个新增默认pass须有统计稳定运行收益或明确产物/覆盖价值，并单独报告compile增量；编译预算先恢复到baseline的instructions±2%，再按阶段profile制定50%wall下降方案。若收益未越过噪声且编译成本上涨，不默认启用。不能保证这些pass累计一定达到指定倍率。

LLVM研究对应：`topics/03-sroa-mem2reg.md`支撑B3/B5的访问范围和表示选择；`04-instcombine.md`、`05-sccp-cfg-dce.md`、`21-demanded-known-bits.md`支撑范围事实及内联后清理；`06-memory-gvn-dse.md`支撑B1与内存证明边界；`09-ipo-lto.md`、`16-phase-ordering-budget.md`支撑先callee简化/后序与预算；`10-instruction-selection.md`、`11-register-allocation.md`支撑B4/B6。研究资料固定LLVM21.1.0，本次运行是21.1.8；这些是机制对应，未逐pass运行证明最终汇编由某单一pass产生。

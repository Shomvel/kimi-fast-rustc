# unicode-segmentation 1.13.3：LLVM 差距专项审阅

优先处理反向UTF-8调用链、标量select和字符类别查询闭包。前向ASCII的1.09倍LLVM结果不等于完整KFRC分割实现已接近LLVM：当前公共driver由LLVM编译，而`next_boundary`源码有`#[inline]`，库对象也没有它的独立符号。必须记录实际后端归属后再定义“接近”。

## 1. 输入与覆盖

基线bc718cc，transport为现有优化合集，ranges增KnownBits Phi/差值规则；源码hash与后端hash见[inputs.json](inputs.json)及suite metadata。Unicode的transport/ranges九个object逐一同hash、全量静态统计相同，性能仍按实测的transport标记，不把另一个arm当作已测。Rust1.94.1/LLVM21.1.8、x86_64 Linux、O3/CGU16、panic=abort、MIR0 sysroot。

证据根目录`/Users/shom/Downloads/kfrc-artifact-fixes-20261110/systematic-review-evidence/evidence/results`，汇编在`static-review/{arm}/unicode-segmentation`，原始对象在`suite/{arm}/unicode-segmentation/ref/objects`，21轮runtime在`suite/runtime/unicode-segmentation.json`。统一统计与精确ELF字节账本见[evidence.md](evidence.md)。`niche_constants`失效字段不使用。

全量自动盘点覆盖四arm共254条函数体记录、每arm九个CGU；同名本地泛型在多个object出现是多份实体，不合并成一个。人工深读为prev_boundary、is_boundary、next_code_point_reverse、category/bsearch/binary_search/select调用闭包、GraphemeState::eq、is_boundary_result；另外检查三个context handler、word/sentence主要调用结构。未逐条人工解释全部冷panic/fmt和重复泛型体。

按符号所属/用途归入以下互斥族（句子/词闭包中的泛型计入对应族，其余core计包装）：

| 函数族 | LLVM函数体/字节 | ranges函数体/字节 | 审查重点 |
|---|---:|---:|---|
| GraphemeCursor/State | 7 / 5252 | 9 / 5594 | 反向循环、状态查询、上下文处理 |
| Unicode tables | 5 / 1552 | 18 / 3754 | category→bsearch→binary_search→select |
| sentence | 7 / 751 | 8 / 1431 | match_sb8/8a/9/11、状态match、alphanumeric |
| word | 2 / 731 | 3 / 2399 | AsciiWordBoundIter::next_back、emoji |
| 其余core/包装 | 3 / 112 | 38 / 4357 | UTF-8、pointer、slice、char、select等 |

源crate为`/Users/shom/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/unicode-segmentation-1.13.3/src/`。下面crate文件行号均相对此目录。

## 2. 实测与口径限制

| arm | 函数体 | 符号字节合计 | 指令 | 静态call | branch |
|---|---:|---:|---:|---:|---:|
| LLVM | 24 | 8398 | 2356 | 46 | 450 |
| baseline | 78 | 17878 | 5314 | 218 | 936 |
| transport / ranges | 76 | 17535 | 5225 | 211 | 926 |

| workload | transport/baseline及95%区间 | transport/LLVM | 动态指令/LLVM |
|---|---|---:|---:|
| graphemes-mixed | 0.9882 [0.9838,0.9931] | 1.3228 | 1.3295 |
| graphemes-ascii | 0.9939 [0.9872,0.9969] | 1.0889 | 1.0454 |
| graphemes-rev | 0.9857 [0.9733,1.0035] | 3.7307 | 3.5099 |

mixed精确区间为[0.9838497,0.9931323]；采用同目录measurement-summary.json的4000次配对bootstrap。reverse区间跨1，不能声称本轮稳定加速。perf为独立运行计数，不与wall区间混用。

编译wall/baseline为1.05082 [1.03405,1.06912]，CPU为1.03413，指令为1.04423。即运行变化很小，编译成本仍增加约5.1%。ranges对本库的额外静态收益为零。

对象文件总量179864→255192B（LLVM112552B），rlib与只读数据见统一表。+75328B增长中headers+31680B、strings+43923B，code−955B；无需把object增大误判成运行代码膨胀。当前没有Unicode最终消费者独立GC验收，不能套用base64的链接结果。

`tools/llvm-parity-suite/drivers/unicode_segmentation.rs:31`仅对三种grapheme workload计时，word/sentence只在corpus里。driver头注释称next/prev均non-inline，但`grapheme.rs:667`紧接`#[inline]`的next_boundary，以及产物缺少next_boundary，说明这个注释不准确。前向主控制流可在LLVM driver实例化；其余helper的实际归属应通过driver反汇编/重定位核实。当前三个比值仍是有效的指定library后端差分，不能外推全部算法由KFRC生成。

## 3. 关键产物与根因

### U1：反向UTF-8的短包装调用及类型转换（P0）

ranges prev_boundary为1304B/359指令/15call，LLVM为2906B/761/19。LLVM把is_boundary与UTF-8解码合进caller，KFRC分别调用2082B/570指令/36call的is_boundary和484B/140/7的next_code_point_reverse，因此只看prev_boundary会得出相反结论。

具体证据：prev_boundary在+0xe8、+0x303调用reverse解码，+0x105、+0x326调用3B/2指令的from_u32_unchecked；+0x3f2调用is_boundary。reverse helper在+0x46、+0x98、+0xe6、+0x130调用12B/5指令的NonNull::sub，另有3个unreachable_unchecked冷call。最先该消除的是每字节指针递减包装，不能把全部7call都当作热调用。

源码入口：`codegen.rs:62`的wrapper preparation与`transforms/ipo/mod.rs:218`预算检查；`transforms/ipo/identity.rs:96`只识别原类型参数直接返回，不能把u32→char当作无条件identity。优先对已存在的pointer arithmetic小body做callee-first简化并复用摘要，再审查char转换的有效值约束。参考LLVM内联/InstCombine/IPSCCP；不按函数名替换指针操作，不全局抬高预算。

验收：在reverse helper内消除四处NonNull::sub call；char转换在已有有效Unicode scalar证明下消失，缺证明则保留要求。负例覆盖空/1字节、2/3/4字节有效字符、U+10FFFF、组合字符和跨chunk请求；单独验证pointer provenance、地址空间、减法边界、callee可能带观察/副作用时不能归纳成算术摘要。invalid char属于unsafe前置条件测试，不能作为合法字符串输入给std。记录整个prev_boundary闭包动态指令/cycles、编译fuel/失败原因。

### U2：select_unpredictable退化成调用、分支和栈对象（P0）

六份select_unpredictable<usize>各104B/35指令，显式`sub rsp,64`；有三次condition test，前两次JNE目标为下一条指令；值经多个栈槽复制后选出。binary_search循环每轮在+0xac call它，穿过五个callee-save寄存器的调用约定。LLVM grapheme_category在+0x91/+0x99使用cmov，整个搜索循环内没有此call。

源码`rustc_codegen_kfrc/src/convert/intrinsics/select.rs:6`把intrinsic转换为SwitchInt+两个value-copy分支，是为scalar/pair/aggregate统一保留按值快照语义。后续缺少可靠的“纯标量select恢复”路径；`lower_layout/private_memory.rs:71`在Phi仅接受相同writer，选不同输入的栈槽不能完全提升。它是具体标量化/选择缺口，不应仅称regalloc差。

方案：先支持已求值、已定义、Copy标量的select或pure diamond→SSA select，再在x86选择cmov；保留aggregate现有复制语义。必须把public hint wrapper对未选值的drop纳入检查，不能仅用intrinsic名改写所有泛型。参考LLVM SROA/mem2reg、SimplifyCFG select转换与机器指令选择。

验收：usize helper没有私有栈槽；搜索loop没有select call；cmov版本与分支版本各自ablation，记录branch misses/cycles而非强制所有select都cmov。负例覆盖bool条件、指针值、窄整数、两输入同值、aggregate别名、Drop类型、未选值副作用、poison/Undefined语义；选后值仍为原值，不得把聚合地址当成结果。汇编里的临时未初始化槽搬运需结合S/L证明死值后删除，不能仅凭读栈模式宣称发生可观察错编译。

### U3：类别搜索边界与常量状态没有跨调用消除（P1，依赖U2）

KFRC category347B/109ins/3call并非只比LLVM324B/94ins/1call略差；它还调用bsearch_range_value_table345B/111ins/3call、binary_search_by340B/107ins/1call及上述select。LLVM将搜索内联进category。`tables.rs:520`搜索范围，`:542`先以c/128选子表，`:559`搜索；LLVM直接使用shr与缩放寻址，并把Ordering比较化成cmov。KFRC仍从只读匿名对象加载Ordering常量，及get/map_or包装的分支。

方案：以子表长度/常量参数驱动有界调用点特化，先简化再计费；对只读无身份、无relocation标量常量加载做合法折叠；与base64共享x86 SIB寻址选择，减少index*2/8/12再add的链条。读取常量必须尊重类型、offset、初始化、relocation与mutable/TLS边界，不跨未知store使用缓存。这个crate暂不需要完整SCEV或向量化。

验收：category→search闭包内无select包装、无可证明常量Ordering load；对所有合法char逐值比较返回category及lower/upper缓存区间，并测ASCII/稀有非ASCII/相邻区间/缓存命中与失配。不得只校验category，因为错误bounds会影响后续缓存结果。检查失配时插入点0/len、空子表、最高codepoint。编译预算采用compile-budget.md，新增inline不允许无上限克隆。

### U4：状态枚举与Result路径的构造/复制（P1）

is_boundary_result为182B/50ins/3call，LLVM84B/25ins/1call。KFRC两次调用146B/45ins的GraphemeState::eq，分别与只读常量Break/NotBreak比较；返回Result先写三个候选栈槽，在+0xa3开始复制两字到sret。LLVM直接检查tag并写返回字段。prev_boundary显式栈预留264B，LLVM120B（均不含全部push）；不能把全部栈访问当spill。

源码`grapheme.rs:597`起按state、pre_context、cat_before/after区分路径，state含Emoji{seen_zwj}payload。泛化方案为SCCP/IPSCCP传播已知variant到派生eq，仅读活跃字段；SROA支持不同入边值的lane Phi及出口destination forwarding。入口为`boundary_transport.rs:128`、`private_memory.rs:71`；当前含循环函数全拒绝快照提升是安全限制，不能直接删。

验收：与已知无payload variant比较简化为tag测试，Emoji两payload值仍区分；输出copy在可证明不别名/完整初始化时消除。负例涵盖全部GraphemeState组合、niche编码、不同入边variant、partial init、MaybeUninit/union、循环动态实例、输入输出别名、跨chunk恢复、PreContext/PrevChunk/InvalidOffset错误值。

### U5：pointer compare分支仍物化bool（P1，小独立改动）

reverse helper +0x1a→+0x26，以及+0x7d→+0x89等模式是cmp pointer→sete→movzx→test→jcc。`crates/kfrc-x86/src/lower/branch.rs:69`已有单use、同block的direct compare选择，但`:73`明确排除pointer；这解释了整数分支已有优化、迭代器pointer equality仍没覆盖。

方案只扩展已降至目标标量地址表示的Eq/Ne，保留单use/调度/flags约束，不泛化为任意pointer有序比较。验收是每个已覆盖分支只需cmp+jcc，同时测试共享bool用途不被删、跨block不错误重算、地址空间/宽度、flags中间指令、带控制锚定或加载副作用的operand。LLVM instruction selection是机制参考，预计收益需实测。

### U6：剩余模块不能因未计时而遗漏（P2诊断）

AsciiWordBoundIter::next_back为2000B/561ins/32call（LLVM705B/192/4），静态差距明显，调用链多为is_char_boundary、get_unchecked、slice_error_fail、ASCII分类和bounds panic。sentence match_sb8/8a/9/11也保留UTF-8与tuple eq包装。它们由语料覆盖而未单独计时，不能纳入三个grapheme runtime收益。

先补词/句正反向、ASCII/非ASCII小负载并明确实例化后端，再复用U1/U3/U4和semver区间规范化；优先处理被profile证实的循环边界，不因有32个静态call就估计32倍运行开销。其他冷fmt/panic路径只要求正确性与链接可回收。三个context handler较LLVM更小，同样需闭包判断，暂不以扩大内联为目标。

## 4. 执行顺序与停止条件

1. 固定ranges/transport hash和driver归属；补reverse按ASCII、2/3/4字节、emoji ZWJ、RI、Indic、chunk恢复分层，区分UTF-8、category与state处理占比。
2. U1短pointer包装与U5 pointer Eq/Ne分支选择先独立实现/验收，局部且可明确观测。
3. U2纯标量select→SSA→cmov与U3搜索闭包特化，分别保留开关；对所有合法Unicode scalar验证category与缓存bounds。
4. U4枚举常量化和lane Phi，先无环出口再处理循环快照，负例先于扩大范围。
5. U6补词/句基准后按profile决定投入；复测前向与反向，防止优化reverse使已接近的ASCII退化。

建议第一阶段预算：任何已测前向workload不得稳定回退超过2%，编译指令先恢复baseline±2%，reverse收益必须越过配对噪声。接近LLVM要同时报告每个负载的wall、动态指令和后端归属，不设未经测量的收益承诺。完整unit构建现受atomic_xsub缺失阻塞；8项integration通过、corpus一致，不能称全套测试完成。

参考`llvm-study/topics/03-sroa-mem2reg.md`、04-instcombine、05-sccp-cfg-dce、09-ipo-lto、10-instruction-selection、16-phase-ordering-budget。这里只对应机制；未对LLVM逐pass做归因实验。本专项未改生产代码。

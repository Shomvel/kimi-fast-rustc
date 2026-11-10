# humantime 2.4.0：LLVM 差距专项审阅

本报告接续已中断的 humantime 专项 subagent 审阅，复用归档 ELF、汇编、源码与测量数据，不修改编译器，也没有新增编译或性能实验。结论是先隔离 timestamp 格式化回退，再处理单位字符串匹配、Result 边界搬运与 L 层恒假条件。duration parser 的大幅静态收缩已经发生，不能再次把旧产物的全部枚举膨胀列为尚未修复。

## 1. 版本、证据与覆盖边界

基线是 `bc718cca4a4c0e26af6e3695f13d1878e61d3bd1`。transport 是当前未提交改动的一组固定 overlay；ranges 是其上的 KnownBits Phi/差值事实增量。完整文件与哈希见 [inputs.json](inputs.json)。transport 有同机 21 轮交错运行和 21 对编译测量，ranges 只有静态产物和正确性证据，尚不能宣称后者运行加速。

- 固定库版本 humantime 2.4.0，实际工具链 rustc 1.94.1 / LLVM 21.1.8，x86_64 Linux；库 O3、CGU16、panic=abort、debuginfo=0、MIR opt-level=0，共用 MIR0 sysroot。
- 持久证据根目录为 `/Users/shom/Downloads/kfrc-artifact-fixes-20261110/systematic-review-evidence/evidence/results`。`static-review/functions.json` 与 `static-review/{arm}/humantime/` 保存函数表、汇编，`suite/{arm}/humantime/ref/objects` 保存原始对象；具体目录结构以该归档为准。
- 全量自动盘点覆盖四 arm 各 13 个 ELF object，合计 294 个函数体记录；函数范围由 ELF `st_size` 界定，不把 relocation 行、符号外对齐或别名重复计为指令。完整索引见 [function-inventory.csv](function-inventory.csv)。
- 人工深读集中在 `Parser::parse`、`parse_fractional_part`、`parse_unit`、`add_current`、`OverflowOp::{mul,add}`、`Option::ok_or`、`Unit::from_str`、`parse_rfc3339_weak`、`Rfc3339Timestamp::fmt` 及其 `Result<&str, Utf8Error>::expect` helper。其余 fmt/debug/allocator/wrapper 函数做了清单与调用关系盘点，不声称逐条人工通读了所有函数。
- CGU00 是 duration parser/format/checked 包装，CGU01 是 date parser/format；CGU02–03 是分配和 to_owned，04/10 是 ends_with，05/07/09/12 是格式化包装，06 是公开 wrapper，08 是 drop，11 是数组 equality。最大差距集中在前两个 CGU；其余闭包仍须随实际调用测量。

公共 driver 由 LLVM 编译，四 arm 的 driver object 哈希逐项一致，local text 均为 11184 B。这有助于固定输入，但不意味着整个程序由 KFRC 编译；std、格式化框架及部分泛型路径仍可能由 LLVM 实例化。这里只测替换 humantime crate 后端的差分。

驱动入口 `tools/llvm-parity-suite/drivers/humantime.rs:9` 定义的范围尤其重要：

| workload | 实际输入与操作 | 归因限制 |
|---|---|---|
| dur-parse | 10 个字符串，整数单位组合及 `bad`、`1 fortnight` 两个错误输入 | 没有小数输入；不能用此计时证明 fractional parser 优化有效 |
| dur-format | 循环生成 Duration，写入复用 String | 包含 core::fmt 与字符串写出 |
| ts-parse | 8 个 timestamp，含小数、错误日期、错误时区 | 主要是 weak parser，混合成功与失败路径 |
| ts-format | 生成 SystemTime，固定 `format_rfc3339_nanos`，写入复用 String | 不代表 seconds/millis/micros/smart 全部精度 |

四 arm 的 corpus 都有 553 行，SHA-256 均为 `a9425cd88ba9a9d39fae77ce8c1399af21168d6e2938ad1377b560e49a029b1d`。corpus 含小数、溢出、多精度和 round-trip，但一致只证明这些输入；完整 upstream unit 的缺失 intrinsic 阻塞仍见 [统一证据](evidence.md)。

## 2. 已测改善、回退与静态账本

下表按逐 round 配对比值中位数，95% 区间沿用 [measurement-summary.json](measurement-summary.json) 的 4000 次 bootstrap；只描述本次实验内部波动。动态指令来自独立 perf 计数，不附伪造的 21 轮区间。

| workload | transport / baseline（95% 区间） | transport / LLVM | 动态指令 / LLVM |
|---|---|---:|---:|
| dur-parse | 0.8289 [0.8249, 0.8395] | 8.660× | 6.669× |
| dur-format | 0.9486 [0.9379, 0.9574] | 1.133× | 1.150× |
| ts-parse | 1.0003 [0.9918, 1.0095] | 3.117× | 4.105× |
| ts-format | 1.0351 [1.0335, 1.0387] | 1.714× | 1.973× |

transport 的 humantime 完整编译 wall / baseline 为 0.8338 [0.8150, 0.8440]，CPU 为 0.8641，instructions 为 0.8235。该库同时获益于运行和编译简化，但不能抵消其他库的编译回退，也未证明历史“编译时间减半”目标达成。

ts-parse 区间跨 1，不宣称稳定加速。ts-format 的约 3.51% 回退应单独解释。dur-format 的动态指令 / baseline 为 0.999999，计时却下降约 5.14%，表明代码布局、微架构或其他运行环境因素仍需单独核查，不能把全部时间差都解释为删除指令。

| arm | 函数体 | 函数字节 | 指令 | 静态 call | 静态 branch |
|---|---:|---:|---:|---:|---:|
| LLVM | 32 | 10426 | 2651 | 60 | 450 |
| baseline | 98 | 58034 | 14423 | 317 | 2835 |
| transport | 82 | 37393 | 9442 | 322 | 1551 |
| ranges | 82 | 36305 | 9136 | 322 | 1479 |

ranges 相对 transport 再减少 1088 B、306 指令、72 分支，但 call 数不变。相对 LLVM 的函数体总量仍为 3.48×，不是运行倍率预测。

| arm | .text B | 只读数据 B（suite 口径） | object 总 B | rlib B |
|---|---:|---:|---:|---:|
| LLVM | 10426 | 1656 | 53768 | 148680 |
| baseline | 58709 | 11149 | 165960 | 261116 |
| transport | 37393 | 10930 | 247640 | 342796 |
| ranges | 36305 | 10930 | 246536 | 341692 |

transport 比 baseline 的 object 增加 81680 B，其中 headers +49216、strings +52665、code −21316、relocations −192、symbols +696，其余字节见 [ELF 账本](evidence.md)。这是独立 section 与名称元数据的代价；不能把机器码缩小说成整个 rlib 缩小。humantime 最终消费者的 linked size 尚未独立验收，不能借用 base64 单根 consumer 结果。

重点函数的最新形态如下。LLVM 常把 parser/helper 内联进上层，缺独立符号不等于没有成本。

| 函数 | transport B/指令/call | ranges B/指令/call | LLVM B/指令/call |
|---|---|---|---|
| Parser::parse | 6953/1521/19 | 6711/1453/19 | 主要并入 parse_duration |
| Parser::parse_unit | 5094/1198/90 | 5094/1198/90 | 1093/293/9 |
| Parser::parse_fractional_part | 4737/1123/12 | 4254/987/12 | 无同名独立符号 |
| add_current | 1813/497/4 | 1450/395/4 | 无同名独立符号 |
| Unit::from_str | 1294/355/40 | 1294/355/40 | 675/187/0 |
| parse_rfc3339_weak | 2574/684/15 | 2574/684/15 | 1150/298/1 |
| Rfc3339Timestamp::fmt | 4265/1027/20 | 4265/1027/20 | 1759/431/5 |
| FormattedDuration::fmt | 1086/272/10 | 1086/272/10 | 1494/325/18 |
| Option<u64>::ok_or<Error> | 871/216/2 | 871/216/2 | 无同名独立符号 |
| OverflowOp::mul | 465/127/2 | 465/127/2 | 无同名独立符号 |
| OverflowOp::add | 447/122/2 | 447/122/2 | 无同名独立符号 |

`parse_unit` 已从 baseline 23799 B/5674 指令/1436 分支缩至 5094 B/1198/236。剩余显式 `sub rsp` 预留为 4496 B 加一个 8 B 调整，LLVM为24 B；这些数字不含全部 push/save，不能直接当完整 frame，更不能把全部栈访问称为寄存器 spill。

## 3. H1：先隔离 timestamp format 回退（P0，独立诊断）

**产物证据。** baseline、transport、ranges 的 `Rfc3339Timestamp::fmt` 符号机器码均为4265 B，SHA-256 均为 `c9b47aa666f65c5cb09d54e8591191d5130b22fba42f408aea59108a360ef926`。这是 ELF 未链接符号字节哈希，不能排除 relocation 目标和最终地址变化。主函数变大不是本轮回退的解释。

它调用的 `Result<&str, Utf8Error>::expect` 却从103 B变为136 B，显式栈预留从16 B变为48 B。baseline成功路径直接读出两个返回字段；transport先将Ok的16 B字段复制到私有槽，最后重新load返回，Err也生成额外快照。这是具体的候选原因。该 helper 的字节哈希从 `a716e38a835c51affea85408a014daf032eadcc89aedd15b580137dd4b4557c9` 变为 `f4af25d08f9236153654f4ef696e33a23fee43af59196e3f747227630d77b11d`。ts-format动态指令比baseline增加约1.165%，与增加运行工作相容，但尚不能证明该helper解释全部3.51%。

**源码入口。** humantime `src/date.rs:258` 的格式化最后把ASCII buffer经UTF-8转换再写出；KFRC `crates/kfrc-son/src/lower_layout/boundary_transport.rs:234` 的 `capture_owned_field` 分配snapshot并raw_copy，`private_memory.rs:71` 的writer目前只接受各入边同值且严格访问匹配。查明上述snapshot为何未消除，优先局部字段转发，不整体关闭已有效的枚举边界机制。

**可证伪实验。** 固定相同输入、driver与链接配置，分别比较仅修helper snapshot、仅改变section布局、两者累计；记录最终helper与caller反汇编、relocation、指令及cycles。若helper清理后指令下降却耗时回退不消失，则继续按最终链接布局/调用目标定位，不宣布根因闭环。

**验收与负例。** Ok路径返回原slice且无多余临时copy；Err保留正确panic/诊断，覆盖Utf8Error字段、嵌套Result、callee改写输入副本、地址逃逸及输出与输入别名。原ts-format配对实验不再稳定回退超过2%，duration三项不被牺牲。需同时检查seconds/millis/micros/nanos/smart与格式化失败，不只原nanos输入。

## 4. H2：单位匹配先暴露长度常量，再优化比较（P0，依赖有界特化）

**产物证据。** `Unit::from_str` 三个KFRC arm均1294 B/40 call/80 branch，含20处str equality和20处slice equality；LLVM为675 B/0 call/35 branch，以长度1–7分派，再用受长度保护的字节/字比较。40是不同匹配路径的静态调用点，不是每次输入都执行40次。

**源调用与实现。** humantime `src/duration.rs:321` 是40个单位拼写的match，包含ASCII单字节和UTF-8 `µs`。KFRC `rustc_codegen_kfrc/src/codegen.rs:62` 已做两轮wrapper preparation，`kfrc-son/src/transforms/ipo/mod.rs:217` 的普通内联预算仍在调用点特化前检查callee规模；`kfrc-x86/src/lower/abi.rs:146` 已能把常量≤32 B的bcmp变为loads/XOR/compare。当前缺口是让固定字符串/长度穿过wrapper并保留支配长度证明，不能直接断言现有x86不支持小比较。

**方案与可证伪性。** 先采集当前callee命中/拒绝原因，按简化后的图计费和缓存参数事实；移植内联后常量传播、长度分派及小常量比较。先验证具体包装链变成已支持的bcmp或直接整数比较，再考虑通用字符串match重排。不按humantime函数名特判，也不全局提升inline预算。若包装已消失但cycles没有改善，应检查原输入分布与分支预测，而不是继续盲目扩内联。

**验收。** 所有40个合法单位及同长度near-miss与LLVM结果相同，空串、大小写M/m、前后缀、NUL、`µs`、其他非ASCII、页末/对象末输入都不越界。受支持短字符串路径不再调用str/slice equality；比较caller+callee闭包、编译成本和按单位分层运行数据。错误路径仍生成原UnknownUnit字符串及偏移。

## 5. H3：消除Result边界的重复对象搬运（P0体积/栈，运行优先级待profile）

**产物证据。** 最新 `parse_unit` 90个call中56处为memmove，其余含13处OverflowOp::mul、9处OverflowOp::div、4处ok_or等。抽查相对函数偏移+0x2b5和+0x2db，先后复制48 B到两个私有槽，均由Err tag分支控制；相邻成功路径跳过这些copy。因此不能把“56处memmove”写成成功解析每次调用56次，也不能仅凭这个数解释8.66×运行差距。其静态代码、frame和错误路径成本明确存在，动态贡献需分成功/错误输入测量。

**源码入口。** `boundary_transport.rs:234` 建立完整owned-field快照，`lower_layout/maybe_uninit.rs:34` 的raw_copy输出memmove；`kfrc-x86/src/lower/abi.rs:87` 的小copy展开阈值仅32 B，48 B调用不被展开。`private_memory.rs:71` 不创建不同writer的lane Phi，`:127`之后要求非逃逸且精确访问；`boundary_transport.rs:128`的选择对含回边函数保守拒绝。这些约束有语义理由，不能直接删掉。

**方案。** 优先证明连续snapshot的中间副本不逃逸、生命周期不重叠且无需独立身份，消除整个copy或直写最终destination；随后才考虑受限48 B展开与栈槽活性复用。单纯将阈值32提高到48只会把call换成load/store，未必降低frame或成功路径成本。不同incoming字段可用正确lane Phi，循环只先支持出口局部传输；避免把错误payload的未初始化padding当有效标量。

**可证伪与验收。** 将copy-elimination、48 B展开、stack-coloring各做独立候选；分别记录Err/Ok执行计数、frame、load/store、code、compile instructions。若只有冷错误copy变少，只报告静态/错误路径改善。负例包含部分初始化、inactive variant、MaybeUninit/union、重叠memmove、逃逸地址、callee修改副本、sret与未读输入重叠、跨循环动态snapshot。已有四库corpus和解释器/native LLVM差分均须保留。

## 6. H4：checked算术与错误包装一起按闭包优化（P1，与semver共享）

**证据与源头。** humantime `src/duration.rs:91` 的OverflowOp::mul/add本体只是checked运算加ok_or，当前却保留127/122指令、144 B加8 B显式栈调整及错误清理；mul尾部还从临时Result槽复制6个字到sret。`parse_unit`乘法检查有先mul得到高位、成功后再计算低位结果的模式。`parse`/fraction也有checked_mul(10)/checked_add循环，LLVM把大量包装与常量单位计算并入caller。

**实现入口与方案。** `kfrc-son/src/build/semantic/values.rs:311`及`:319`分别建value和overflow，`kfrc-x86/src/lower.rs:511`和`machine/x86_encode.rs:797`分别选择；与[semver S1](semver.md)共用(value, overflow)计算及flags消费。按调用点常量特化OverflowOp::div、ok_or和错误variant，再以简化后的callee计费。仅“Result传播变小”不足以证明算术共享已经完成，两项需独立观察。

**验收。** 每次checked操作只有一次算术，错误路径精确；覆盖u64最大值、×10/+digit边界、单位倍率、nsec进位、0分母和不能整除的小数（具体由原API决定错误），同时测只使用value/flag、跨block和flags被覆盖。parse_fractional_part须新增小数专门workload，不能借原整数dur-parse成绩验收。相对LLVM测caller+callee闭包，防止把代码移入caller记作消失。

## 7. H5：在L层消费恒定布尔事实（P1，可独立落地）

**具体模式。** `Rfc3339Timestamp::fmt` 最新汇编+0xbf到+0xd9先比较signed MIN，再 `and rax,0`、test、jne，目标是永不执行的除法溢出panic。全函数仍有7处div-overflow和2处rem-overflow panic调用点；固定除数不为−1时该MIN检测无用。常量除法的乘法/移位优化已经存在，不能再把这些调用残留描述成“尚未优化常量除法”。

**源码入口。** `kfrc-son/src/lower_layout/cleanup.rs:11`的bit_conditions只处理Compare作为If条件；`kfrc-son/src/transforms/scalar/instcombine.rs:358`明确不做可能抹除poison的 `x & 0 → 0` 等S层改写。应在已发布L层查询整个布尔condition的KnownBits并折叠可证明0/1的分支，保持fixed load/call/trapping操作原控制路径；不能为了几条机器指令破坏S层语义。

**验收。** 上述恒假分支及不可达panic块消失，已知非−1除数的signed MIN路径仍正确；保留MIN/−1、除零、未知除数、Undefined/poison、多用户condition与guarded load负例。记录L图、机器码、动态指令和编译成本。由于该模式在baseline也存在，删除它可以改善绝对差距，但不能据此宣称已经解释transport新引入的格式化回退。

## 8. H6：timestamp parser按两位数字闭包简化（P1，依赖有界特化）

`parse_rfc3339_weak`从baseline2873 B/24call降至2574 B/15call，LLVM1150 B/1call；运行却与baseline持平。humantime `src/date.rs:66`的two_digits将u8转char、to_digit、Option和Result组合，`:108`反复调用该路径并检查日期。应先采集最新调用图及skip reason，暴露两位ASCII事实并转发小聚合返回，再研究u8区间规范化。不能因代码收缩就宣称有效，也不应把所有to_digit一般化为ASCII。

验收先穷举两字节65536组合的结果/错误，再测完整日期；覆盖闰年世纪规则、非法日期、1970前后、9999边界、闰秒、小数精度、Z/+00:00及不支持时区、长度不足和UTF-8。拆valid/invalid与秒/小数输入测量。两位转换、长度guard和错误enum合起来比较，不单看helper是否被内联。

## 9. H7：扩大覆盖与保护编译预算（P1诊断，贯穿上述工作）

原dur-parse没有小数，原ts-format只有nanos。新增整数/小数/单位查找/错误返回及五种格式精度的独立场景，同时保留原四项连续趋势。用只接收bytes的预分配fmt sink隔离算法和String/core::fmt成本，再与现有真实String链路对照，不能只报告更窄microbenchmark。

阶段观测复用KFRC_PROFILE和wrapper preparation窗口，记录每函数S/L节点、snapshot数量、copy字节、inline命中/拒绝原因、各pass时间及instructions。限制graph clone、重复清理和摘要失效成本；humantime当前编译wall已有16.62%收益，应以transport和固定ranges复测作为预算基准，不随意回到更慢候选。详见[编译预算](compile-budget.md)。

建议执行顺序为H1和H7固定基线 → H2/H5及共享checked规则独立候选 → H3逐闭包消除copy → H4/H6剩余包装和循环。H1诊断不等于必须等到完整性能归因才能进行其他静态工作，但任何合并结论必须保留已知回退的状态。

每项验收同时记录运行wall、cycles/instructions/branch、编译wall/CPU/RSS、符号code、object/rlib与linked consumer。首先争取duration parse降至LLVM的2×以内，再逐项争取1.25×；这是工程目标，当前证据不能保证达到。接近LLVM的duration format不得被平均数掩盖稳定回退。最终以[统一计划](plan.md)的单项→累计差分顺序执行。

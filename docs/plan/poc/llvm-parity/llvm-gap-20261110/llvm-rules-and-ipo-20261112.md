# LLVM 规则研究与三个 IPO pass 的实测

FunctionAttrs、TailCallElim、ArgumentPromotion 的保守子集已接入真实后端，统一候选通过正确性验证。三项 IPO 的四库整体运行收益尚未成立；宽 copy 对 base64 decode 的指令收益已用独立消融确认。后续保留32byte函数对齐、默认关闭masked TEST。本轮进一步实现表示分量证书与条件常量返回摘要：独立条件摘要使duration parse少4.81%指令、耗时下降7.51%，但duration format慢3.95%。同沙箱候选/LLVM耗时几何平均仍为2.142071×，未达parity。最终默认条件摘要开、metadata改写显式实验，完整正确性和已测机器产物绑定通过。完整研究与证据见文末。

## 输入与源码

LLVM 研究固定 `llvmorg-21.1.8`；本轮完整读取 `FunctionAttrs.cpp` 2454行、`TailRecursionElimination.cpp` 1014行、`ArgumentPromotion.cpp` 949行。用户给出的2921/1349/988次和低于1%的pass耗时来自LLVM23.1.1/musl另一配置，未作为本次命中或成本。

候选 `candidate-ipo-v3` 基于 HEAD `7fb99c5b057e7ce7f8481f8ce9cd2e05f2d599e6` 加冻结工作树修改：

- 完整源码树 SHA-256：`99dad11d250fc03c5634adb2f9031698a0389509297ce826bf937127aa7e0314`，1346个源文件。
- 源码 tar SHA-256：`c7b08124d54b0774b7640d3353492ad6da17ef6331fbecd2ffad6e88480bb9ef`。
- backend SHA-256：`cd5687c3ae3fffe22dc91bf36a7c617a7ab83357d39d667e60e97226ca6275e3`。
- E2B `iiuhp487uc2w8zq5b5awe`，8vCPU/16GiB，AMD EPYC family25/model17；Rust1.94.1/LLVM21.1.8-rust，MIR0/shared std/O3/CGU16；运行固定CPU7。

同沙箱串行运行原版/候选；profile、图诊断与正式计时分别执行。诊断对象与正常对象逐字节相同。

## 实现与边界

| 机制 | 实际行为 | 当前限制 |
| --- | --- | --- |
| FunctionAttrs | 全crate固定点推导内存None/ReadOnly/Unknown、参数nocapture/访问、noreturn/willreturn；跨已知no-write调用复用load snapshot | 仅exact internal body，核对ABI；保留调用执行、失败与poison观察。DSE仍以调用为屏障；不从只读推导终止 |
| TailCallElim | self-call转参数Phi和Effect Phi循环，支持Call→Jump→共享返回Region拆边 | 拒绝帧分配、byval/indirect参数与varargs；逐实参保留RequireDefined。无accumulator与一般机器尾调用 |
| ArgumentPromotion | 私有直接调用pointer参数改为最多3个scalar parts；定义与全部caller一次事务更新 | 排除导出/地址引用/static relocation/adapter；入口load必执行且必严格观察；无writes/calls/barriers，最终最多6个Direct参数 |
| 扩宽比较缩窄 | 普通整数ext与可无损round-trip常量比较改为窄比较 | 保护VariantWitness与poison；拒绝bool/char/受限有效性/等宽转换 |
| 私有copy宽度保留 | 同一SSA scalar的连续bytes按8/4/2/1写出，6bytes为4+2 | 不新增源load、不越界，先snapshot后store；原初始化、writer、alias、lifetime准入保持生效 |

源码落点：`analysis/function_attrs*`、`transforms/ipo/{tail_recursion,argument_promotion}*`、`transforms/memory/forward.rs`、`lower_layout/private_copy_chunks.rs`、`scalar/instcombine/comparisons.rs`，后端接入在`rustc_codegen_kfrc/src/{lib,codegen}.rs`及`codegen/argument_promotion.rs`。

管线：原S准备→收齐CGU/data/adapters→promotion→既有wrapper化简→attrs推导与窄forward→原内联/seams→TRE→验证→layout→x86。三个独立消融开关为`KFRC_FUNCTION_ATTRS=0`、`KFRC_TAIL_RECURSION=0`、`KFRC_ARGUMENT_PROMOTION=0`；`KFRC_OPTIMIZE=0`统一关闭。

## 正确性与原生机制

FunctionAttrs21、TRE6、ArgumentPromotion5、copy11、extcompare5、matcher和VariantWitness负例均实际执行通过；workspace **1230 passed /0failed /1既有ignored**，release backend、原八组native加新IPO native、四库corpus与13个workload checksum均匹配LLVM。

三个尾递归fixture各消去1个self-call，最终机器码回跳且无self-call；512KiB栈完成100000/100001/250000深度。`scalar_read`真实改为u64 ABI，汇编为`mov rax,rdi`；addressed/static function-pointer/raw pointer负例保持ABI。`sum_fields`在promotion阶段仍有wrapping_add调用，后续才内联，**尚未真实提升**。

独立fixture补充用冻结v3 backend验证无helper call的两/三字段XOR：真实S Start变成2/3个u64参数，机器码仅XOR/返回、无load；共3functions/3args/6parts/3calls。10006case、深递归与原checksum通过，normal8线程与diagnostic1线程全部对象相同。它补齐原生2/3part链路，不改冻结v3源码/四库测量，也未掩盖`sum_fields`负证据。归档SHA为`7416ea29ce5b50276c158370f6e87a3fe542f22c9905ae40efd6f4aa8e9120ea`。

3/6byte copy生成2+1/4+2写宽，10006组odd destination与完整16byte sentinel差分通过。base64四组六字节输出均成为`movl+movw`，同源ByteSwap SSA证据表明无需新增广义CSE。

历史失败完整保留：v1新增深递归native栈溢出，静态定位并批修共享返回形状；v2 extcompare非法类型负例失败且copy filter执行0项，修复测试合同与计数门槛后才恢复验证。v3结果不覆盖这些失败证据。

## 运行与编译结果

三轮串行PMU：v3/baseline的13workload instructions几何平均`0.989733×`；三个IPO全开/全关为`1.000281×`，整体指令收益尚未成立。TRE-off四库whole object全同，省略其独立PMU；attrs-off、promotion-off有实质对象变化及正负交互，不能相加归因。

copy-off与cmp-off各由v3冻结源码制作单文件overlay，分别通过定向测试、九组native与四库corpus/checksum。cmp-off四库所有完整对象逐字节相同，未做独立PMU。copy-off只改变base64 CGU00：

| base64 | 全开instructions | copy-off instructions | 全开/copy-off |
| --- | ---: | ---: | ---: |
| encode | 339573464 | 339573740 | 0.999999 |
| decode | 836910391 | 945710460 | **0.884954** |
| short | 687235439 | 698035358 | **0.984528** |

decode指令下降11.50%、short下降1.55%可归因宽copy。正式wall21为四臂、每项2warmup+21正式轮，轮换起点并反转，共1196次运行：

| workload | v3/baseline耗时 | v3/IPO全关耗时 |
| --- | ---: | ---: |
| semver parse | 0.942708 | 0.940645 |
| semver req | 0.985368 | 0.985205 |
| semver identifier | **1.155470** | **1.156016** |
| base64 encode | 0.999498 | 0.998719 |
| base64 decode | **0.914815** | 1.000570 |
| base64 short | **1.016401** | 1.003111 |
| unicode mixed | 0.998944 | 1.001238 |
| unicode ascii | 0.999848 | 1.000632 |
| unicode reverse | **1.039069** | **1.044953** |
| duration parse | 0.997566 | 0.995714 |
| duration format | 0.946572 | 0.939624 |
| timestamp parse | 0.996629 | 0.998720 |
| timestamp format | 1.002531 | 1.001829 |
| 几何平均 | **0.998188** | **1.003938** |

decode整候选快8.52%，short虽少指令却慢1.64%；identifier和reverse回退必须保留。v3/LLVM耗时几何平均 **2.227459×**，instructions几何平均约2.11709×，尚未达到LLVM水平。

full/meta编译共64次（1warmup+3轮、四库、全开/IPO全关）：full instructions增加0.67%–0.99%，metadata近相同。3轮compile wall噪声较大；这不等于逐pass耗时占比。

## 回退定位与后续依据

FunctionAttrs在unicode reverse减少reload，却使值跨call保活，新增spill/scratch；`prev_boundary` frame由0x218变为0x220加alignment8。duration parser也出现分配器边move变化，不能按S节点数或省掉一次load认定便宜。

semver全开/IPO全关仅三个linked函数指令改变；identifier热函数normalized指令相同但地址平移16B。独立rlib不可达padding对照恢复主要热cluster地址，738个linked函数normalized asm相同，原样重链SHA复现、corpus/checksum通过。identifier三臂2warmup+21轮中，pad0/IPO-off中位为1.152281，pad16/IPO-off为0.999777；cycles230.17M→199.10M，instructions约911.60M不变，支持该回退的布局因果关系。没有恢复所有函数地址，也未证明具体I-cache/BTB机制；原正式wall负结果仍成立。冻结v3仍为16byte函数对齐；后续通用32/64byte函数section对齐用独立ET_REL复制评估，loop bytes保持16byte相对对齐，没有按符号特判。

InstCombine的16个C++/header文件共50218行已实际阅读，台账929条，区分semantic/analysis/delegation/基础设施；源码SHA与区间静态核验通过。用户指定五个后端CPP已逐段读完**117776/117776行**，连额外TargetLowering共**130153/130153行**，三份scalar TD也已全读。后端主账**2553条**分支/分析/契约；六CPP按文件、canonical名称与函数体范围去重，完整具名函数体1813个，不把正则导航候选计入总分母。87文件扩展清单11全读、76未读，共134117/264647行，余130530行；调用点已读不隐含外部helper或生成matcher已读。10份receipt、108段range SHA与主账静态校验通过，dynamic share仍为null。完整源码阅读不等于全部语义变体已证明或已移植；下一批以真实热点与精确kfrc合同为准：表示分量事实、条件常量返回摘要与精确memory operand fold优先，不按条目数预测收益。

LLVM逐规则L1插桩与大规模knockout尚未执行；本轮真实profile与独立消融针对已冻结kfrc产物。源码阅读修正了原评审按“返回改写”语句插桩/禁用的方案：helper可先变图后返空/原节点，value/chain/flags可分步提交，地址matcher也会改图。必须在首个变更前门控、完整提交后计成功；强制ABI和合法化不能无fallback禁用。具体证据见[插桩边界](/Users/shom/Downloads/kfrc-llvm-rule-study-20261112/research/dag-x86/INSTRUMENTATION.md)。

## 后续 TEST 与函数对齐候选

`candidate-mask-v1`在v3上实现register masked TEST，保留16byte函数对齐。source tree为`5f1f00d8910785c733897656f1248f53d3e2e1ca456dab410771caba62a4bc6b`，backend为`6907416c24bde87d93d408b8b678b7c82a307efe34b5e8ed93b119d6343d9c5e`。workspace **1233 passed / 0 failed / 1既有ignored**；新增lower差分、84函数×1006输入的真实编码E2E、九组native和四库corpus/checksum均通过。`KFRC_MASKED_TEST=0`关闭后，四库所有whole object逐字节等于冻结v3。

开启后终码新增41个register TEST（semver11/base649/unicode12/humantime9），其中39个TEST32、2个TEST8，无实际TEST64命中，未融合或缩窄用户内存访问。真实年份检查的`mov rax,r12; and rax,3; test rax,rax; jne`变为`test r12b,3; jne`。规则保留完整source、原观察与effect，只清除dead的单用AND/copy；拒绝多定义、多用、跨cast、其他flags消费者及不能精确编码的mask。首版归档SHA为`cdcf457d37c55f1447558629597d27bb54e1af58517359981885b907dfe09116`。

通用align16/32/64的七轮screen中，32/16 wall几何平均为0.974214×，64/16为0.986340×，instructions约相同；同时保留32的duration-format回退1.021648×，正式复测前不认定稳定收益。32相对16的四库执行段增量为1008/240/448/512B，全部来自padding。

`candidate-mask-v2`只在v1上增加函数section32的三文件delta，source tree为`bcb5f03c69e93aed9a42851244641407e0b58ca25569d119d05985d8b755cd5b`，tar为`fac3b771b7e144fbe369029fc2e2c82613bf63a3650b95cc5c0c7c032a9b9fd8`。loop仍按16byte相对对齐。定向、loop/native、workspace和四库on/off全部通过，所有对象section数据/symbol/relocation与对应v1/v3相同；v2-off最终exe与v3-align32独立overlay逐字节相同。

四臂mask32/v3-align32/v3-align16/LLVM正式测量完成1196次wall及156次独立PMU，checksum与二进制指纹一致。32/16配对wall中位几何平均 **0.973022×**；identifier/decode/reverse约改善13.50%/7.17%/3.95%，instructions近同。duration-format正式中位仍为1.014480、IQR0.970999–1.019212跨1，负结果保留。故保留32函数对齐，loop仍16。

masked TEST相对v3-align32的wall几何平均 **1.000186×**；req/dur-parse/ts-parse分别1.009982/1.013171/1.010878，IQR均高于1且cycles同向。reverse快0.61%、instructions减少0.58%，duration-format快2.60%但instructions不变，含布局效应。四库full编译instructions增加0.11%–0.23%，metadata近同；3轮wall波动明显，不能证明稳定耗时成本。静态少指令并未换来全局收益，因此默认关闭，仅`KFRC_MASKED_TEST=1`显式开启。

最终`candidate-default-v4`只改变TEST gate，tree为`33aea8764d1d665890a32e739a8f2a7402261b42e74fa6d6897dcc3e26bd5028`，tar为`28c7b886748ea6f55aa801f2c803a7c8cabf252829fc8a0ad372c18af65e0eeb`，backend为`436ac2b091bc26083037cc7743f6b6d68ecfeeefea26bcea4c6022c324ede428`。定向、完整workspace（1233通过、0失败、1既有ignored）、九组native、四库default/`=1`语料和13项checksum全部通过。两模式各50个whole object及各4个exe分别逐字节等于正式v3-align32/mask32臂，直接绑定已测结果，无需重复计时。默认版本相对LLVM为 **2.170501×**，req约6.18×、dur-parse约7.94×仍是主要差距，不能称为parity。最终正确性归档SHA为`2489382f618865012106e6c2ddc6410d22e65105f22cd7cc649a39fed6eb4c83`，完整support归档SHA为`2fe965c7cf4b9ed82fde982ad31d0ec6e67d678024748e25ca07fbd52aca7ef6`。所需日志和产物已下载验SHA，本任务E2B已于2026-11-11T22:47:29Z释放。

按最终默认二进制绑定的原始3轮PMU与21轮wall复算，duration parse、semver req、unicode reverse分别占13配置批次超额instructions的30.3496%、18.8565%、12.5344%，合61.7405%。这是本批暴露权重，不是生产流量或任一规则可消除的份额。下一轮依此优先补表示分量/snapshot/原观察点definedness传递和参数关系条件摘要，再为精确memory-source fold统计真实同宽单用load候选。当前无条件FunctionAttrs尚不表达“长度不符→false且无效果”，单纯增加局部比较规则无法替代这个前提。[复算输入与公式](/Users/shom/Downloads/kfrc-llvm-rule-study-20261112/research/profile-priorities-default-v4.json)。

## 表示分量与条件常量返回

新一轮从上述已验证的`candidate-default-v4`继续，正常源码冻结为`candidate-relations-v4`：1360文件，tree SHA-256 `4a4195aba35dadf5485f557055da6a79336ac28181d3a581a4b53939bb1f5015`，backend `e7844afa18f5bc6bdbe75c00457078243fd30101f5ee40e57cf0ea07502fa0a5`。E2B换为独立环境`iu0z55mmnkgigix78u3sf`，实际8vCPU/约16GiB、同一CPU型号、Rust1.94.1/LLVM21.1.8，运行固定CPU7；不同宿主机的绝对耗时不混合。该候选与前述默认v4名称不同。

表示证书只接受slice/str宽指针之间已执行的完整固定转换，要求RecordedTypes的语义、字段类型、布局、大小、偏移和metadata有效性一致。它从原SSA snapshot复用长度，不reload，保留完整TransmuteValue、poison观察及目标decode；拒绝Region/Phi与循环动态实例，control/effect和snapshot均有256步预算。Field改写与只读证书分离，条件摘要可在关闭Field改写时独立使用证书。

条件摘要只处理exact internal S图：入口参数或至多两层Field的整数Eq/Ne，选中分支64步内无读取/额外观察、effect精确透传并返回bool常量。caller至少一侧selector为结构性常量，在原调用位置依原顺序RequireDefined全部参数后加guard，另一臂保留原call。摘要只推导一次，在post-inline/seams后消费一次，每caller最多64次split，不重复展开fallback。这是通用参数关系规则，无函数名或workload特判，也不把FunctionAttrs的无条件no-write扩充为可删call的授权。

独立conditional-only实证Unit的40个call中split11个：32个direct slice候选中的其余21个因当前spine预算拒绝，另8个str wrapper保留。64个固定TransmuteValue和128个RequireDefined保留；11个guard在Layout共用一次入口metadata load。Unit由1294增至1451B，parse_unit本体仍6230B，Layout仍含52个memmove节点；这是静态图数量，不是动态libc调用次数，也未证明全部可删。真实parse_unit调用了新的Unit，因此覆盖该热点调用链。semver的into_searcher虽S图少节点，四gate最终Layout/机器码相同，不能把它记成终码收益。

v4已通过metadata开/关各11、conditional在两种metadata配置下各10、backend11、workspace **1254 passed / 0 failed / 1既有ignored**、13次native脚本，以及四库四gate语料和每臂13项checksum。conditional-only的semver/base64/unicode全部对象和exe逐字节等于旧默认，只有humantime的1/13 object改变。单次正常backend DUMP确认11次split，诊断的全部13个humantime对象与正常臂相同；日志overlay不参与计时。

正式四臂为旧默认、conditional-only、两项全开、LLVM，独立3轮PMU与24轮wall，共156次计数和1352次wall（含2warmup）。每4轮Latin rotation后将整个4轮块reverse，共3个8轮块；每arm每slot恰6次。所有过程记录纳秒时间、checksum与二进制SHA，PMU、wall和编译成本分阶段执行。

| workload | 仅条件摘要/旧默认wall | 两项全开/旧默认wall | 仅条件摘要instructions | 两项全开instructions |
| --- | ---: | ---: | ---: | ---: |
| duration parse | **0.924922** | **0.883857** | **0.951889** | **0.938512** |
| duration format | **1.039508** | **1.037475** | 1.000000 | 0.999999 |
| semver identifier | 0.998737 | **1.151746** | 1.000000 | 0.965709 |
| unicode reverse | 0.998477 | 0.939289 | 0.999999 | 0.970692 |
| 13配置wall几何平均 | 0.993336 | 0.993827 | — | — |

duration parse仅条件摘要的wall IQR为0.916593–0.941923、MAD0.015661，cycles为0.928085；长度不符绕过调用与少4.81%指令形成直接机制证据。metadata引入的identifier回退在正式wall与cycles再次出现，超过该项position/drift变化，不能因少3.43%指令而默认接受。formatter不执行parser，已保存的热函数归一指令相同而地址变化；其wall IQR为1.020078–1.047039、cycles1.043583，回退保留，具体前端/缓存/预测器因果仍未证明。

整体0.993336只是本批观测，不能宣称稳定的通用0.67%收益：conditional-only与旧默认的semver req二进制SHA完全相同，wall仍为0.978389（IQR0.967424–0.988035）。平衡顺序未消除全部进程状态/地址效应。同沙箱conditional-only/LLVM为 **2.142071×**，旧默认/LLVM为2.157042×；不与旧宿主机2.170501×直接计算改进。最终源码选择conditional默认开、metadata Field改写仅`KFRC_REPR_WIDEPTR_METADATA=1`显式开启，只读证书始终独立可用。

最终`candidate-relations-default-v5`只改metadata gate与测试的enabled预期，冻结1360文件tree为`99302efa761c6ff781cca35cd098f6e5fe8741984b78cda478ed571c79d56d54`，source tar为`dd1224f23288057dadcf9abd541826b38277d1f1b017a3e622c0faebf6350302`，backend为`7d6f17dfdd21ca1d1d6df5f365c824c369b3fd2455d7be9b63e075f89e12c876`。metadata default/0/1各11、conditional default/0/1各10、backend11、完整workspace1254/0/1既有ignored、13次native与四gate四库语料/checksum全部通过。四gate共200个whole objects和16个exe均逐字节绑定：default对应正式conditional-only，显式metadata=1对应all-on，conditional-off/both-off对应旧默认。由精确机器产物直接继承正式运行结果，未无意义重跑计时；最终backend的编译成本另行测量，不能凭产物相同继承编译器运行成本。

编译成本另行串行测量full/meta、旧backend/新backend两项全关/conditional-only三臂，四库每臂1warmup+3正式轮。v4中间backend的96过程保留于归档`7fe6b518d1f111749803abf632dd2839fae3b1c0f5422384f1866cb8051b8125`；最终v5 backend另测96过程，48个full编译的全部objects与正常臂精确相同。最终默认/同backend conditional-off的full instructions在semver/base64/unicode为1.000560/1.000476/1.000583，humantime为 **1.014595**，wall依次1.019337/1.019087/1.004477/1.032345。即有命中的humantime机制成本约+1.46% instructions，3轮wall只报观测。默认/旧backend full instructions依次0.993729/0.989924/0.996014/1.005174，wall依次1.004246/0.997070/1.007681/1.017406；新backend关机制本身也有差异，不能把其变化归为机制节省或声称稳定编译提速。meta两组instructions比范围0.999941–1.000223。最终成本归档SHA为`3a13944fedc0a188046361725d24fb9ef802508729742180977ea2bff84b0c6e`，最终正确性归档SHA为`ae5b58251c7b732337e55481db0d6a31b16b492a502928e440d8d2121986870b`。

完整本轮入口为[README](/Users/shom/Downloads/kfrc-next-20261112/README.md)，[正式原始结果](/Users/shom/Downloads/kfrc-next-20261112/validation/relations-v4-formal24/summary.json)、[position/drift与配对几何平均](/Users/shom/Downloads/kfrc-next-20261112/validation/relations-v4-formal24/position-drift-analysis.json)。正式归档SHA为`e3e0f059d398c8df7c8805e9878b9b1a39fd06564b15156e5b4c5c82a05ff81b`，已下载并逐文件验SHA。

从本轮3次PMU的每项中位instructions按原13配置迭代批次复算，默认候选的超额instructions总计5,892,067,732；duration parse/semver req/unicode reverse分别占29.1273%/19.1874%/12.7543%，合61.0691%。次数已含迭代，不再次乘权重，也不代表生产流量或某规则可消除份额。下一轮按[具体源码、证伪条件与证据](/Users/shom/Downloads/kfrc-next-20261112/research/next-priorities.md)优先研究共享关系证明/字面量分派、typed snapshot分量复用、精确宽度reload/零比较融合。当前没有把全部LLVM规则做动态L1/knockout，静态inventory不冒充逐规则收益。

最终正确性1574文件、编译成本1129文件均已下载逐项核对；support另存三个normal backend实物、冻结源码、输入/工具链指纹、命令/harness、格式化回执与失败历史指针，共61项全部SHA一致，归档SHA为`6f4809343873d0c592b70478396f54f1d01de07fc5b5dfb4f1b610adbb059980`。[当前1360个build输入静态核对](/Users/shom/Downloads/kfrc-next-20261112/validation/current-default-v5-source-check.json)与冻结v5集合和SHA全部相同；报告不在build manifest内，独立记录SHA。本轮独立沙箱已于2026-11-12T05:10:34Z释放，后续get_info返回SandboxNotFoundException确认不可用，[释放回执](/Users/shom/Downloads/kfrc-next-20261112/validation/sandbox-release-receipt.json)。无commit、push或PR。

## 可追溯证据

- [研究入口](/Users/shom/Downloads/kfrc-llvm-rule-study-20261112/README.md)：固定LLVM源码、InstCombine与DAG/X86规则台账、阅读receipt、优先级与移植边界。
- [按实际效果排序](/Users/shom/Downloads/kfrc-llvm-rule-study-20261112/research/EFFECTIVENESS.md)：已证实收益、三个IPO边界和下一轮重点缺口。
- [当前工作树与已验证v4对应](/Users/shom/Downloads/kfrc-llvm-rule-study-20261112/research/current-default-v4-source-check.json)：原冻结范围1349个文件与source tree SHA全部一致；仅静态文件核对。
- [验证与完整归因](/Users/shom/Downloads/kfrc-bottom-up-20261112/validation/candidate-ipo-v3-report.md)：correctness、单项IPO消融、正式wall、compile-cost、copy/cmp消融与归档SHA。
- [正式wall原始与配对结果](/Users/shom/Downloads/kfrc-bottom-up-20261112/validation/candidate-ipo-v3-wall21/paired-dispersion-position.json)。
- [copy/cmp单规则结果](/Users/shom/Downloads/kfrc-bottom-up-20261112/validation/rule-overlay-screen-v1/summary.json)。
- [深递归与ABI原生链路](/Users/shom/.codex/worktrees/decode-throughput/kfrc-skeleton-20261108/scripts/poc-validation/ipo.sh)。
- [masked TEST实现与首版验证](/Users/shom/Downloads/kfrc-llvm-rule-study-20261112/research/dag-x86/MASKED-TEST-IMPLEMENTATION.md)、[函数对齐方案](/Users/shom/Downloads/kfrc-llvm-rule-study-20261112/research/dag-x86/FUNCTION-ALIGNMENT.md)。
- [TEST与函数对齐的正式结果](/Users/shom/Downloads/kfrc-bottom-up-20261112/validation/candidate-mask-v2-report.md)：包括负结果、各项LLVM差距与编译成本。

# LP3：保留调用结果表示，避免动态 Error 拆开后重新编码

日期：2026-11-09。只读研究，不修改生产／测试、不编译或计时。
范围是 candidate23→24 的 `semver::Version::from_str`；candidate26 正在修复中间 Phi copy，
本文讨论其后仍存在的 constructor 编码。统计来自冻结产物，不能当作 candidate26 实测。

## 已解释完的放大：13 个 constructor，只有 7 个含动态 Error

输入：

- `evidence/poc-llvm-parity-lp3-20261109/results/candidate23/from-str-comparison.json`
- `evidence/poc-llvm-parity-lp3-20261109/results/candidate24/from-str-comparison.json`
- 两候选同目录 `attribution/kfrc/graphs/stderr.gz`，仅提取包含 `6semver5parse`、`7Version`、
  `from_str` 的 Specialized／Layout 图；用 Python 解析节点与地址根，不打印完整大 JSON。

| 项目 | candidate23 | candidate24 | 差值 |
|---|---:|---:|---:|
| 最终 S 节点 | 745 | 745 | 0 |
| L 节点 | 4,447 | 5,556 | +1,109 |
| Store | 80 | 181 | +101 |
| If | 136 | 188 | +52 |
| Region | 168 | 220 | +52 |
| field_loads | 180 | 180 | 0 |

24 的 8 个 owned Phi 收集 13 个 MakeVariant constructor 和 7 条 Phi→Phi 入边。
冻结24仍每边 stage→destination，Phi来源还先复制到stage，另有最终 materialize：
`13 + 7×2 + 1 = 28` 个 memmove，每个40字节，静态调用参数之和1,120字节。
这不是一次执行必然复制的字节数。26的immutable pointer Phi修复负责减少这些copy。

独立于copy，24把原来最终一次编码，移到了每个constructor边：

- 7个动态Error constructor，各生成 **9 If、9 Region、14 Store**。
- 其余6个constructor分别为：空Error 2 Store、两种EmptySegment各3 Store、
  UnexpectedCharAfter 4 Store、两个Ok(Version)各5 Store，共22 Store，无动态编码If。
- 所以新constructor总计 `7×14 + 22 = 120 Store`、`7×9 = 63 If/Region`。
- 23最终return slot（L n3801）的统一编码为19 Store、11 If、11 Region。
  净变化正好是 `120−19=101` Store，`63−11=52` If/Region，完全解释上表差值。

ErrorKind有12个声明variant，但3个fieldless variant不需要payload分支；因此动态编码有9个If，
不是12个。14 Store = 12个声明payload leaf的store + ErrorKind tag + 外层Result tag。
其payload按条件执行；该计数是静态代码体积，不表示每次错误返回执行14次store。

## 七条动态路径的确切来源

下表S节点均为冻结24最终745节点图；L节点均为冻结24最终5556节点图，二者不能混作同一编号。

| S Result<Version,Error> constructor | Error 来源 | L stage根／memmove | 静态编码 |
|---|---|---|---|
| n713(Err n293) | numeric_identifier call n27，返回projection n30，Err field | n3965／n4143 | 9 If、14 Store |
| n716(Err n383) | dot内Result Phi n356/n357，Error::new calls n334、n342 | n4147／n4325 | 9 If、14 Store |
| n721(Err n410) | numeric_identifier call n36，projection n39 | n4329／n4507 | 9 If、14 Store |
| n724(Err n500) | dot内Result Phi n473/n474，Error::new calls n451、n459 | n4511／n4689 | 9 If、14 Store |
| n732(Err n527) | numeric_identifier call n45，projection n48 | n4693／n4871 | 9 If、14 Store |
| n738(Err n563) | prerelease_identifier call n78，projection n81 | n4923／n5101 | 9 If、14 Store |
| n744(Err n608) | build_identifier call n142，projection n145 | n5155／n5333 | 9 If、14 Store |

其余constructor是n24、n59、n96、n160、n228、n730。不能为它们泛化增加动态表示处理。
特别是n228的Position动态选择仍是fieldless enum scalar，constructor本身不需要9路ErrorKind分支。

上游也已经拆过一次同一类Error：

| 调用来源 | S calls | L calls | 当前每处读取 |
|---|---|---|---|
| numeric_identifier | 27、36、45 | 24、30、36 | 外层Result共17 Load，含Error子树13 Load和9个内层If |
| prerelease_identifier | 78 | 58 | 同上 |
| build_identifier | 142 | 91 | 同上 |
| Error::new | 334、342、451、459 | 192、197、263、268 | Error本体13 Load、9 If |

9个调用结果的Error子树合计 **117 Load、81 If**；其中5个外层Result仍需要自己的tag与Ok路径，
不能把完整外层decode一起删除。9份decode经5个直接Err field和2个Phi合并，最后变成7份encode。

## 当前链路为何重复

`lower_layout::run` 先将ABI边界改成LayoutValue，再展开Rust语义值：

1. `values::expand(Projection)` 调 `memory::decode_boundary`，把调用返回的物理对象当读取地址。
2. `decode_fields_demand` 对composite递归；遇到Result和ErrorKind调用 `decode_variant_demand`。
   后者对每个有需求的payload variant生成条件、guarded loads、merge与Undefined Phi。
   结果是`Parts::Variant { variant, variants }`，已经忘记“这些字段来自同一份现成表示”。
3. `VariantField`只从这个递归Value树取子树；MakeVariant又包上外层Result。虽然类型和值都相同，
   它携带的是拆出的tag及所有payload SSA，不再携带可搬运的原表示。
4. 24 `finish_snapshot_phis`在constructor边调用 `encode_fields`；进入上述子树就执行
   `encode_variant`，重新按动态ErrorKind tag走9条guard，写payload与tag。

`values`缓存避免同一S节点再次decode，但不能避免“已decode的值又在另一个sink编码”。
仅记住外层Result constructor是已知Err不能解决，未知的是嵌套ErrorKind。

## 下一项最小设计：在需求树内标出只搬运的完整子值

建议先只处理上表 **5个直接call Result的Err字段**，之后再扩展dot的2个Phi路径。
这五处没有新Error字段修改，也不需要新跨Phi分析；可独立验证decode与encode同时减少。
规则不按semver类型名，而按consumer闭包与具体类型身份判断。

### 需求与表示必须是两个维度

现有`Demand::All`只表达“所有语义字段被需要”，无法区别最终只搬运完整值，还是每个字段被观察。
保留现有Demand计算；增加独立的、每个subtree的transport证明，至少区分：

- 必须读取语义字段：现有SSA展开；
- 完整representation只用于构造器字段、整值Return/StoreValue、或已证明安全的transport Phi：
  保留一份owned snapshot。

不能把`All`直接重新解释成“可raw copy”。任何字段观察、tag检查、Transmute、未知consumer、
类型变化或同一子值的其他用途都应取消该subtree优化。外层Result仍需tag与Ok字段，
但Err字段可以独立保持storage。`EncodedVariant`目前只保存tag，不能冒充完整Error表示。

### 所有权：采用私有快照，不借用源地址

最稳妥的首版是在当前decode的同一固定点，为选中的完整子值建立独立private snapshot：

- 5个Result的Err字段：保留外层Err guard，在guard内按已核对layout offset复制Error子对象；
  不读其中inactive payload，不额外按ErrorKind分类。保存`Parts::OwnedStorage`及其类型。
- constructor编码外层Err时，将这个完整Error表示复制到新Result的Error字段，再按原规则写外层tag；
  不重新decode/re-encode ErrorKind。
- 当前consumer若在原Err witness作用域之外，拒绝；不能让Error storage在Ok路径被读取。
  若用pointer Phi表达条件定义，inactive路径只能携带未解引用sentinel，不能随意创建真实snapshot。
- 原Rust Source/Return/RequireDefined观察契约仍须在S验证；raw copy本身不把未初始化padding或
  inactive bytes转成已定义的Rust值。不要通过整宽typed u64 load/store模拟raw copy。

首版copy即使来自call结果也应保留，除非新增明确的owned ABI契约。当前ABI实现确实分配结果slot：
`kfrc-x86/lower/cast.rs::emit_cast_call`的Indirect路径为每个call result分配StackSlot；
Direct aggregate／Pair／Cast也在接收返回值后创建本地slot。但“已分配slot”不足以证明它在
后续调用中不会通过捕获的sret地址被修改；`FunctionAbi`只记录PassMode，没有nocapture证明。
LLVM的`sret`属性本身也没有“永不被捕获”的含义。

因此可以优先针对**由返回寄存器在caller构造的Direct/Pair/Cast aggregate slot**建立私有证明，
它的地址未传给callee；对于Indirect结果，要么在return continuation立即复制，要么先引入
可验证的Rust ABI no-capture保证。不能把Start的Indirect参数、LoadValue源地址、任何用户
可见FieldAddr、callee可变参数或外部源地址直接标成OwnedStorage。

另外，当前`raw_copy`无论size都生成libc memmove；直接应用会增加调用。第一版可用它验证语义，
但恢复性能计时前须确认小Error copy被合理lower，或存在可保留init/provenance的原始copy节点。
不能把63个分支消失当作净性能收益，也不能以不安全整宽typed load规避memmove。

### 扩展到两个dot Phi需要什么

Error::new的四个返回值只被完整包装进Result，再穿过两层Phi与Err projection，最后进入外层Err。
要消除它们的36个decode If与两份18个encode If，需要将transport闭包沿
`Call result → MakeVariant → Phi → VariantField → MakeVariant`传播。

复用26的不可变pointer Phi思想，但在**子值**级别选中Error；每个真实来源只初始化一次private
snapshot，Phi只选择指针且不覆写任何来源。初版拒绝所有循环SCC、可写alias与跨动态实例引用。
不要只给outer Result的Phi做pointer Phi，再把Error子树恢复成逻辑SSA；那样仍有这18个encode If。

## 可验收的静态上限，而不是已实现收益

以24图为参照，且consumer闭包逐项验证成功：

| 范围 | 可移除内层decode | 可移除内层encode | 新行为 |
|---|---:|---:|---|
| 首版5个直接Err字段 | 65 Load、45 If/Region | 65 Store、45 If/Region | 5处snapshot capture＋5处Error表示搬运；外层Result tag store保留 |
| 扩展全部9个来源／7个sink | 117 Load、81 If/Region | 91 Store、63 If/Region | 9处capture＋7处搬运；dot子值pointer Phi |

这里只计ErrorKind递归decode/encode，外层Result guards与实际业务分支保留。
例如首版的If理论变化为188−90=98，完整扩展为188−144=44；这些数是假定静态形状不被其他pass
改写的核对目标，不能预先承诺candidate26后的精确计数。原始copy节点和ABI复制需另计。

## LLVM参考：表示搬运与call slot优化有不同证明义务

核对版本为LLVM `llvmorg-21.1.8`，缓存于`/tmp/kfrc-llvm-memory-21.1.8`。

- [MemCpyOptimizer.cpp::performCallSlotOptzn](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/MemCpyOptimizer.cpp#L838)
  处理call写src再copy到dst的优化。它要求src为alloca、完整覆盖、同block、dst在中间无访问、
  目标dereferenceable且对齐、源use受限，还检查捕获与unwind可观察性。不是看到sret就复用任意地址。
- [LangRef sret](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/LangRef.rst#L1326)
  只说明返回存储的ABI和caller保证有效／对齐，不提供自动nocapture保证。
- [SROA.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L161)
  按实际内存访问slice划分并可拆memcpy；对pointer Phi/select的load speculation有独立安全检查。
  对本问题有用的是保留物理搬运事实再按真实使用切分，不是把Rust enum全部声明payload先展开。

MemCpyOptimizer SHA-256：`7180f01312e76679da78fe8efa43ff57d4fcaf3107682d6e4c208ebf0ea5f9b4`。
SROA SHA-256：`b29efe3d5c94642b938d6dad3ddf1d20d1ea545ed04166a47d743abc6a1f6a8a`。

## 验证fixture与停止条件

1. call返回Result<Small,NestedEnum>，Err子对象原样嵌入Result<Big,NestedEnum>；覆盖每个内层variant，
   无payload variant、有char/整数payload、外层Ok路径；S/L与真实LLVM运行结果一致。
2. 同一call返回的Ok字段被实际读取、Err整体被transport；验证mixed demand不丢Ok字段与tag。
3. 返回后执行可修改原输入的调用，先前owned值必须不变；Start间接参数与LoadValue源不得被借用。
4. 两个Error来源经diamond合流，再完整transport；每源只初始化一次、无Phi写回；循环负例拒绝。
5. padded enum、inactive undefined、MaybeUninit子字段与含指针payload：raw copy保留初始化和provenance，
   不新增typed observation；tag-only或字段观察用途退出优化。
6. 真实semver先做三臂行为验收，再比较Error内层分支、Store、Load、raw-copy调用与字节数。
   若新copy数量或调用成本抵消收益，暂停计时，先解决copy表示／lowering；不扩大到一般lazy地址值。

## candidate28 最小实现契约：仅直接调用返回的一个完整variant字段

本节细化为可直接实现的方案。candidate26已将运输copy由28降为1，但Store181／If188仍在；
本节以此为起点，不改变26的pointer Phi，也不依赖27的Return copy消除。
实现由aggregate agent负责全部`lower_layout/`与对应测试；本文只记录接口与约束。

### 先选定证书，不在递归decode时猜用途

新增一个独立结构分析，输出形如：

```text
TransportFields = Map<RootProjection, Set<(VariantIndex, FieldIndex)>>
```

其中key是**确切调用结果的NodeId和声明字段路径**，不是TyId、类型名或callee symbol。
相同TyId的另一调用、同一enum另一个field不会继承该证书。选中的是outer enum的直接payload
field，不扫描任意深度path，也不改变普通Demand合并规则。

初始化顺序保持：先`erase_requirements`、`erase_ignored_transport`，再建立projection index、
`demand::analyze`和`owned_transport::select`，最后建立本证书。分析针对该时刻的graph；后续run
不compact直到发布末尾，key因此稳定。原始745节点S图的编号仅用于追踪证据，不能硬编码到分析。

选择规则逐项为：

1. **整函数无控制环。** 使用一次`ControlFlow::new`；其RPO任一predecessor position ≥ 当前position
   就保守拒绝全函数。该判定在`analysis/refinement.rs::PredicateFacts::new`已有相同用法。
   冻结24 from_str控制骨架静态提取为407个可达节点，无环。这条规则会漏掉带无关循环的函数，
   但避免capture slot在下一次调用执行时覆写仍被旧SSA snapshot引用的内容。
2. root必须为`Projection[2]`，唯一operand直接是`Opcode::Call`，结果为有完整记录的enum。
   首版拒绝CallIndirect、Start、Opaque、LoadValue、TransmuteValue以及任何Phi root。
   要求outer enum `TagEncoding::Direct|Niche`且有tag，拒绝Single／Empty，以保证后面统一走guard。
3. root所有use只接受`Operand(0)`的VariantSwitch、VariantOf、Discriminant、VariantWitness或
   VariantField。source被完整Return、Store、Aggregate、Phi、Call使用时拒绝所有selected field；
   否则缓存中的mixed Value可能流进未支持的generic fill_phi或整值观察。
   **不要求root只有一个use，也不要求其所有variant字段都搬运。** 五个真实root都有Ok字段真实读取。
4. 候选`(variant,field)`的有效需求必须是完整`Demand::All`，且field为非零大小composite，
   有sized layout和准确TyId。outer field offset + child size用checked运算并在parent layout内。
   递归检查**值字段**：拒绝MaybeUninit、union、Param、Other、无layout／无字段表的ADT等未知结构。
   enum各声明variant字段也检查；pointer/reference只检查它自己的layout，不递归pointee，
   因为搬运指针值不搬运它指向的对象。零大小field继续旧路径。
5. 同一root同一key的**全部**VariantField extractor都必须输入恰好为`[root,witness]`；witness是
   `VariantWitness`，value identity、variant均准确匹配。每个extractor自身也需`Demand::All`。
   任一同key extractor有不安全use，该key整体取消，不能让values缓存一边是storage一边是fields。
6. 每个extractor所有use仅允许MakeVariant的实际payload operand，并检查目标variant的声明field
   类型等于extractor TyId。禁止Field、VariantOf、Discriminant、InsertField、语义Phi、Call、
   TransmuteValue以及直接返回／存储extractor；这是首版故意比理论能力更窄的边界。
7. 每个上述MakeVariant所有use只允许Return的operand1、StoreValue的operand2，或者
   **`owned_transport::select`已经选中的Phi**的operand>0。后者的全部后继闭包由现有selector
   确认最终只到完整sink；不再新建另一套Phi闭包。普通Phi即使无环仍拒绝，避免generic fill_phi。
   MakeVariant出现其他use，包括被拆字段、作为Call实参等，一律取消该key。
8. VariantWitness是编译证明use，不是运行时field/tag观察；不能因root有witness就拒绝。
   RequireDefined在分析前已经擦除，不能在原S图上因其use而误判五条路径均不满足规则。
   也不能为选择规则重新verify擦除Assume后的临时S图；原发布入口已要求Verified，现有witness的
   dominance／动态instance／consumer-site契约在擦除前已验证。

规则6／7闭包之所以必须同时存在：一个Error字段只进MakeVariant，不代表它之后不会进入普通
Result Phi；如果普通Phi对Error创建逻辑字段树，`fill_phi(target_fields,OwnedStorage)`会报错。
选中的owned Phi则先在来源边materialize完整constructor，其nested OwnedStorage已由encode支持。

### 五条真实路径如何满足

以下编号仍是冻结24原S图。每个field还有一个RequireDefined use，分析前擦除后仅余表中的constructor。

| Call | Root Projection2 | 所选key | VariantField／Witness | 唯一constructor | 首个已选owned Phi |
|---:|---:|---|---|---:|---:|
| n27 | n30 | (1,0) | n293／n294 | n713 | n249 |
| n36 | n39 | (1,0) | n410／n411 | n721 | n250 |
| n45 | n48 | (1,0) | n527／n528 | n732 | n251 |
| n78 | n81 | (1,0) | n563／n564 | n738 | n253 |
| n142 | n145 | (1,0) | n608／n609 | n744 | n255 |

每个root另有两个VariantSwitch、一个Ok VariantField、Ok/Err两个Witness。
因此“所有root users必须都是所选VariantField”会漏掉全部五处；“root只能有一次VariantSwitch”也会
漏掉全部五处。dot两条路径的source是Phi，按规则2主动拒绝，不在candidate28纳入。

### 不改变Demand，也不扩充generic fill_phi

建议接口：

```text
memory::decode_boundary(root,ty):
    if certificate[root] nonempty:
        decode_variant_with_transport(address=root, ty, control, effect,
                                      demand(root), selected_keys)
    else:
        existing decode_encoded / decode_fields_demand

variants::decode_variant_demand(..., demand):
    decode_variant_with_transport(..., demand, empty_keys)
```

`decode_variant_with_transport`只在当前outer enum层读取keys；递归`decode_fields_demand`不传这些keys。
这比给所有递归decode函数增加root/path参数更小，也防止嵌套相同variant/field误用证书。
selector需要拒绝与encoded_values重复的root，或证明不可能重叠；当前encoded只接tag用途或精确
niche pointer payload，所选非零composite字段不满足它，但显式排除更容易维护。

原outer variant循环已有`yes/no` guard和`yes_effect`。在选中field处替换这一次递归decode：

```text
source = field_address(call_result, recorded_offset, field_ty)
private = opaque_slot(field_ty)
raw_copy(source, private, size(field_ty), yes_control, yes_effect)
field_value = Value { ty: field_ty, parts: OwnedStorage(private) }
```

这里`opaque_slot`只是分配slot的已有helper，Value必须是**OwnedStorage**，不能是OpaqueStorage；
后者属于MaybeUninit与其可覆写Phi transport协议。capture固定在原call结果decode时点，copy与原
outer branch通过`decode_boundary`已有control/effect splice串联，不能延迟到constructor处再读原地址。
不论call的物理结果是Indirect还是寄存器重建，这份private slot地址都不交给原callee。

原outer merge得到`region`和`merged_effect`后，逐个field处理：普通field保持现在的
`phi_tree_demand → undefined_demand → fill_phi`；仅选中的OwnedStorage专门生成：

```text
pointer_ty = pointer_type(field_ty)              // PlaceAddress<field_ty>
inactive = Undefined : Rust(pointer_ty)
pointer = Phi(region, private, inactive) : Rust(pointer_ty)
merged_field = Value { ty: field_ty, parts: OwnedStorage(pointer) }
```

Phi operand顺序必须与`merge(yes,no,yes_effect,original_effect)`的Region输入一致。
不能使用`NodeId::NONE`作为operand、不能用`Undefined : Rust(field_ty)`代替pointer类型、不能调用
`undefined_demand(field_ty)`构造一个完整Error树，也不能将inactive指针作为任何copy或load的输入执行。

`Undefined`在Phi中不会被观察。它只会在source enum不处于该variant时被选中；原VariantWitness
保证所有extractor的运行时consumer sites只在active case执行。在active case，pointer必然指向此前
guard已初始化的private slot，effect链保证capture先于consumer。对整个函数拒绝循环，保证这个
private slot只初始化一次；同一源码NodeId不会代表下轮的新调用结果，旧snapshot也就不会被覆写。

### Value缓存与后续消费者的现有支持

| 环节 | 当前支持情况／最小处理 |
|---|---|
| `values` cache | `decode_boundary`最后把整个`Parts::Variant`混合树缓存到root；同key多个VariantField共享同一个已捕获pointer，不能每次extractor重新capture |
| `VariantField` | 当前`Parts::Variant`分支直接取指定子Value并clone其parts，已有能力返回OwnedStorage；它不调用memory、不产生effect |
| `MakeVariant` | expand每个operand形成`KnownVariant.fields`，可直接容纳OwnedStorage子值 |
| owned outer Phi | 26的pointer Phi在constructor入边初始化独立whole-Result snapshot，调用encode_fields处理nested OwnedStorage；Phi选择不写回来源 |
| `encode_fields` | 已有OwnedStorage→raw_copy完整typed layout大小的分支；因此只搬运Error表示再按已知外层variant写tag，不再按ErrorKind生成9个guard |
| `Demand` | 完全不改；选中field仍为All。None、部分field需求与encoded/tag-only用途沿旧逻辑 |
| generic `phi_tree_demand` | 不支持OwnedStorage target，也不需要新增；选中字段在decode merge处专门造pointer Phi，后续用途不进入generic phi |
| generic `fill_phi` | 目前没有OwnedStorage arm，不能直接把field_value交进去；必须在decode merge循环中分流 |
| `undefined_demand`／`fill_phi_poison` | 保持现有语义值Undefined逻辑；selected decode merge直接造pointer Undefined，不调用这两者 |
| semantic `Field`／tag query | 对OwnedStorage不支持，selector必须拒绝；不能在使用时临时decode，否则会扩大fixed-point与所有权问题 |

现有Parts::OwnedStorage注释宜明确“指针可能是带inactive Undefined的内部Phi，但所有允许消费者必须
有active证明”；不能只靠Rust枚举类型把该指针视为所有路径有效地址。首版不增加通用lazy decoder。

### 受影响文件与测试验收

最小生产文件范围：

- 新`lower_layout/transport_fields.rs`：结构selector、type eligibility、whole-function cycle gate，
  以及在本模块内实现private capture／owned merge helper（或后两者留在variants.rs）。
- `lower_layout.rs`：模块声明、初始化时建立certificate、LayoutLowering保存只读map；可增加
  `owned_field_captures`计数便于判断恰好五处，不把raw_copy数量当选择计数。
- `lower_layout/memory.rs`：decode_boundary仅在root命中时分派outer enum入口。
- `lower_layout/variants.rs`：原decode逻辑包一层带keys入口、guard内capture、merge处OwnedStorage分流。
- `values.rs`、`demand.rs`、`variant_encoding.rs`按上述方案不必修改行为；若实现需要为它们新增
  generic OwnedStorage消费，说明范围已超出本最小方案，应先检查selector漏了哪种use。

新增验证应同时覆盖行为与结构：真实五条路径命中数；outer Ok保留读取；Err每个内层variant；
同key两个extractor共享一capture；给其中一个extractor增加Field或tag query则该key全退回；
给outer root增加整值sink则退回；MakeVariant进入普通Phi退回；无witness或错witness由S验证拒绝；
MaybeUninit嵌套／union／未知layout退回；CallIndirect/Start/Opaque退回；函数内无关循环也退回。
copy保持现有raw_copy，不为本轮正确性试验提前引入小copy lowering。

动态instance负例必须有“第一次call结果经Phi保存，第二次call覆写同一静态capture slot，再读取旧值”
的形状，确保whole-function cycle gate在表示选择之前触发；不能只测试一个永不读取旧值的循环。
额外负例在原源storage改变后读取snapshot，确认本规则没有把caller参数或load源当作私有所有权。

本方案的机会漏选是显式的：含无关循环函数、经dot Phi的两个路径、CallIndirect、具有少量字段观察
的完整搬运值、ZST、MaybeUninit／union、outer root同时整值消费、普通Phi、直接Return extractor都不选。
这些边界不影响已确认的五个直接Call Err子树；后续放宽必须各自补证明，不能通过类型名白名单规避。

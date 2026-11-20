# 能力缺口与实现位置

基线 `4ccaa863a3cabaa8c87b127a715d34118f35e604`，rustc 1.94.1。以下来自静态源码核对；数量与涉及crate由 `baseline/classification.md` 和 `candidate/classification.md` 的动态调查给出。难度按能独立验收的单位评估，不能把删除第一层拒绝检查当作完整支持。

## 原子操作

- 当前拒绝发生在 `crates/rustc_codegen_kfrc/src/convert/intrinsics.rs`。Body 和 SoN 没有原子内存操作；普通 Load/Store 的转发、消除和移动规则不适用于并发原子访问。
- rustc_codegen_cranelift 参考：锁定工具链源码 `compiler/rustc_codegen_cranelift/src/intrinsics/mod.rs:847` 起，load/store 调用 CLIF `atomic_load/atomic_store`，fetch-sub 调用 `atomic_rmw(Sub)`，CAS 使用 `atomic_cas` 并比较旧值返回 `(old, success)`。128位单独拒绝。fence与single-thread fence均用CLIF fence；后者是保守加强。
- 首个实现单元选择独立、crate私有的Cranelift helper。语义IR保留未知read/write函数调用，现有优化器不能把它当普通可转发内存操作。helper使用真实原子指令；弱序保守提升为seqcst。支持8/16/32/64位有符号/无符号整数load/store/xadd/xsub/xchg/CAS、薄指针load/store/xchg/CAS和fence，保留旧值与CAS成功标志。weak CAS使用无伪失败的strong CAS。helper不进入语义图内联，代价是函数调用与保守屏障。
- 生产性能后续可通过显式Body/SoN原子节点降低调用成本，但必须同步补全verify、schedule、function_attrs、memory analysis、interpreter和优化器的并发语义。该重构独立于首次能力解锁。

## 浮点：ABI、值与运算分别处理

- `rustc_codegen_kfrc/src/abi.rs:scalar_abi` 明确拒绝Float。`kfrc-body/src/oracle.rs:ScalarAbi`仅表达整数宽度/扩展，没有浮点寄存器类。结构体Cast ABI也只记录整数eightbyte，混合SSE/INTEGER ABI需分别建模。
- `kfrc-son/src/build/semantic.rs:check_semantic_types`拒绝浮点，`semantic_scalar`也不认浮点。常量、算术、比较、NaN与signed-zero、cast（含饱和float-to-int）需完整语义。
- `kfrc-cranelift/src/export.rs:scalar_of_type`及`export/abi.rs`只有整数标量通道，调用边界要正确使用F32/F64与XMM寄存器，不能以I32/I64位模式冒充。
- cg_clif参考：`abi/pass_mode.rs`、`cast.rs`、`intrinsics/mod.rs`及`codegen_f16_f128.rs`。应先做f32/f64入参/返回、常量、identity及内存roundtrip，再做基本算术/比较/转换，最后libm与聚合ABI。浮点ABI单元本身不能解锁ryu/serde_json全crate。

## i128/u128

- TypeShape已有W128，但SoN semantic type gate拒绝，scalar ABI限制1/8/16/32/64。常量池能表达u128不等于运算/ABI已支持。
- 需统一两lane ABI与算术carry、shift、比较、乘除及runtime symbols。cg_clif参考 `codegen_i128.rs`，不能简单放开gate。

## union 与 SIMD

- 当前类型转换已记录union字段布局及SIMD lane事实，`convert/intrinsics/simd/`也有整数SIMD操作展开；然而语义路径 `kfrc-son/src/build/semantic.rs`拒绝一般union与SIMD值，call ABI gate拒绝Vector。
- MaybeUninit是生产者鉴别的专用union路径；不能把全部union当逻辑product字段。重叠字段、未初始化字节、padding与provenance需要storage语义及reinterpret操作。
- cg_clif通过 `value_and_place.rs` 的storage/value操作处理union并保留内存布局；SIMD参考 `intrinsics/simd.rs`、`intrinsics/llvm_x86.rs`。先调查memchr依赖的具体LLVM intrinsic与vector ABI，再划最小子集。
- 调查错误文本已区分union/SIMD并打印类型，避免把原来的联合错误信息猜成一个具体类别。

## aggregate constants 与 RefCell

- `kfrc-son/src/verify.rs`要求R/S aggregate必须使用value constructors。当前某些producer把非标量aggregate以Const表达，违反语义契约；应修正producer或构造函数，保留verifier。
- `RefCell<Vec<_>>::new`只是代表案例，需覆盖零大小newtype、单字段wrapper、空数组和嵌套aggregate的scalar/indirect CTFE表示。先最小化成独立库，观察Body/R/S中第一次出现错误Const的位置。

## enum/Phi panic

- 全函数调查暴露 `lower_layout/values.rs:fill_phi_poison` → `graph.rs:set_input` 的 `inline nodes have at most two inputs`。同类 `fill_phi`、variant lane写入均需检查目的节点是否仍是growable phi，及scalar/known-variant归一化后的形态。
- cargo还暴露CLIF `ir/instructions.rs:228` 的`payload < (1 << 30)`断言，栈进入`egraph::EgraphPass::run`。该类与SoN enum/Phi panic分开保留原始堆栈；后续应最小化export出的CLIF函数并检查ValueDef/alias表示及优化输入契约，不能直接把断言关掉。
- 调查模式仅在owned graph prepare/lower范围catch unwind，废弃失败函数图并记录panic；不catch rustc查询/转换fatal，不改变默认编译行为。先记录代表函数与完整堆栈，再独立最小化修复，不能把graph.set_input自动扩容当作语义修复。

## TLS、dyn、inline assembly、unwind

- TLS已有 `Rvalue::ThreadLocalRef` / `Opcode::ThreadLocalAddress`，Cranelift输出ELF general-dynamic relocation及`__tls_get_addr`；但 `kfrc-son/src/build.rs` 和 `build/semantic/values.rs` 明确拒绝TLS进入当前semantic路径。已有机器层支持仍不能使RandomState的TLS初始化函数通过；需补齐固定effect节点与R/S转换，而不能仅按机器层代码声称TLS可用。
- dyn已有 `convert/trait_object.rs`、`unsize.rs`、`unsize_adt.rs`，包含virtual call和dynamic drop。分类按具体失败形态，不把调用链中出现dyn的全部函数归成dyn缺口。
- inline assembly在MIR terminator转换被拒绝，global_asm单独拒绝。可先针对稳定的pause等指令映射intrinsic，但完整inline asm需约束/寄存器clobber/内存效果契约；参考cg_clif `inline_asm.rs`、`global_asm.rs`。
- backend init只接受panic=abort。调查也使用abort，不能据此判断panic=unwind支持情况；完整unwind涉及landing pads、personality、unwind tables及运行时，参考cg_clif `debuginfo/unwind.rs`。

## executable入口

- `shell_entry.rs:UnitEntryPlan`只支持unit main，`main -> ExitCode/Result`在开始调查前就fatal。constructor抽出try_new，默认new保留fatal，调查记录entry缺口后继续mono functions；非unit main未获得运行时支持。
- 下一实现需按rustc Start/Termination真实ABI适配返回值与report，覆盖unit/ExitCode/Result，并验证进程退出码与错误输出。

## 调查新暴露的相邻能力

- C variadic：`abi.rs`当前统一拒绝c_variadic。memmap2/parking_lot等通过libc syscall调用整型变参，首先区分调用与定义，再核对SysV的AL浮点寄存器计数和栈上传参；不能只删拒绝。
- DST：`convert/intrinsics.rs`的size_of_val与`kfrc-son/src/build/semantic/places.rs`动态尾部要求生产者证明。ArcInner<[T]>/ArcInner<dyn Trait>需要动态size/align以及尾部offset对齐；trait object虚调用支持本身解决不了这个问题。
- volatile：intrinsics转换缺volatile_load/store。普通Load/Store会被优化，不能借用；可以先采用未知效果helper或明确volatile节点，保留每次访问与顺序，结合MMIO/系统库要求设定内存标记。
- unreachable control：`kfrc-cranelift/src/export.rs`与`kfrc-son/src/schedule.rs`报告“control node unreachable”。需最小化layout残留节点/region后继关系，而不是无条件把不可达节点发布为可达。
- uninhabited/Never：layout在重建Never/不可居住variant时报错。必须区分未执行分支的poison、实际不可达控制以及非法构造，避免用零值代替Never。
- pause：锁定cg_clif `intrinsics/llvm_x86.rs:19`将`llvm.x86.sse2.pause`当无业务语义的spin-loop hint。U1按该契约做精确symbol/无参unit ABI检查并消除该hint；实际同步保留在原子操作，未声称发出PAUSE指令。

## 调查结果的解释

每个target unit逐个尝试全部mono items，不因依赖错误跳过下游；每个函数只报告首错，MIR intrinsic调用则独立完整枚举。依赖metadata来自LLVM，std/build-script/proc-macro来自LLVM。调查跳过crate级图变换/跨函数内联，不发object、不链接；“零首错”只代表候选，不等于全kfrc构建或行为验收成功。

## cargo新增的独立边界

- `enum-phi-shape`：`kfrc-son/src/lower_layout/variant_lanes.rs:524`、`values.rs:593`要求来源/目标Parts形态一致；已有representative触发`Phi field shape mismatch`。应与上面的enum/Phi poison同批审计，覆盖known/unknown variant与shared lane，不把Malformed降格成成功。
- `semantic-place-type`：`kfrc-son/src/build/semantic/places.rs:133`在写place前核对`rust_type(value)`与`place_type`。应从producer的类型事实与newtype/投影转换定位第一次不一致；保持严格类型校验，避免按位宽相同强行接受。
- `static-linkage`：`rustc_codegen_kfrc/src/constants.rs:146`拒绝任意link_section/import_linkage。`INIT_CTOR`是已观察案例；初始化器section、顺序、relocation与ELF writer是独立实现边界，需原生执行构造器的副作用与链接器保留测试。
- `dyn`的具体形态：`convert/trait_object.rs:172`的dynamic drop只接受已deref的trait-object pointer；`drop_in_place<UnsafeCell<dyn FnMut(...)>>`暴露transparent wrapper尾部路径。先确保布局/metadata来源，再扩展wrapper投影，不能替换成忽略drop。
- `verification-other`：verifier的“outside the specialized value graph subset”包括SIMD、i128和部分嵌套类型，错误本身未给出完整type fact。报告保留验证类别和原函数，不按函数名猜测合并到SIMD/i128；后续可让错误携带oracle类型以准确重分类。

## 默认speed模式的规模与优化器限制

- 低频gdb采样显示gix的一个map_err函数持续在`machinst/copy_cleanup.rs:Equalities::copy -> Copies::equalities -> run`，CPU99.9%、RSS约1.2GiB。该文件是kfrc自定义补丁；不能把它概括为上游Cranelift无条件开销。
- `Equalities`保存每location的leader/count；transfer中的copy会遍历leaders，worklist在CFG上反复传播，join-placement还可能多次重做analysis。已核实最坏工作量随location、CFG与copy数增长；是否不收敛需进一步证据。
- `machinst/compile.rs:78`仅在opt_level非None时执行该优化，现有`KFRC_CRANELIFT_OPT=none`可供完整能力扫描的独立配置。它也停用egraph；这种扫描结果须与default speed首错及超时分开保存。默认程序实际能编译和执行仍应另行验收。

- none配置进一步暴露SoN布局阶段的规模问题：`analysis/cfg.rs:ControlFlow::build -> thread_variants::Analysis::build_dominance -> run_with_edge_facts -> lower_layout::lower_validated`。ControlFlow本身线性收集边/DFS/建前驱表；thread_variants在每次图编辑后废弃analysis并重建Uses/CFG/Dominators，重复重建可放大开销。该点与CLIF copy cleanup分开记录，尚未实施修复。`KFRC_LAYOUT_EDGE_THREAD=0`只关闭边事实，不会停用整个thread_variants，因此不以该开关保证绕过慢点。

## 难度与工作量判断

“小”指有明确窄边界、可在45–90分钟内用原生差分验收；“中”指先最小化、核对跨层契约后可形成一个该量级子集；“高”指应拆成多个这样的单元，不能据类别名承诺一次完成。

| 能力 | 难度 | 判断依据 |
|---|---|---|
| 基础原子 / 剩余bitwise原子 | 小 | crate私有helper路线避开整个优化器并发语义重构；基础部分已原生验收，新增op复用同一路径 |
| C变参调用的整型SysV子集 | 小–中 | 调用/定义分离、额外参数ABI与AL规则需核对；暂不包含浮点变参和va_list定义 |
| aggregate常量构造 | 中 | 已知producer/verifier契约冲突；须处理CTFE scalar/indirect以及零大小/嵌套wrapper，不放松verify |
| unreachable / Never / enum-Phi形态 | 中–高 | 需要最小化实际函数、审计known/unknown variant与CFG出口；不能用假值或无条件可达绕过 |
| TLS semantic / dyn wrapper drop / DST动态tail | 中–高 | TLS机器层已有，但S层effect和类型事实缺失；DST/drop跨布局、metadata和析构，需要分开验收 |
| float / i128 / union / SIMD整体 | 高 | 类型、常量、值流、调用ABI、运算、机器导出多层契约；每类都已拆成多个子集 |
| volatile / abort下catch_unwind | 中 | 首先建立保守内存效果/abort运行时契约；完整unwind还需要额外平台支持 |
| 特定LLVM intrinsic / asm | 小–高 | 精确pause hint已完成；cpuid/xgetbv等可逐小组映射，任意asm约束/clobber则需完整独立设计 |
| static link section / INIT_CTOR | 中 | 需ELF section、relocation、初始化顺序和链接器保留的真实执行验证 |
| 模糊verification / semantic place type | 中 | 先增强类型事实与producer来源证据；不能根据函数名猜测归类或删校验 |
| copy cleanup / thread_variants规模问题 | 中 | 有实际慢函数和gdb栈；先量化图/位置数与重建次数，再做预算或数据结构改进，保守路径须行为对照 |

# Walkthrough 发现汇总

每条：编号（来源 walkthrough）、问题、改法、目标文件、状态。状态 `已改` 表示已改回 [design/](../../design/README.md)。

## 健全性（优先处理）

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F11 | W10-1 | release 原子/fence 只排序 `Unknown`：`*x = 42; fence(Release)` 后经 Relaxed 发布 `x`，另一线程 fence 同步后可能读到旧值；`Local` 槽在 fence 后才逃逸同理 | Release/AcqRel/SeqCst 以所有 `Unique` 和非 `Frozen` 的 `Local` token 为排序输入（只约束 placement，DSE 不视为读）；Acquire 只排序 `Unknown` | memory-partitions 规则 7、nodes | 已改 |
| F12 | W03-1 | `Load`、`VariantField` 无控制依赖：`Option<&T>` 降到 L 级后 load 可被提到 null 判断之前；裸指针 `if !p.is_null() { *p }` 同理 | `VariantField` 只接受固定在控制上的 `Refine{variant}`，L 级降成 tag 事实并保留控制；`Load`/`Store` 默认带 `ctl`，只有 `Frozen`/`Unique` 分区可解除固定 | nodes、values-and-types、memory-partitions | 已改 |
| F13 | W10-2 | “所有引用 `dereferenceable(size_of T)`”过宽：rustc 对 `&!Freeze`、`&mut !Unpin`、`Box` 给 0（`rustc_ty_utils/src/abi.rs`，#98017） | 只有 `Frozen`/`Unique` 分区在调用期间可解引用 | values-and-types、memory-partitions 规则 2 | 已改 |
| F14 | W03-2、W01 D3、W04-6、W09 E8、W08-1/3、W05-8 | 机器码键漏依赖：非泛型函数的具体类型布局查询（`CrateType` 编码取决于 std `String` 私有字段；niche 常量 1.94.1 与 1.99.1 不同）；直接被调者的摘要（决定 `call`/`invoke`、能否省拷贝）。上游只改私有布局或被调者摘要会错误命中 | L 级对**所有**类型的布局查询登记为闭合孔位（与 F02 统一）；机器码键加入读过的全部被调者摘要哈希 | generics-specialization、cache | 已改 |
| F15 | W10-3 | 原子“只在 `Unknown`”与派生规则 1 矛盾（栈上原子属 `Local`；rustc 对 `&mut Atomic` 发 noalias，应属 `Unique`） | 原子访问用地址所属分区，排序语义由 F11 表达 | memory-partitions 规则 6/7、nodes | 已改 |
| F24 | W04-3 | `panic=abort` 裁剪只覆盖本地函数体：上游发布的 R 图带着上游会话的策略（rustc `fn_can_unwind` 读当前会话策略，abort 下仍留 `resume=8` 的死 cleanup） | 发布的 R 图一律保留 unwind 边；裁剪移到 S 级，panic 策略进答案键 | ownership-drop-unwind、generics-specialization、cache | 已改 |
| F46 | W05-1 | 规则 4 的“此后并入 Unknown”是流敏感的，但分区按根静态指定：只在冷路径逃逸时，汇合后 Unknown 的写与 Unique 的读失去顺序 | 逃逸是数据流状态，汇合处合并 token；首版可用“任一处逃逸则整函数视为 Unknown” | memory-partitions 规则 4、son-verifier | 已改 |
| F47 | W05-2 | `Reborrow` 子分区逃逸后父分区不跟着逃逸 | `EndBorrow` 把逃逸状态传给父分区 | memory-partitions 规则 4/5 | 已改 |
| F48 | W05-3 | 规则 2“有 dereferenceable 就能把 Frozen 读上提出条件”：`&[T]` 没有该属性，src 读提前会越界 | 投机读只落在 sized pointee 的静态大小之内 | memory-partitions 规则 2、values-and-types | 已改 |
| F49 | W05-4 | 规则 1 未覆盖 Phi/Select、指针 cast、聚合字段；根混合时默认 Unknown 会让 Unique 内存的访问失序 | 根按集合计算，混合时取所有相关分区的 token（并入 F23 的 Cast） | memory-partitions、son-verifier | 已改 |
| F50 | W05-5 | 规则 3 只看指针实参，没看聚合实参字段里的指针（`IterMut`、闭包环境） | 改为“沿值字段可达的全部指针” | memory-partitions 规则 3 | 已改 |
| F51 | W08-2 | CheckedBody 键不含 trait 选择读过的 impl 集合（含“只有一个候选”这类否定事实），新增 impl 会命中旧选择 | 依赖清单记录每个做过选择的 trait 的 impl 集合指纹、固有 impl 集合、不透明类型隐藏类型 | rust-son/cache、architecture/cache | 已改 |
| F52 | W08-4 | 优化器若折叠 `Location` 内容，行号就进了代码（rustc O3 把 `line_here` 折成 `mov w0, #27`） | `Location` 条目对 R/S/L 级不透明 | values-and-types、cache | 已改 |
| F53 | W08-5 | “优化配置”“L 级配置”未列字段，错误命中风险 | 逐项列出：panic 策略（定义方/消费方）、overflow-checks、debug-assertions、target features、unwind 表、`--remap-path-prefix` 等 | cache | 已改 |

## 答案键与特化

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F01 | W06 D1 | 照搬 rustc `BackendRepr` 会把符号和有效范围带进答案，`Vec<u32>::push` 与 `Vec<i32>::push` 分键；去掉范围又可能让 S 级从类型里读到 `char` 范围而错误合并 | 拆成 `ScalarShape(T)`（无符号位）和 `ValidRange(T)`；`ValidRange` 只在 T 的值流入比较/switch/cast/算术/调用实参时登记；S/L 级只能经答案表读类型事实，debug 构建越权即 ICE | generics-specialization、values-and-types、levels | 已改 |
| F02 | W06 D2 | 比较的模式未定义；具体类型是否进 R 图哈希未定义（newtype 无法与底层类型共享，叶子类型名破坏递归去重） | `Compare{int_signed, int_unsigned, float_ordered, float_unordered}`；具体类型按与孔位答案相同的规范描述哈希（“已闭合孔位”） | values-and-types、nodes、generics-specialization | 已改 |
| F03 | W02-3 | 被折叠分支里的孔位仍在答案向量里，`u32` 与 ZST 实例因问了用不到的问题而不能共享 | 答案键只含特化后仍可达的孔位：先折叠控制孔位，再收集剩余孔位 | generics-specialization | 已改 |
| F04 | W06 D5 | 特化先整图复制，被 `IS_ZST` 删掉的分支也复制 | 边复制边折叠，不可达分支不复制（对应 rustc `mono_reachable`） | generics-specialization | 已改 |
| F05 | W02-1 | 调用者私有闭包类型的单用实例各自登记答案键、特化、缓存，最后全部内联 | “嵌入实例”：实参含某 typeck root 私有的闭包/fn item 类型且只被它使用 → 在调用者 S 级里直接展开，不登记、不缓存；被取地址或多处调用时退回普通实例 | generics-specialization、cache | 已改 |
| F06 | W06 D6 | R 级内联后 mentioned 集合未合并；多个实例共享键时 post-mono 错误怎么报未定义 | 内联时并入被调者的 mentioned 集合；常量求值结果按答案键缓存，诊断按 `InstanceKey` 用真实类型报告 | generics-specialization | 已改 |

## 优化流水线

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F07 | W02-2 | R 级内联条件写得含糊 | 非泛型函数 R=S，内联被调者 S 图；泛型函数只内联实参全由本函数 `Param` 原样转发和具体类型组成的被调者，拼接时 `Param` 换名、孔位表合并 | optimization-pipeline | 已改 |
| F08 | W06 D3 | 传给无摘要 `TraitCall` 的 `&x` 让槽位落入 `Unknown`；S 级内联后槽位提升不在种子里 | `&T`（`T: Freeze`）实参按 rustc 已发的事实（readonly、只捕获 read_provenance）保持 `Local`；“被删除/内联的调用的实参槽位”加入 S 级种子 | memory-partitions 规则 3、optimization-pipeline | 已改 |
| F09 | W06 D4 | “S 图不存”“内联拼接被调者 S 图”“C2 每键跨 crate 只特化一次”三者冲突 | 只缓存作为内联源的 S 图（本来就小）；C2 改为“每键机器码与摘要只生成一次” | cache、compile-time、optimization-pipeline | 已改 |
| F10 | W02-4 | L 级缺常量除法/取模强度削减和 if-conversion | 列入必须自己做的部分 | runtime | 已改 |

## enum 与变体（W03）

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F16 | W03-3 | `Discriminant` 与 `VariantOf` 不打通，derive 的判别值比较折叠不了 | `Discriminant` = 判别值表[`VariantOf`]；判别值互异，所以判别值相等 ⇔ 变体相等 | nodes、optimization-pipeline | 已改 |
| F17 | W03-4 | 内存中 enum 只能整值 load 再取变体，L 级落 value slot | 加 `VariantOfAt[P]`，或整值 load 的使用全是投影时按投影窄化 | nodes、values-and-types | 已改 |
| F18 | W03-5/6 | R 级缺规则：常量分配上的 `Load` 折叠；`VariantSwitch(Phi(MakeVariant…))` 按前驱线程化；同一 `VariantOf` 的常量比较链合并为 `VariantSwitch` | 加入 R 级规则表 | optimization-pipeline | 已改 |
| F19 | W03-7 | niche `VariantSwitch` 先解码再比较，正是 rustc 留给 InstCombine 的活 | L 级按 tag 值直接分支；多 case 走区间/位测试的 switch 降低 | nodes、runtime | 已改 |

## 内存分区（W10）

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F20 | W10-4 | `Freeze` 粒度未写明 | 整类型、浅层；不做按字段 `Frozen`（强于 rustc，违反红线 1）；`&&Cell` 只冻结外层 | memory-partitions 规则 6 | 已改 |
| F21 | W10-5 | 泛型 `&T` 的分区依赖孔位；“无 `UnsafePinned`”条件 1.94.1 并不检查 | R 级当 `Unknown`；S 级以该根上所有内存节点为种子拆链；`IsFreeze`/`IsUnpin` 进答案键；条件改为照 1.94.1 | memory-partitions、generics-specialization | 已改 |
| F22 | W10-6 | rustc 对 Scalar/ScalarPair 聚合中的引用也发 noalias，现设计只看直接参数 | 首版记为已知差距；扩展时按语义定义，`tests/ui/codegen/issue-63787.rs` 进差分集 | memory-partitions | 已记为已知差距 |
| F23 | W10-7 | 派生链不穿过指针 `Cast`（`UnsafeCell::get`），`RefCell` 计数与值无法判不相交；缺“写回已知当前值的 store 删除” | 派生链加 `Cast`；加该规则（借助 `Refine`） | memory-partitions 规则 1、optimization-pipeline | 已改 |

## drop、unwind 与调用摘要（W04）

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F25 | W04-1 | `DropValue`/`DropInPlace` 没有 unwind/Terminate 出口 | 结果 `(Control, Control?, Effect…)`，按 drop glue 摘要裁剪 | nodes、ownership-drop-unwind | 已改 |
| F26 | W04-2 | `Landing` 不能合并多条 unwind 边，无异常值（O3 有 `phi { ptr, i32 }`） | `Landing` 是 Region 型合并节点，产出 `Exception` 值，`Resume` 消费，合并用 Phi | nodes | 已改 |
| F27 | W04-4 | SCC 从保守值迭代证不出 nounwind（LLVM 证出了互递归 `even`/`odd`） | SCC 内从 ⊥ 迭代到最小不动点；SCC 内限制内联 | optimization-pipeline | 已改 |
| F28 | W04-5 | 摘要在 S 级结束才产出、`size_estimate` 取自 L 级，调用图深度变成串行等待（`work` 高度 8，高层每层 1 个函数） | 摘要改为未优化 S 图上的独立廉价遍历；`size_estimate` 用 S 图节点数估计；精度损失用 A/D/B 测 | optimization-pipeline、compile-time | 已改 |
| F29 | W04-7 | “drop flag 折叠”只有名字 | 就是常量 Phi 变条件（`Phi(If c; false, true) = Not(c)`），不需要专用节点、不做尾复制 | optimization-pipeline | 已改 |

## 动态分发（W07）

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F30 | W07-1 | 没有整张 vtable 的孔位 | `Hole::VTable{ty, trait_ref}`，答案 `VTableKey`，S 级为 `Const(VTable K)`，登记进 mentioned | generics-specialization、nodes | 已改 |
| F31 | W07-2 | unsize 强转没有节点，`Box<dyn>` 这种嵌套写不出 | `Cast.Unsize{meta}` + 投影穿透；`&T`/切片直接产出 `WidePtr` | nodes、values-and-types | 已改 |
| F32 | W07-3 | 没有去虚化规则 | S 级：`CallVirtual{slot}(WidePtr(_, Const K))` → `Call(K.entries[slot])`，查答案表不读内存 | optimization-pipeline | 已改 |
| F33 | W07-4 | `dyn` 的 drop 未规定；O3 拿到常量 vtable 仍运行时读槽 | 读槽 0、判空、间接调用，Box 按 vtable size/align 释放；vtable 已知时折叠为具体 drop | ownership-drop-unwind | 已改 |
| F34 | W07-5 | 没有常量内存（rustc 对 vtable 指针 `readonly`，槽 load `!invariant.load`） | 只读 `Const` 分区：load 是纯值，常量偏移 load 直接折叠（同时覆盖 F18 的常量分配折叠） | memory-partitions | 已改 |
| F35 | W07-6 | 删不掉未逃逸的堆分配（O3 把 `local_devirt` 压到 3 条指令，连 Box 分配都删了） | 分配器 lang item 产生 `Alloc`/`Dealloc` 节点，结果是新分区；配对且未逃逸时整体删除 | nodes、memory-partitions、runtime | 已改 |
| F36 | W07-7 | shim 的键没写（三个 dyn drop shim 屏蔽符号后完全相同） | shim 生成后按图内容进答案键表，与普通实例同样去重 | generics-specialization、size | 已改 |
| F37 | W07-8 | `CallVirtual` 摘要没写 | 被调者未知取顶；“二进制 crate 已知全部 vtable 时取同槽摘要的并”列为后续 | optimization-pipeline | 已改 |

## 聚合、大值与放置（W01、W09）

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F38 | W01 D1 | `?` 内联后 `ControlFlow` 是合并点上的 Phi，不是已知变体；rustc 也因 GVN 在 JumpThreading 之前而错过 | “switch 穿过 Phi(MakeVariant) 按前驱拆分”，与聚合折叠同一个 worklist（与 F18 的线程化合并为一条规则） | optimization-pipeline、nodes | 已改 |
| F39 | W01 D2 | 原地构造规则 3 要求“唯一使用是 sret”，`?` 的调用结果在 Ok 路径还有使用，按现规则仍剩 4 次 40B 拷贝 | 拷贝亲和合并：两槽间只有整槽同偏移拷贝、且编码重写是恒等（`Err(Field(e))` 与 `Err(e)` 编码逐字节相同）时合并调用结果槽和 sret 槽 | ownership-drop-unwind | 已改 |
| F40 | W09 E1 | “`Memory` 聚合变 value slot”与 Rust ABI 不符：≤8B 的 `Memory` 按整数传 | 按 (backend_repr, PassMode) 映射：≤8B 为 `Int`，>8B 为槽，>16B ScalarPair 只在返回边界走 sret | levels、values-and-types | 已改 |
| F41 | W09 E2 | Indirect 参数归被调者所有、可能被写；摘要判断不了能否省拷贝 | 摘要加 `writes_indirect_args` | optimization-pipeline、ownership-drop-unwind | 已改 |
| F42 | W09 E3/E7 | 规则 3 未覆盖嵌套（值是父聚合/变体的字段）；`InsertField` 若分配新槽，`b.n += 1` 是 136B 拷贝 | 规则 3 递归：目标是父值槽 + 字段偏移；`InsertField` 在操作数最后一次使用处复用其槽 | ownership-drop-unwind | 已改 |
| F43 | W09 E4 | 目标地址必须在值产生前存在，`Box::new(call())` 下规则 3 静默失效（rustc 留 136B memcpy） | 写明前提；首版保留一次拷贝；分配前移需语言层确认且被调者纯/不 unwind/不发散 | ownership-drop-unwind | 已改 |
| F44 | W09 E5/E6 | 没有 `Repeat`；没有拷贝/填充降低策略；没有“分配后整块置零 → `alloc_zeroed`” | `Repeat{count}`；降低阈值进 `TargetSpec`；加 S 级 alloc_zeroed 规则（依赖 F35 的 `Alloc`） | nodes、optimization-pipeline | 已改 |
| F45 | W09 E9 | 数组值的动态下标没有节点 | 带非常量 `Index` 的局部按“地址被取走”处理 | values-and-types | 已改 |

## 分区与增量的其他发现（W05、W08）

| # | 来源 | 问题 | 改法 | 目标 | 状态 |
|---|---|---|---|---|---|
| F54 | W05-6 | 在 Unique/Local 父分区下再建 `Reborrow{Unique}` 不带来新信息 | 只在实参根是 Unknown 时建子分区 | memory-partitions 规则 5 | 已改 |
| F55 | W05-7 | 只要循环里有一条分支调用，分区 token 就随循环变化，placement 外提不了 cap/ptr；rustc O3 也每次重读 cap/ptr、写回 len | 穿过分区 Phi 的 load 转发与 PRE；store 下沉可选 | optimization-pipeline、runtime | 已改 |
| F56 | W05-9 | 非泛型函数调用具体类型的泛型实例（`Vec<u8>::push`）：R 级不能内联，S 级又没有种子 | S 级种子包含所有已解析的直接调用（与 F07 合并：非泛型函数 R=S） | optimization-pipeline、generics-specialization | 已改 |
| F57 | W08-6 | CheckedBody 键含全部 cfg，panic=abort 让所有 Body 失效，但 Body 与 panic 策略无关 | 只记录真正求值过的 cfg 谓词 | 两份 cache | 已改 |
| F58 | W08-7 | R 图全在 metadata 里，跨 crate 内联非 `#[inline]` 函数会扩大失效范围（rustc 增量模式特意关闭跨 crate 内联推断，E3 时 bin 不重做） | 内联策略区分构建模式：增量/dev 只内联 `#[inline]` 和小于阈值的函数 | optimization-pipeline、compile-time | 已改 |
| F59 | W08-8 | cache.md 引的 28.7% 来自非增量 release；rustc 增量下注释修改不重做任何 CGU | 改写 early cutoff 段：kfrc 的真实优势在行号移动（E2）和 panic 策略/配置变化（E9） | cache | 已改 |
| F60 | W05-10、W05-11 | `CopyMem` 需要读写两个分区；扩容路径（`inline(never)`，非 `#[cold]`）按现定义不是冷区域 | `CopyMem[Psrc→Pdst]`；冷区域定义写明只认 `#[cold]`、`-> !`、panic/unwind | nodes、ownership-drop-unwind | 已改 |

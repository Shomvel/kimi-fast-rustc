# LLVM 21.1.8：按需标量化、叶子 Phi 与聚合快照

日期：2026-11-09。范围：LP3 的实现准备，固定参考 `llvmorg-21.1.8`。
本研究实际下载并读取 LLVM 源码，结合当前 `lower_layout` 的表示与 effect 契约提出适配方案；
没有修改业务源码，没有编译或运行新性能实验，也不构成 code review。
`from_str` 的 L Phi 695、Undefined 473 来自 LP3 已有证据，本文没有重新测量。

## 1. LLVM 实际做了什么

### 1.1 SROA 根据使用切片，不按类型穷举所有字段

[SROA.cpp 开头](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L9)
明确说明：识别 aggregate alloca 中能提升的部分，采用最少切片；仅在外部内存间搬运的区域
应保留原状，不拆成标量代码。其 `Slice` 表示某个实际 use 覆盖的字节范围，
[AllocaSlices](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L587)
把 use 按偏移及可拆分性排序，`Partition` 组合相交且不能分开的范围。
这不同于沿 Rust struct／enum 的所有逻辑叶子展开。

[rewritePartition](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L4926)
根据共同 use 类型、原类型的子区间、合法整数或向量类型选择表示；无法选择可提升类型时，
可以保留适当大小的 i8 array alloca。只有成功形成可提升访问的分区才进入 promotion。
[promoteAllocas](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L5797)
调用 `PromoteMemToReg`，所以“切分内存区域”和“为该区域建立 SSA”是两个相连步骤。

**没有从这些源码得出一个通用的“大于 N 字节就不标量化”阈值。**
[splitAlloca 中的 1024](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L5350)
是限制构造逐字节 bitvector 的分析开销：较大 alloca 对部分 load/store 采用更保守的可拆分规则，
不是停止 SROA 的统一大小阈值。
[isIntegerWideningViable](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L2525)
也检查位宽上限、bit padding、类型转换及 covering use；这些是表示合法性与可提升性约束，
不能直接作为 kfrc 的叶子数量阈值。

### 1.2 Phi 受真实使用的活跃性约束

[PromoteMem2Reg::ComputeLiveInBlocks](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Utils/PromoteMemoryToRegister.cpp#L1040)
从有 load 的使用块开始向前驱传播；块内首次访问若为 store，则旧值并非 live-in。
传播在定义块停止。调用者把 live-in 集合与定义块集合交给
[迭代支配边界计算](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Utils/PromoteMemoryToRegister.cpp#L867)，
从而避免在不通向真实使用的合流块生成死 Phi。
`QueuePhiNode` 建占位，再由 rename 遍历补输入。

这支持“先收集真实叶子需求，再建对应 Phi”的方向，但 LLVM 在这里处理的是经过 SROA 的
alloca，而 kfrc 已有 S 级聚合 SSA Phi。kfrc 第一版不需要重新跑一次 IDF；应把需求沿现有
聚合数据流反传，在已有 Phi 的每个需求叶上建立标量 Phi。

### 1.3 将 load 穿过 Phi 需要安全证明

[SROA::isSafePHIToSpeculate](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp#L1556)
要求用户是可处理的简单 load，同块、相同类型、Phi 和 load 之间无可能写内存的指令。
把 load 放入前驱时，还要检查插入位置、边是否关键，以及指针是否可无条件安全读取。
LLVM 不把“最后只用了一个字段”当成允许随意移动读取的依据。

对 kfrc，LP3 第一版只改变读取集合，不改变已有 load 的控制／effect 时点，
也不把 inactive enum payload 的读取从分支中投机提出。

### 1.4 MemCpyOptimizer 消除中间副本前检查别名与时间

[processMemCpyMemCpyDependence](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/MemCpyOptimizer.cpp#L1107)
可把 `copy(a ← b); copy(c ← a)` 改成直接从 b 复制，但检查长度、偏移、volatile，
并用 `writtenBetween` 排除中间对 b 的修改；源码明确给出 `*b = 42` 的反例。
如果新源和目标可能重叠，则使用 memmove，不能直接维持 memcpy 的不重叠假设。

[processStoreOfLoad](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/MemCpyOptimizer.cpp#L628)
把聚合 load/store 转成内存复制时同样检查中间写入；不能只在 store 位置读取源地址，
否则丢失原 load 的快照。

[performCallSlotOptzn](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/MemCpyOptimizer.cpp#L842)
可让调用直接写最终目标槽，但要求源是受控 alloca、覆盖尺寸、目标可写且对齐、
中间无人观察目标、源没有其他用途，并处理捕获与 unwind 可见性。
这说明“复用调用返回槽／消除私有副本”应是有独立证明的后续优化，不能成为留槽表示的默认假设。

## 2. 当前接缝与最小实现路线

当前 `values.rs::expand(Phi)` 先通过 `phi_tree` 递归建立整棵叶子树，再发布到 `values`，
最后遍历前驱填入每个叶子。发布占位保证循环回边终止，应保留。
`run()` 还会遍历所有语义值调用 `expand`；只改 `phi_tree`，却仍让所有 aggregate 输入
和 `LoadValue` 全量展开，会保留大量无用地址、load、常量与 Undefined。

建议先做 **需求掩码 + 保持现有 Value 树形状**，不同时引入大值槽和完整 lazy memory：

1. 在 `erase_requirements`、`erase_ignored_transport` 之后、修改 S 图之前收集 demand。
   以解析后的 `NodeId` 为键，记录 `None / All / Sparse`，Sparse 是字段路径 trie。
   路径区分普通字段、固定数组下标、enum tag、`variant(v).field(i)`。
   `MaybeUninit` 是不可拆的 raw-storage 叶；不能把其 inner type 当成普通 demand 树。
2. 从实际执行消费者播种，再用 worklist 单调反传，直到不再增长。只对新增路径传播，
   不按“所有节点 × 所有叶子”反复扫描。`All` 可以压缩表示，不必先枚举大数组的每个元素。
3. 对当前树增加内部 `Omitted`／`Unmaterialized` 标记，或等效的稀疏子树表示。
   **它不是 IR Undefined。** 未被需求的叶子不分配 NodeId；真实消费 Omitted 是内部错误，
   不能静默补零／poison。保持字段索引稳定，减少第一版侵入所有使用 `Parts::Fields` 的代码。
4. `expand` 与 `phi_tree` 接收冻结后的 demand。只给有需求的叶子建 Phi，
   缓存整棵稀疏占位后再遍历 incoming。回边再次命中同一个 `(node, leaf path)` 占位，
   不继续递归。所有输入填完前，不做依赖“Phi 已完整”的化简。
5. `run()` 的纯语义值遍历跳过 `None`，最终仍退役相应 S 语义节点；有 effect 的节点不能
   因值未使用而整条跳过，沿用现有 effect／control lowering 契约。
6. `lower_load_value` 和 `decode_boundary` 根据需求只捕获相应叶子，仍在原 effect／ABI
   边界处插入读取。参数在函数入口捕获；调用结果在调用后捕获；真实 typed load 在原读取点捕获。
   `All`、未知 use、raw transmute 等第一版回退现有完整路径。

### 2.1 需求传递表

| S 节点／消费者 | 从输出需求反推输入需求 |
|---|---|
| `Field(base, i)` | 给 base 加路径 `field(i) + demand(result)`；不要求其他字段 |
| `VariantField(base, v, i)` | 给 base 加 `variant(v).field(i) + demand(result)`；物理 decode 若需要选择分支，另需 tag |
| `Discriminant`、`VariantOf`、`VariantSwitch` | 需求 enum tag，遵守 LP1 的 Direct／Index 域，不默认需求 payload |
| `Aggregate`／`WidePtr` | 各需求子路径只传给对应输入 |
| `MakeVariant(v)` | tag 可由已知构造常量生成；只有 v 的被需求 payload 传给输入，其余变体不访问 |
| `InsertField(base, i, replacement)` | i 子树的需求传给 replacement；其余路径传给 base。覆盖字段不再反传到 base |
| 聚合 `Phi` | 同一需求集传给所有 value incoming；Region 本身不作值叶路径 |
| `Repeat(element)` | 任意被需求下标的剩余路径合并给 element；保留 Repeat 紧凑表示，不按 length 展开 |
| 数组固定字段覆盖 | 覆盖下标的路径给 replacement，其余给 Repeat/base；不能把 overrides 与默认 element 混淆 |
| StoreValue、Return、外调参数、Opaque transport | 第一版 `All`；需完整可见值。enum 保留 tag 及可能活跃变体的数据，不能省掉将来外部可能读的字段 |
| Call／LoadValue 的聚合结果 | 需求来自实际使用者；输出物理 ABI schema 仍按既有规则保留，不能删掉有副作用的调用 |
| TransmuteValue／未知操作 | 第一版 `All` 或不可拆 opaque；不从类型字段需求推断完整对象表示的需求 |

这不是 LLVM 的原始 SROA 数据结构，而是适配当前语义 SSA 的方案。
最初可以只启用 struct／tuple／direct enum 的显式投影，其他情况 `All`；
但 fallback 必须在 demand 固定点中体现，不能在构图中途发现缺叶才追补，否则循环占位和
早已插入的快照读取点会变得难以维护。

### 2.2 inactive payload 与真正 Undefined 必须分开

对于一条已知构造为 A 的 Phi 入边：需求 B.payload 不读取 A 的字节，
而只给已经存在的 B 需求叶填 Undefined。若 B 的该叶根本没有消费者，连 Undefined 也不建。
LP2 继续跳过其机器拷贝。动态 enum 的被需求 payload 仍由 tag 分支保护。
只有 tag 需求时，不应建立任何 payload Phi；已有 `VariantTag` 是可沿用的表示先例。

对 `TagEncoding::Single` 不可居住变体，沿用原合法性边界；对 niche 与物理 tag 同字节的 payload，
沿用已有 `EncodedVariant` 及需求判定，不能把 tag-only 规则机械扩展到任意 payload。
共享 Undefined 节点可以减少 IR 分配，但不能改变每次使用的语义，不应以共用 SSA 身份
推导两个 undefined 使用必须相等。第一轮可以只避免未需求节点，不同时做这一去重。

## 3. 大聚合留槽：先测表示代价，再选策略

叶子 demand 并不保证 `from_str` 的所有 Phi 都能消失。如果聚合最终作为完整 ABI 结果返回，
其所有可能活跃的 payload 仍是实际需求。此时优化重点是“用标量跨多个合流传递”与
“保持私有内存对象、在消费者处读取”的表示选择，而非假装字段不活跃。

### 3.1 先收集的证据

为每个聚合 SSA 值／Phi 记录：类型尺寸与对齐、总叶数、需求叶数、enum 每变体需求、
Phi 入边数、经过的循环与合流、消费者类别、当前 spill/edge-copy 数、固定 load/store、
ABI materialize 次数、已有私有槽及是否逃逸。按热点函数汇总，不只全模块平均。

冻结对照至少包含：稀疏字段消费、完整返回、跨调用、循环携带大 enum、仅内存复制。
对实际热点比较 `.text`、动态复制字节／访存、栈帧、编译阶段指令与 runtime。
阈值候选可以以“需求叶数 × 需要搬运的入边数”及真实回边频次作起点；不能把已有
`max_unrolled_memory_bytes = 128`（控制内存展开）直接兼作聚合表示阈值。
LLVM 的 1024 分析 bitvector 限制也不是这个阈值的依据。

### 3.2 最小私有槽契约

- 留槽值应是一个独立的 **owned snapshot**，不是源地址的别名。`LoadValue` 在原位置
  将对象表示复制到私有槽；之后的字段读取来自私有槽，源内存的别名写入不会改变它。
- `InsertField` 产生新逻辑值时不能原地改写仍被旧 SSA 值使用的槽。第一版新槽加复制再更新；
  后续只有证明旧值死亡且槽不逃逸时才能复用。
- 普通聚合与 raw `MaybeUninit` 分清契约。raw 容器可复制未初始化字节，但不得为了搬运
  先用 bool／char／enum 的 typed load 解码，也不能读取 padding 当成已初始化标量。
- 聚合 Phi 的内存实现需要边上复制与 effect 合流；不能只是一个指向复用存储的地址 Phi。
  循环多值互换时必须在任何目标覆盖前捕获所有旧源，或使用经证明安全的并行复制调度。
  当前 `finish_opaque_phis` 的两阶段 staging 提供了正确性先例，但复制成本要实际计入。
- `decode_opaque`／`raw_copy` 已保留源时间点并使用 memmove；可复用其原语，不宜把普通
  聚合直接标为 `MaybeUninit`，避免混淆类型有效性与表示运输权限。
- 调用返回槽直接成为 owned snapshot、或把调用改写成写最终目标槽，须另证槽唯一、
  生命周期足够、没有别名／捕获观察，不能跳过现有 ABI ownership 规则。

建议先让表示决策统一落在 lowering 的一处，返回明确的 `ScalarLeaves` 或 `OwnedStorage`，
避免 `Field`、Phi、load、ABI materialize 各自按大小选一次而导致路径不一致。
从构造、复制、Phi、消费者到 ABI 的全链都支持后再启用阈值。

## 4. 随实现验证的最小矩阵

| 用例 | 要证明的事实 |
|---|---|
| 多字段 tuple 两分支合流，只返回第 2 字段 | 只建该字段的 Phi，无其他字段 Undefined／前驱物化；两臂输出一致 |
| 嵌套字段＋InsertField 覆盖 | 被覆盖路径不需求 base；旧 snapshot 的同字段仍保持旧值 |
| loop-carried aggregate，自回边与两条回边 | 先发布需求叶占位；构建终止，入边数正确，每轮值与 LLVM 一致 |
| A/B 大 enum，合流后只读 tag | payload Phi、payload load、inactive Undefined 均为零 |
| A/B enum，分支后各读一个 payload 字段 | 只建两个实际需求叶；inactive 边不读字节，niche 情况保持原语义 |
| borrowed aggregate load，随后别名写入，再取字段 | 需求叶在原 load 点捕获，结果保持快照；不可仅返回源地址 |
| aggregate 参数和调用返回值随后被别名写入 | ABI 边界快照与 load 快照一致 |
| 大 Repeat＋少量 overrides／零长数组 | 节点数量随需求和覆盖数增长，不随数组总长增长；无错误默认元素需求 |
| MaybeUninit 部分初始化、padding、raw Phi 互换 | 无 typed read；复制未初始化字节不变为观察；循环旧值不被覆盖 |
| 完整 StoreValue／Return／外部函数消费 | 不因当前函数只取少量字段而漏掉对外可见 payload |
| 大值私有槽跨调用、循环及旧值复用 | 新值更新不影响旧 owned snapshot，copy/effect 顺序和别名契约正确 |

现有 `semantic_build`、`semantic_inline_memory`、`semantic_maybe_uninit` 和真实语义 E2E
已有相关快照／raw 容器例子，应扩展其原链路。结构断言明确排除 effect Phi，只计值叶 Phi；
不能把 total Phi 下降作为唯一验收。最终仍跑真实 LLVM／kfrc off/on 差分、semver 和生成式测试，
并报告 `from_str` L 节点、Phi、Undefined、机器 mov、真实 spill 与动态 runtime。

## 5. 可复核来源

已实际读取的固定版本文件与 SHA-256：

| 文件 | SHA-256 |
|---|---|
| `SROA.cpp` | `b29efe3d5c94642b938d6dad3ddf1d20d1ea545ed04166a47d743abc6a1f6a8a` |
| `MemCpyOptimizer.cpp` | `7180f01312e76679da78fe8efa43ff57d4fcaf3107682d6e4c208ebf0ea5f9b4` |
| `PromoteMemoryToRegister.cpp` | `e1a388df510c2c8392d073c81cf541ff8b7fc700d8e4319ed607b0a0028ac214` |

源码暂存于本机 `/tmp/kfrc-llvm21-lp3-research/`，长期依据是以上固定 tag 链接及哈希。
本仓库事实来自当前 `lower_layout.rs`、`values.rs`、`memory.rs`、`semantic_memory.rs`、
`variants.rs`、`repeat.rs`、`maybe_uninit.rs`；本文描述的是读取时状态，不声称后续候选已实现这些建议。

## 6. candidate24 之后：不可变私有快照地址 Phi 的最小设计

本节是下一候选的设计，尚未实现。candidate24 的生产代码保持冻结；先测量 24，
再由主 agent 冻结仅改变 narrow forwarding 的 25，之后才实现此方案，避免混合归因。

### 6.1 现有复制成本与真实目标

candidate24 给每个完整运输 enum Phi 一个私有目标槽；每条入边先写 staging 槽，
所有源都捕获后再 raw copy 到目标。constructor 只写活跃字段和 tag，padding 与
inactive bytes 只通过原始表示复制。普通 typed enum 的 `OwnedStorage` 与
`MaybeUninit` 的 `OpaqueStorage` 仍是不同契约。

对已提取的 frozen22 `from_str` Specialized 图静态遍历：目标为 n312–n319 共 8 个
`Result<Version, parse::Error>` Phi，Region 为 n12–n19。它们有 20 条输入边：
13 个 constructor 输入位置、7 条 Phi→Phi 边，最后只有 n312 被 Return 使用。
从每个 Region 的前驱反向走完整控制图，均没有返回该 Region 的路径。
这不是 candidate24 的性能测量；后续图编号可能改变，应以候选的
`LayoutStats.owned_transport_phis` 和原始图重新核对命中数。

现有实现每个 constructor 输入需要一次完整布局 copy，每个 owned 输入需要
source→stage→destination 两次 copy，Return 还有一次到 ABI 槽的 copy。
按这套规则，这个尾部闭包会引入 28 个 Layout 层完整 enum raw copy 调用位置
（13 + 7×2 + 1；不计 constructor 内部字段自身可能需要的复制），并非每次调用
都执行全部位置。新方案预期保留 8 个机器地址 Phi，仅在 13 个 constructor 输入
位置写快照，7 条 Phi→Phi 边不复制，最终 Return 在 Layout 层复制一次。

这里必须区分 ABI 层次：`materialize_before` 先从 owned 槽复制到新的 ABI 槽，
随后 x86 的 `Return PassMode::Indirect` 还会 `copy_bytes` 到 hidden return pointer。
因此新方案的“最终复制一次”仅指 Layout 层；间接返回的机器级路径仍有这两次复制。
下一候选保持 ABI 路径，以隔离地址 Phi 的收益。后续可单独研究仅对 OwnedStorage
的 Return 直接以 LayoutValue PtrOffset 包装私有地址，因为 Return 消费只读源；
不能直接推广到 Call 参数，callee 可能修改按值传参空间。实际代码大小、栈槽分配、
指针存活期与运行时间仍须同环境测量。

### 6.2 表示与构造规则

保留 candidate24 的整个选择条件：完整值需求、只接 Return／StoreValue 或同组选中
Phi、输入只来自 MakeVariant 或同组选中 Phi、逻辑叶子数大于物理布局字数，且
目标 Region 不属于任何控制环。不能把“没有值 Phi 回边”作为最后一项的替代。

每个选中 Phi 建一个普通 `RawPointer<Enum>` Phi，并在递归展开输入前缓存
`OwnedStorage(pointer_phi)`。输入分两种：

- MakeVariant 输入：为该输入位置分配独立私有槽，在对应前驱 Jump 前 encode 一次，
  仅写 constructor 的活跃字段与 tag；地址成为该 Phi 的这个输入。
- 已选中 Phi 输入：直接传入其 `OwnedStorage` 地址，不分配复制目标、不做 raw copy。

不按 constructor 节点或多个输入位置合并槽。Return 和 StoreValue 仍在原来的固定
控制／effect 点，从快照向其最终目标 raw copy 一次。暂不让 constructor 提前写
用户的 StoreValue 目标，也不直接借用参数、load 或调用结果的可变地址。

### 6.3 一次初始化、不覆盖与多消费者证明

控制环检查必须使用整个函数 CFG。现有 `cyclic_region` 从 Region 的全部入边 Jump
反向追踪 control，遇到 Projection 追踪 producer，遇到 Region 追踪全部前驱；
这些边与 `analysis/cfg.rs` 的控制骨架一致。如果能回到起点，则拒绝选中。
因此，外层循环反复经过一个看似没有数据回边的内层值 Phi，同样会被拒绝。

若 Region 不存在到自身的非空控制路径，它在同一次函数调用内最多执行一次。
每个输入 Jump 执行后必进入该 Region，故它也最多执行一次。在该 Jump 前写独立
私有槽，意味着该槽在这一调用内至多初始化一次；递归函数调用拥有独立栈帧。
S 图验证器已有严格的 Phi 输入前驱支配检查，保证 constructor 的逻辑源值在该边可用。

槽地址由 lowering 新建，原来的 Rust 图不能引用它。它只进入内部地址 Phi 或作为
raw copy 的源，不成为原程序可写的地址，也不提前成为外部 ABI 参数。
因此初始化后没有修改源槽的操作。一个地址被多个选中 Phi 或多个 Return／StoreValue
消费者共享是安全的：所有消费者复制同一不可变表示，后续对某个外部目标的写入
不能回头改变内部快照。这里允许传递未初始化 padding／inactive bytes，不产生对它们
的 typed read，也不放宽原来值的有效性约束。

### 6.4 与 MaybeUninit 同边操作的顺序

保留 raw MaybeUninit Phi 原来的两阶段并行复制。typed constructor 的字段可能引用
当前旧 raw 快照，所以同一 Jump 上必须先执行全部源捕获，包括 typed constructor
encode 和 raw MaybeUninit source→stage，再写任何 raw Phi destination。
typed 组不再有第二阶段目标写入；每个 constructor 输入槽本身就是最终不可变快照。
不能先完成整个 raw Phi 组的目标写入，再 materialize typed constructor。

### 6.5 为什么暂不共用整个闭包的一个目标槽

只有单个终点或每个 Phi 只有单个数据 use，并不足以证明槽可合并。例如先计算 Phi a，
再计算 Phi b，最后按一个新条件选择 a 或 b；两者均只有一个 Phi use，控制图也无环，
但把 a/b 的快照共槽会使 b 的初始化覆盖之后仍可能选中的 a。
共槽需要额外证明所有写之间控制互斥，或旧值的最后使用发生在下一次写之前；
这属于完整 CFG 的写入与存活范围分析，不从数据闭包形状直接推断。

仅取消 staging 的折中方案可以尝试 constructor 直接写每个 Phi 的私有 destination、
owned 输入直接 copy 到 destination，但仍需证明同边没有 source 依赖任何将被写入的
目标。不可变地址 Phi 的不变量更直接，且连 Phi→Phi 的复制也一起消除，优先验证它。

### 6.6 LLVM 对应事实与下一候选回归

[MemCpyOptimizer.cpp:1107](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/MemCpyOptimizer.cpp#L1107)
的 memcpy→memcpy forwarding 检查复制范围，在 1201 行附近明确禁止两次复制之间
源被写入的情况；`writtenBetween` 使用 MemorySSA 与别名分析。新 source/destination
可能重叠时保留 memmove 语义。这里的不可变私有槽由构造规则直接满足 no-clobber，
不代表任意聚合地址都可做同样的转发。

[performCallSlotOptzn:842](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/MemCpyOptimizer.cpp#L842)
还要求 source 是 alloca、复制覆盖完整范围、局部控制位置和受限 source uses，并检查
目标在中间的访问、capture、unwind 等条件。因此，直接使用外部目标作为构造槽是
独立的后续优化，不在此候选内。

除现有真实 Rust transport_choice／transport_chain 差分，下一候选必须增加：

- 外层循环中的内部 enum Phi：值输入本身没有数据回边，整个控制图却会重复到达该
  Region；确认拒绝 owned address Phi，保持原来的每轮值语义。
- 多 sink 与多 Phi 共享：一个快照多次 StoreValue；随后覆写其中一个外部目标，再读
  另一个目标及旧快照，结果仍等于最初值。
- 两个顺序计算的候选 Phi，最后按条件选择其中之一：捕捉错误的单槽共用。
- 同一边 typed constructor 引用 raw MaybeUninit 旧快照，且该边另有 raw Phi 更新：
  保证先捕获旧字节再写 raw 目标，未初始化字节的初始化掩码也保持不变。
- 完整返回 enum 的多级 Phi 链：活跃 payload 与 tag 正确，内部只传递地址；以统计和
  Layout 图证明内部 raw copy 消失，再做真实 LLVM／kfrc off/on 对拍和性能测量。

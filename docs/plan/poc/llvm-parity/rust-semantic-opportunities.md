# LP3 可直接利用的 Rust 语义事实

日期：2026-11-09。本调查读取当前代码和 candidate15 保存的 S/L 图，不运行新编译或测评。
叶子 demand 由另一实现者负责；本文聚焦在进入 layout 之前，如何减少需要合并的 Rust 值。

## `from_str` 扩张的实际构成

来源：`evidence/poc-llvm-parity-lp2-20261109/results/candidate15/attribution/kfrc/graphs/{graphs.json,stderr.gz}`。
目标为 `semver::Version::from_str`，以包含 `6semver5parse`、`7Version` 并以 `from_str` 结尾的
mangled symbol 精确筛选。原始输入 S 532 节点，优化／内联后的 S 845 节点，L 4,767 节点。
早期计划的 4,859 是另一候选的数据，不能混作本轮计数。

| 类别 | 最终 S Phi 数 |
|---|---:|
| Effect | 43 |
| Result<Version, Error> | 8 |
| Result<&str, Error> | 4 |
| ControlFlow<Result<Infallible, Error>, …> | 7 |
| error::Position | 2 |
| (Prerelease, &str)、(BuildMetadata, &str) | 2 |
| 合计 | 66 |

L 的 695 Phi 中，154 是 Effect，541 是机器值（u8 290、char 135、usize 41、u64 30、
两种指针各 22、u32 1）。因此不能把“66 → 695”全归为 66 个聚合的叶子拆分。

明确的值流有两条：7 个 `Try::branch` 风格的 ControlFlow 包装在合流后马上再次测试；
8 个 `Result<Version, Error>` Phi 串起多个错误返回，最终 `Return(n312)` 观察完整返回值。
前者适合在 S 层路由掉；后者即使 demand 正确，也仍可能需求完整 ABI 返回，
不能承诺仅靠未使用叶剪枝就消除整个错误返回链。
当前 `phi_tree` 为 enum 的 tag 与所有声明变体 payload 递归建立 Phi；
`fill_phi` 对非活跃变体填 Undefined。嵌套 Result／ControlFlow／ErrorKind 正是放大器。

## 第一优先级：让已有 threading 接受内联产生的 Effect Phi 锚点

文件：[thread_variants.rs](../../../../crates/kfrc-son/src/transforms/control/thread_variants.rs)，
接口 `run` → `Plan::new` → `Plan::apply`。现有 pass 已具备：

- 沿每个 Region predecessor 选择对应 Phi operand；构造器决定 VariantSwitch 去向。
- 同时拆分全部 value/effect Phi，按同一 predecessor slot 保留顺序。
- 只克隆显式纯 opcode；拒绝 loop-carried 依赖、未知／poison 决策和无法放置的 use。
- 克隆预算为 dependent ≤128、总展开 ≤512，递进深度 ≤8。

已有证据中的第一个实例：

```text
n32  = VariantSwitch ctl=n367 (n375)
n371 = Phi ctl=n367 (n367, n372, n374) : Effect
n375 = Phi (n367, n376, n380) : ControlFlow<Result<Infallible, Error>, (u64, &str)>
n376 = MakeVariant[1](…)
n380 = MakeVariant[0](…)
```

`Plan::new` 遍历 `uses.of(region)` 时只接受 Operand(0) Phi 或 Control If/VariantSwitch。
`n371` 的额外 Control use 因而进入 `_ => return None`。同形状还出现在 Region
465、500、598、633、677、730，共 7 处。

最小改动是允许 **Control use 的用户为 Phi 且 input(0) 正是该 Region**。
Phi 仍只由 Operand(0) 路径收集一次；不允许任意有副作用 control user，也不放宽纯锥或循环条件。
这只消除当前确定的第一个拒绝原因，后续 witness／消费者／预算仍可能使个别候选拒绝，
需要测试确认，不能静态宣称 7 处必然全部消除。

回归应复现带 control 锚点的 effect Phi，比较 S 解释器两分支输出和真实 effect 顺序，
并断言至少一个候选被 thread。增加无关 effect user 或 loop backedge 的负例，确认仍拒绝。

## 第二优先级：统一查询“某条边上这个确切值的变体”

文件：[analysis/refinement.rs](../../../../crates/kfrc-son/src/analysis/refinement.rs)。
已有 `variant_on_edge(graph,value,edge)` 能读取 VariantSwitch 的精确 case；default 只在
剩余声明变体恰好一个时返回事实。`PredicateFacts` 进一步验证 proof dominance 和动态
instance：循环中的 Phi/fixed 定义可能让同一 NodeId 表示下一轮新值，旧证明不得沿回边泄漏。

目前 `thread_variants::Plan::new` 的 semantic-switch 分支只接受 `MakeVariant` incoming。
下一项可独立扩展为：构造器优先，其次询问 incoming 在 **对应 predecessor 执行点** 的 scoped
variant fact。先抽出一个复用现有 `same_instance` 检查的公开查询，不让调用者绕开证明作用域。
这能处理“前驱已测试 enum、原 enum 值穿过合流”的形状，不必先把它重建成 MakeVariant。
不要把 `variant_on_edge` 对任意祖先 edge 的结果直接用于任意后继；dominance 与动态实例检查仍必需。

`variants::normalize` 已在 promotion 后把 Discriminant equality 转成 VariantSwitch，
并给 VariantField 添加 witness；负判别值按源整数位宽匹配。
`pipeline::promote` 仅 promotion 真改变时调用 normalize；`optimize_specialized_seams` 在内联后
先处理 observation 再 threading。新的事实消费应复用这条链，不额外全图反复 normalize。

证据不能等价为需求：VariantWitness 是编译证明，不应让 payload 变成真实 machine demand。
同样，tag witness 只证明活跃变体，不证明 payload 初始化；
`transforms/memory/observation.rs::Proof::prove` 仍要求原值 defined。

## 第三优先级：利用具体泛型类型的 inhabited variants

事实生产已存在：`rustc_codegen_kfrc/src/convert/types.rs` 对 fully monomorphized ADT 调 rustc
inhabited predicate，写入 `RecordedTypes::record_semantic_inhabited_variants`。
查询为 `oracle.semantic().inhabited_variants(ty)`；`None` 表示未知。
`transforms/ipo/types.rs::TypeImporter` 会导入并核对该事实，不能因相同大小就合并类型身份。

当前主要消费者是 `semantic_value::unique_empty`、`specialize/inhabited::impossible` 和 ignored ABI。
一般 VariantSwitch 简化仍只看 MakeVariant；可以在**已知 defined 的具体 Rust 值**上，
用 singleton inhabited variant 折叠 tag query／switch，或者删除不可能 variant 的边。
例如 `Result<Infallible, Error>` 只有 Err 可构造，但 Error 的内容并非唯一值，不能将整个值
替换为某个空构造器。该改动需先复用 source signature／call result／RequireDefined 后的 defined
事实：Undefined 的 tag 查询不得凭类型被无条件变成已定义常数。
MaybeUninit<Uninhabited> 以及 pointer-to-uninhabited 也不继承 inner 的不可居留性。

`RustDefinition::specialize` 已按需求只实例化可达控制和使用到的 hole；分支先剪再复制。
具体类型更强事实应作用在 S seam 的 dirty 节点，并保留 R 每定义一次和 specialization identity
契约，不能为 LP3 改成每实例重扫整份 R 或仅以 layout 大小做缓存键。

## 内存与可变性：已有证据可复用，但不能擅自增强

`analysis/memory.rs::MemoryFacts` 提供 root、local identity、is_unescaped_local。
地址沿 FieldAddr／VariantFieldAddr／IndexAddr／PointerCast 传播；进入 Phi、打包进 aggregate、
未知 call、存储地址或 pointer-to-int 则整函数视为逃逸。`&T` 和 `&mut T` 均不自动产生
noalias／immutable 结论，Interior mutability 及 raw-pointer 派生仍需遵守这一边界。

`promotion::collect_slot` 可提升完整 enum 写入与 payload 路径读取；payload 局部写、SetVariant、
动态索引、Copy 和原始访存继续留存储。`States::read` 按需求在 Region 先发布整个 Rust 值 Phi，
然后回填入边；这是 LP3 得到 whole-value Phi 的来源之一。不要为消除叶子 Phi 而让提升后的
owned 值重新指向可变源 slot。

`forward::stored_snapshot` 只转发 control 与 effect 都紧邻、地址完全相同的 StoreValue；
unknown call／copy／lifetime 操作会阻断。`partition::run` 的 Local/Const 只加证明标签，
不重排 effect；Const 要求 data object 非 writable 且非 TLS。
layout 生成调用／地址后会重新计算 MemoryFacts，S 的 nonescape 证书不能直接沿用到 L。

能独立做的小改是 **为因 aggregate 打包后仅被 Field 解包而逃逸的局部地址增加保守 provenance
恢复**，但应先由 aggregate scalar fold 把可消除的包装去掉，再重算 MemoryFacts；
只有真实仍需要包装的根才值得扩展分析。该方向先记录 promotion 拒绝原因，优先级低于上面的
已证实 threading 阻碍；目前没有数据证明它主导 `from_str`。

## 验收与实施边界

1. threading Effect Phi 锚点兼容：独立小候选，S interpreter 正负用例＋真实 E2E，
   对照 S 的 variant_regions_threaded、ControlFlow Phi、RequireDefined 与 call/effect 顺序。
2. scoped variant facts：必须覆盖 Phi edge、default 唯一／多余 case、loop proof refresh、
   sibling 不可达推断不能跨 join。
3. inhabited variant：defined/Undefined、MaybeUninit、零长度数组、指针、类型导入冲突均覆盖。
4. 与需求实现整合后再报告 `from_str` 的 value/effect Phi 分项、Undefined、机器搬运和 runtime。
   不把 witness 数减少、S 节点减少或类型事实存在本身当作性能收益。

## 2026-11-09 续研：精确 effect 下的分支内 snapshot 转发（candidate25）

candidate23 已将紧邻 store→load 扩展为 load→load，保留首次读取及其 observation。
本节以 candidate22 的实际 S 图核对下一项；该图早于 candidate23–25，性能结论不能混用。
证据文件为 `evidence/poc-llvm-parity-lp2-20261109/results/candidate22/attribution/kfrc/graphs/stderr.gz`。

### 一个确定的热点，不需要 noalias

`semver::parse::numeric_identifier` 的最终 S 图共 388 节点，其中：

```text
n5  = LocalSlot[13] : addr(u8)
n33 = StoreValue ctl=n30 (n31, n5, n32)
n34 = Projection[0](n33)
n35 = Projection[1](n33)
n39 = If ctl=n34 (n38)
n40 = Projection[0](n39)
n43 = LoadValue ctl=n40 (n35, n5) : (Control, Effect, u8)
```

n33 和 n43 地址完全相同，n43 的 effect 仍直接来自 n33，只有 control 经过 If 的 case。
现有紧邻规则因此拒绝。n5 又经 n107 PointerCast 包装进 n108 closure Aggregate，触发现有
whole-function escape 规则，不能依赖局部 slot promotion 自动解决。
扫描 candidate22 全部最终 S 图，只走 If／VariantSwitch case control 链、且 effect 直接来自
同地址 store/load，发现 **1 个非紧邻匹配**，就是 n33→n43；该计数不是动态执行次数。

同函数 n281 又读 n5，effect 往上穿过 RequireDefined，control 穿过 Region。
它是另一项机会，**不在本轮范围**。不能把这两处都计为本规则收益。

最小实现只扩展 `forward::available_snapshot` 的 control 判定：

1. effect 必须仍是同一个 StoreValue／LoadValue 的 Projection[1]。
2. 读取地址 resolve 后 NodeId 完全相同，snapshot 与读取结果 Rust TyId 完全相同。
3. control 向上仅穿过合法的 If／VariantSwitch Control projection，最多 32 层，最后必须到
   上述 access 的 Projection[0]；拒绝 Region、Jump、其他 fixed 操作、未知／越界 projection。
4. 转发时用 **原 load 的 control 与 effect** 替换输出。不能把 control 替换为 store 的
   continuation，否则后续操作会绕过分支，也会破坏条件本身的 poison observation。
5. 保留原 store／首次 load，既不提前读取，也不移除原地址观察；不放宽内存 partition。

此证明只涉及已有 snapshot 和执行路径，不依赖 `&T`、`&mut T`、静态可变性或 escape。
同地址 raw pointer／UnsafeCell 在这条链上也没有可能改变内容的操作；Call、Store、Copy、
SlotLive／SlotDead 都会改变 effect 并阻断。即使数值地址相等，不同 SSA 地址也不匹配。
Region 一律拒绝，避免把上一轮循环的相同 NodeId 当作当前动态实例。

本轮实现范围为 `transforms/memory/forward.rs`；测试在 `tests/semantic_memory_optimize.rs`。
新增差分覆盖：store/load 两种来源、两层动态 If 的所有输入、VariantSwitch case/default、
不同地址／类型、隔着 store/call/SlotLive/RequireDefined，以及 join 和 loop header。
真实 Rust fixture 由主 agent 添加；编译与 E2E 由统一验证 agent 执行，不能把格式检查当验收。

### 当前保存的 provenance 到哪里为止

| 信息 | 实际保存位置 | 可证明的范围 |
|---|---|---|
| 引用与 raw pointer 区别、mutability、pointee | `TypeShape::{Reference,RawPointer}`；rustc convert/types 与 definition/types | 类型身份，不含 borrow scope、Freeze、noalias、解引用有效期 |
| 本地分配身份与派生地址 | `MemoryFacts` 的 root/local/escaped；S LocalSlot，L StackSlot＋stack_slot_locals | FieldAddr／VariantFieldAddr／IndexAddr／PointerCast；L 指针型 PtrOffset／IntCast |
| whole-function escape | 同上，通过 Uses 扫描 | Phi、aggregate 包装、call、存储地址、ptr→int 保守逃逸；RequireDefined 本身不捕获 |
| 编译期 relocation provenance | rustc `constants::emit_allocation` → `DataObject.relocations` → graph data_objects → IPO import | 符号目标与 addend；不能把 relocation placeholder 的 0 当实际指针值 |
| 静态 writable/TLS | `DataObject`；writable = 显式 static mut 或 CTFE allocation mutable | CTFE 将 UnsafeCell 静态标为 mutable；partition Const 仅非 writable 且非 TLS |
| 内存访问分类 | `MemoryPartition::{Unknown,Local,Const}` | 证明标签，不改变 effect 顺序；layout 之后需重算 |
| 生命周期 | `SlotLive`／`SlotDead` | 前者激活并清除 init，后者终止并清除 init；不能只比较地址绕过 |

因此只读引用 loop hoist 不是下一项小补丁：当前没有 Freeze 和借用活跃作用域，也没有足够的
call memory summary。即使某个参数标 readonly，也不意味着其他指针／调用不能写该内存。

### LLVM 对照：保留现有局部证明，先不引入完整 AA/MemorySSA

核对 LLVM `llvmorg-21.1.8` 源码：

- [EarlyCSE.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/EarlyCSE.cpp#L1120)
  的 `isSameMemGeneration` 先比较 generation；不同时只有 MemorySSA 可以继续证明无 clobber。
  `getMatchingValue` 还核对 matching ID、volatile/ordering/atomic 兼容；可用表按 dominance scope
  管理。我们的“相同 effect＋纯 case 控制脊柱”是更窄的同代／支配证明，不需要照搬全套 AA。
- [LangRef readonly](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/LangRef.rst#L1647)
  明确 readonly pointer argument 仅表示不**通过该参数**写，别的路径仍可能写该地址。
- [LangRef Object Lifetime](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/LangRef.rst#L3350)
  明确地址计算与访问存活对象不同；保持地址值不代表 load 可跨 lifetime end。
- [Rust UnsafeCell 文档源码](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/library/core/src/cell.rs#L2120)
  明确 `UnsafeCell` 取消 shared reference 的 immutability，不取消 `&mut` 唯一性；raw pointer
  使用者仍须满足别名与生命周期规则。本项目缺少这些规则的可消费证书，所以当前不增加类型推断。

下载核对文件保存在 `/tmp/kfrc-llvm-memory-21.1.8`。EarlyCSE SHA-256 为
`57b77366a1f8b3e599b762382fa1b200490bfa22d674d1e12ec1b370cea2c3cb`；LangRef 为
`8175f0398b92ecf658ba4e2d64e83be0ab806f2e8bef5c7ce1e4c01a3c6baa45`。

### 具体类型／常量方向：两个真实命中，但先补初始化证据

candidate22 `Version::from_str` 最终 S 图的 n113、n186 分别读取 Prerelease 和 BuildMetadata 的
只读常量。两者都有 SymbolAddress→PointerCast→LoadValue，partition 为 Const，随后进入
`(Prerelease,&str)`／`(BuildMetadata,&str)` aggregate。源码空 Identifier 在 x86-64 是八字节
全 1、零长度 tail；它经 NonNull 包装，不能随意按普通整数替换 Rust aggregate。

这两处值得后续做 **编译期静态对象 snapshot 解码**，但现在不是可以直接读取 bytes 的安全规则：
`constants::emit_allocation` 使用 `inspect_with_uninit_and_ptr_outside_interpreter` 得到 bytes，
`DataObject` 只保存 bytes/relocations，没有保存 allocation 的 init mask。
因此不能由“有 bytes”推导所有语义字段已初始化；padding 与 MaybeUninit 尤其不能被补成已定义值。

最小后续设计应先由 producer 保存可验证的常量初始化信息，或直接生成 producer-validated 的
typed constant snapshot；对于整段无 relocation、所有实际字段均已初始化、非 writable、非 TLS、
本编译单元 local binding 的静态对象，按**具体 monomorphized 类型的 layout**解码字段。
首版仅支持 scalar／struct／tuple 和零长度 array，拒绝 enum、MaybeUninit、union、动态 offset、
relocation 重叠和任意不完整初始化；指针不能借裸 bits 创造 relocation provenance。
若选择在 L 折叠 scalar load，就保持 S 的定义性证明与 demand 阶段分工，不在 S 任意使用物理 offset。

[LLVM ConstantFoldLoadFromConstPtr](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/ConstantFolding.cpp#L744)
要求 `isConstant()` 与 `hasDefinitiveInitializer()`，按 DataLayout 累加 constant offset 再解码 initializer。
它的 initializer 仍保留 LLVM undef/poison 与 symbolic relocation 表达能力；不能只复制其
“global bytes→constant”外形，忽略本项目 DataObject 契约更弱。

常量方向验收 fixture：嵌套包装的只读整数、零长度 tail、UnsafeCell static/static mut/TLS 不折叠、
带 relocation 的指针不折叠、partial initialized aggregate 与 MaybeUninit 不补定义、越界／偏移拒绝。
真实验收观察上述两个 load 是否消失、后继常量传播及 semver 行为；当前只有静态机会证据，未测收益。

# LP3：具体类型的 inhabited variant 与 LLVM/Rust 分工

2026-11-09。仅做源码和已有图的静态研究，未改生产代码、测试或运行编译。

结论：项目已经保存具体 Rust 类型的 inhabited variants，但 candidate22 的真实 semver 图
没有发现可由 `Result<Infallible, Error>` singleton 事实直接消除的 tag query/switch。
此方向可作为语义完整性补充，当前不应优先于已有命中的 threading、demand 和内存转发。

## 真实图：singleton residual 没有直接 tag 消费者

证据：`evidence/poc-llvm-parity-lp2-20261109/results/candidate22/attribution/kfrc/graphs/stderr.gz`。
对每个 symbol 保留最后一个 `(Specialized, Verified)` block，再按节点的**外层类型**筛选，
避免把 `ControlFlow<Result<Infallible, Error>, T>` 当成 singleton enum。

| 统计 | 数量 |
|---|---:|
| 最终 S 函数图 | 598 |
| 含外层 `Result<Infallible, …>` 值的函数 | 47 |
| 此类值 | 116 |
| 直接消费这些值的 VariantOf／Discriminant／VariantSwitch | 0 |

`semver::Version::from_str` 的确切 symbol 为
`_RNvXNtCslLN4iPamcnI_6semver5parseNtB4_7VersionNtNtNtCshX3mvxHm3fC_4core3str6traits7FromStr8from_str`。
其中此类值共 14 个：n37、n48、n62、n73、n87、n130、n204 为 VariantField[1,0] 结果，
n377、n475、n510、n608、n643、n687、n740 为 MakeVariant[1] 构造器。
类型为 `std::result::Result<std::convert::Infallible, parse::Error>#e4cb3b3a34194255-5ffcc78fe35f3266`。

例如 n32 `VariantSwitch ctl=n367 (n375)` 检查的是外层 ControlFlow；n375 的两个 incoming
为 n376 MakeVariant[1] 与 n380 MakeVariant[0]。Break 的 residual 只有 Err 可构造，
并不意味着 Break 与 Continue 只有一侧可达。这里的机会属于此前记录的 threading。
candidate22 早于后续 LP3 候选，本统计不能充当 candidate25 之后的当前图或收益证明。

## Rust 负责产出类型事实，LLVM 消费降低后的整数事实

Rust 源码固定到 `e408947bfd200af42db322daf0fadfe7e26d3bd1`：

- [inhabitedness/mod.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/inhabitedness/mod.rs#L78)：variant predicate 是字段 predicate 的 conjunction；enum 是 variants 的 disjunction。union 保守 inhabited；Never 不 inhabited；引用等类型不继承 pointee 的不可居留性；零长度数组 inhabited，非零数组才继承元素事实。
- [inhabited_predicate.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/inhabitedness/inhabited_predicate.rs#L71)：`apply_ignore_module` 忽略可见性限制，可以使用私有字段的不可居留性。泛型必须先按具体 args instantiate；未知不能当作 false。
- [UnreachableEnumBranching](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/unreachable_enum_branching.rs#L53)：识别紧邻 enum discriminant assignment 的 SwitchInt；由 layout 的 Empty／Single／Multiple 得到允许 discriminants，把不可能目标改为 unreachable。generic layout 不可得时有较弱的 variant_range 回退。它还限制某些 default 重写，避免 LLVM early-tailduplication 放大 CFG；不是不计代价地拆分所有分支。

LLVM 固定到 `llvmorg-21.1.8`：

- [SimplifyCFG eliminateDeadSwitchCases](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Utils/SimplifyCFG.cpp#L5795) 对 integer condition 查询 KnownBits 与 significant bits，裁掉不可能 case；case 覆盖所有可能值时可删 default。此处没有 Rust ADT 或 inhabited variant 概念。
- [ValueTracking definedness](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/ValueTracking.cpp#L7572) 分别处理 noundef arguments／call results、poison／undef constants、freeze 和 Phi predecessor contexts，递归有预算。范围事实与 definedness 是不同证据，不能用 singleton 类型替代后者。
- [LangRef noundef](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/LangRef.rst#L1532) 约束值表示，不把 padding 算作值；LLVM 对 UB 的 refinement 权限也不能直接代替本项目 S interpreter 的 poison-observation 保持契约。

因此最合适的位置是 **S 层具体 Rust 类型上的语义规则**，之后让现有控制清理和 layout 使用结果；
不需要为这项机会移植 LLVM integer range analysis，更不能在 R 层按未实例化泛型下结论。

## 项目已有接口与证明边界

`rustc_codegen_kfrc/src/convert/types.rs` 对 concrete enum variant 调用
`inhabited_predicate → instantiate(args) → apply_ignore_module(fully_monomorphized)`，
写入 `RecordedTypes::record_semantic_inhabited_variants`。
`oracle.semantic().inhabited_variants(ty)` 的 None 是未知，Some([]) 是没有可构造变体，
Some([v]) 才是 singleton。事实绑定完整 TyId，不能以大小、ABI encoding 或 layout 相同替代类型身份。
`transforms/ipo/types.rs::TypeImporter` 导入该事实，先检查与目标已有事实冲突再写入。

`specialize/inhabited::impossible` 目前是 source-signature boundary 证明，明确排除
MaybeUninit、union、pointer/address 的 inner 推导，并正确区分零长度数组。
`semantic_value::unique_empty` 只重建唯一空值；`Result<Infallible, Error>` 的 Error payload
并不唯一，因此不能借 singleton tag 构造任意 Error 或整个 residual。

`transforms/memory/observation.rs` 目前有私有 definedness 证明：Start 的 Rust 参数、Call／
CallIndirect 的 Rust 返回投影在 continuation 后 defined，RequireDefined 的输入在该观察的
continuation 后 defined。Source contract 独立于物理 ABI transport，包括逻辑零大小参数。
解释器在入口和调用结果观察完整值；VariantSwitch 对顶层 Poison 报 PoisonObserved，
VariantOf／Discriminant 则传播 Poison。类型信息本身不能把 Undefined query 变成正常常数。

结构证明允许常量、全部输入 defined 的 aggregate/constructor、空 Repeat 等；VariantField
还需要 matching constructor 或 scoped witness。**Tag witness 不证明 payload 已初始化**。
完整值 defined 是首版可接受的保守条件；MakeVariant 的已知 tag 仍由现有规则处理，无须扩大
到“任意 partial value 的 tag 应该可知”。

还有两处不能省略：

1. **动态实例。** observation 的现有私有 same_instance 检查 definition anchors 支配 proof；
   refinement 的 `PredicateFacts::same_instance` 另有 `unchanged_until` 路径检查，避免循环中
   相同 NodeId 代表新实例、旧事实绕回边流入。复用 definedness 时应让两者采用同一动态实例
   契约，或首版明确拒绝循环；不能只复制 dominance 判断。Phi incoming 必须在各自 predecessor
   edge 证明；循环 Phi 不能用自身递归证明 defined。
2. **Witness 存续。** `analysis/refinement.rs::check` 当前只接受 matching MakeVariant、
   exact VariantSwitch edge 或 VariantOf predicate 证明。若删掉提供证据的 switch，原
   VariantWitness 即使仍在 reachable scope 也不再合法。首版跳过有存活 witness 的 switch
   最简单；若要优化 payload 分支，必须同步增加 exact type＋defined-at-scope 的 witness
   规则，并对每个 payload execution site 检查作用域和动态实例，不能制造无证据 witness。

## 最小可实施方案

先只支持 Specialized 中的 singleton inhabited set，不处理 empty set 或部分 multi-variant
case pruning。后两者牵涉 unreachable、default、Phi 入边编号与更多 witness 改写，且没有本轮收益证据。

- **固定 switch：** 在该 switch 的 control 点证明 exact source defined；由 singleton ordinal
  选显式 case，否则选 default。保留先前 RequireDefined 和 effect，使用现有 projection／CFG
  清理规则。先跳过 live witness 来源分支，或同时完成上述 witness 证明扩展。
- **浮动 query：** VariantOf 返回 ordinal，Discriminant 必须查 semantic discriminant 并按
  结果整数宽度归一化。只在某一点 defined 不够全局 `graph.replace`；需证明 query 的每个
  execution site（包括 Phi predecessor edge）都被有效事实覆盖。初版也可仅接受源参数／
  call result 且所有 uses 在其 continuation 支配范围内的情况。
- **集成：** 在已有 S seam 上消费 dirty 节点，共享 immutable Uses/CFG/dominance snapshot，
  查询有预算；改写后重建失效证据。保留 R 每定义一次和 specialization identity，避免每个
 实例重扫 R。当前缺乏热点命中，暂不增加一轮无条件全图扫描。

## 后续验证矩阵

| 场景 | 必须比较的结果 |
|---|---|
| source 参数、direct/indirect call 返回的 singleton enum | 值、effect/call 次数不变；tag query／switch 可消失 |
| Undefined enum；RequireDefined 之前／之后 | 未证明时不折叠；保留首次 poison observation |
| payload 含 Undefined 的 constructor、VariantField | 不把 tag witness 当 payload defined；已有 constructor fold 行为保持 |
| sibling branch、join、Phi incoming | 单侧事实不越 join；各 incoming 在其 predecessor 点检查 |
| loop Phi、loop 内 call/load、proof refresh | 旧动态实例不泄漏；只有重新执行的有效 proof 可使用 |
| live VariantWitness 与后续 payload use | 验证器接受合法新证据；不留下失效 switch-edge 证明 |
| None／空／多 inhabited variants | 首版全部保守跳过；default 不因声明顺序错误选边 |
| 显式正/负 discriminant 与不连续 ordinal | VariantOf 与 Discriminant 结果各自正确 |
| MaybeUninit、union、pointer-to-Never、零/非零数组 | 不越存储与 pointee 边界推导 |
| concrete generic 实例、IPO 类型导入冲突 | 按精确 TyId；冲突仍拒绝导入，布局同形不合并事实 |

实现后由统一验证 agent 做 S interpreter 差分与真实 Rust E2E；记录新候选真实命中数量，
不能把类型事实数量或 witness 数减少当 runtime 收益。当前没有实施或动态验收结果。

## 指纹

图 gzip SHA-256：`14bdd2a55d5f4dd81c05261d9b3bde57c54643ea293bdfb6fa1f77db8507566c`。
上游源码缓存：`/tmp/kfrc-inhabited-sources`。

| 源码 | SHA-256 |
|---|---|
| Rust inhabitedness/mod.rs | `d4641cc8def7f91c671b77d344046f1fad1d8f318b54fabba374dfa057b5a973` |
| Rust inhabited_predicate.rs | `5b635a6968eb2a23fdd0879ade318e070c93a7995723737d7910c02a74cde01f` |
| Rust unreachable_enum_branching.rs | `c08045505d2e3890e69c1618250fff69e3197d609f60f4c132dc732648927cf1` |
| LLVM SimplifyCFG.cpp | `4efb32367752de87b5a872fe6343b076c57413723a75311d28a8db9362aa6e21` |
| LLVM ValueTracking.cpp | `4442332d0d1997952fc373e2ddd309cd26fd1bec19aa5453bf1eec18e3c8ff57` |

# LP3 S→L 的叶子 Phi 与聚合搬运

## 问题

`from_str` 从 S 层的 845 个节点（Phi 66）降到 L 层后有 4,859 个节点，其中：

- Phi 695、Undefined 473；
- Projection 1,138、Const 728、PtrOffset 475。

从计数看，降低时把每个聚合值拆成叶子，并在每个合流点为每个叶子建一个 Phi，
不活跃变体的叶子用 `Undefined` 填充。每个 Phi 在机器码中至少变成一次 mov
（LP2 之前是两次，且经过栈）。

这是 [W09 large values](../../../research/walkthroughs/w09-large-values.md) 和
[W01 try error path](../../../research/walkthroughs/w01-try-error-path.md) 讨论过的问题。
Observe／Refine 重设计已经去掉“仅为证明而物化”，剩下的是**真实数据流的表示选择**。

## 目标

1. **只给活跃叶子建 Phi**：先做一次叶子级活跃分析（合流点之后哪些叶子真的被用到），再建 Phi。
   没有使用者的叶子不建 Phi，也不在前驱上物化。
2. **按变体合并**：enum 值在合流点只合并 tag 和**当前前驱上活跃变体**的 payload。
   某个前驱已知是变体 A 时，变体 B 的 payload 叶子对该前驱取 `Undefined`，并且在 LP2 中不产生拷贝。
   能按前驱确定变体的，沿用 thread_variants 的拆分，不在合流后再分支。
3. **大值留在私有槽里**：以真实需求叶数、物理布局和合流搬运成本选择表示。
   当前先限于无控制环、只传递完整 enum 的闭包；构造时捕获活跃字段，sink 复制表示。
   后续若支持字段读取，须先表达读取的 effect 时点，不能在现有纯 Field 上延迟读原地址。
   LLVM SROA 的分析上限不是这里的字节阈值。
4. **常量与地址去重**：layout 内对 `Const`、`PtrOffset(base, k)` 做哈希去重（局部 GVN），
   同一基址的连续偏移交给 LP4 的地址模式折叠。

## 必须保留的正确性边界

- 快照语义：已读入的 owned 值不受之后别名写入影响。留在槽里的值必须是**私有副本**，或者已证明期间无别名写。
- inactive payload、padding、MaybeUninit 不被当成已初始化读取。
- Undefined 只在 IR 层“任意值”有意义；对外可见的位（返回值、存入内存后被读出）不得依赖它。

## LLVM 参考

- [SROA.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SROA.cpp)：按使用切片、只提升可提升的部分；
- [MemCpyOptimizer.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/MemCpyOptimizer.cpp)：大值拷贝消除。

LLVM 的 rustc IR 对大 enum 不做全量 SROA，而是保留 alloca 加 memcpy，这正是 `from_str` 在 LLVM 下只有 232 条指令的原因之一。

## 验收

- 结构：W01／W03／W09 的 fixture 中，L 层 Phi 数不超过“合流点 × 实际使用的叶子数”；Undefined 输入不产生机器指令。
- 差分：三臂语义语料、MaybeUninit 和快照用例全过。
- semver：`from_str` 的 L 节点数、Phi、Undefined 与机器 mov 前后对照。

## 实施前研究（2026-11-09）

[LLVM 21.1.8 聚合研究](llvm-aggregate-research.md) 由独立研究 subagent 完成，
附 SROA、PromoteMemoryToRegister、MemCpyOptimizer 的固定源码链接与哈希。
第一步采用冻结的叶子 demand worklist、内部 Omitted 标记和先发布再填充的 Phi 占位；
不把未需求叶表示成 IR Undefined，也不把真实读取推迟到字段消费者。
完整 ABI 返回仍可能需求全部叶子，因此 demand 本身能减少多少 `from_str` Phi 要实测。
大值留槽必须是私有快照，其阈值需根据叶子数、入边搬运与真实热点确定，
LLVM 的 splitAlloca 1024 分析上限不能直接用作“不标量化阈值”。

## 当前实施方向：以 Rust 语义减少数据量

用户明确将重点转到 LP3（2026-11-09）。LP2 candidate22 作为冻结对照，
继续保留其未解决的编译指令回退，不再仅围绕机器分配器迭代。

首批实现分工如下，测试通过不等于性能验收：

- 布局 subagent：对 S 聚合值进行字段需求 worklist，内部 Omitted 与 Undefined 分开；
  只创建所需字段 Phi／load，循环占位先发布，未知消费／完整 ABI 返回用 All。
  数组、Repeat、MaybeUninit 初版保守保持原完整表示。
- 变体 subagent：修复内联 Effect Phi 上 `control=region` 阻止已有按前驱变体穿线的情形；
  保持原有循环、纯节点复制及 effect/value Phi 同步处理的边界。
- 主 agent：连续且同 control/effect 链、同地址、同 Rust 类型的 LoadValue 复用已捕获
  快照；不跨调用、写入或生命周期边界，不从引用可变性直接推出 noalias。
- 验证 subagent：新增真实 Rust `support_lp3_demand.rs`，在冻结 candidate22 上先验证
  LLVM／优化关／优化开三臂，再对候选做相同验证；覆盖单字段选择、enum payload、
  owned 写后快照、循环 Phi、整体返回、MaybeUninit、const generic 与覆盖字段。

下一批才考虑大值私有槽；先测首批对 `from_str` L Phi／Undefined／load／mov 的实际影响。
留槽不得使用原可变源地址代替 owned 值；复制消除需要别名、逃逸和观察时点的证明。
引用／逃逸事实与具体泛型类型的机会记录于 [Rust 语义机会](rust-semantic-opportunities.md)。

### candidate23 冻结与验证

源码 SHA-256：`3d073bfa349ee32e8c34648287d31084dafc012f7012ce061446d4806c4f8363`。
本机 610 passed、1 ignored；E2B 先验证最终 Rust 语义三臂每臂 510 行一致，
再通过高压力、旧 seed5、workspace、features、CLI、semver、legacy 与生成式全套。
基线为冻结 candidate22，而非在 LP2 中挑选某个负载最快的候选。
正式性能与 S/L 图对照记录在 `evidence/poc-llvm-parity-lp3-20261109/`。

candidate22 → 23 的实测结果：7 个 `ControlFlow` 包装合流被穿线消除，
`from_str` S 节点 845 → 745，L 节点 4767 → 4447、Phi 695 → 571、
Undefined 473 → 363；机器字节 29304 → 26950，显式内存操作 2783 → 2535。
总 `.text` 减少 3.39%，总编译指令减少 3.70%（仍为含 MIR/collector 的旧口径）。
parse／req／identifier 中位耗时变化为 +3.77%／−1.12%／+4.28%，未取得运行时验收。

针对 identifier 回退的静态核查：`parse::identifier` 与 `bytes_for_varint` 的对象符号
机器序列在 22／23 间相同；链接后前者仅 4 条 RIP-relative `lea` 位移不同，归一化后
1456 条指令与 116 个内部跳转相同，后者 5397 字节仍逐字节相同。
已观察到链接布局变化，但没有证据证明它导致运行时差异；回退原因仍未定位。
证据为 `results/candidate23/identifier-static-comparison.json`。

### 私有槽的下一步边界

实际 candidate22 的 `from_str` S 图中确认 8 个 `Result<Version, Error>` Phi
构成纯传递到 Return 的链，无 Field／Discriminant 消费者。
下一候选只对结构上满足同类条件的无回边 enum Phi 闭包实验留槽；
未知 consumer、call 参数、字段／tag 消费或回边均回退到现有表示。
选择比较完整标量叶数与物理布局机器字数，针对变体重叠造成的表示膨胀，
不采用未经测量的任意“大于 N 字节”阈值。

`OwnedStorage` 与 MaybeUninit 的 `OpaqueStorage` 明确区分。
每条入边先将全部来源捕获到私有 staging，再拷贝到目标槽，维持同时赋值；
构造器只编码其活跃字段，padding/inactive 字节通过表示拷贝传输，不作 typed load。
纯 Field 缺乏 effect 输入，因此不在这一批实现“在字段消费者处临时读取私有槽”。

### Rust 信息的实施状态

| Rust 信息 | 已落地 | 下一步及边界 |
|---|---|---|
| 枚举变体、分支细化 | 直接 tag 数据流；Effect Phi 锚点穿线；按需 payload | scoped variant fact 需保留 dominance 与循环动态实例约束 |
| 字段实际使用 | demand worklist、稀疏叶子 Phi/load、覆盖字段裁剪 | 数组、未知消费、raw 表示仍保守；不改变快照时间 |
| owned 快照 | 连续重复读取复用；candidate24 私有槽完整传递已通过正确性验证 | 先量化副本成本，再证明私有不可变槽的地址 Phi 可以替代逐边复制 |
| 别名、可变性、逃逸 | exact pointer/type、同 effect 的快照转发 | candidate25 扩至纯分支控制链；不从 `&mut` 或 `&T` 直接推导全局 noalias/readonly |
| 具体泛型类型、常量、布局 | monomorphized 类型、inhabited variants 和布局已保留 | 消费更强类型事实需要 defined 证明；常量字节折叠还需初始化掩码契约 |

### candidate24：正确，但复制放大导致回退

冻结 SHA-256 为 `bd584ea6ae8e8d6e07bbbe9d67d8c6419d54d561d81ea40c5adfa1c2903a3004`。
本机 611 passed、1 ignored；E2B 语义三臂 518 行、压力三臂 2048 输出以及完整
workspace／features／CLI／semver／legacy／generated 均通过。

实际选中 `from_str` 的 8 个整值 Phi，但 23 → 24 的 L 节点从 4447 增至 5556，
Phi 571 → 477、Undefined 363 → 238，同时 Store 80 → 181、Call 31 → 59。
新增 28 处 40 字节 `memmove`，静态复制尺寸和为 1120 字节，不能当作一次运行的动态流量。
机器字节 26950 → 31456，指令 4979 → 6037，frame 分配 0x1060 → 0x1570。

同轮 LLVM／22／23／24 串行测量中，24 相对 23 的 parse／req／identifier
中位耗时分别上升 12.04%／12.44%／2.06%；相对固定 LP3 基线 22 上升
19.15%／10.80%／7.19%。因此本候选只有正确性通过，性能不接受。
详见 `candidate24-comparison.md` 和 `results/candidate24/from-str-comparison.json`。

后续先停止重复计时，对照已完成的 LLVM copy-forwarding 研究消除逐边复制：
在 constructor 入边写各自的不可变私有槽，Phi 仅选择地址，保持 sink 的表示复制。
控制环检查须包含外层循环，多个存活值不得共用一个可覆盖的槽；MaybeUninit 的同边
同时赋值仍保留先捕获后写目标。candidate25 仅先冻结纯分支快照转发并做正确性验证，
candidate26 完成上述复制修复后再恢复正式性能对照。

### candidate25：纯分支控制链上的快照转发

冻结 SHA-256 为 `ba6873e1c00acc5f3f45df86937cd21da92bd7bac4b302eea6282f04ca8eece5`。
在同一 effect、同一地址、同一 Rust 类型的前提下，控制链可穿过最多 32 层
If／VariantSwitch case，终点必须为原 store/load 的 continuation；保留读取处的控制。
Region／Jump／其他 fixed 节点、不同地址／类型、调用、写入和生命周期边界仍拒绝。
实际机会是 `numeric_identifier` 的一次 StoreValue → If → LoadValue。

新增回归首次遇到的失败是测试直接比较了 `PoisonObserved(NodeId)` 的节点编号，
compact 后定位改变但异常类别相同。静态检查同组后仅归一该异常的位置，保留所有其他
错误明细和成功值；修复后本机 616 passed、1 ignored，E2B 完整正确性通过。
真实 Rust 三臂覆盖 526 行，包括嵌套分支和别名写入。本候选只做正确性与结构验收，
不重复测量仍含 candidate24 复制放大的性能方案。

已保存的结构对照确认 `numeric_identifier` 首轮 loads_forwarded 从 1 增至 2，
S LoadValue 3 → 2，L Load 22 → 21；机器字节 5796 → 5740、指令 1094 → 1084、
mov 812 → 804、显式访存 560 → 553。`from_str` 与 candidate24 的 S/L 计数及机器
指令计数相同。证据为 `results/candidate25/numeric-identifier-comparison.json`；
这些是静态机器形状改进，未单独测量本项的运行时收益。

### candidate26：不可变私有槽地址 Phi

冻结 SHA-256 为 `461793a0948e18aa89471ea426f5593b52a198bfc5a65dcefc010bf055b35ec8`。
constructor 入边各自初始化独立私有槽，完整值 Phi 只选择地址；Phi→Phi 不再复制。
选中规则仍要求无完整 CFG 控制环；新增合法 S 图回归确认外层循环中的内层 Phi
不会误选。MaybeUninit 仍保留全源捕获后写目标的调度，部分初始化字节不被补定义。

本机 618 passed、1 ignored，E2B 完整正确性通过，真实 Rust 三臂扩至 534 行，
包含多个存活值顺序合流、多 sink、外部目标完全别名以及外层循环。
此候选保留 Return 的中间 ABI 槽，单独测量运输复制的消除。实际 24 → 26：
`from_str` memmove 28 → 1，L 节点 5556 → 5333、Call 59 → 32；
Phi 477 → 485（新增 8 个地址 Phi），Store 181／If 188／Region 220 等重编码成本未变。
机器字节 31456 → 30318、mov 3938 → 3778、frame 0x1570 → 0x12b0。

同轮固定 22 → 26，`.text` 减少 4.39%，编译总指令减少 5.58%，但 parse／req／identifier
中位耗时仍上升 9.23%／2.87%／5.76%。这轮没有同轮 candidate23 runtime，不能跨轮
把 23 的耗时直接作新对照。性能仍不接受，停止追加计时，先解决已定位的重编码成本。
后续独立 candidate27 仅为 OwnedStorage+Return 复用现有私有地址，继续保留
LayoutValue 类型检查和机器层最终 sret 复制，不扩展到 Call／Opaque 参数。

constructor 重编码仍是独立成本，精确来源与下一步方案见
[调用结果表示研究](lp3-transport-followup.md)。

### candidate27：返回直接消费私有表示

冻结 SHA-256 为 `0d23a8d35cacca4d810cb6c2add3ff5749edb2a2965d89a14a1c818b4d2c60c7`。
本机 619 passed、1 ignored，E2B 全套正确性通过，真实 Rust 三臂各 534 行。
`from_str` ABI 槽 25 → 24，L memmove 1 → 0、Call 32 → 31、节点 5333 → 5324；
机器字节 30318 → 30298、指令 5835 → 5833，mov／显式访存不变。
重编码的 Store 181／If 188／Region 220 未变，因此不重复正式计时。

candidate28 将单独处理调用结果中只作完整传递的 variant 字段：在原 ABI decode 的
活跃 variant guard 内捕获私有表示，合流只选择私有地址，避免嵌套 enum 的拆分与重建。
首版仅直接 Call 的结果，拒绝函数控制环、字段观察、未知消费及不兼容的普通值 Phi。
具体类型和实际 consumer 决定选择，不按 semver 函数名或 Error 类型名特判。

### candidate28：完整子对象的调用边界快照

冻结 SHA-256 为 `336ff13f1bbd5bb6250521471d8ce8fd92fdf793ca746a5b71866030f079e5fb`。
本机独立 checkout 623 passed、1 ignored；E2B Linux 652 passed、1 ignored，
真实 Rust 三臂各 550 行及完整压力／语料差分均通过。冻结包排除了并行 RS3 草稿。

5 个 boundary 字段捕获均实际命中。相对 candidate27，`from_str` L 节点 5324 → 3464、
Load 180 → 115、Store 181 → 116、If 188 → 98、Region 220 → 130、Phi 485 → 275、
Undefined 238 → 118。新增 5 次捕获与 5 次运输 memmove，Call 31 → 41。
机器字节 30298 → 19142、指令 5833 → 3687、mov 3778 → 2477、显式访存 2735 → 1719。
重编码成本已按预期消除，新增 10 个 memmove 均为 8 字节，静态尺寸和为 80 字节。
随后一次固定 22／27／28／LLVM 同轮正式对照测得：22 → 28 的总 `.text`
349165 → 291261 字节（−16.6%），编译总指令 6,479,207,987 → 5,421,562,837（−16.3%）。
parse 99.921 → 95.175 ms，但候选／基线单侧 95% 上界 1.024，尚未证实稳定改善；
req 109.415 → 101.989 ms（−6.79%，上界 0.933）；identifier 103.857 → 110.813 ms
（+6.70%，上界 1.088）。不宣告全负载性能验收，不重复同一计时寻找更好结果。
原始表见 `candidate28-comparison.md`。后续按用户指定 RS3 标量事实计划并行推进。

identifier 回退的 [后续静态归因](lp3-runtime-followup.md) 确认 28 已不同于 23：
`parse::identifier` 从 7892 缩至 5670 字节，外层 wrapper 也改变；varint 和多数 Identifier
分配／复制／比较正文未变。同轮 28 比 27 仅慢 0.19%，不能把相对 22 的累计回退归咎于
28 新增捕获。新增错误路径复制不在本次固定成功输入执行；未保存 28 链接后反汇编，
不以对象布局变化代替动态 profile 证据。

# 唯一语义 IR：实现与验收

日期：2026-11-09。范围仍以 [scope.md](scope.md) 为准，指标不变更。
这里区分已冻结验收和下一候选工作树；单态 S 图不算完整定义级 R/S。

## 最新验收：candidate13；两批迁移已完成

本轮 ObserveValue / Refine 重设计采用[独立契约](../../design/ir/requirements-and-facts.md)
与[分批迁移记录](requirements-migration.md)。candidate12 为第一批冻结结果，源码
SHA-256 `e69122714140141678153b4eb1b2133bb5d9e20b3834990d6f28c4212e1fb970`。
[完整回执](../../../evidence/poc-semantic-only-20261109/candidate12-acceptance.json)
记录 Linux 66 组 / 592 passed / 1 ignored、真实功能三臂、语义 381×3、CLI、
semver 34 项/臂以及 7 套完整 corpus 均通过。

第一批的 `RequireDefined` 无业务结果，保留 R/S 源码 Ignore 值，在表示需求分析前
擦除证明要求。from_str 初始 S 532、优化后 S 845、L 5,216、If 150、Load 180、
Store 80，函数 48,229 B；机器指令 9,314、mov 7,745、条件跳转 150、无条件跳转 142。
其 L 仍有 280 个旧 Refine，不能称整个重设计完成。

同环境 semver 编译中位数 0.64 s、crate text 998,290 B，parse 145.098 ms、
req 340.290 ms、identifier 139.922 ms。完整 codegen 窗口 5,021,183,569 指令，
LLVM 为 1,667,852,731；仍包含 collector/MIR 转换，不能当作排除转换的冻结指标。
LLVM 对应函数 917 B、232 条指令、62 条 mov，parse 2.801 ms。尚远未达到 LLVM 水平。
第一批对 candidate11 有改善，但 parse 仍慢于 candidate10，不能挑选指标宣称全面收益。

第二批 candidate13 已通过集中验收，源码归档 SHA-256：
`5316798289d820c4aa9443043197095f592bcc4e4791544b5e894511071067ca`。
[完整回执](../../../evidence/poc-semantic-only-20261109/candidate13-acceptance.json)
记录 Linux 67 组 / 608 passed / 1 ignored；真实三臂 Assume（包括定义级泛型）、
其他 features、381×3 语义语料、CLI、semver 原版每臂 34 项、7 套完整 corpus 通过。
另有 5 个种子 × 40 个生成函数 × 6 输入，共 1,200 个向量，优化 off/on 分别与 LLVM 对拍通过。

通用 Refine 已删除，改为无机器值的 VariantWitness、独立 Assume 和带作用域的
谓词分析；取消逐叶 pin，真实消费者保留控制与效果。MIR statement／intrinsic call、
Typed Body 与定义级 R 均接入 Assume。结构测试确认冗余 definedness 要求不增加
机器指令，witness 不产生搬运，proof-only 数据不迫使物化。

| 指标 | candidate11（旧协议） | candidate12（要求拆分） | candidate13（凭据拆分） |
|---|---:|---:|---:|
| from_str 初始 S → 优化 S | 532 → 845 | 532 → 845 | 532 → 845 |
| L 节点 | 6,133 | 5,216 | 4,859 |
| L If / Load / Store | 213 / 180 / 80 | 150 / 180 / 80 | 150 / 180 / 80 |
| 机器指令 / mov | 9,947 / 8,055 | 9,314 / 7,745 | 8,810 / 7,241 |
| 函数字节 | 51,116 | 48,229 | 45,111 |
| crate text 字节 | 1,030,574 | 998,290 | 983,122 |
| 编译 wall 中位数 s | 0.64 | 0.64 | 0.67 |
| codegen 编译指令 | 5,223,361,663 | 5,021,183,569 | 5,563,263,785 |
| parse / req / identifier ms | 150.178 / 351.745 / 142.568 | 145.098 / 340.290 / 139.922 | 144.057 / 354.595 / 138.493 |

13 的 S 包含 35 个 witness 与 24 个 RequireDefined；L 不保留这些节点、Assume 或
旧 Refine。实际条件／无条件跳转为 150 / 142；显式内存读／写指令为 1,911 / 2,180。
这些机器计数限于该 ELF 函数，不含 call/ret 等隐式栈访问；rbp-memory mov 3,751
仅是栈帧访问代理，不能称为已证实的 spill 数。

13 对照 LLVM：编译 0.26 s，crate text 18,515 B，codegen 指令 1,667,909,650，
parse / req / identifier 为 2.841 / 2.222 / 9.532 ms；from_str 917 B、232 指令、
62 mov。13 的代码体积和搬运减少，但 codegen 指令比 12 增加 10.8%、req 慢 4.2%，
parse 仍比 10 慢约 2.0%。本轮契约与结构验收完成，不是整体性能或 LLVM 对齐验收。
compile 为同环境 5 轮中位数，runtime 为 7 轮；profile 独立 3 轮，仍包含 collector／
MIR 转换，不能冒充排除转换的冻结指标。

完整 Rust validity、引用 provenance 和权限系统并未实现；不新增空壳 RequireValid。
regex caller-location adapter、f64/atomics 的已知范围缺口仍保留，未宣称本批修复。

## candidate10 历史验收

源码 SHA-256：`e706d171426f1e6bd3010f06fa9df732992bbdc637ca1fae08603953961f771f`。
Linux workspace 571 passed / 1 ignored，generic/nominal/MaybeUninit/emission
三臂功能验证、语义语料 381×3、CLI、semver 原版测试与工作负载均通过。
完整 corpus、walkthrough 与 regex 仍在继续，不宣称全部正确性门槛已完成。

同 sysroot 的 5 轮编译 / 7 轮运行对照：编译中位数 0.65 s，目标 `.text`
1,057,726 B，parse 141.191 ms、req 352.155 ms、identifier 141.970 ms。
对 09 分别降低约 15.6%、6.5%、8.3% 的运行时间。完整 codegen 窗口
kfrc 5,192,455,564 / LLVM 1,668,287,251 指令；此窗口仍包含 collector/MIR 转换，
不能冒充冻结的排除转换指标，也远未达到“编译指令三倍更少”的要求。

`from_str` 优化后 S 1,129→851，L 12,473→7,753，If 607→267，
L Observe 545→31，函数 text 80,781→63,455 B。
Load 258 / Store 158 仍未减少：观察消除减少了重复检查树，ABI enum
编解码与聚合物化仍是下一步重点。candidate11 正在实现结构化身份调用摘要，
消除原样返回参数的调用边界，不增加一般内联预算；收益尚未测量。

## candidate09 验收增量

源码 SHA-256：`d2fe659c4b9071cef684bba28b7bdc75e4fe65f55e2e15caa01072642ea15ab7`。
Linux workspace 554 passed / 1 ignored；真实 W05、`known<T>` 定义级分支消除强断言、
语义语料 381×3、CLI、semver 原版 34 项测试及工作负载已通过。
这不是全范围验收：MaybeUninit 优化开启时出现布局后 Local 分区证据失效，
修复属于下一候选；regex 与完整 W01–W10 仍需继续验证。

与同环境 candidate08a 相比，09 的 semver 对象 `.text` 从 1,142,645 B 降至
1,134,533 B，compile wall 中位数 0.79→0.76 s，parse 170.883→167.356 ms。
这些小幅变化不能解释为达到 LLVM 水平。
`from_str` 的 S 仍为 1,129 节点，L 12,689→12,473，Refine 531→280；
If 607、Load 258、Store 158 均未减少，函数 text 82,658→80,781 B（LLVM 917 B）。
因此已知变体收窄减少了细化节点复制，但主要分支与物化仍未消除。
测量边界、原始记录与其他指标见
[指标状态](../../../evidence/poc-semantic-only-20261109/metrics-status.md)。

candidate10 随后接入 S 级冗余观察消除、指针拆装折叠、最终机器流冗余跳转
删除，以及单次 verifier 内按根类型复用参数查询。观察消除安排在变体前驱拆分之前，
针对 W01 的“构造、合并、再判别”形状；这些不属于本节 candidate09 的性能结果。

## 唯一生产入口

```text
rustc 定义 MIR → eligible R（每定义准备一次）→ 按实例惰性特化 → Specialized
rustc 其他 Instance → Typed Body → build_graph → Specialized
  → verify → 语义优化／同 crate 内联 → verify
  → lower_layout → Layout → verify → x86
```

旧布局 builder、LegacyLayout 和 IR 模式开关已删除。无函数级回退。
`KFRC_OPTIMIZE=0` 仅关闭优化，`KFRC_INLINE=0` 仅关闭内联。
上述 R 入口是 candidate04 之后的工作树改动：明确记录 NotEligible 原因，
未覆盖的构造继续进入同一 semantic S；一旦 R 选中，准备或特化失败直接报错。
目前不宣称所有定义均走 R、答案键去重或跨构建缓存已完成。

## 已冻结 candidate01

来源：[验收回执](../../../evidence/poc-semantic-only-20261109/candidate01-acceptance.json)。
源码归档 SHA-256：`c15a798ff880775d1eb234dccca7ec6f92fcb2d8842e228ad58cdb3afe80c500`。

| 能力 | 当前实现 |
|---|---|
| 值 | owned 聚合、字段更新、enum 活动变体与源码判别值、宽指针、同时更新 Phi；move 不生成拷贝 |
| 内存 | LocalSlot、结构化字段/索引地址、LoadValue/StoreValue 快照、SetVariant、lifetime、raw copy、fixed transmute；只有真实取地址或动态索引的局部需要存储 |
| 布局 | Direct/Pair/Indirect/Cast ABI；动态 direct/niche enum、嵌套 product、enum Phi；按活动变体读写，不读非活动字段 |
| 优化 | 整数规则、聚合/变体折叠、纯值 GVN、常量控制与 DCE；直接相邻 store→load 快照转发；有预算的多块 S 内联 |
| 内联边界 | 保留实参/返回 poison 观察；类型表完整重映射；当前冻结版本不内联含局部存储的函数、不复制 Local 数据对象身份 |
| 验证 | S 与 L 边界严格区分，x86 只接受 L；语义解释器有初始化、存储寿命、readonly 与快照检查，不宣称完整 Rust provenance 模型 |

| 验证 | 实际结果 |
|---|---|
| Linux rustc 1.94.1 backend | 编译通过 |
| workspace | 96 个测试组，837 passed、5 ignored |
| LLVM／semantic-off／semantic-on | 每组 378 行一致；借用、动态索引、动态 enum、niche 正例通过 |
| 范围外 | union／TLS／SIMD 整 crate 拒绝，无成功 rlib |
| 其他链路 | scalar 565 行、5×40 生成函数、Cargo 5 行、4 类真实入口通过 |
| semver | 首次编译在 wrapping_add 整数重组指针处失败；未运行候选 crate 测试，已暂停并静态批修 |
| regex-syntax／W01–W10 | 此候选尚未执行，等待同类指针问题修复 |

三臂样例 S 节点 397→335；L 节点 855→818。但内联后的物化不全下降：
StackSlot 12→13，Store 33→35。减少 S 节点不能替代内存、运行时间和编译指令的验证。

## 下一轮：按 walkthrough 消除不必要的工作

| 依据 | 正在实施的机制 | 要证明省掉什么 |
|---|---|---|
| W01/W03，F18/F38 | 已知变体 Phi 的按前驱拆分，统一 CFG/Phi 编辑 | `?` 构造再判别的分支、错误路径包装，不先生成物理 tag 再恢复语义 |
| W02/W06，F05/F08/F28 | typed-memory 内联、接缝种子、同被调者验证复用 | 每调用整图克隆/验证；内联后留下的实参槽 |
| W03，F17/F19 | enum load 仅用于判别时只读 tag | payload 解码、value slot、无关字段读取 |
| W05/W10，F46/F49/F50/F60 | 共用逃逸/根分析，Local/Const 标注、槽位提升 | 重复分析与无逃逸局部的 load/store；不把 &mut 当不别名证明 |
| W09，F42/F44 | 保留紧凑 Repeat，后续目标槽放置 | 大数组过早逐叶展开、无必要的拷贝与槽 |

当前 Unknown/Local/Const 标签还不等于独立分区效果链。
W06 的每定义一次 R 优化、孔位与递归去重，W08 的缓存/源位置隔离，规则 DSL、
完整 placement/LICM、常量除法与 switch 降低仍待完成。工作量必须同时报告
构建/克隆/访问节点、未生成的槽与拷贝、实际 `instructions:u`，不只报告删节点数。

## 测量边界

共享 MIR-opt=0 std 已在 E2B 构建并通过 sysroot 使用检查。kfrc 阶段有线程级硬件
指令计数窗口，但还需核对边界覆盖。LLVM 的 measureme instructions:u 只计创建
线程，LLVM worker 事件为零；即使 no-parallel-backend 也未解决，因此该测法已拒绝。
新的 LLVM forwarding backend 已通过当前线程、新线程及预存线程计数探针。
相同 semver/shared MIR0 std/O3/16 CGU/ThinLocal 的三次包装编译与原 LLVM
各对象 SHA 完全相同，完整 codegen→join 指令数约 16.68 亿。kfrc 新增同类完整
窗口，包含 collector/转换，单列为保守上界；不把阶段和或该上界冒充冻结的精确边界。
当前没有正式性能达标结论，六项门槛仍按 [metrics.md](metrics.md) 验收。

## 历史切片验收（旧路径删除之前）

E2B candidate02 使用 rustc 1.94.1（LLVM 21.1.8）、8 vCPU / 16016 MiB。
冻结输入归档 SHA-256：
`1093a67110507db948d51f88dd64c10c1566c24fb6e14d7aa2d9b60f497b71fc`。

| 检查 | 结果 |
|---|---|
| 本机 Body/SoN | 79 项通过；布局功能测试 3 项通过 |
| Linux workspace | 94 个测试组，816 passed、5 ignored、0 failed；含新旧机器链路 |
| Rust 四组对拍 | 11 个函数，每组 314 行输出一致；含 Pair/Indirect 双向调用、聚合 Phi/快照、数组、已知 enum、slice 宽指针与空 slice |
| 结构检查 | 每函数 S 输入/优化后 S/L 对应；S 无提前布局操作，L 无残留语义节点；`tuple_field` 优化后无聚合/字段节点且 L 无槽读写 |
| 不支持输入 | borrow、动态 index、动态 enum ABI、niche enum ABI 四类均整 crate 报错，无成功 rlib |
| 旧链 semver 1.0.28 | LLVM/kfrc 两组原版集成测试各 34/34；不计为新语义链的 semver 验收 |

首次布局测试暴露 ABI 参数 snapshot 被原地址替代，已统一修复调用、间接调用、
返回和 Opaque 消费端，并补外部别名修改后返回快照的机器回归。
Linux 旧栈对齐测试还暴露测试自身覆盖真实 callee-saved 列表；实际汇编确认
R13–R15 被遗漏。测试已改为保留分配器结果并检查全部五个保存寄存器，原对齐、
参数和相邻存储断言保留。

四组编译/执行第一次均成功，随后新摘要脚本的跨行正则解析失败；修正解析器后
仅重放已有产物摘要，没有重复执行或隐去原失败记录。对应脚本修正及源码指纹
记录在 [acceptance.json](../../../evidence/poc-semantic-20261109/acceptance.json)。
该回执同时列出原始日志、对象来源、实际 dependency lock 与沙箱清理状态。
正式性能计时未运行，这些正确性和结构结果不构成六项性能主张达标结论。

## candidate02 验收增量

独立快照 SHA-256：`5cad243ba499b5455dd129fe0fc7c0bc12b0e4d651d17e148616a8080a816ea7`。
[回执](../../../evidence/poc-semantic-only-20261109/candidate02-acceptance.json)记录
Linux workspace 98 组、865 passed、5 ignored；三臂 378 行一致。
加入了 typed-memory 内联、局部槽提升、共用内存事实、按前驱拆分、tag-only
读取与逐叶 ObserveValue。S 397→303，L off855/on751。
inline_caller 相比 candidate01 的 L61 降为 L37，额外 Slot1/Load2/Store2 均消除。

semver 越过 wrapping_add 后，在 Formatter::write_fmt 的间接调用处停止：
vtable 方法地址读取被错误记为 usize，候选03 正在改为 typed function pointer
读取，并同时覆盖 virtual drop 与 size/align 元数据。候选02 未完成 semver；
不能把上述样例数字外推为正式 runtime/text/compile-instructions 达标。

## 工作树：旧 IR 移除与新任务

根目录旧 Graph/MIR/手写 frontend/native 编译链的 27 个模块已删除，
机器分配、编码、执行测试工具迁入 `kfrc-x86::machine`。
53 个依赖旧 API 的测试文件已撤下，其行为通过真实 rustc corpus、机器精确编码
测试与现有语义回归迁移；逐项覆盖状态见
` scripts/poc-validation/fixtures/legacy-corpus/migration-status.json `。
其中 pending 尚未补齐，不把测试数量下降当作新链通过。
移除前输入与 candidate03 快照指纹保存在 `evidence/poc-semantic-only-20261109/old-ir-removal-inputs.json`。
当前 CLI `kfrc native` 只调用 pinned rustc semantic backend，旧 native-batch 明确拒绝。

用户新指定的 TraitCall／Hole／VariantSwitch／Refine 已分给独立 generic_semantics
subagent 主责，已落盘定义级 R→惰性答案 S、控制锚定事实及真实 producer。
最小独立 R 解释器直接执行原图与按需 Hole/TraitCall，和 S 差分；不先特化冒充 R。
R 动态内存/间接调用仍明确拒绝。真实泛型三臂验证与完整候选验证尚待完成。

## candidate03/04 进展

candidate03 源码 SHA-256：`c87bdcdbd707f4c37cc06dfd004327766a764698d560d484edef19b6176886f6`。
Linux body/son/x86 专项通过，包含 compact Repeat、大填充/sret 与机器调用保存；
新增真实 dyn method/drop/size/align 后三臂 381 行一致。
semver 1.0.28 的 LLVM/kfrc 原版集成测试各 34/34，公共 workload 一致。
这是新语义链的 semver 验收，不沿用旧链结论。

该快照完整 workspace 被尚未删除的旧测试 include_str(.kfr) 阻塞；corpus230
有 rustc 常量算术 lint 的夹具问题，因此不报 candidate03 workspace/corpus 全通过。
CLI 修正版附件去掉触发范围外 array IntoIter union 的夹具写法后真实通过；附件
与原快照严格区分。完整日志由验证 worker 写入候选回执。

candidate04 源码 SHA-256：`ace2071f7e22ed27844b32da4776cbef9acb38c6232ba3e0a5f5f5c3fa002490`。
旧源码与测试移除、真实 Rust corpus365、power-of-two 除法规则、11 项额外机器
编码回归纳入此包。本机全 workspace 通过；Linux 48 组、453 passed、1 ignored。
三臂语义 381 行、CLI、365 corpus 的 1134 vectors/19 traps、semver 原版各34项通过。
W01/W02/W04 通过；W03/W05 首次暴露 ZST enum 唯一可居住变体丢失，已暂停同类验证。
工作树已批修常量和 Ignore 返回入口，语义 inhabited 元数据不依赖物理 enum 编码。
regex-syntax 仍在内部 MaybeUninit union 处超出已实现范围，不计成功。
四概念实现从这份快照之后的新工作树开始，不混入此候选的验收结论。

同机 semver 诊断显示 candidate04 未达标：目标对象 `.text` 为 LLVM 18,515 B、
kfrc 3,931,990 B；parse/req/identifier 负载约慢126/277/24倍。原始数据在
`evidence/poc-semantic-only-20261109/results/candidate04/semver-performance-02/`。
`Version::from_str` 从优化后 S 1067 节点膨胀到 L 34517 节点，主要重复生成 enum
观察与编码树。工作树正从生成源削减已知变体和空 payload 的分支，尚未用新结果替换旧数据。

## candidate05/06 与四概念接入

candidate05：`4e6fac81db7d288bc9bd88217a086005e19b17bf5affa7f95460f9a894218c00`。
Linux 后端及 486 项测试通过，但真实 std `size_of_val` 包装函数被 R producer
误判为可处理，特化遇到 intrinsic 后报错。该候选不报完整验收通过。

candidate06：`698c56ecd944c3ef44f4fd0a63e45748f167a1e199b7a4652899abaf85b3ed17`。
包含定义级输入资格批修、Ignore ABI 语义观察、特化变化种子、直接 constructor
VariantSwitch 折叠、按真实引用省略内部函数、已知 enum 编码/观察缩减和机器零偏移/复制简化。
Linux 55 组、494 passed、1 ignored；真实语义三臂各 381 行一致。
泛型及发射专项三臂、CLI 通过。泛型日志记录 3 个定义、7 个实例、8 个孔位答案和
4 次分支裁剪；`choose::<()>` 仅回答 SizeAlign(0,1)，未解析死分支的 trait 目标。
`greater` 的 u32/i32/custom 三实例目标不同，输出 true/false/false。
完整 corpus 在循环样例暴露同控制锚的 Rem 先于其新 Refine 操作数调度的问题，
已暂停同类验证，后端正在按依赖排序批修。此候选不报完整通过或性能达标。

R 支持 sized generic 子集。`TraitCall` 用 source signature 表达，`Resolve`
答案是保守的目标实例身份；`SizeAlign`/`NeedsDrop` 按存活控制查询。
`VariantEncoding` 问题类型已定义，但生产布局尚由 S→L recorded oracle 提供，
未完成布局依赖的统一答案键。`VariantSwitch`/`Refine` 已接入真实单态 builder、
验证、解释、优化和布局；最小独立 R 解释支持结构值及按需静态调用，动态内存仍拒绝。
不宣称递归去重或完整 Rust R 已完成。

candidate07 工作树另有 sparse KnownVariant、布局 projection 索引和
[有限 MaybeUninit 支持](maybe-uninit-semantics.md)；这些都不计入 candidate06 的结果。

## candidate07/08a 验收增量

candidate07：`186df0f5b6e476fb93fed0be80b3a8f23b360ee7334aa0ae710e33a0500e8c56`。
Linux 519 项通过；完整 corpus 394 functions / 1295 vectors / 19 traps 全部三臂通过；
语义三臂各381行与CLI通过。MU数组迭代器clone暴露DST字段地址缺口，semver暴露
旧Ordering按显示名识别与稳定类型身份的冲突。两类均保留首次失败证据后批修。

candidate08 因rustc-private IndexVec没有 `.last()` 方法而未通过Linux编译。
candidate08a仅从08冻结归档修这一处为 `.iter().last()`，未混入09的R优化改动。
08a源码SHA：`6e2b2e16b7c388e58d3e1b042913a72d1b330169281022806a0e9a3d81f23f15`。
Linux 62组、543 passed、1 ignored；语义三臂各381行、CLI通过；semver1.0.28
原版LLVM/kfrc两组各34项及公共workload通过。

同sysroot、同机串行诊断已量化04→08a：编译wall中位3.08→0.79秒；目标text
3,931,990→1,142,645字节；parse362.418→170.883ms、req600.099→377.502ms、
identifier229.515→154.477ms。08a对应LLVM2.827/2.306/9.544ms，仍显著未达标。
`from_str` S输入532→优化1129→L12689；L If607、Region619、Phi1238、Observe545、
Load258、Store158。相比04 L节点减少63.2%，但剩余布局展开仍很大；不能据此称LLVM水平。

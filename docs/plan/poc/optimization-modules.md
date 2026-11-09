# 优化模块与任务分工

日期：2026-11-09。本文区分实际代码与后续工作目录；目录规划不代表优化已实现。
当前实际实现与冻结验收以 [semantic-ir-status.md](semantic-ir-status.md) 为准。
semantic 已是 rustc 外壳的唯一生产入口，旧布局 builder 和模式开关已删除；
根目录的早期独立演示 IR 已拆出机器工具后移除；测试迁移仍按逐项清单补齐。

**范围以 [scope.md](scope.md) 为准，六项门槛保持 [metrics.md](metrics.md) 不变。**
当前 Linux candidate01 验收 837 项 workspace 与 378 行三臂差分通过；semver
在指针算术转换处失败，正在批修，不能使用旧链 semver 的结果作为新链通过证据。

## 当前按 walkthrough 的实施分工

| owner | 具体职责 | 文档依据与减少的工作 |
|---|---|---|
| semantic_builder | builder 正确性；control/thread_variants 与回归 | W01/W03：已知变体 Phi 按前驱拆分，避免构造、解码、重复分支 |
| semantic_contract | 公共内存事实、partition、promotion 与回归 | W05/W06/W10：共用逃逸分析，内联后局部槽提升，少生成 load/store |
| semantic_interpreter | verifier 与共享内存差分 oracle；旧 IR 依赖盘点 | W03/W09/W10：活动字段、padding、生命周期、真实别名回调；不得为少工作放宽语义 |
| semantic_layout | S→L、x86 接缝与真实机器回归 | W03 仅 tag 读取；W09 避免 ObserveValue 物化，后续 compact Repeat 与目标槽放置 |
| semantic_optimizations | typed-memory 内联、scalar seam worklist | W02/W06：一次函数事务、被调者检查复用、仅改动接缝传播，单列索引与规则访问成本 |
| semantic_validation | HK E2B、原版 crate/W01–W10、结构/性能证据 | 固定快照；同机 LLVM/off/on；首个错误类别暂停动态、静态批修 |
| root | shell、pipeline、跨 CGU 符号安全、注册与状态文档 | 缓存具体 S 优化源、阶段种子/计数，后续真实 R 定义与答案身份；不把 S 去重叫 R |

下面是完整责任边界和历史规划，不是已完成清单；每轮实际改动继续遵守唯一文件 owner。

## 前一轮模块拆分（历史记录）

原实现的 SoN 优化集中在 `kfrc-son/src/optimize.rs`，只有整数常量折叠和
恒等式化简。机器层的 CFG 活跃性和内存路径分配位于 `kfrc-x86/src/lower/`。
旧的 `src/son_optimize.rs`、`src/optimizer_pipeline.rs` 等仍用于旧 Graph 链路，
并未自动参与 `rustc_codegen_kfrc` 的 FunctionGraph 编译；不得把旧实现的能力
列为当前 Rust crate 已有能力。

前一轮先按职责拆已有实现，保持公开 `optimize_graph` 和 `OptimizeStats` 调用兼容：

```text
rustc_codegen_kfrc            MIR → Body，调用优化入口及 verifier
        ↓
kfrc-son
  build/                     Body → FunctionGraph
  graph.rs                   图存储、类型、基础访问与修改
  analysis/{cfg,dominators,uses}.rs  已提取的公共图分析
  pipeline.rs                优化入口、汇总统计、结束时 compact
  transforms/scalar/
    instcombine.rs           现有整数规则和有界局部迭代
  optimize.rs                旧入口兼容导出
  verify.rs                  图结构/类型/支配检查
        ↓
kfrc-x86
  lower/                     调度、指令选择、ABI、LowerInstr → 机器指令
    memory_liveness.rs       从 LowerInstr 提取定义、使用和控制转移
  analysis/liveness.rs       公共 CFG 数据流与活跃区间，只读分析
  regalloc/memory.rs         内存路径位置分配、spill 与 callee-save 集合
  object_file.rs             对象文件、数据与重定位
```

`pipeline` 不包含折叠规则；transform 不压缩重编号 NodeId。优化入口开始修改前
撤销 `Verified`，调用者仍须在机器降低前重新运行 verifier。拆分当时只运行已有
整数 pass；后续语义切片新增的 pass 见本文开头的状态链接。

机器活跃性分析接受每条指令的 defs/uses/flow；指令选择负责提供这些事实，
分析不依赖私有 LowerInstr。分配器消费活跃区间，机器发射消费位置分配结果。
当前内存分配候选只用五个 SysV callee-saved GPR，保留参数/返回/固定 scratch
寄存器；调用 clobber、安全 spill、栈参数与对齐必须由真实测试验收。
标量路径继续使用现有 `native::compile_virtual_function`，不声称已统一两条发射器。

## LLVM 参考与适用范围

参考固定版本 **LLVM 21.1.8**，对应本项目 rustc 1.94.1。参考的是职责、
作用域和分析失效契约，不复制它的全部 pass 组合：

| LLVM | kfrc 职责 | 说明 |
|---|---|---|
| `lib/Passes/PassBuilderPipelines.cpp` | `pipeline` | 注册执行次序、阶段、种子、预算和统计；规则在 transform 中 |
| `lib/Analysis` | SoN `analysis/`；机器 `analysis/` | CFG、dominators、loops、别名事实、调用图等只读分析 |
| `lib/Transforms/InstCombine`、`Scalar` | `transforms/{scalar,memory,control,loops}/` | 函数内变换按责任分工；目录分类不必照抄 LLVM |
| `lib/Transforms/IPO` | `transforms/ipo/` | 内联、去虚拟化、跨函数属性；需要多函数图提供者 |
| `lib/Transforms/Utils` | `transforms/utils/` | 原子修改 CFG/Phi/effect、克隆和拼接，集中维护不变量 |
| `lib/CodeGen`、`Target/X86` | `kfrc-x86` | 机器活跃性、寄存器分配、帧和指令清理，与 SoN 变换分开 |

LLVM 新 PM 的作用域为 Module → 可选 CGSCC → Function → Loop；pass 返回
`PreservedAnalyses`。kfrc 首版无须实现通用 pass trait、插件注册器或复杂分析
缓存管理器：函数级分析可按需要重算；CFG、效果、调用和 NodeId 变化均应显式
声明，无法证明仍有效的分析一律丢弃。以后有实际缓存需求再细分保留集合。

**R/S/L 是运行阶段，scalar/memory/control/loops/ipo 是变换职责，两者分开。**
同一规则可以在多个阶段使用，不能复制三份优化器。保留设计中的 R 级每定义一次、
S 级变化种子、L 级布局降低和 placement；不照搬 LLVM pre/post-link 反复扫全图。
首版 integer pass 的四轮局部扫描仍在，聚合折叠已使用变化驱动 worklist；
完整 S 特化的变化种子与跨 pass worklist 仍待实现。
内存分析沿用已有 effect 链及后续分区，不为目录相似另外构建重复的 MemorySSA。

官方来源：

- [PassBuilderPipelines.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Passes/PassBuilderPipelines.cpp)
- [Analysis](https://github.com/llvm/llvm-project/tree/llvmorg-21.1.8/llvm/lib/Analysis)、[Transforms](https://github.com/llvm/llvm-project/tree/llvmorg-21.1.8/llvm/lib/Transforms)
- [New Pass Manager：作用域、分析与失效](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/docs/NewPassManager.rst)
- [机器流水线 TargetPassConfig](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/TargetPassConfig.cpp)

21.1.8 的常规目标机器发射仍使用 legacy TargetPassConfig；不得把 SoN 优化和
机器发射混为一套已统一的新 PM。已有研究 [mapping-to-design.md](../../research/llvm-mir/mapping-to-design.md)
的“承接”指设计已覆盖，不能当作实现完成清单。

## 目录与接口状态

下表区分已落盘基础和待实现职责。除明确标为已有的部分，其余路径是待创建或
待扩展的规划，不创建空模块占位。`son/`、`body/`、`x86/`、`shell/` 分别指
`crates/kfrc-son/src/`、`crates/kfrc-body/src/`、`crates/kfrc-x86/src/`、
`crates/rustc_codegen_kfrc/src/`。

| 目录 | 拥有的事实或变换 | 前置条件 |
|---|---|---|
| `son/analysis/{cfg,dominators,uses}.rs` | 已有 CFG、支配关系、uses 基础 | 已从 verifier 提取并注册；不代表编辑器及语义阶段检查已实现 |
| `son/analysis/memory.rs` | 地址根、常量偏移、访问范围、逃逸、alias | 当前只有单一效果链；未知指针/调用必须保守 |
| `son/analysis/{callgraph,call_summary}.rs` | SCC、读写、捕获及发散摘要（PoC 固定 panic=abort） | 函数图提供者、可见实例集合和外部调用契约 |
| `son/transforms/utils/` | 已有 Region/Phi 同步保留前驱；效果节点绕过和克隆映射待实现 | 图修改失效规则和 verifier 契约 |
| `son/transforms/scalar/` | 已有手写整数 InstCombine、聚合/变体折叠和精确纯值 GVN；规则 DSL 待补 | 语义值节点、poison、uses/worklist；现有整数规则不计作 DSL 已交付 |
| `son/transforms/memory/` | 必交付槽位提升、分区内 load 转发；DSE/拷贝消除按负载需要追加 | Unknown/Local/Const 的来源及 effects editor；不能以单一效果链代替分区 |
| `son/transforms/control/` | 已有常量分支裁剪、不可达控制/死值清理；按前驱拆分和完整检查消除待补 | CFG editor；前驱与全部 Phi 同步更新 |
| `son/lower_layout.rs`、`son/lower_layout/` | 已有 product/宽指针与已知变体布局、ABI 快照；动态 enum、switch 降低、常量除法、elaboration placement（含 LICM）待补 | S 图阶段检查、布局答案、支配/循环与效果安全事实 |
| `son/transforms/ipo/` | 必交付 R/S 级内联；dyn 去虚化属于扩展目标 | 定义图/实例提供者、调用摘要/SCC、子图克隆、mentioned-items/预算 |
| `son/specialize/`（待创建） | 孔位表、答案/特化身份键、递归去重、嵌入实例、S 级种子 | 定义级泛型图与 mentioned-items 契约；特化是必交付，不是可选优化 |
| `son/cache/`（待创建） | 内容寻址 R 图/机器码缓存键、源位置隔离 | 稳定定义/答案/目标指纹、内联依赖；build 级存储接入 shell |
| `x86/peephole/`（可选，待创建） | 按负载归因追加机器级冗余 move、地址模式、分支清理 | 不替代必交付语义优化；启用前单独分配 owner 与验收 |
| `x86/frame/`（可选拆分，待创建） | 从既有 lower/regalloc 提取槽位布局职责 | 有实际实现需要时由 D/I 明确交接文件；保持独立 outgoing args、callee-save、对齐 |

等 pipeline 有多个实际运行阶段，再从 `pipeline.rs` 拆目录；不提前造 r/s/l 空文件。

基础编辑接口先由一个 owner 交付：

- `CfgEditor::retain_predecessors(region, keep)` 已实现，先验证再同步编辑该 Region
  的全部 Phi；常量分支及不可达控制裁剪复用它。
- `EffectEditor::bypass(...)` 维护 control/effect/value projections。pass 证明能删，
  editor 保持结构；不能由工具接口偷偷作别名假设。
- `MemoryAnalysis::alias(a, b)` 返回 MustAlias/NoAlias/MayAlias，访问事实包含
  root、offset、size；未知时为 MayAlias。
- 子图克隆返回 NodeId 映射；`compact` 会重编号且现不返回映射，所以由 pipeline
  在阶段结束统一调用，丢弃跨 compact 的节点分析/种子。
- 未来 transform 结果声明 Values/Effects/CFG/Calls/Renumbered 变化及新种子；
  这是一组具体数据，不要求新增动态 pass 框架。

目前 `graph.rs` 已增加 mutation 撤销 `Verified`、`set_inputs`、`remove_node` 和
边解析，CFG/dominance/uses 已提取为公共分析并注册。CfgEditor 已有保留前驱
接口，S/L verifier 已用于首个语义切片；EffectEditor、克隆接口和完整定义级
R verifier 仍待交付。这些基础不代表任意编辑都安全或完整迁移已经完成。

## 当前工作包与唯一文件 owner

下表是待执行分工，不是已完成清单。A–F 对应迁移方案；G–I 承接范围内剩余
交付，不能在完成 A–F 的单态切片后省略。工作包数量不表示同时启动的 agent 数。
同一文件的修改权只有表中一个 owner；跨包需要修改入口时向其 owner 提交接口需求。

| 工作包 | 独占修改范围 | 依赖与可验收产物 |
|---|---|---|
| A 语义契约与图基础 | `body/{body,types,oracle}.rs`、`son/graph.rs`、`son/analysis/{cfg,dominators,uses}.rs`、`son/transforms/utils/` | 冻结语义类型/布局答案、值/地址、聚合/变体/宽指针、孔位和分区公共契约；补 CFG/Phi/effect 编辑与克隆；不能靠 level 枚举冒充阶段 |
| B 语义 builder | `son/build.rs`、`son/build/`、`crates/kfrc-son/tests/semantic_build.rs` | 依赖 A；先贯通 tuple/struct，再 enum/宽指针、借用/drop、字段初始化；无取地址聚合布局前无栈槽/字段 load-store；move 转交值、Phi 保留聚合 |
| C 解释器与 verifier | `body/verify.rs`、`son/verify.rs`、`son/interpret.rs`、`son/interpret/`、`crates/kfrc-son/tests/semantic_verify.rs` | 依赖 A，与 B/D 同步；真聚合/活动变体/字段初始化解释，拒绝阶段非法节点；整数/聚合/enum 的逐 pass 与 R→S 相等检查，不扩为完整三级解释器项目 |
| D 布局降低与 ABI 接缝 | `son/lower_layout/`、`x86/lower.rs`、`x86/lower/`、`x86/object_file.rs`、`crates/kfrc-x86/tests/semantic_layout.rs` | 依赖 A，与 B/C 构成可执行切片；在 L 决定字段偏移、tag/niche、值槽及实参/返回物化；补 switch、常量除法、elaboration placement（含 LICM）；真实 SysV Direct/Pair/Indirect 与只读数据/重定位对拍 |
| E 定义级 R 与 S 特化 | `shell/convert.rs`、`shell/convert/`、`shell/abi.rs`、`shell/abi/`、`son/specialize/`（待创建） | 依赖 A；保留 Param/const/关联类型与 where-clause，定义级构图；孔位答案、特化身份键、递归去重、嵌入实例及 mentioned-items 成套交付；每定义 R 优化恰好一次有计数证据 |
| F 真实验证与测量 | `scripts/poc-validation/`、`crates/rustc_codegen_kfrc/tests/`、`evidence/` | 各切片先解释器与机器差分，再 W01–W10、semver、regex-syntax；HK 控制 E2B，同沙箱串行原版/候选，固定源码/输入/工具链/资源；正式计时与 profile 分开 |
| G 语义优化与调用摘要 | `son/transforms/{scalar,memory,control,ipo}/`、`son/analysis/{memory,callgraph,call_summary}.rs`（后三项待创建）、`crates/kfrc-son/tests/semantic_optimize.rs` | 依赖 A/B/C，R/S 内联另依赖 E；补 DSL 窥孔、GVN、聚合/变体折叠、按前驱拆分、R/S 内联、槽位提升、Unknown/Local/Const 分区内 load 转发、检查消除、DCE、调用摘要（含 SCC）；每个 pass 保留效果/poison/trap 并记录变化种子 |
| H build 级缓存 | `son/cache/`、`shell/cache/`（均待创建）、`crates/kfrc-son/tests/semantic_cache.rs`（待创建） | 依赖 E/G 的定义、答案、内联依赖清单及 D 的机器产物；内容寻址 R 图与机器码缓存，源位置隔离；行号移动命中、注释零重编和函数编辑失效范围实测 |
| I 机器寄存器分配 | `x86/analysis/`、`x86/regalloc/`、`crates/kfrc-x86/tests/semantic_regalloc.rs`（待创建） | 保留现有候选，D 接缝稳定后验收；线性扫描跨调用/固定寄存器/spill/栈参数/对齐正确，真实 ELF 与汇编双路径；需要改 lower 时由 D 执行 |
| 整合者 | 各 crate 的 `lib.rs`/模块注册与 Cargo 配置、`son/pipeline.rs`、`son/optimize.rs`、`shell/lib.rs`、计划文档 | 统一阶段入口、运行顺序/预算/种子/统计与开关；验收前固定快照，记录实际启用 pass；其他包不争改共享入口 |

A 先固定共享契约；B/C/D 贯通单态切片时，E 可以实现同一契约下的定义级入口。
A 的原子编辑与 B/C 的值语义可用后才接 G 的对应 pass；R/S 内联须等 E 的图提供者
和实例身份可用。H 不能用临时 NodeId 或源行号作稳定缓存身份。F 贯穿各切片，
I 的机器候选正确性独立记录，旧 semver 结果不替代候选验收。

若一个工作包再拆给多个 worker，须先细分文件归属并写明交接，不能把表中的一个
目录同时授予两人。`pipeline` 只列真实启用的 pass；发现第一类动态错误后暂停同类
验证，静态排查全部实例并批量修复，再恢复 E2E。

## 完整范围与六项验收

以上分工覆盖 [scope.md](scope.md) 的中后端必交付项，出口仍以
[phases.md](phases.md) 和 [metrics.md](metrics.md) 为准：

| 主张 | 工作包与最终证据 |
|---|---|
| 1 正确 | B–G/I 经 F 跑 W01–W10、semver 和 regex-syntax 原版测试；报告 kfrc 编译覆盖及对象来源，过渡链通过不计完整 R/S 通过 |
| 2 编译工作少 | A–G/I 接通完整链后，由 F 按 metrics 的中后端边界测 `instructions:u`，目标比 LLVM 少 ≥ 3× |
| 3 运行不差 | F 的解析负载同环境差分与原始样本，runtime 比值单侧 95% 上界 ≤ 1.10 |
| 4 体积不差 | D/G/I 机器产物由 F 统计 strip 后目标 crate 的 `.text`，比值 ≤ 1.0 |
| 5 泛型只优化一次 | E/G 的每定义 R 优化计数恰好 1、特化身份去重比不低于 `-Zmono-dedup` |
| 6 增量 | H 的 R 图/机器码缓存和源位置隔离由 F 重放编辑场景；行号移动 100% 命中、注释修改 0 重编，函数编辑仅失效本体及内联调用者 |

Unknown/Local/Const 是首版必交付分区；Frozen/Unique 仅为独立扩展开关。
`dyn` 属扩展目标；SIMD/SSE/AVX、TLS、union、原子等不加入新迁移工作的必交付范围。
已有范围外代码和历史证据保留，但不以继续扩展它们替代范围内的 R/S/L、优化和缓存。
遇到不支持构造须明确报错；选择 LLVM 时按整个 crate 标记，不做函数级静默回退。

## 已有验证与未验证范围

此前模块拆分记录的本机检查：

- `cargo test -p kfrc-son --test optimization --test pointer_comparisons --test thread_local --test pointer_metadata`：12 passed，0 failed/ignored。
- `cargo check -p kfrc-x86`：通过，无 warning。
- `cargo test -p kfrc-x86 --lib`：编译通过，实际 0 tests，仅作编译检查。

随后冻结的 E2B baseline 已完成 semver 每组 34 项、P1 1155 行输出、3 类 panic、
4 项入口检查；234 个结果文件校验通过，沙箱已释放。证据为
[`paused-summary.json`](../../../evidence/poc-perf-20261109/paused-summary.json)。
其中 `candidate_started=false`、`performance_measured=false` 描述的是此前暂停时
的运行，不是后续语义切片的当前状态。后续候选与证据另存
`evidence/poc-semantic-20261109/`，不覆盖这份旧基线回执。

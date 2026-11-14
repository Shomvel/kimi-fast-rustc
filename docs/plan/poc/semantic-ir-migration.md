# 语义 IR 调整与迁移顺序

2026-11-09：暂停原内存/控制流/内联优化任务，先修正 Rust 语义表示与布局降低接缝。
[optimization-modules.md](optimization-modules.md) 的职责分类继续适用，原工作包执行顺序
由本文替代。本文定义完整迁移目标；已落盘的首个 S→L 切片及验收另见
[semantic-ir-status.md](semantic-ir-status.md)，不代表完整多态 R/S/L 已交付。

## 范围与完成标准

[scope.md](scope.md) 是本次纠正的范围依据：前端和标准库借用 rustc，
**中后端按设计完整实现**。下面两步是施工顺序，两步都必须完成；单态语义值图
只用于打通接缝，不能代替定义级 R、孔位答案和 S 特化，也不能作为 PoC 出口。
[phases.md](phases.md) 的 P0–P4 出口和 [metrics.md](metrics.md) 的六项门槛保持不变。

| 必交付范围 | 迁移后的落点 | 完成证据 |
|---|---|---|
| R/S/L 与值语义 | 定义级 R、答案驱动 S、显式 `lower_layout`；同一个 FunctionGraph 的阶段契约 | 逐阶段 verifier，聚合/enum 小解释器逐 pass 对拍，R→S 相等检查 |
| 内存与 drop | `Unknown`、`Local`、`Const` 分区；固定内存效果、真实借用、drop 顺序 | 分区来源及存活期证据；别名、未知调用、部分初始化与析构的真实执行差分 |
| R/S 优化 | 规则 DSL 窥孔、GVN、聚合与变体折叠、按前驱拆分、R/S 内联、槽位提升、分区内 load 转发、检查消除、DCE | 实际启用的 pass、变化种子和统计；规则前后行为及错误路径一致 |
| L 与后端 | value slot、原地构造、PassMode、switch 降低、常量除法、elaboration 式 placement（含 LICM）、线性扫描、只读数据与重定位 | 真实 x86-64 SysV 混链、参数/返回与栈对齐对拍；布局前后结构 dump |
| 特化 | 孔位表、特化身份键、递归去重、嵌入实例、mentioned items、调用摘要 | 每定义 R 优化恰好一次；递归/SCC、诊断保留及符号来源证据；与 `-Zmono-dedup` 对照 |
| 缓存 | 内容寻址 R 图和机器码缓存、源位置隔离 | 行号移动、注释修改和单函数编辑场景；冷/热缓存分别测量 |
| 真实负载与指标 | W01–W10、semver、regex-syntax；同一份未优化 MIR std | crate 测试与六项主张逐项报告；编译指令数、runtime、体积均按既定协议 |

`Frozen`/`Unique`、一般 `dyn Trait` 和 toml 保持扩展目标。TLS、SIMD/SSE/AVX、
union、浮点、i128、原子、unwind 等按 scope 的排除项处理；已有范围外实现与
历史证据保留，但不为迁移继续扩展，也不把它们列为语义 IR 的前置条件。
设计 semantics 文档中的 union 字节值等更广范围不自动扩大本 PoC。
小解释器服务于规定的整数、聚合、enum 检查，不要求完整三级语义解释器。

不支持的构造明确报错；需要 LLVM 时，显式选择整个 crate 并标记覆盖率。
过渡期间旧链只作为单独选择、记录来源的对照后端，不在新链中按函数或节点
静默回退。范围内能力暂未实现仍记为待交付，不能借回退宣称达成阶段出口。

## 已确认的差距

以下是迁移前旧链的历史差距（该布局 builder 现已删除）。Typed Body 已保留字段投影、
聚合构造、enum、move/copy 和引用，但旧 SoN 构图时：

- `build.rs::memory_locals` 将 tuple、array、ADT 及大于 8 字节的局部统一判为内存；
  builder 在入口分配栈槽、复制聚合参数。
- `build/aggregate.rs` 按字段偏移逐项 store；`build/memory.rs` 用聚合类型的
  `PtrOffset(address, 0)` 伪装聚合值；值和存储地址没有严格区分。
- `build/enum_value.rs` 已展开 direct/niche 编码；`build/calls.rs` 为非标量实参
  再分配快照槽。优化必须重建原本可直接保留的聚合与变体关系。
- rustc converter 按具体 `Instance` 实例化类型；`GraphLevel` 尚未由真实阶段
  转换和 verifier 限制支撑。只有 R/S/L 枚举不足以证明语义分层。
- 当前解释器的非标量值主要按地址表示，不能独立验证语义聚合值的快照与别名行为。

因此，仅继续优化现有 load/store 链不能兑现 Rust 专用 IR 的主要设计收益。
现有布局/ABI、对象生成及真实 crate 验收仍是可复用的迁移基准。

## 必须先固定的契约

1. **值与地址分开。** `Rust(T)` 始终表示 T 的值，指向 T 的存储必须具有指针类型。
   R/S 的聚合、enum、宽指针是值；大小不决定是否提前落栈。真实取地址及明确需要
   地址的操作才使用存储。借用局部首版可整函数预判，后续再做流敏感物化。
2. **语义类型与布局事实分开。** 语义视图提供逻辑字段/变体/泛型参数/函数签名；
   布局答案提供 size/align、字段偏移、tag/niche、PassMode/Cast lanes。
   旧 RecordedTypes 可作过渡 adapter，但不能让新的语义 builder 任意读布局。
3. **布局降低有真实入口。** `lower_layout` 才决定值槽、物理字段、enum 编码、
   参数/返回值 materialize。低级图应具有明确的值位置表示，不能再让 aggregate
   TyId 暗示“这个 NodeId 是地址”。保留同一个 FunctionGraph 的设计，阶段合法性
   由 verifier 约束，不为了命名复制三套 IR。
4. **真内存仍有顺序。** Load/Store、未知调用、drop、volatile/atomic 保留控制与
   效果；首版 Unknown/Const/不逃逸 Local 的事实必须有来源。`&T` 不能自动等同
   不变内存，`&mut T` 不能代替重借用、UnsafeCell、Unpin 和激活写的证明。
5. **布局前能解释。** 解释器应能直接执行聚合/变体/宽指针值；初始化状态按字段
   保留，不能把一个未初始化字段扩大为整值 poison，也不能读取非活动变体。
6. **真实 R/S。** 多态定义图、孔位/答案、S 特化、实例身份和诊断清单成套交付。
   具体化图不能仅改名为 R 并宣称“每定义优化一次”。

## 两步迁移，不混淆完成范围

### 第一步：单态输入的语义值图 → 显式布局降低

这是过渡路径；输入暂时仍来自具体 Instance，不宣称完整多态 R/S：

```text
当前具体 MIR → Typed Body → 单态语义值图
                            ↓ 聚合/变体层的局部化简
                         lower_layout
                            ↓
                   低级布局图 → 既有 x86/ABI/对象链
```

先贯通 tuple/struct 的完整构造、Field、InsertField、聚合 Phi、参数和返回值；
接着覆盖 MakeVariant、VariantOf/VariantField、VariantSwitch、宽指针，再处理
借用/drop 与未初始化的边界。每个切片都贯通 builder、解释器、verifier、lowering
和真实机器对拍，避免只增加不可执行节点。

对应结构性验收：`Field(Aggregate(x,y),0)` 直接变成 x；无取地址的简单聚合
在布局前不产生栈槽和字段 load/store；已知变体匹配无需解码 tag/niche；实际 ABI
边界仍与 rustc 一致。真实借用、动态索引或尚未迁移的操作必须显式表示/拒绝或
由调用方显式为整个 crate 选择 LLVM 进行对照，不允许语义节点悄悄再退化成
聚合类型的指针，也不允许新链逐函数回退。

### 第二步：定义级 R → 孔位答案 → S 具体化

普通 Item 的 `instance_mir` 本来可取得按定义查询的 MIR；目前 converter 的
逐处实例化才丢失了泛型。新增定义级转换使用 body 的 typing environment，
保留 where-clause；语义签名不预先依赖物理 FnAbi。

需要一起落地 Param/const 参数/关联类型表示、Hole/Answer/Resolve、S 阶段填充
与变化种子，以及答案依赖指纹。普通函数按真实 MIR body 身份缓存；closure 等
不同 body 不能因共用 typeck root 而合并，shim 按 InstanceKind/生成契约区分。
对上游函数尊重 MIR availability 与 should_codegen_locally。

继续复用 rustc mono collector/CGU 和 required-const/mentioned-item 诊断契约。
控制流剪枝后再决定真实链接引用，诊断依赖不能全部变成必须发射的符号引用。
R 无布局编码，S 无 Param/未回答的语义孔位，L 无残留聚合/enum 语义操作；
L 才读取的布局答案仍须登记并纳入身份键，不能绕过答案依赖。verifier
逐阶段检查。此时才能测“R 每定义一次”、泛型特化去重及编译侧收益。

两步贯通后继续交付上表中的完整优化、内存分区和缓存。S→L 接通不代表
优化或 P3 出口已经完成；semver 正确性通过也不能代替主张 2–4 的实测。

## 重新分工与依赖

| 工作包 | 先交付的内容 | 依赖与验收 |
|---|---|---|
| A 语义类型与节点 | 语义类型/布局答案接口，聚合/变体/宽指针节点、值地址契约 | 先冻结共享数据契约；旧 ABI 实现保持可复用 |
| B 语义 builder | 去掉因聚合类型/大小统一落槽；Field/InsertField、move 值转发、聚合 Phi | 依赖 A；先覆盖单态 tuple/struct 完整切片 |
| C 解释器与 verifier | 真聚合/变体值、字段初始化、活动变体、阶段合法性 | 与 A/B 同步；布局前也能验证语义 |
| D 布局降低 | 物理聚合、tag/niche、值槽、ABI 参数/返回物化，接既有 x86 | 依赖 A；与 B/C 完整贯通，对照旧链和 LLVM |
| E 多态 R/S | 定义级转换、孔位/答案、S 特化/实例身份/mentioned-items | 先冻结与 A 的接口；不挤进单态切片冒称完成 |
| F 验证 | 旧链基准、语义前后解释器、lowering 后真机、semver及性能 | 每切片先正确性，再计时；快照与输入指纹完整 |
| 整合者 | 接口、注册、阶段开关、文档与依赖协调 | 共享 graph/types/verifier 文件唯一 owner，避免并发争改 |

A 的公共类型/图存储与 C 的 verifier 修改须按文件再分配，不能两个 worker 同时
拥有同一入口。机器寄存器分配和已有优化模块拆分保留为候选；等语义→布局接缝
稳定后继续验收，不作为这次语义迁移的前置优化成果。

## 不可忽略的边界

部分字段初始化与部分 move、SetDiscriminant、drop 次数/顺序、MaybeUninit、
padding、transmute、动态索引、地址可见的内存快照及未知调用要分别列边界项；
涉及 union 或其他范围外构造时，验收明确拒绝/整 crate 选择，而非新增支持。
“聚合是值”不允许强制把未初始化字节解释为合法字段值；NeedsDrop=false 才能
删除对应 drop，不能因值已无用途而提前析构。

## 接手时的历史状态（2026-11-09）

B/C/D/E 的新优化 pass 尚未落盘；A 已写公共 CFG/uses/dominators 及基础 mutation
失效重构，保留并补齐模块注册/导入，使暂停后的工作树可编译。编辑器和内联未交付。
baseline E2B 已完成 semver 34/34、P1 1155 行及三类 panic、入口 4 项；234 文件
下载校验通过，沙箱已释放。未运行候选或性能计时，不能产生新的优化收益结论。
证据见 `evidence/poc-perf-20261109/paused-summary.json`。

接手后已由 A/B/C/D 共同实现 tuple/struct 值语义切片，并交付 G 的首批聚合、
GVN 和控制优化；实际状态见 [semantic-ir-status.md](semantic-ir-status.md)。
定义级 Body、孔位与语义签名仍待 E 交付。文件归属及后续全部工作包见
[optimization-modules.md](optimization-modules.md)。

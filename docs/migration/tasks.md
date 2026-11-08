# 任务依赖与派工清单

状态：全部 `planned`。本文件的 T 编号是派工权威；F/SB 编号用于指向工作流说明，不是另一套进度。M0–M6 的出口见 [milestones.md](milestones.md)，共享接口见 [architecture.md](architecture.md)。以下目标路径是规划落点，不表示本 PR 已创建相应实现。

## 派工规则

- 任务开始前所有硬依赖必须为 `verified`。仅需要提前阅读或准备测试的关系不是硬依赖，不得据此提前宣布实现完成。
- 负责人角色：A 语法/解析；B 语义/类型/Body；C SoN/效果；D ABI/机器码/对象；E driver/构建/验证；I 集成。每个任务只设一位最终负责者，协作者由该负责人协调。
- 状态只有 `planned → in_progress → verified`，阻塞时设 `blocked` 并记录缺口。运行中的内部任务状态机与这里的项目状态不同。
- 契约先于依赖实现冻结；T02 不要求后期 borrowck、runtime 或 sysroot 已实现。后续发现契约不足时先更新契约和受影响任务，不能形成“等待未来实现才能定义现在接口”的循环。
- T00–T14 可直接拆为首轮实现 PR。T15 以后的任务是有明确出口的阶段包：开始前必须把各类型/语义/平台能力拆成子项 `Txx.a`、`Txx.b` 等，列出来源、范围、测试和依赖，并更新此表；大包未全部验收不得整体标 `verified`。
- 每项证据沿用 [验证协议](validation.md#证据包与执行环境)：源码与工具指纹、命令/配置、原始输出、产物与失败记录。未支持合法输入记覆盖缺口，无效输入被正确拒绝才记拒绝测试通过。
- 每次派工先核对[逐文件迁移清单](migration-inventory.md)的调用者、测试去向和退役条件。下文的模块任务允许先完成有明确契约的层内验证；完整源行为的验收在其组合出口落实。层内 `verified` 不能代替 M1–M6 的真实执行出口。

## 首轮：冻结与纵向链

<a id="t00"></a>
### T00 — 固定版本、来源和能力清单

- **阶段 / owner / 硬依赖 / 状态：** M0 / I / 无 / `planned`。
- **工作流：** [来源选择](rustc-source-map.md#m0-来源选择的完成条件)。
- **范围：** 构建 manifest、`docs/migration/sources/`、支持能力矩阵；固定候选源码、官方 rustc 源码/oracle/sysroot、目标项目 lockfile、edition、target 和配置。
- **交付：** 可从独立环境取得的输入；本地派生 rustc 与官方实现差异单列；许可和首批移植来源模板。
- **验收证据：** 版本查询、源归档/二进制摘要、目标项目要求与所选版本的符合性；不得以本地 commit 冒充官方 tag。

<a id="t01"></a>
### T01 — 从 clean 源码复现已知错编译

- **阶段 / owner / 硬依赖 / 状态：** M0 / E / T00 / `planned`。
- **工作流：** [永久控制流回归](validation.md#首个永久回归有状态-if-表达式)。
- **范围：** 冻结构建、回归 fixture、独立驱动和失败证据；不直接信任旧 debug binary。
- **交付：** `check(false)` 样例及同类 if/match/短路/loop 静态风险清单；区分 old binary 观察、clean 复现和后续修复。
- **验收证据：** 参考/候选完整命令、ELF 哈希和真实执行输出；若 clean 不复现，保存差异定位，不预设失败结果。

<a id="t02"></a>
### T02 — 冻结共享数据与阶段契约

- **阶段 / owner / 硬依赖 / 状态：** M0 / I / T00 / `planned`。
- **工作流：** [共享身份与接口](architecture.md#4-共享身份与接口契约)、[产物和发布](workstreams/frontend-semantics.md#产物和发布契约)。
- **范围：** IDs、类型驻留、Place/Typed Body、实例身份、TargetSpec/FnAbi、SoN 值/控制/效果、产物发布与错误分类。
- **交付：** 字段级契约和最小表示样例；各 owner 对输入/输出及依赖的共同确认。尚未支持的类型和效果必须可显式表示为拒绝。
- **验收证据：** 同一例子在 AST→Body→实例→SoN→Machine 的身份/类型映射；没有相互竞争的 TypeId 或函数编号。

<a id="t03"></a>
### T03 — 实现源级差分与证据 runner

- **阶段 / owner / 硬依赖 / 状态：** M0 / E / T00, T01 / `planned`。
- **工作流：** [差分执行协议](validation.md#差分执行协议)。
- **范围：** `tests/` fixture 与拟建 `tools/` runner；支持程序接受、明确拒绝、真实 x86 执行、结果比较和证据保存。
- **交付：** 一个成功样例、一个拒绝样例和 T01 已知失败的可重复记录；候选命令可替换，C driver 与目标对象来源分开。
- **验收证据：** 干净目录重放成功；故意错误结果会被 runner 判失败；超时、跳过、unsupported 不计成功。

<a id="t04"></a>
### T04 — lexer、AST 与标量函数语法迁移

- **阶段 / owner / 硬依赖 / 状态：** M1 / A / T02, T03 / `planned`。
- **工作流：** [F1](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** `src/syntax/`、source/span/token tree；按选定 rustc 的字面量、优先级、函数/块语法规则适配。
- **交付：** AST 与来源映射；支持范围外的语法清晰拒绝；不直接写 SoN 或在解析器中保存 SSA。
- **验收证据：** 语法正反例、边界 literal、作用域结构及来源；移植文件/许可/测试来源可追溯。

<a id="t05"></a>
### T05 — 定义身份、作用域与简单模块解析

- **阶段 / owner / 硬依赖 / 状态：** M1 / A / T04 / `planned`。
- **工作流：** [F1](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** `src/resolve/`；定义登记、显式路径、词法作用域、前向/递归声明、可见性。
- **交付：** ResolvedInterface、依赖清单与稳定 DefKey；先声明后检查函数体。
- **验收证据：** 多模块、遮蔽、递归、重复/未定义/不可见名称的正反例；完成顺序变化不改变定义身份。

<a id="t06"></a>
### T06 — 标量类型检查与 Typed Body/CFG

- **阶段 / owner / 硬依赖 / 状态：** M1 / B / T05 / `planned`。
- **工作流：** [Body 与 SoN 交接](workstreams/frontend-semantics.md#typed-body-与-son-交接)。
- **范围：** `src/semantics/`、`src/body/`；标量、函数调用、局部变量、赋值、分支/循环/return。
- **交付：** 有类型 LocalId/Place/CFG、初始化检查、求值顺序与源映射；旧 `src/mir.rs` 不充当已完成的新 Body。
- **验收证据：** 不合法赋值/返回/调用/初始化被拒绝；同一变量在所有控制路径上可追踪；Body verifier 及支持范围表。

<a id="t07"></a>
### T07 — SoN 表示与 verifier

- **阶段 / owner / 硬依赖 / 状态：** M1 / C / T02, T03 / `planned`。
- **工作流：** [SB-01](workstreams/son-backend.md#sb-01锁定表示与-verifier)。
- **范围：** `src/son/` 的节点类型、控制/值/效果通道、图身份、Phi 与循环封闭规则。
- **交付：** 新 verifier 与可重放图 dump；旧标量图显式适配；预留能力不能被误识为已实现内存。
- **验收证据：** 错误类型、跨图引用、Phi/前驱不对应、支配/效果错误被拒绝；合法循环通过。

<a id="t08"></a>
### T08 — 标量 ABI 与机器接口接入

- **阶段 / owner / 硬依赖 / 状态：** M1 / D / T02, T03 / `planned`。
- **工作流：** [SB-04](workstreams/son-backend.md#sb-04machine-irabi-与线性扫描)。
- **范围：** 当前 `native.rs`、`x86.rs`、`regalloc.rs`、`native_program.rs` 的标量调用入口；规划 `src/machine/`。
- **交付：** 明确 FnAbi、寄存器约束、栈参数/返回与 ObjectFragment 接口；接受新图所需的窄适配。
- **验收证据：** C ABI 驱动的标量调用、递归、跨调用活跃值、高压力 spill 真实执行；保留线性扫描且无 ILP 调度器。

<a id="t09"></a>
### T09 — Typed Body 到 SoN 的统一控制降低

- **阶段 / owner / 硬依赖 / 状态：** M1 / C / T06, T07, T08 / `planned`。
- **工作流：** [SB-02](workstreams/son-backend.md#sb-02从-typed-body-构建值与控制图)。
- **范围：** `src/son/build.rs`；CFG 前驱驱动的值/控制合并、多回边、break/continue、不可达块。
- **交付：** 唯一生产 Body→SoN 入口；从语法各分支移除重复的 SSA 构造职责。
- **验收证据：** T01 错误类与嵌套赋值/退出组合在图层及 T03 runner/T08 机器入口的真实执行均正确；该窄驱动不依赖 T10 的产品 CLI，仅图层通过不能标 verified。

<a id="t10"></a>
### T10 — 新 driver 串行纵向链

- **阶段 / owner / 硬依赖 / 状态：** M1 / E / T09 / `planned`。
- **工作流：** [现状与迁移位置](workstreams/build-pipeline.md#现状与迁移位置)。
- **范围：** 新 `.rs` 入口与阶段连接；串行调度、对象、链接、诊断/来源清单。
- **交付：** 接入 T09 builder 的完整 CLI 纵向 runner；新链与旧 `.kfr` 入口显式区分，出错不 fallback。
- **验收证据：** 使用已完成的 builder 实际跑过源码→目标对象→执行；目标对象无 rustc 生产者；诊断、来源和退出状态可经公共 CLI 取得。

<a id="t11"></a>
### T11 — 有界任务、预算和取消最小实现

- **阶段 / owner / 硬依赖 / 状态：** M1 / E / T07, T08, T10 / `planned`。
- **工作流：** [资源与确定性](workstreams/build-pipeline.md#cpu内存与确定性)、[SB-08](workstreams/son-backend.md#sb-08并行缓存与资源预算接入)。
- **范围：** driver/scheduler/pipeline；函数或语义组任务，初始无持久缓存。
- **交付：** 不可变产物发布、CPU/jobserver 预算、字节背压、fail-fast 和等待者唤醒；单图独占修改。
- **验收证据：** jobs=1/2/4 结果/对象一致；循环依赖、队列满、上游失败、取消有限时收束；串行参考仍可用。

<a id="t12"></a>
### T12 — trait/关联类型最小移植实验

- **阶段 / owner / 硬依赖 / 状态：** M1 / B / T02, T03, T06 / `planned`。
- **工作流：** [有约束泛型实验](workstreams/frontend-semantics.md#有约束泛型与关联类型)。
- **范围：** 独立可运行语义实验，固定 impl、泛型约束、关联类型规范化与拒绝集合。
- **交付：** 自有推导状态/goal 的最小实现、实际 rustc 依赖闭包、保留/替换清单与后续估算；不宣称完整 trait 支持。
- **验收证据：** 正例、缺少实现、冲突/歧义和循环边界与 oracle 一致；没有永久携带 TyCtxt/query 运行时。

<a id="t13"></a>
### T13 — move、借用与分支 drop 最小移植实验

- **阶段 / owner / 硬依赖 / 状态：** M1 / B / T02, T03, T06 / `planned`。
- **工作流：** [借用/drop 实验](workstreams/frontend-semantics.md#move借用与分支-drop)。
- **范围：** Place、move path、loan/region、分支初始化与析构 CFG 的小型事实模型。
- **交付：** 代表性合法/非法案例、状态与 cleanup dump、依赖成本报告；未支持路径明确拒绝。
- **验收证据：** 独立分析原型对 move 后使用、借用冲突的拒绝分类与 oracle 一致；分支和提前退出的析构 CFG 可检查，依赖成本报告可重放。本任务的 `verified` 只认证分析实验；析构的真实执行由 T22/T27/T26 组合验收，不在首轮宣称该生产能力已完成。

<a id="t14"></a>
### T14 — M1 集成出口与迁移估算

- **阶段 / owner / 硬依赖 / 状态：** M1 / I / T09, T10, T11, T12, T13 / `planned`。
- **工作流：** [M1 出口](milestones.md#m1贯通自有生产链)。
- **范围：** 首轮支持矩阵、串行/并行链、接口变更与剩余风险。
- **交付：** 普通 Rust/简单多模块完整链证据；移植实验报告；M2/M3 子项与资源估算。
- **验收证据：** 控制流错误类已在真实机器对拍；有效/无效输入分类正确；不存在为扩展语法而绕过统一 Body 的新入口。

## M2：类型与内存闭包

<a id="t15"></a>
### T15 — 类型、Place 与目标布局扩展

- **阶段 / owner / 硬依赖 / 状态：** M2 / B / T14 / `planned`。
- **工作流：** [F2](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** 整数宽度、浮点、tuple/struct/enum、array/slice、指针/引用、repr/layout；每类拆子项。
- **交付：** 自有类型/布局与 Place 投影，目标 ABI 所需分类输入；明确稳定语言契约与内部布局约定。
- **验收证据：** 大小/对齐/投影/有效值的正反例；不以读取 padding 或未指定布局作为差分 oracle。

<a id="t16"></a>
### T16 — M2 安全子集的初始化、move 与借用检查

- **阶段 / owner / 硬依赖 / 状态：** M2 / B / T13, T15 / `planned`。
- **工作流：** [F2](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** 已接受的引用/聚合语义，受限借用、move 和初始化分析。
- **交付：** M2 CheckedBody；复杂生命周期未实现时明确拒绝，不能等 M3 才检查已接受程序。
- **验收证据：** 别名可变借用、move 后使用、分支未初始化等拒绝用例与 oracle 对照。

<a id="t17"></a>
### T17 — SoN 类型与内存效果

- **阶段 / owner / 硬依赖 / 状态：** M2 / C / T15, T16 / `planned`。
- **工作流：** [SB-03](workstreams/son-backend.md#sb-03类型内存与效果降低)。
- **范围：** 通用 Load/Store、地址计算、静态数据、间接调用、整数扩展/截断及效果合并。
- **交付：** 保守效果链与调用未知效果；内存/类型 verifier；不启用未证明的别名优化。
- **验收证据：** 用固定的带类型 Body/图 fixture 验证别名写后读、循环与调用效果、检查失败路径及对应图不变量；记录 oracle 的预期观察。此处不等待尚未完成的 T18 机器支持；真实机器语义差分必须在 T18/T19/T20 的组合出口完成，不能把图检查当成执行证明。

<a id="t18"></a>
### T18 — 完整 M2 ABI 与机器寄存器类别

- **阶段 / owner / 硬依赖 / 状态：** M2 / D / T15, T17 / `planned`。
- **工作流：** [SB-04](workstreams/son-backend.md#sb-04machine-irabi-与线性扫描)。
- **范围：** 聚合/浮点/不同宽度参数返回、寄存器类别、栈、间接调用和分配约束。
- **交付：** 按目标调用约定拆分的 ABI 实现、Machine IR 与线性扫描扩展；固定寄存器/flags/clobber 不变量。
- **验收证据：** 双向 C FFI、递归、栈参数、高压力/跨调用活跃值真实执行；不能用 i64 ABI 代替所有类型。

<a id="t19"></a>
### T19 — 数据对象、符号与基本多 crate 链接

- **阶段 / owner / 硬依赖 / 状态：** M2 / D / T17, T18 / `planned`。
- **工作流：** [SB-06](workstreams/son-backend.md#sb-06对象实例符号与跨-crate)。
- **范围：** `.text`/数据/只读数据、可见性、必要重定位、稳定符号；此时是使用明确声明与 C ABI 的独立对象链接 fixture，不要求完整 Cargo metadata，也不宣称 Rust 跨 crate metadata 兼容。
- **交付：** ObjectFragment 合并与基础跨编译单元调用/常量；未知 relocation 明确拒绝。
- **验收证据：** 系统链接器检查和真实执行；线程完成顺序不改变对象；重复/缺失符号正确失败。

<a id="t20"></a>
### T20 — M2 能力出口

- **阶段 / owner / 硬依赖 / 状态：** M2 / I / T16, T17, T18, T19 / `planned`。
- **工作流：** [M2 出口](milestones.md#m2类型内存布局和-abi)。
- **范围：** 类型/内存/ABI/跨函数与受控跨 crate 组合。
- **交付：** 支持/拒绝/缺口矩阵、组合差分证据与 M3 细分清单。
- **验收证据：** 受支持安全程序正确、无效程序被拒绝；未把 spill 指令当通用内存或局部类型例当全部 Rust。

## M3：完整语义能力逐项推进

<a id="t21"></a>
### T21 — 泛型、trait 与实例约束

- **阶段 / owner / 硬依赖 / 状态：** M3 / B / T12, T20 / `planned`。
- **工作流：** [F3a](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** substitution、推导事务、obligations、关联类型、方法查找、coherence、动态分发；按能力细拆。
- **交付：** 自有 solver/类型环境，canonical goal 与循环/歧义规则；来源依赖清单。
- **验收证据：** 约束成功/失败、关联类型规范化、冲突 impl、对象安全与递归边界对照；不一律接受循环。

<a id="t22"></a>
### T22 — 生命周期、move/borrow 与 drop 显式化

- **阶段 / owner / 硬依赖 / 状态：** M3 / B / T13, T20 / `planned`。
- **工作流：** [F3b](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** loan/region、重借用、部分 move、初始化、drop flags、closure 捕获及异常 cleanup；逐项子包。
- **交付：** CheckedBody 与显式正常/异常析构控制，拒绝规则覆盖表。
- **验收证据：** 有效/无效程序 oracle 一致；Body 上可证明各退出路径析构意图，实际 runtime 组合由 T27/T26 后续出口落实。

<a id="t23"></a>
### T23 — 声明式宏、hygiene 与展开解析迭代

- **阶段 / owner / 硬依赖 / 状态：** M3 / A / T20 / `planned`。
- **工作流：** [F3c](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** token tree 匹配/转录、fragment/重复、macro_rules、可见性、cfg/属性、edition 与递归限制。
- **交付：** 带 hygiene 的定义/展开产物、依赖就绪协议；每类宏语义拆子项。
- **验收证据：** 捕获/遮蔽、跨模块、嵌套与重复、edition 差异正反例；卫生身份不随诊断 span 被删除。

<a id="t24"></a>
### T24 — 常量解释、内存与有效性

- **阶段 / owner / 硬依赖 / 状态：** M3 / B / T15, T20, T21 / `planned`。
- **工作流：** [F3d](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** 自有 const interpreter、目标内存/指针、Body 求值、有效性与必要 const generic。
- **交付：** 纯整数→目标内存→所需内建项的子包；常量依赖循环与资源限制诊断。
- **验收证据：** 有效常量与运行时对照、溢出/越界/无效值/循环拒绝；不以当前字符串求值器替代完整计划。

<a id="t25"></a>
### T25 — 单态实例、可达性、vtable 与 drop glue

- **阶段 / owner / 硬依赖 / 状态：** M3 / B / T21, T22, T24 / `planned`。
- **工作流：** [单态化来源](rustc-source-map.md#迁移方式)、[SB-06](workstreams/son-backend.md#sb-06对象实例符号与跨-crate)。
- **范围：** `src/instantiate/`；InstanceKey、可达图、递归先声明后定义、静态 allocation 引用。
- **交付：** 具体 Body、调用目标与依赖摘要；泛型实例身份与符号/缓存一致。
- **验收证据：** 多实例、递归、trait 动态派发、drop glue 引用确定性；相同源码换并发顺序不产生不同实例集合。

<a id="t26"></a>
### T26 — M3 语义组合出口

- **阶段 / owner / 硬依赖 / 状态：** M3 / I / T21, T22, T23, T24, T25, T27 / `planned`。
- **工作流：** [M3 出口](milestones.md#m3rust-语义闭包)。
- **范围：** 自包含泛型集合/迭代器、trait、所有权、常量和析构组合；不依赖完整 alloc/std 已编译。
- **交付：** 需求→实现→差分矩阵，支持范围和 sysroot 阻塞表。
- **验收证据：** 受支持失败/析构路径真实运行；无效输入拒绝；缺失 async/内建机制等不被测试通过数掩盖。

<a id="t27"></a>
### T27 — runtime、异常与并发机器能力

- **阶段 / owner / 硬依赖 / 状态：** M3–M4 / D / T19, T22, T25 / `planned`。
- **工作流：** [SB-05](workstreams/son-backend.md#sb-05droppanicunwind-与-runtime-操作)。
- **范围：** panic abort/unwind、展开表/personality、drop cleanup、atomics/volatile/fence、TLS、intrinsic/target features。
- **交付：** 按冻结 runtime/sysroot 需求细拆的实现与显式拒绝表；可独立测试的 runtime fixture。
- **验收证据：** 与 C/独立驱动链接的异常/析构/同步观察、栈展开与目标特性；测试 driver 的外部辅助对象单列，不因此宣称 full-native sysroot。

## M4：Cargo 和 sysroot

<a id="t28"></a>
### T28 — Cargo 编译器协议

- **阶段 / owner / 硬依赖 / 状态：** M4 / E / T10, T11, T20 / `planned`。
- **工作流：** [Cargo 协议](workstreams/build-pipeline.md#cargohosttarget-与-sysroot)。
- **范围：** `-vV`/`--print`、cfg、emit、crate 类型、extern、dep-info、JSON 诊断/产物、单层 wrapper 或编译器入口。
- **交付：** 普通 Cargo 可调用的新 driver；未知语义参数明确拒绝，测试/构建不会强制改 panic。
- **验收证据：** 最小无跨 crate 依赖 workspace 的查询、编译、错误、重复构建；不存在双 wrapper 嵌套失败。初始 fixture 使用冻结的无 runtime 输入或明确标记的 bootstrap sysroot，不提前假设 T29 metadata 或 T31–T33 自有 sysroot 已完成，也不据此宣称目标闭包全 native。

<a id="t29"></a>
### T29 — 自有 metadata/rlib 与跨 crate 实例

- **阶段 / owner / 硬依赖 / 状态：** M4 / E / T19, T25, T28 / `planned`。
- **工作流：** [产物与任务依赖](workstreams/build-pipeline.md#产物与任务依赖)。
- **范围：** 自有格式/schema、定义/类型/泛型体与依赖身份、rlib 包装、目标与版本校验。
- **交付：** 两个以上冻结 fixture crate 的跨 crate 泛型/符号链；生产者清单。这里验证自有 metadata 协议，runtime-free fixture 或显式 bootstrap 的来源单列；真实依赖库及自有 sysroot 组合由 T31–T34 验收。
- **验收证据：** 旧 schema/错误目标/缺失依赖拒绝、接口和泛型体变更正确失效；不假装兼容官方 rmeta。

<a id="t30"></a>
### T30 — host 构建脚本与过程宏

- **阶段 / owner / 硬依赖 / 状态：** M4 / E / T23, T28, T29 / `planned`。
- **工作流：** [host/target 与 sysroot](workstreams/build-pipeline.md#cargohosttarget-与-sysroot)。
- **范围：** host 图、过程宏协议/runner、build script 输出与动态输入、host target 区分。
- **交付：** 自有协议及明确标记的 bootstrap 方案；环境/文件输入跟踪，不可跟踪任务不跨构建缓存。
- **验收证据：** host/target 不混链、宏输入/环境变化不错误命中；不得假设官方 proc-macro dylib 能直接加载。

<a id="t31"></a>
### T31 — core 与 compiler builtins

- **阶段 / owner / 硬依赖 / 状态：** M4 / B / T26, T27, T29 / `planned`。
- **工作流：** [F4](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** 冻结 core 的实际语言/属性/lang item/intrinsic 清单及 builtins，阻塞特性先拆子项。
- **交付：** 自有目标代码生成的 core/builtins 与来源清单；必要 sysroot 编译入口。
- **验收证据：** core 代表功能与原生依赖库运行；所有目标 Rust 对象生产者可追踪，官方预编译对象不得混入。

<a id="t32"></a>
### T32 — alloc 与分配器闭环

- **阶段 / owner / 硬依赖 / 状态：** M4 / B / T31 / `planned`。
- **工作流：** [F4](workstreams/frontend-semantics.md#逐批语义闭包)。
- **范围：** 冻结 alloc、分配器 runtime、集合/字符串/Box 及相关特性缺口。
- **交付：** alloc 原生对象和分配/释放链；在真实实现上验证泛型、布局、drop。
- **验收证据：** Vec/String/Box 等实际库测试、分配失败与析构路径；不能用自写小集合代替 alloc 验收。

<a id="t33"></a>
### T33 — std 与平台依赖闭环

- **阶段 / owner / 硬依赖 / 状态：** M4 / B / T27, T30, T32 / `planned`。
- **工作流：** [sysroot](workstreams/build-pipeline.md#cargohosttarget-与-sysroot)。
- **范围：** 冻结 std、平台接口、线程/TLS、IO、panic runtime、test harness 需求；按阻塞能力细拆。
- **交付：** 可用于目标项目的自有 sysroot、Rust/C 库来源明细。
- **验收证据：** 代表 std/test 功能、正常与失败路径；早期无法运行 test harness 时只报告局部 fixture，不能宣布完整套件通过。

<a id="t34"></a>
### T34 — M4 工具链出口

- **阶段 / owner / 硬依赖 / 状态：** M4 / I / T28, T29, T30, T31, T32, T33 / `planned`。
- **工作流：** [M4 出口](milestones.md#m4cargometadata-与-sysroot)。
- **范围：** clean Cargo workspace、真实依赖库、sysroot 与 host/target 来源审计。
- **交付：** “混合 bootstrap / 目标 Rust 全 native / host 也 native”三个明确标签的能力与证据清单。
- **验收证据：** 至少目标 Rust 闭包全 native；全链功能与失败恢复测试，不借旧 cache/target 伪造覆盖。

## M5–M6：真实项目与产品质量

<a id="t35"></a>
### T35 — ripgrep 依赖闭包与功能验收

- **阶段 / owner / 硬依赖 / 状态：** M5 / I / T34 / `planned`。
- **工作流：** [F5](workstreams/frontend-semantics.md#逐批语义闭包)、[M5](milestones.md#m5真实项目正确性)。
- **范围：** 冻结 lockfile/features 的各依赖 crate，按实际阻塞项补齐 SIMD、target features、宏及 intrinsic。
- **交付：** 子任务级依赖与特性销项清单、完整原生 rg 和功能测试。
- **验收证据：** 搜索/匹配/输入规模组合的 stdout、stderr、退出码和文件行为对拍；不改业务源码绕过缺失能力。

<a id="t36"></a>
### T36 — Cargo 及 native 依赖功能验收

- **阶段 / owner / 硬依赖 / 状态：** M5 / I / T35 / `planned`。
- **工作流：** [M5](milestones.md#m5真实项目正确性)。
- **范围：** 冻结 Cargo 及 C/C++ 库构建、host 工具和系统接口；每类阻塞先细拆。
- **交付：** 原生 Cargo、完整来源账本、代表实际工作集；保留 native 工具链独立成本。
- **验收证据：** 构建/解析/依赖操作及正式功能测试通过；不能仅用 metadata --no-deps 验收。

<a id="t37"></a>
### T37 — M5 正确性冻结与质量差距分类

- **阶段 / owner / 硬依赖 / 状态：** M5 / I / T35, T36 / `planned`。
- **工作流：** [阶段出口](validation.md#阶段出口矩阵)。
- **范围：** 功能矩阵、无 fallback 证明、代码质量/编译瓶颈初测。
- **交付：** 可重放产品输入和性能工作负载，按机器码/优化/前端/调度/链接分类差距。
- **验收证据：** 所有要求对象来源明确、语义与配置等价；性能尚未达标时仍保持未完成产品状态。

<a id="t38"></a>
### T38 — SoN 优化与机器码质量达标

- **阶段 / owner / 硬依赖 / 状态：** M6 / C / T37 / `planned`。
- **工作流：** [SB-07](workstreams/son-backend.md#sb-07优化规则迁移与机器码质量)。
- **范围：** 依据 profile 拆内联/标量替换/内存与循环优化/向量化/指令选择；基础纯值规则早期可在所属任务实现。
- **交付：** 每个 pass 的语义前提、预算、终止条件与独立关闭手段；跨 crate 优化如需要采用自有摘要/图。
- **验收证据：** pass 启停均正确、关键 workload runtime/size 数据；不任意浮点重排、不引入 ILP 调度器或 LLVM 目标生成。

<a id="t39"></a>
### T39 — 完整依赖缓存与增量复用

- **阶段 / owner / 硬依赖 / 状态：** M6 / E / T29, T30, T37 / `planned`。
- **工作流：** [缓存与增量](workstreams/build-pipeline.md#缓存与增量)、[SB-08](workstreams/son-backend.md#sb-08并行缓存与资源预算接入)。
- **范围：** 完整对象键→经验证 Body/实例/图逐层缓存；初期可提前做键设计和不发布的实验。
- **交付：** namespace/schema、依赖解释、失效矩阵、原子发布与损坏恢复。
- **验收证据：** 禁用/cold/warm 结果一致，泛型体/trait impl/hygiene/target/panic/overflow 变更失效；build script 输入不明则不缓存。

<a id="t40"></a>
### T40 — 跨 crate 提前发布与关键路径调度

- **阶段 / owner / 硬依赖 / 状态：** M6 / E / T11, T29, T37 / `planned`。
- **工作流：** [产物与任务依赖](workstreams/build-pipeline.md#产物与任务依赖)、[资源与确定性](workstreams/build-pipeline.md#cpu内存与确定性)。
- **范围：** 接口就绪/对象就绪通知、世代与依赖校验、jobserver token、布局/泛型体等真实消费条件。
- **交付：** 可独立关闭的协议和必要 Cargo 变更；正确性优先于签名早发带来的理想并行度。若实验无收益，保存可重复的拒绝采用结论和关闭配置，也可完成本优化决策任务；T41 测量实际采用的配置，不强制启用失败实验。
- **验收证据：** 取消/崩溃/旧产物/循环等待/内存压力验证；无稳定收益时可保留 crate 内调度，本优化不强行开启。

<a id="t41"></a>
### T41 — 公平产品比较与统计证据

- **阶段 / owner / 硬依赖 / 状态：** M6 / E / T38, T39, T40 / `planned`。
- **工作流：** [性能比较](validation.md#性能比较契约)。
- **范围：** cold/no-op/叶子修改/接口修改/miss/hit，portable release 与等约束最佳配置；每个代表 workload。
- **交付：** 原始多轮采样、置信区间、等 strip 文件/可加载节大小、CPU/内存/阶段成本、host/target 指纹。
- **验收证据：** runtime 与体积候选比 ≤1.0 的既定目标，统计证据不足不宣称通过；编译收益可重复，不用平均数隐藏回归。

<a id="t42"></a>
### T42 — 产品出口与下一阶段决策

- **阶段 / owner / 硬依赖 / 状态：** M6 / I / T37, T41 / `planned`。
- **工作流：** [M6 出口](milestones.md#m6机器码质量与编译性能)、[停止恢复条件](validation.md#停止和恢复条件)。
- **范围：** 最终功能/质量/速度与范围核对。
- **交付：** 可重放发布候选证据，未达标项和后续计划；不把此任务等同于自动合并或发布授权。
- **验收证据：** 所有原始目标逐项满足；若性能或兼容性仍有缺口，状态保持 blocked/in_progress，保留已有成果与失败证据。

## 工作流覆盖与关键路径

| 工作流 | 统一任务 |
|---|---|
| F0/F1 与初始语义链 | T00–T06、T09、T10、T14 |
| 语义移植实验 | T12、T13 |
| F2 | T15、T16、T20 |
| F3a/F3b/F3c/F3d | T21、T22、T23、T24 |
| 实例化、F4、F5 | T25、T31–T36 |
| SB-01/SB-02/SB-03 | T07、T09、T17 |
| SB-04/SB-05/SB-06 | T08/T18、T27、T19/T29 |
| SB-07/SB-08 | T38；T11/T39/T40 |
| 验证与阶段出口 | T01/T03、T14、T20、T26、T34、T37、T41/T42 |

首轮关键链为 `T00 → T01 → T03 → T04 → T05 → T06 → T09 → T10 → T11 → T14`，共享契约 T02 与 SoN/机器接口 T07/T08 必须在合并点前就绪。T09 使用 T03/T08 的窄驱动先取得真实执行证据，T10 再接公共 CLI，避免相互等待验收。T12、T13 在 T06 完成后可与后端/调度任务并行，均会影响 T14 出口。

初始只有 T00 无硬依赖；可以并行收集资料和准备样例，但不能为维持五个活跃 agent 而越过依赖。T02/T03 稳定后五个职责流才有足够独立实现任务。后期共享语义模型和 sysroot 形成关键路径，多 agent 数量不能直接换算工期。

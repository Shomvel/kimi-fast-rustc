# M1：贯通自有编译链

**目标：** 普通 Rust 源文件完整走过新链：AST → 名称解析 → 类型检查 → Typed Body → SoN → 现有 native 基础 → ELF → 真实执行。同时用两个小实验测出 trait 和借用/drop 的移植成本。

**依赖：** M0；共享契约（T02）可用。

**出口：**

- 一组普通 `.rs` 文件和简单多模块程序经过新链运行，与 rustc 一致：分支、循环、赋值、递归调用、整数边界。
- if 错编译这一类问题有真实机器回归。
- 无效程序被拒绝；新链与旧 `.kfr` 入口显式区分。

**通过后仍不代表：** 支持完整 Rust 或能编译大型 crate。

**之后的决策：** 根据 T12/T13 的成本报告决定 M3 的移植粒度。如果最小 trait 或借用例子就得搬入大半 `TyCtxt`/query/arena，先改自有数据模型或缩小语义闭包，而不是继续加语法。

---

<a id="t04"></a>
## T04 lexer、AST 与标量函数语法

`M1 · A · 依赖：T02, T03`

细化：[m1/t04-syntax.md](m1/t04-syntax.md)

- **做什么：** `src/syntax/`：token tree、AST、HIR 级节点 ID。`Span` 现在就带 `SyntaxContext`，edition 按 span 查询，token tree 保留不可见分组和 spacing——这些晚加会改遍下游。解析器只产出 AST。
- **词法：** 依赖 `rustc_lexer`（决策 D1），token cooking 自写。1.94.1 没有 token 级的 `-Zunpretty`，token 对拍需要另做 oracle（例如打印 `TokenStream` 的过程宏）。
- **布局：** token 与 AST 节点用 u32 下标、struct-of-arrays，span 放旁表；只建一棵 HIR 级 AST，脱糖原地完成（[overview](../design/frontend/overview.md#crate-内的阶段与冻结点)）。
- **完成标志：** 语法正反例、边界字面量、作用域结构和源位置；移植来源、许可、测试来源有记录。

<a id="t05"></a>
## T05 定义身份、作用域与简单模块

`M1 · A · 依赖：T04`

细化：[m1/t05-resolve.md](m1/t05-resolve.md)

- **做什么：** `src/resolve/`：定义登记、模块图、导入不动点（含 glob）、晚期 rib 规则、可见性；查找从 M1 起就返回三态（已确定/失败/待定），以便 T23 接入宏展开。同时产出签名级生命周期（省略、early/late-bound）。
- **完成标志：** 多模块、遮蔽、递归、重复/未定义/不可见名称的正反例；完成顺序变化不改变 DefKey。

<a id="t06"></a>
## T06 标量类型检查与 Typed Body

`M1 · B · 依赖：T05`

细化：[m1/t06-typeck-and-body.md](m1/t06-typeck-and-body.md)

- **做什么：** 两步。T06.a 类型检查产出 TypeckResults：全部整数宽度（含 `i32` 字面量 fallback）、`!`、推导表与快照、rustc 的阶段顺序、默认 Deny 的溢出 lint。T06.b 用 TypeckResults 构建 [Typed Body](../design/frontend/body.md)：作用域栈与 DropTree、`StorageDead`、`Assert` 的 unwind 边、region scope tree；即使 `panic=abort` 也建出 cleanup 边。
- **模式检查：** 模式穷尽与可反驳性检查（E0004/E0005）在 M1 的 `match` 就需要；先冻结模式 IR（对应 rustc THIR 级的 `Pat`），依赖 `rustc_pattern_analysis`（决策 D1），kfrc 提供它要求的类型上下文接口。
- **完成标志：** 无推导变量泄漏；Body 通过 verifier；正反例的接受/拒绝和错误码与 rustc 一致。

<a id="t07"></a>
## T07 SoN 表示与 verifier

`M1 · C · 依赖：T02, T03`

细化：[m1/t07-son-verifier.md](m1/t07-son-verifier.md)

- **做什么：** `src/son/{graph,types,verify}.rs`：节点结果和输入的类型约束；控制、值、效果三类输入与多结果投影；图内引用与跨实例引用分开；循环“未封闭 → 已封闭”状态。旧 `src/son.rs` 在调用者迁移前保留兼容入口，旧标量图通过显式适配器构造。
- **Rust 语义 SoN：** `GraphLevel` 和按分区的效果字段从第一版起就有；M1 只有单态标量，R/S 级重合（[levels.md](../design/ir/levels.md)）。
- **语义：** 与 verifier 一起写 [semantics.md](../design/ir/semantics.md) 的四类节点登记；R/S 级解释器骨架（整数、控制、`Phi`）在本任务开始。
- **数据结构：** 新图直接用 16 字节 `NodeRec` + 列存 + 转发表（[graph.md](../design/ir/graph.md#节点)）。现有 `son.rs` 的 `replace_uses` 每次扫全图（64k 节点时慢约 2,400 倍）和整图循环到不动点，可以先于新契约单独修（[current-gap](../research/data-structures/current-gap.md)）。
- **规则 DSL 首版：** 自写的小规则 DSL 与生成器（规则语法、类型检查、优先级、overlap 检查、生成匹配代码）；把现有 `son_optimize.rs` 的常量折叠与代数化简改写成规则表。SMT 导出放到 T38。
- **完成标志：** 故意破坏的 Phi 前驱、跨图引用、类型、支配、效果依赖都被拒绝；合法循环/多前驱图通过；同一输入的图 dump 稳定。

<a id="t08"></a>
## T08 标量 ABI 与机器接口

`M1 · D · 依赖：T02, T03`

细化：[m1/t08-scalar-abi.md](m1/t08-scalar-abi.md)

- **做什么：** 从 `native.rs`、`x86.rs`、`regalloc.rs`、`native_program.rs` 抽出标量调用入口，定义 FnAbi、寄存器约束、栈参数/返回和 ObjectFragment 接口（规划 `src/machine/`），做接入新图所需的最小适配。
- **T08.b 最小 panic runtime 与只读数据：** M1 的溢出、除零、越界失败路径要调用 panic lang item（`core::panicking::panic_const::*`），并传 `&'static Location`；Location 含指向文件名的指针，所以 M1 就需要 `.rodata` 和数据重定位。本任务提供：
  - 只读数据片段的发射与 `R_X86_64_64`/`PC32` 重定位（T19 再扩展到完整数据对象）；
  - 一个隔离的最小 runtime：实现 M1 用到的 panic 入口，按 rustc 默认格式向 stderr 打印 `thread 'main' panicked at <file>:<line>:<col>:` 和消息，以退出码 101 结束（对没有 drop 的 M1 程序，这与 rustc 默认的 `panic=unwind` 可观察行为一致）；T27 用真正的 runtime 取代它。
  - 覆盖标签：用到该 runtime 的用例标 `panic-stub`（总览红线 4），不计入 full-native 覆盖；M1 只支持这一种 panic 配置，需要真正 unwind 的输入明确拒绝。
- **完成标志：** 经 C ABI driver 真实执行：标量调用、递归、跨调用活跃值、高压力 spill。

<a id="t09"></a>
## T09 Typed Body → SoN 的统一控制降低

`M1 · C · 依赖：T06, T07, T08`

细化：[m1/t09-body-to-son.md](m1/t09-body-to-son.md)

- **做什么：** `src/son/build.rs`，唯一的生产入口。按 CFG 前驱统一构造值和效果合并，处理 if/match/短路/loop/return、多回边、break/continue、不可达块；变量按 LocalId 识别。不再在各语法分支里各自拼 Phi。
- **Rust 语义 SoN：** builder 产出 R 级图；M1 程序无泛型，`specialize` 是恒等。被取地址的局部变成 `StackSlot`，其余是 SSA 值（[values-and-types.md](../design/ir/values-and-types.md#什么是值什么在内存里)）。
- **解释器差分接入：** 用 T07 的解释器骨架对 builder 产出的图和每个 pass 前后的图做逐 pass 差分，不一致即报 pass 名和规则序号（[semantics](../design/ir/semantics.md#解释器的用途)）。
- **完成标志：** T01 错误类及嵌套循环、遮蔽、多出口、分支赋值组合，经 T03 runner + T08 机器入口真实执行与 rustc 一致。一个测试能同时导出 Body、SoN 和机器码。只过图层不算 verified。

<a id="t10"></a>
## T10 新 driver 串行纵向链

`M1 · E · 依赖：T09`

细化：[m1/t10-new-driver.md](m1/t10-new-driver.md)

- **做什么：** 新 `.rs` 入口串起各阶段：串行调度、对象、链接、诊断和来源清单。
- **完成标志：** 通过公共 CLI 跑通 源码 → 对象 → 执行；目标对象没有 rustc 生产者；诊断、来源和退出码可从 CLI 取得。

<a id="t11"></a>
## T11 有界任务、预算和取消

`M1 · E · 依赖：T07, T08, T10`

细化：[m1/t11-tasks-budget.md](m1/t11-tasks-budget.md)

- **做什么：** 函数/语义组级任务（暂无持久缓存）：不可变产物发布、CPU/jobserver 预算、字节背压、fail-fast、唤醒等待者；单图独占修改，图之间并行。设计见 [build-pipeline](../design/build/pipeline.md)。
- **前端并行：** 冻结共享表后按 typeck root 并行检查函数体，worker 数固定、切分确定（[overview](../design/frontend/overview.md#crate-内的阶段与冻结点)）；M1 先做串行和 2 worker 两种配置，验证结果一致。
- **完成标志：** jobs=1/2/4 结果和对象一致；循环依赖、队列满、上游失败、取消都在有限时间内收束；串行参考路径仍可用。

<a id="t12"></a>
## T12 trait：复用 next-solver

`M1 · B · 依赖：T02, T03, T06`

细化：[m1/t12-trait-experiment.md](m1/t12-trait-experiment.md)

- **注意：** rustc 1.94.1 的默认结果来自旧求解器，新求解器只用于 coherence。算法参考新求解器，oracle 两种模式都跑，结果不一致的用例记为 `solver-divergence`。
- **输入闭包：** 一个 trait、一个关联类型、两个候选 impl、一个带 `where` 约束的泛型函数及跨函数调用。先记录 rustc 对“有效 / 无 impl / 歧义或不满足约束”三类程序的结果。
- **做什么：** 依赖 `rustc_type_ir` + `rustc_next_trait_solver`（`=0.148.0`），为 kfrc 的驻留表实现 `Interner` 和推导上下文接口；推导变量的驻留与回滚接到 T06 的推导表。高阶约束、opaque type、specialization 等未覆盖项列出。
- **性能闸门：** rg `check` 在 `-Znext-solver=globally` 下指令数是默认模式的 1.15 倍；kfrc 的 trait 求解指令数要与 rustc 默认模式对比，超出时记录原因（缓存、规范化路径）。
- **完成标志：** 正例、缺 impl、冲突/歧义、循环边界与 oracle 一致；没有携带 TyCtxt/query；`Interner` 实现覆盖用例所需的方法，并给出 M3 的剩余方法清单与估算。

<a id="t13"></a>
## T13 实验：move、借用与分支 drop

`M1 · B · 依赖：T02, T03, T06`

细化：[m1/t13-ownership-borrow-experiment.md](m1/t13-ownership-borrow-experiment.md)

分两个子项：**T13.a** move/初始化/drop 显式化（先做）；**T13.b** loan 与最小 NLL（loan 失效点由 region 决定，绕不开 region 推导）。

- **输入闭包：** 结构体字段读取、可变借用与重借用、分支部分 move、提前返回，以及一次有副作用的析构。
- **做什么：** 在 Body 上建立 Place/move path、初始化状态和 loan 事实，再做分析和 drop 显式化（显式析构 CFG）。
- **性能重点：** 约束生成与 region 求解（借用检查中 dataflow 只占 4.5%）；不复制 Body，region 放旁表。
- **完成标志：** move 后使用、借用冲突的拒绝分类与 oracle 一致；分支和提前退出的析构 CFG 可检查；依赖成本报告可重放。析构的真实执行由 T22/T27/T26 验收。在 parser 里直接插析构算实验失败。

<a id="t14"></a>
## T14 M1 出口与迁移估算

`M1 · I · 依赖：T09, T10, T11, T12, T13`

细化：[m1/t14-m1-exit.md](m1/t14-m1-exit.md)

- **做什么：** 汇总首轮支持矩阵、串行/并行链、接口变更、剩余风险；把 M2/M3 拆成子项并估算资源。
- **覆盖标签：** 支持矩阵分“可表示 / 可检查 / 可执行”三列（例如 M1 的 `i128`）；`panic-stub` 用例单列。
- **完成标志：** 满足本阶段出口；没有为扩展语法而绕过统一 Body 的入口。

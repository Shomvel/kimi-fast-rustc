# rustc 实现来源与迁移映射

状态：`planned`，调研与迁移计划。本 PR 不复制 rustc 源码，也不声称已有完整兼容层。与前端相关的工作闭包见 [前端与语义迁移](workstreams/frontend-semantics.md)。

## 版本来源必须分清

本次检查了一个 `src/version = 1.94.1` 的本地 rustc 派生树。其 HEAD 为 `acea19e8bf9280a1bc7fa606d3b7670d0fa7d734`，提交标题是 P2b metadata 流水线修复，该树没有配置 Git remote。它包含本地修改，不能当成官方 1.94.1，也不能给这个本地 commit 拼接官方 GitHub 链接。

通过 GitHub Git Data API 核实，官方 `1.94.1` annotated tag 对象为 `4c540d92e07b57f8a961905e3c391feaa0a8cc95`，指向官方 commit [`e408947bfd200af42db322daf0fadfe7e26d3bd1`](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1)。下面的公开链接固定到这个官方 commit，作为路径导航；本次详细实现阅读来自本地派生树。开始实际移植前必须比较相应文件差异，不将本地修改误归因于官方实现。

**产品将采用的参考版本尚未选定。** 历史 benchmark 使用的 1.99.1 工具链不意味着该版本源码已验证，也不意味着应混用它和 1.94.1 的内部类型。M0 要按目标项目的 MSRV、edition、sysroot 和可复现工具链选择一个精确官方源码 commit，同时固定 oracle 的 `rustc -Vv`、源码归档摘要和适用本地补丁。

## 迁移方式

“可借算法”表示可在较小语义闭包内复用规则和经过核实的实现，不表示复制后无需验证。“重写适配”表示保留语义，但改成自有 ID/type/Body/任务模型；不是重新发明 Rust 规则。高度耦合的模块优先做依赖实验，再确定移植范围。

| 语义领域 | 已检查的参考位置与官方导航 | 主要耦合 | 迁移方式与自有产物 |
|---|---|---|---|
| 词法 | [rustc_lexer](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_lexer/src)：`lib.rs`、`cursor.rs` | Unicode、literal 规则、token 身份 | 可借扫描和字面量规则；适配自有 token/source 结构，edition 语义在正确层处理 |
| 语法与模式 | [rustc_parse](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_parse/src/parser)：`expr.rs`、`item.rs`、`pat.rs`、`ty.rs` | AST、session、feature gating、错误恢复 | 按语法闭包移植到自有 AST；不把 `rustc_session` 和诊断系统整体搬入 |
| 宏与 hygiene | [rustc_expand](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src)、[hygiene.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs) | token tree、SyntaxContext、resolver、host 过程宏协议 | 重写调度和存储，保留展开/卫生语义；输出带展开来源及 hygiene 的 token/定义 |
| 名称与可见性 | [rustc_resolve](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src)：`def_collector.rs`、`macros.rs`、`effective_visibilities.rs` | 宏展开反馈、多个命名空间、导入迭代、DefId | 复用作用域/解析规则，适配稳定定义身份和依赖产物；不把解析当成语法后的单次全局扫描 |
| 类型关系与推导 | [rustc_infer](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src)、[rustc_hir_typeck](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src) | TyCtxt、interned Ty、HIR、region、推导事务和 obligations | 强耦合，先移植标量/函数关系及 snapshot/rollback 概念，再迁移复杂 coercion 和方法解析 |
| trait 与 coherence | [rustc_next_trait_solver](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src)：`canonical/`、`solve/search_graph.rs`、`coherence.rs`；[rustc_hir_analysis](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src) | 参数环境、universes、canonicalization、缓存及循环规则 | 先做有约束泛型闭包；后续扩展必须对应 oracle 行为。不能把“发现递归就成功”当作一般求解策略 |
| Body 构建与模式降低 | [rustc_mir_build](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder)：`expr/as_place.rs`、`matches/`、`scope.rs` | typed AST/THIR、Place、临时值生命周期、清理作用域 | 借鉴求值顺序和生命周期规则，生成自有 Typed Body/CFG；不采用 MIR 文本作为运行时协议 |
| move、借用和生命周期 | [rustc_borrowck](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src)：`nll.rs`、`dataflow.rs`、`type_check/`、`region_infer/` | MIR locations、Place、move data、region/type 推导 | 强耦合；先冻结自有事实模型，再移植分析算法和拒绝规则。当前测试语言的变量可变性检查不能替代 borrowck |
| drop 显式化 | [elaborate_drops.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs) | 初始化数据流、move path、drop flags、清理 CFG | 把正常/早退/失败路径改写为显式 Body 操作，再进入 SoN；不可只在正常 return 前插析构 |
| 常量解释与有效性 | [rustc_const_eval](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src)：`interpret/eval_context.rs`、`memory.rs`、`machine.rs` | 目标布局、allocation、pointer/provenance、Body、类型有效性 | 强耦合，构建自有解释状态和限制；先纯整数，再目标内存和受支持内建项。不是复用现有字符串递归求值就完成 |
| 类型布局与调用 ABI | [rustc_abi/layout.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs)、[rustc_target/callconv](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv) | target spec、Ty、niche、PassMode、有效值约束 | 布局算法和目标调用约定优先借鉴，适配自有 Layout/Abi；逐项验证 repr、聚合参数和返回，不能按字段大小猜 |
| 单态化与可达性 | [rustc_monomorphize/collector.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs) | Instance、泛型参数、drop glue、vtable、静态 allocation、跨 crate 定义 | 复用依赖发现规则，改成自有实例任务图；先声明后定义，稳定身份不依赖完成顺序 |
| Body→后端 | [rustc_codegen_ssa/mir](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir)、[traits/builder.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/traits/builder.rs) | ABI、内建函数、内存有效性、异常、backend builder | 参考语义 lowering 的覆盖清单，直接构造自有 SoN；不为挂接 rustc 实现整套 backend traits |

上表是迁移责任地图，不代表上述官方源码在本轮全部经过逐行分析。移植任务必须补充其实际依赖范围、具体函数和对应回归用例；不得仅引用目录就声称算法已正确移植。

## 每次移植的来源记录

首次代码迁移时增加仓库内来源清单（建议 `docs/migration/sources/`），一个记录对应一个可审阅的语义闭包。至少包含：

1. 官方 repository URL、tag（若有）、精确 commit、源文件、原始内容摘要；若采用派生实现，单列基线和补丁。
2. 采用方式：代码复制后修改、算法移植、仅行为参考；列出本地目标文件与负责模块。
3. 保留的不变量、主动变更、没有移植的行为及显式拒绝范围。
4. 原始许可和 notice 依据、保留位置；代码和复制测试分别记录。
5. 来源测试 ID/路径、自己的差分用例、oracle 版本/配置、原始运行结果。
6. 上游更新影响：数据格式、语义变更、既有本地差异、需要重新运行的验收集合。

没有这些来源字段的改动可以作为调研草稿，不能标为已完成移植。不要把一次升级拆成不同模块随意取不同 nightly 的私有表示；跨版本组合必须在语义契约上重新证明。

## 许可与测试复用

参考树 [REUSE.toml](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/REUSE.toml) 对多数 compiler/library/tests 文件给出 MIT 或 Apache-2.0 标注，也包含覆盖和例外。实际复制前逐文件检查原有 header、REUSE 注释、第三方代码及测试附件，不从根目录双许可证推断所有文件都相同。保留适用版权、许可证和 NOTICE，记录本地修改。

rustc 的 compiletest/UI 测试含版本、target、feature、auxiliary crate、预期错误等指令。迁移测试时保存原始来源及语义目的，适配运行 harness，不要求照抄易变的完整诊断文本。有效程序比较行为，无效程序比较拒绝类别；未支持语义记为 unsupported，而不是把测试删掉或将错误结果标为通过。

采用 nightly 内建项、`no_core`、特殊 lang items 等试验时，单列这些测试的环境。它们能证明局部 lowering，不自动证明普通稳定 Rust 或 sysroot 可用。引用其他后端实现只作架构/语义参考，不将其机器码生成路径接入正式产品来绕过 native 约束。

## M0 来源选择的完成条件

- 官方源码、oracle 二进制、目标 sysroot 和目标项目源码/lockfile 均有固定身份。
- 本地派生树中的 P1/P2b 等流水线改动与官方实现分开保存，采用与否可单独追溯。
- 已选版本满足冻结项目需求；若不满足，先重新选择版本再开始批量移植。
- 第一批移植闭包的来源清单、许可和回归样例可由另一执行者在没有本地历史目录的情况下取得。
- 本 PR 的路径导航不被误当成已批准采用官方 1.94.1 或已证明 1.99.1 兼容。

# rustc 模块映射

链接固定到官方 commit `e408947b`（见 [README](README.md)）。这是迁移责任地图，不代表这些源码都已逐行分析；每个移植任务要补充实际依赖范围、具体函数和对应回归。

## 移植方式

- **可借算法：** 在较小语义闭包内复用规则和已核实的实现，仍需验证。
- **重写适配：** 语义不变，换成自有 ID/类型/Body/任务模型——不是重新发明 Rust 规则。
- **强耦合：** 先做依赖实验（如 [T12](../roadmap/m1-vertical-chain.md#t12)、[T13](../roadmap/m1-vertical-chain.md#t13)）。如果最小例子就要带上大半 `TyCtxt`/query/arena，先重划自有数据模型或缩小闭包。

## 前端

| 领域 | 参考 | 主要耦合 | 方式与自有产物 |
|---|---|---|---|
| 词法 | [rustc_lexer](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_lexer/src)：`lib.rs`、`cursor.rs` | Unicode、字面量规则、token 身份 | 可借；适配自有 token/source，edition 语义在正确层处理 |
| 语法与模式 | [rustc_parse](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_parse/src/parser)：`expr.rs`、`item.rs`、`pat.rs`、`ty.rs` | AST、session、feature gating、错误恢复 | 按语法闭包移植到自有 AST；不搬 `rustc_session` 和诊断系统 |
| 宏与 hygiene | [rustc_expand](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src)、[hygiene.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_span/src/hygiene.rs) | token tree、SyntaxContext、resolver、过程宏协议 | 重写调度和存储，保留展开/卫生语义 |
| 名称与可见性 | [rustc_resolve](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src)：`def_collector.rs`、`macros.rs`、`effective_visibilities.rs` | 宏展开反馈、多命名空间、导入迭代、DefId | 复用作用域/解析规则，适配稳定定义身份；解析不是一次全局扫描 |

## 语义

| 领域 | 参考 | 主要耦合 | 方式与自有产物 |
|---|---|---|---|
| 类型推导 | [rustc_infer](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src)、[rustc_hir_typeck](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src) | TyCtxt、interned Ty、HIR、region、推导事务 | 强耦合；先移植标量/函数关系和 snapshot/rollback，再做 coercion、方法解析 |
| trait 与 coherence | [rustc_next_trait_solver](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_next_trait_solver/src)：`canonical/`、`solve/search_graph.rs`、`coherence.rs`；[rustc_hir_analysis](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src) | 参数环境、universes、canonicalization、缓存、循环规则 | 先做有约束泛型闭包，逐步扩展；“发现递归就成功”不是通用策略 |
| Body 构建 | [rustc_mir_build](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder)：`expr/as_place.rs`、`matches/`、`scope.rs` | THIR、Place、临时值生命周期、清理作用域 | 借鉴求值顺序和生命周期规则，生成自有 Typed Body |
| move/借用/生命周期 | [rustc_borrowck](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src)：`nll.rs`、`dataflow.rs`、`type_check/`、`region_infer/` | MIR location、Place、move data、region 推导 | 强耦合；先冻结自有事实模型，再移植分析算法和拒绝规则 |
| drop 显式化 | [elaborate_drops.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/elaborate_drops.rs) | 初始化数据流、move path、drop flag、清理 CFG | 正常/早退/失败路径改写为显式 Body 操作 |
| 常量求值 | [rustc_const_eval](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src)：`interpret/eval_context.rs`、`memory.rs`、`machine.rs` | 目标布局、allocation、pointer/provenance | 强耦合；自有解释器，先纯整数再目标内存 |

## 布局、实例与后端

| 领域 | 参考 | 主要耦合 | 方式与自有产物 |
|---|---|---|---|
| 布局与调用 ABI | [rustc_abi/layout.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/layout.rs)、[rustc_target/callconv](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv) | target spec、Ty、niche、PassMode | 优先借鉴，适配自有 Layout/Abi；逐项验证 repr 和聚合传递 |
| 单态化 | [rustc_monomorphize/collector.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs) | Instance、drop glue、vtable、静态 allocation | 复用依赖发现规则，改成自有实例任务图 |
| Body → 后端 | [rustc_codegen_ssa/mir](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir)、[traits/builder.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/traits/builder.rs) | ABI、内建函数、异常、backend builder | 当作 lowering 覆盖清单参考，直接构造自有 SoN；不实现 backend traits |

其他后端（如 cg_clif）只作架构/语义参考，不接入产品。

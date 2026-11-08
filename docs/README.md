# kfrc：自有 Rust 编译器计划

状态：**计划，尚未实施**。基线 commit `44c9d80b02ab140d7edf5348b6946d8122d842b6`。这里描述的接口、目录和能力都还不存在。

## 一句话

把 kfrc 从“标量子集编译器”变成能原生编译 ripgrep 和 Cargo 的 Rust 编译器：语义规则参考并移植 rustc；数据模型、带 Rust 语义的 SoN 优化器、x86 后端和构建流水线都是自己的。目标是运行时间和体积不差于 rustc+LLVM，编译更快，缓存更小。

## 现状 → 目标

| | 现在 | 目标 |
|---|---|---|
| 前端 | `frontend.rs` 边解析边建 SoN；类型只有 `i64/u64/bool/()` | 自有 AST → 名称解析 → 类型/trait → Typed Body → 借用/drop 检查 |
| 中端 | 整数 SoN，`memory` 只是预留输入 | 三级 SoN：Rust 语义多态（R）→ 单态（S）→ 布局（L） |
| 后端 | 标量线性扫描、spill 用 Load/Store、ELF 文本段 | 完整 System V ABI、通用内存、数据段/TLS/展开表 |
| 构建 | 函数级并行、图对象缓存；Cargo 显式拒绝 | 实例级任务图、按答案键的内容寻址缓存、Cargo 驱动、自有 sysroot |

## 固定的技术决策

- **自有实现，参考 rustc。** rustc 是算法、测试和对照结果的来源，不做 `CodegenBackend` 插件，也不把 `TyCtxt`/query 运行时包进产品。
- **优化在带 Rust 语义的 SoN 上做。** 聚合/enum 是值、借用是内存分区、drop 和 trait 调用是节点，最后才按布局降低；不先降成 LLVM 式低级 IR 再重建语义。
- **依赖少量 rustc 发布的算法 crate。** 以 `ra-ap-*` `=0.148.0` 形式依赖（与 rustc 1.94.1 源码逐字节相同）：`rustc_lexer`、`rustc_parse_format`、`rustc_pattern_analysis`、`rustc_abi`（布局计算），以及 `rustc_type_ir` + `rustc_next_trait_solver`（kfrc 实现 `Interner`）。其余语义自写；rust-analyzer 只作设计参考，不作移植来源。
- **后端不用 LLVM/Cranelift。** 线性扫描寄存器分配，不加静态 ILP 指令调度。rustc+LLVM 只作对照工具链。
- **首个目标平台** Linux x86_64 System V。

## 红线（所有文档共用）

1. 不支持的输入**明确报错**；不静默回退到 rustc、旧 `.kfr` 链或其他后端。
2. 不为通过测试而改变 panic/overflow/features/target 等语义配置。
3. 正确性以**真实 x86 执行**与 stock rustc 对拍为准；内部 evaluator、图测试只用于定位。
4. 混合 bootstrap、人工 IR fixture、分析原型单独标记，不计入 full-native 覆盖。
5. 局部 pass 加速、缓存命中不代表整项目冷编译加速。
6. 发现一类错误后先静态排查同类路径、批量修复，再恢复压测。
7. 移植 rustc 代码、或依赖 rustc 发布的 crate（`ra-ap-*`）时，记录来源 commit/版本、文件和许可。rust-analyzer 只作设计参考，不作移植来源（它有部分代码本身派生自旧版 rustc）。

## 目录

| 目录 | 读者要什么 |
|---|---|
| [design/](design/README.md) | **要做成什么样**：编译链、三级 SoN、前端产物、后端、构建与缓存、四个目标怎么保证 |
| [plan/](plan/README.md) | **怎么做**：阶段 M0–M6、任务 T00–T42、首轮十天、验收 |
| [research/](research/README.md) | **为什么这样设计**：rustc 精读、LLVM/MIR 流水线、编译模拟、先例调研、前期测量。不读也能理解计划 |

# rustc 移植与自有 SoN 编译器：迁移计划

状态：**计划，尚未实施**。基线 commit `44c9d80b02ab140d7edf5348b6946d8122d842b6`。本目录描述的接口、目录和能力都还不存在。

## 一句话

把 kfrc 从“标量子集编译器”变成能原生编译 ripgrep 和 Cargo 的 Rust 编译器：语义规则参考并移植 rustc，数据模型、SoN 优化器、x86 后端和构建流水线都是自己的。

## 现状 → 目标

| | 现在 | 目标 |
|---|---|---|
| 前端 | `frontend.rs` 边解析边建 SoN；类型只有 `i64/u64/bool/()` | 自有 AST → 名称解析 → 类型/trait → Typed Body → 借用/drop 检查 |
| 中端 | 整数 SoN，`memory` 只是预留输入 | 带类型、内存和异常效果的 SoN，带 verifier |
| 后端 | 标量线性扫描、spill 用 Load/Store、ELF 文本段 | 完整 System V ABI、通用内存、数据段/TLS/展开表 |
| 构建 | 函数级并行、图对象缓存；Cargo 显式拒绝 | 实例级任务图、语义缓存、Cargo 驱动、自有 sysroot |

## 固定的技术决策

- **自有实现，参考 rustc。** rustc 是算法、测试和对照结果的来源，不做 `CodegenBackend` 插件，也不把 `TyCtxt`/query 运行时包进产品。
- **后端不用 LLVM/Cranelift。** 保留 SoN 和线性扫描寄存器分配，不加静态 ILP 指令调度。rustc+LLVM 只作对照工具链。
- **首个目标平台** Linux x86_64 System V。

## 红线（所有文档共用，后文不再重复）

1. 不支持的输入**明确报错**；不静默回退到 rustc、旧 `.kfr` 链或其他后端。
2. 不为通过测试而改变 panic/overflow/features/target 等语义配置。
3. 正确性以**真实 x86 执行**与 stock rustc 对拍为准；内部 evaluator、图测试只用于定位。
4. 混合 bootstrap、人工 IR fixture、分析原型单独标记，不计入 full-native 覆盖。
5. 局部 pass 加速、缓存命中不代表整项目冷编译加速。
6. 发现一类错误后先静态排查同类路径、批量修复，再恢复压测。

## 路线图摘要

| 阶段 | 要证明什么 | 详情 |
|---|---|---|
| M0 | 输入、工具链和已知错误可复现 | [roadmap/m0-baseline.md](roadmap/m0-baseline.md) |
| M1 | 普通 `.rs` 走通新链：AST → Body → SoN → ELF → 运行 | [roadmap/m1-vertical-chain.md](roadmap/m1-vertical-chain.md) |
| M2 | 类型、布局、内存和 ABI 正确 | [roadmap/m2-types-memory-abi.md](roadmap/m2-types-memory-abi.md) |
| M3 | 泛型/trait、借用/drop、宏、const、单态化 | [roadmap/m3-semantics.md](roadmap/m3-semantics.md) |
| M4 | Cargo、自有 metadata、core/alloc/std | [roadmap/m4-cargo-sysroot.md](roadmap/m4-cargo-sysroot.md) |
| M5 | ripgrep 与 Cargo 原生构建且功能等价 | [roadmap/m5-real-projects.md](roadmap/m5-real-projects.md) |
| M6 | 运行时间/体积不退化，编译更快 | [roadmap/m6-quality-performance.md](roadmap/m6-quality-performance.md) |

第一步是十个工作日的时间盒，见 [first-round/](first-round/README.md)。

## 目录

| 目录 | 内容 |
|---|---|
| [roadmap/](roadmap/README.md) | 每个阶段一份：目标、出口、任务 T00–T42 |
| [first-round/](first-round/README.md) | 首个十日时间盒：批次、PR 切分、缩减顺序 |
| [architecture/](architecture/README.md) | 编译链、各层表示、共享 ID、流水线、缓存、Cargo |
| [validation/](validation/README.md) | 差分测试、覆盖矩阵、性能比较、证据格式 |
| [sources/](sources/README.md) | rustc 版本选择、模块映射、来源与许可记录 |
| [rustc/](rustc/README.md) | rustc 实现精读笔记：借用、所有权、类型推导、trait、宏、布局/ABI |
| [inventory/](inventory/README.md) | 46 个现有源码文件和测试的去向 |

## 与现有文档的关系

[docs/ARCHITECTURE.md](../ARCHITECTURE.md) 和 [docs/VALIDATION.md](../VALIDATION.md) 描述现有子集实现和历史实验；本目录不追认其中的性能数字。现有 `src/mir.rs` 等同名文件不代表已具备计划中的能力。

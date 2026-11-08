# Cargo、host/target 与 sysroot

## Cargo 协议

Cargo 是外部编排器，自有 driver 负责每次编译请求。现有 [cargo.rs](../../../src/cargo.rs) 是历史 rustc wrapper（同时设置两类 wrapper），native Cargo 目前显式拒绝；新 driver 不沿用它。

新 driver 先完成 Cargo 实际调用的契约：`-vV`、`--print`、crate 类型、`--emit`、`--extern`、cfg、目标、dep-info、JSON 诊断与产物。采用编译器入口模式；如保留 wrapper 模式，只设一层并测试嵌套情形。影响语义的未知参数报错。

M4 先保证标准 Cargo 调用闭环。跨 crate 提前发布如果需要改 Cargo，单独列出协议和兼容开关（[T40](../roadmap/m6-quality-performance.md#t40)）。

## host 与 target

- **host 图：** 构建脚本、过程宏及其依赖。
- **target 图：** 程序、依赖、sysroot。

两者分别记录 target triple、编译器生产者、特性和产物身份。原生过程宏需要自己的 runner/协议；官方 rustc 编译的 proc-macro 动态库不假定能直接加载。早期混合 bootstrap 可以用隔离的 host 工具链或桥接进程，但必须标为混合路径。

## sysroot

按 `core → alloc → std` 推进，同时处理 compiler-builtins、panic runtime、分配器、平台库和内建机制。Rust ABI 不承诺跨任意版本混链。C/C++ 库可以用其原生工具链编译，但在清单中单列。

## 来源清单

每次构建输出：模块/对象、源码版本、编译器哈希、host/target、配置、缓存命中来源、链接归属。按以下标签区分覆盖范围：

1. 混合 bootstrap
2. 目标 Rust 依赖全 native
3. host 工具也全 native

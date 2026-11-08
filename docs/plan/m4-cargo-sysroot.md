# M4：Cargo、metadata 与 sysroot

**目标：** 普通 Cargo 能调用新编译器；自有 metadata/rlib 支持跨 crate；按 core → alloc → std 原生编译 sysroot；处理构建脚本和过程宏。

**依赖：** M2/M3 中对应能力；driver 协议和格式设计可提前。

**出口：**

- 干净环境下构建冻结的依赖库并通过其功能测试。
- 每个目标 Rust 对象有生产者记录；混合 bootstrap 单列；目标库、host 工具和 C/C++ 依赖分开记账。
- 至少达到“目标 Rust 闭包全 native”。

**通过后仍不代表：** 混合 bootstrap 等于 full-native；标准 test harness 缺 unwind 时，局部 fixture 不代表完整测试矩阵通过。

**覆盖标签：** 混合 bootstrap → 目标 Rust 依赖全 native → host 工具也全 native，三者是不同里程碑。设计见 [cargo-sysroot](../design/build/cargo-sysroot.md)。

---

<a id="t28"></a>
## T28 Cargo 编译器协议

`M4 · E · 依赖：T10, T11, T20`

- **做什么：** `-vV`、`--print`、cfg、`--emit`、crate 类型、`--extern`、dep-info、JSON 诊断/产物；采用编译器入口模式（或单层 wrapper）。未知且影响语义的参数报错。
- **完成标志：** 最小无跨 crate 依赖的 workspace 能查询、编译、报错、重复构建；不出现双 wrapper 嵌套。fixture 用无 runtime 输入或明确标记的 bootstrap sysroot。

<a id="t29"></a>
## T29 自有 metadata/rlib 与跨 crate 实例

`M4 · E · 依赖：T19, T25, T28`

- **做什么：** 自有格式与 schema：定义、类型、泛型体、依赖身份、const/const fn 的 CTFE body、泛型与可内联函数的 body、导出宏的 token 与定义 crate 的 edition 和 hygiene 表、目标与版本校验，rlib 包装。不兼容官方 `.rmeta`。
- **Rust 语义 SoN：** metadata 携带优化后的 R 图、孔位表、mentioned 集合和调用摘要；接口先于函数体发布（[compile-time.md](../design/goals/compile-time.md#关键路径)）。
- **读取：** metadata 按 `DefKey` 建索引、惰性解码，一次构建内共享解码结果（rustc 的解码占前端 7–9%）。
- **完成标志：** 两个以上冻结 fixture crate 的跨 crate 泛型/符号链；旧 schema、错误目标、缺失依赖被拒绝；接口和泛型体变化正确失效。

<a id="t30"></a>
## T30 构建脚本与过程宏

`M4 · E · 依赖：T23, T28, T29`

- **拆分：** T30.a 构建脚本（ripgrep 依赖闭包里有 10 个，必需）；T30.b 过程宏协议（ripgrep 默认构建不需要——`serde_derive` 只是 dev-dependency——但 Cargo 构建和 ripgrep 测试需要）。
- **做什么：** host 图、过程宏协议与 runner、build script 输出和动态输入跟踪；早期可用明确标记的 bootstrap 方案。不可完整跟踪输入的任务不跨构建缓存。
- **协议细节：** 版本协商、span 子请求、响应里带 `tracked_env_vars`/`tracked_paths`（进缓存键）；proc-macro 进程池计入 jobserver 预算。RA 的 proc-macro-srv 也只兼容同一工具链的 bridge，所以加载官方 rustc 编译的宏（方案 A）风险更高（[proc-macro-srv](../research/rust-analyzer/proc-macro-srv.md)）。
- **完成标志：** host/target 不混链；宏输入或环境变化不会错误命中。官方 rustc 编译的 proc-macro dylib 不假定能直接加载。

<a id="t31"></a>
## T31 core 与 compiler-builtins

`M4 · B · 依赖：T26, T27, T29`

- **已知需求：** core 1.94.1 打开 `const_trait_impl`、`min_specialization`、`negative_impls`、`auto_traits`（T21 覆盖）；ripgrep 依赖的 zmij 在 x86_64 上用 `asm!`，至少要支持空模板 + 寄存器约束。
- **做什么：** 列出冻结 core 实际需要的语言特性、属性、lang item、intrinsic 和 builtins，阻塞项先拆子项；自有代码生成编出 core/builtins。
- **补充：** 依赖 T30.a（sysroot 有 4 个 build script）；`#![no_builtins]`/`#![compiler_builtins]` 的语义与 169 个导出函数的 libcall 名表。
- **T31.b asm 子系统与汇编器：** compiler-builtins 的 `div`、`rep movsb`，naked 的 `__rust_probestack`（带 `.cfi_*`），std_detect 的 `cpuid`。原来写的“空模板 + 寄存器约束”明显低估。
- **完成标志：** core 代表功能运行；所有目标 Rust 对象生产者可追溯，没有混入官方预编译对象。

<a id="t32"></a>
## T32 alloc 与分配器

`M4 · B · 依赖：T31`

- **做什么：** 冻结 alloc、分配器 runtime、Vec/String/Box 等所需特性。
- **补充：** 编译器生成的分配器 shim（`__rust_alloc` 等转发到 `#[global_allocator]` 或默认实现）。
- **完成标志：** 在真实 alloc 实现上运行 Vec/String/Box 测试，含分配失败和析构路径。

<a id="t33"></a>
## T33 std 与平台依赖

`M4 · B · 依赖：T27, T30, T32`

- **做什么：** 冻结 std、平台接口、线程/TLS、IO、panic runtime、test harness 需求，按阻塞能力拆项；产出可用于目标项目的自有 sysroot，Rust/C 库来源分列。
- **补充：** C `main` shim；SIMD 与按函数 `target_feature` 从 M5 前移到这里（std 的 HashMap 和 ripgrep 都在关键路径上），AVX-512 留在 M5 并单列为风险。
- **完成标志：** 代表 std/test 功能的正常和失败路径。

<a id="t34"></a>
## T34 M4 出口

`M4 · I · 依赖：T28–T33`

- **做什么：** 干净 Cargo workspace、真实依赖库、sysroot 和 host/target 来源审计；按三个覆盖标签出能力与证据清单。
- **完成标志：** 满足本阶段出口；没有借旧 cache/target 目录伪造覆盖。

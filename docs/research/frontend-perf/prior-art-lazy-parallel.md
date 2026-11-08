# 快速前端先例（二）：惰性导入、模块、查询库与并行检查

Go、Clang/Swift、rust-analyzer、TypeScript 7（tsgo）。约定同 [prior-art-layout.md](prior-art-layout.md)。

## 1. 机制与数字

| 项目 | 做法 | 数字 | 来源 |
|---|---|---|---|
| Go 依赖模型 | 包只读直接依赖的 export data，其中带所需的传递类型信息 | Google C++：4.2 MB 源码展开成 >8 GB 编译器输入（约 2000×）；Go 约 40×，“好 50 倍” | [Pike 2012](https://go.dev/talks/2012/splash.article) |
| Go indexed export（1.11） | 导入的声明和可内联函数体按索引**惰性**解码；现行 unified 格式保留索引 | `go build -a`：Juju 墙钟 −12…−19%、user −17…−22%；Kubelet 墙钟 −17…−20%、user −17…−23% | [commit ca2f85fd](https://github.com/golang/go/commit/ca2f85fd3f32b2a4c863a2de602876bd31e9d956)、[cmd/compile README §7a](https://go.dev/src/cmd/compile/README) |
| Go 泛型的前端税 | 1.18 引入泛型后编译约慢 15%；1.20 的 unified IR 前端追回 | 1.18 “roughly 15% slower”；1.20 “up to 10%” faster，回到 1.17 水平 | [Go 1.18](https://go.dev/doc/go1.18)、[Go 1.20](https://go.dev/doc/go1.20) notes |
| Go 并行 | `noder.LoadPackage` 每个文件一个 goroutine 解析；后端按函数并行（1.9）；包级并行由 `go build` 动作图调度 | — | [noder.go](https://github.com/golang/go/blob/master/src/cmd/compile/internal/noder/noder.go) |
| Clang PCH | AST 文件惰性加载：初始只读与文件大小无关的小表；声明按 ID→偏移，名字查找走盘上哈希表 | Cocoa.h PCH + hello world：82,685 个声明只读 20 个（0.024%），15,315 个类型读 19 个 | [PCHInternals](https://clang.llvm.org/docs/PCHInternals.html) |
| Clang modules | 每个头只解析一次，M×N 变 M+N | C++ `<iostream>` hello：源码 81 B、头文件 1,161,033 B | [Gregor 2012](https://llvm.org/devmtg/2012-11/Gregor-Modules.pdf) |
| C++ `import std` | 整个标准库一个预编译模块 | MSVC hello：`#include <iostream>` 0.87 s，`import std` 0.08 s | [P2412R0 §4](https://www.open-std.org/jtc1/sc22/wg21/docs/papers/2021/p2412r0.pdf) |
| Swift | 序列化模块可按名字只加载单个定义（文档称编译器常常加载过多）；primary-file 模式每个作业都解析模块内全部文件，batch mode 缓解；表达式类型检查按选择次数（100 万）和 arena（512 MB）设上限，不按时间 | 某重载密集项目类型检查总时间 42 s（6.1）→ 34 s（6.2）→ 12 s → 10 s（main） | [CompilerPerformance.md](https://github.com/swiftlang/swift/blob/main/docs/CompilerPerformance.md)、[论坛 2025-10-30](https://forums.swift.org/t/roadmap-for-improving-the-type-checker/82952) |
| rust-analyzer | `ItemTree` 是对函数体修改稳定的文件摘要；不变量“在函数体内打字永不使全局派生数据失效”；Salsa durability 让 std 不随编辑重验 | 加 durability 前每次编辑重验 std 约 300 ms；2026-11-07 CI：ripgrep 全量分析 9.1 s / 246 MB（含依赖） | [architecture.md](https://github.com/rust-lang/rust-analyzer/blob/master/docs/book/src/contributing/architecture.md)、[Durable Incrementality](https://rust-analyzer.github.io/blog/2023/07/24/durable-incrementality.html)、[metrics](https://github.com/rust-analyzer/metrics) |
| rust-analyzer 换求解器 | 2025-08 起用 `rustc_next_trait_solver` 替代 Chalk | diesel 推导中位数 286 s（Chalk 时期）→ 8.6 s（2026-10/11）；CI 机器不同且混有其他改动（推算，非 A/B） | [PR #20329](https://github.com/rust-lang/rust-analyzer/pull/20329)、metrics |
| tsgo（TypeScript 7） | 原样移植到 Go；解析和输出按文件并行；类型检查固定 N 个 checker（默认 4），各自一份视图，**允许重复公共工作**，文件确定性划分 | 2025-03：VS Code 150.5 万行 77.8 → 7.5 s（10.4×），内存约减半；7.0 正式版：vscode 125.7 → 10.6 s（4 checker，11.9×）、7.51 s（8 checker，16.7×） | [A 10x Faster TypeScript](https://devblogs.microsoft.com/typescript/typescript-native-port/)、[Announcing TypeScript 7.0](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/) |
| tsgo 收益拆分 | Hejlsberg：“half of it from being native code, and the other half from … shared memory concurrency” | 约 3× × 3×（具体拆分为二手） | [DevClass 2026-01-28](https://devclass.com/2026/01/28/typescript-inventor-anders-hejlsberg-ai-is-a-big-regurgitator-of-stuff-someone-has-done/) |

## 2. 共性

1. **导入要惰性、按索引随机访问。** Go 仅靠惰性解码就省 12–20% 的整构建墙钟；Clang 的 hello world 只触及大头文件 0.024% 的声明。rustc 的 rmeta 已经 mmap + 按查询解码，但每个下游 crate 进程都重新解码同一批 std 条目（[why-slow-infra.md §3](why-slow-infra.md#3-metadata编码在关键路径上解码按查询懒惰但粒度很细)）。
2. **函数体与签名之间有防火墙**（rust-analyzer ItemTree、K2 阶段、Swift/Clang 模块只导出接口）。这是增量截断和接口先发布的共同前提。
3. **共享内存并行检查不需要精细锁**：tsgo 用“N 个各自为政的 checker + 确定性划分”，4 个 worker 就到约 3×，代价是重复工作和内存。
4. **最坏情况在求解器**：Swift 4× 来自选择顺序启发和 arena 复用；rust-analyzer 在 trait 密集的 diesel 上随求解器更换差一个数量级以上。
5. **查询库式（Salsa）适合 IDE 的懒惰与增量，不适合批量冷构建**：rust-analyzer 冷分析 ripgrep 9.1 s，不比 rustc check 快（[time-breakdown.md](time-breakdown.md)）。

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| metadata 接口段按 `DefKey` 索引、惰性解码、零拷贝 | **采纳** | Go −12…−20%；Clang 0.024%；rustc 的 `def_path_hash_map` 已是零拷贝哈希表，可沿用这一形状 |
| sysroot 接口按 durability 只验证一次，跨 crate 进程共享解码结果 | **改编**：首版做“每会话一次 + mmap 共享”，常驻进程留到 T39 后 | rust-analyzer durability；rustc 每进程重复解码 std |
| 并行检查采用 tsgo 式“固定 worker + 确定性划分”，允许 worker 局部缓存重复 | **采纳**（库内 typeck root 并行） | 不需要 rustc 的全局查询锁与死锁检测；结果顺序稳定 |
| 用 Salsa/查询系统驱动整个前端 | **不采纳** | 与“自有实现、不把 query 运行时包进产品”的固定决策一致；冷构建无收益 |
| 复用 `rustc_next_trait_solver` | **已决定复用**（[总览固定决策](../../README.md#固定的技术决策)），补性能闸门 | rust-analyzer 换用后 trait 密集 crate 大幅变快；但 rustc 1.94.1 全局开启新求解器时 rg check 指令 ×1.148（[design-gaps.md](design-gaps.md) G10） |
| 推导设确定性燃料上限（Swift） | **采纳** | 时间上限不确定；与 [design/goals/compile-time.md](../../design/goals/compile-time.md) 的“预算确定性”一致 |
| 接口单独产物，不带函数体（Clang Reduced BMI、Swift module、header jar） | **采纳**（维持 C4） | 见 [critical-path.md](critical-path.md) |

## 4. 未找到原始来源

- Swift/Xcode explicit module builds 的端到端构建时间数字。
- tsgo “3.5× 原生 + 3× 并发”的具体拆分（只有 Hejlsberg “各一半”的访谈原话，具体倍数为二手）。
- Kobzol 2024 “What part of Rust compilation is the bottleneck?” 正文没有百分比，只有图表；perf.rust-lang.org 的逐查询占比未收集。
- rust-analyzer 换求解器前后的受控 A/B（metrics 是 CI 中位数，混有其他改动）。

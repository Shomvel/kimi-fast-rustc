# 泛型代码共享：按布局、按答案、按字典

问题：“R 级优化一次 + 按答案键去重”有没有先例，结果如何？rustc 自己的 polymorphization 见 [rustc-mir-mono.md](rustc-mir-mono.md)。

## 1. 是什么

| 系统 | 来源 | 年份 | 共享依据 | 共享后怎么分派 |
|---|---|---|---|---|
| Carbon specific coalescing | 设计文档 [coalesce_generic_lowering.md](https://docs.carbon-lang.dev/toolchain/docs/coalesce_generic_lowering.html)；Birlea, [LLVM DevMtg 2025-10 幻灯片](https://llvm.org/devmtg/2025-10/slides/quick_talks/sbirlea.pdf) | 2025 | 降低到 LLVM 时的指纹：含所有依赖 specific 的信息与 LLVM 类型；调用 SCC 先假设等价再验证 | 静态：合并后直接调用规范实例 |
| Go GC-shape stenciling | [设计文档](https://go.googlesource.com/proposal/+/refs/heads/master/design/generics-implementation-gcshape.md)（Go 1.18） | 2022 | GC 形状：大小、对齐、哪些部分是指针；所有指针同一形状 | 运行时字典：类型描述符、子字典、方法 |
| .NET 共享泛型 | [shared-generics.md](https://github.com/dotnet/runtime/blob/main/docs/design/coreclr/botr/shared-generics.md) | — | 只有引用类型实例共享 `__Canon` 代码；值类型各自一份 | 泛型字典，槽位懒填充 |
| Swift 未特化泛型 | [SE-0460 Explicit Specialization](https://github.com/swiftlang/swift-evolution/blob/main/proposals/0460-specialized.md)（Swift 6.3 实现） | Swift 6.3 | 一份未特化体处理所有类型 | witness table；SE-0460 加“入口按精确类型重分派到预特化版本” |
| MLton | [MLton 概述](http://www.mlton.org/References.attachments/060916-mlton.pdf) | 2006 | 不共享：全程序单态化 | — |
| Mojo KGEN | [Mojo vision](https://mojolang.org/docs/vision) | 2023– | 参数化代码在实例化前以 MLIR 表示 | 未找到公开细节 |

## 2. 关键证据

| 量 | 数字 | 来源 |
|---|---|---|
| Carbon 合并（压力测试，非典型代码，512–1024 行） | 基本类型与指针混合：函数数、LLVM IR、汇编约 −44%～−56%；以指针为主：约 −98% | sbirlea 幻灯片 |
| Carbon 编译时间 | 混合：lowering 阶段 +0.9%～−2%，opt+codegen（-O3，关内联）−25%～−51%；指针为主：lowering −7%～−9%，opt+codegen 约 −97%。开内联时：有合并的编译时间相近，不合并则因激进内联“爆炸” | 同上 |
| LLVM MergeFunctions 对照 | 只在约 12% 的测试中起作用，最多减 4% 函数，平均约 0.1% | 同上 |
| Carbon 算法 | 每个 generic 内 O(N²) 比较（N = specific 数），计划按指纹分桶；设计文档“编译时间影响不在本文范围” | 设计文档 |
| Go 字典分派的运行时代价 | 手工特化 5.06 µs；接口参数 6.85 µs；泛型 + 指针 7.18 µs；传接口给泛型函数 9.68–17.6 µs。原因：方法经字典，无法去虚化和内联 | Marti, [PlanetScale 2022-03](https://planetscale.com/blog/generics-can-make-your-go-code-slower) |
| Go 设计文档自述 | 方法调用“不能在编译期完全解析”，逃逸分析更保守，内联更少；实例数可能随类型参数个数指数增长 | GC-shape 设计文档 |
| Swift 未特化路径 | 类型擦除路径“大约慢两个数量级” | SE-0460 |
| MLton 单态化的代码膨胀 | 理论上爆炸，实际“最大见到 30%”；编译时间列为缺点 | MLton 概述 |
| `-Zmono-dedup`（本项目前期） | rg/cargo 编译指令 −2.35% / −2.74%；IR 去重 5.1% / 6.8% | [evidence.md](../evidence.md) 第 3 节 |

## 3. 对 kfrc 的结论

| 问题 | 结论 | 理由 |
|---|---|---|
| 有没有 R/S/L 式“多态体优化一次 + 按答案特化 + 去重”的先例 | **没有完整先例** | Carbon 在前端按指纹去重，但每个 specific 仍各自降低和优化（由 LLVM 做）；Go/.NET 共享代码但留下运行时字典；rustc MIR 优化多态体一次但不做单态后优化 |
| 答案里必须含所选 impl，不能只按布局共享 | **采纳（已一致，写明理由）** | Go 的 GC-shape 只按布局共享，trait/方法走字典，运行时变慢；kfrc 的 `Resolve` 答案是被调实例的答案键，共享的代码仍是静态调用 |
| 去重在前端用类型化答案算，不靠事后比较机器码 | **采纳（已一致）** | Carbon：前端合并让函数数降 44–98%，LLVM MergeFunctions 只减 0–4% |
| 递归 SCC 的等价判定 | **采纳（已一致）** | Carbon 先假设等价、沿调用对验证；kfrc 已写“按 `InstanceKey` 分组后做类细化不动点” |
| 合并必须先于内联决策 | **采纳** | Carbon：开内联时不合并会让编译时间爆炸。kfrc 的 S 级内联以答案键为单位，同键实例只做一次内联 |
| 指纹比较的复杂度 | **采纳** | Carbon 的 O(N²) 是已知问题；kfrc 直接用答案哈希做表查找，等价判定只在 SCC 内做 |
| 未特化的共享体 + 运行时分派 | **不采纳** | Swift 的未特化路径慢约两个数量级；kfrc 只共享“答案完全相同”的实例，不引入字典 |
| 只依赖 `SizeAlign(T)` 的冷区域抽成运行时参数 | **维持“可能的扩展”** | 这就是 .NET/Go 式字典共享，只适合冷路径；需要体积数据 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 答案键 | 加“先例与区别”：Carbon（前端指纹，合并后函数数 −44%～−98%，MergeFunctions 只 0～4%）；Go GC-shape（只按布局共享 + 字典 → 运行时变慢，所以 `Resolve` 必须在答案里） |
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 特化 | 写明顺序：答案键查重 → 特化 → 内联决策；同键实例的内联只做一次（依据 Carbon“开内联时不合并会爆炸”） |
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 可能的扩展 | 注明这是字典式共享，只允许冷区域；引用 SE-0460 的“两个数量级”作为不进热路径的理由 |
| [compile-time.md](../../design/goals/compile-time.md) | 指标与闸门 | “实例数 / 答案键数”旁加对照：LLVM MergeFunctions 在同一输入上合并的函数数 |

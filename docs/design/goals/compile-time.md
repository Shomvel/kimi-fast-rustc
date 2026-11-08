# 编译时间：怎么保证

目标见 [M6](../../plan/m6-quality-performance.md)：完整编译时间改善，运行时间和体积不退化。这里列出由 IR 设计保证的结构性性质，以及验证它们的指标。

## 六条结构性性质

| # | 性质 | 实现 | 对应的 rustc 浪费 |
|---|---|---|---|
| C1 | 每个泛型定义的 R 级优化每次构建只做一次 | R 图按内容哈希发布；下游 crate 读 metadata 不重做 | MIR 只优化多态体，LLVM 对每个实例重做 |
| C2 | 每个答案键的机器码和摘要只生成一次，**跨 crate** | build 级内容寻址存储，按答案键查找（[cache.md](../build/cache.md)）；作为内联源的 S 图也缓存，其余 S 图按需重算 | LocalCopy 每 CGU 一份；ThinLTO 导入体每个导入方重新简化（post-link 40%） |
| C3 | 优化工作与变化量成正比 | worklist 种子：特化替换的节点、内联接缝 | 每函数约 8 次 InstCombine，83% 无变化 |
| C4 | 接口先于函数体优化发布 | 接口、R 图、机器码三类产物分开就绪 | MIR 优化代价落在 rmeta 之前，串行、在关键路径上 |
| C5 | 冷区域只做最小工作 | 结构性冷标记（[ownership-drop-unwind.md](../ir/ownership-drop-unwind.md#冷区域)） | O3 对 panic/unwind 路径一视同仁 |
| C6 | 不生成不可达的代码 | 可达性在链接前的实例图上决定（[size.md](size.md)） | 22–31% 的已编译代码被链接器丢弃 |

## 关键路径

```text
上游接口就绪 ──→ 下游类型检查/借用检查
上游 R 图就绪（逐体） ──→ 下游特化用到它的实例
上游摘要就绪（逐实例） ──→ 下游内联决策
```

- 摘要在未优化 S 图上用廉价遍历算出，不等被调者优化完（[optimization-pipeline.md](../ir/optimization-pipeline.md#调用摘要)）。否则调用图深度就是串行链长度：W04 中 `work` 的调用图高度 8，高层每层只有 1 个函数。
- 下游不需要等上游的机器码，除非链接。
- 每个依赖是**逐体/逐实例**的等待，不是整 crate 屏障。签名优先流水线的模型上限是 rg 比 bin pipelining 再降 28.5%（CPU 不设上限），但 8 vCPU 上的原型实测只降 3.3%，cargo 项目无变化：CPU 已饱和时，提前发布只是挪动工作。收益与核数强相关，按 4/8/16 核分别测。
- **接口不带函数体图。** 跨 crate 设两道屏障：接口就绪、所需 R 图就绪；逐个函数体等待首版只在同一进程内做（跨进程按体等待没有先例；rustc #64112 的尝试收益很小，headstart 原型在 16 核上 build 最多 −42%，[prior-art/ct-parallel-pipeline.md](../../research/prior-art/ct-parallel-pipeline.md)）。
- **接口体：** 以下函数体必须在发布接口前检查完，因为接口依赖它们：`impl Trait` 返回（含 trait 中的 RPITIT）、async 函数和 async 块（auto trait 泄漏）、const 与 const fn（CTFE 要用它们的 Body）、数组长度、static 的类型推断、`impl_parent` 这类 impl 级查询。发布接口时断言没有触发其他函数体的检查（P2b M0 的测量列出了这些条目）。
- C6 会把部分代码生成推迟到二进制 crate 一步。为了不把它变成串行瓶颈：库 crate 在有空闲 jobserver token 时**预先生成**可能需要的实例（非泛型 `pub` 函数、已知被用到的泛型实例），写进 build 级存储；二进制 crate 只做缺失的那部分。预生成浪费的只是空闲时间，体积不受影响（体积只看可达性）。proc-macro、dylib、cdylib/staticlib 边界必须整体生成；可达性按变化增量更新（Zig 一次 37 ms 的更新里约 31 ms 花在全量遍历引用图）。
- 链接：首版用确定性拼接 + 快速全量链接（mold 全量已接近拷贝速度），原地增量链接列为 T39 之后的候选（MSVC `/INCREMENTAL` 遇到 `/OPT:REF`/`/OPT:ICF` 就退回全量）。

## 构建模式

**debug 档（opt0）：** R 级只做窥孔和 DCE，S 级不内联，节点固定在 Body 块里，L 级用 TPDE 式单遍分配和编码（TPDE 比 LLVM -O0 后端快 8–26×，运行时间 ±9%；Zig 0.15 debug 约快 5×）。档位进缓存键。不采纳 copy-and-patch：代码慢 2.32×、体积 4.27×（[prior-art/ct-fast-compile.md](../../research/prior-art/ct-fast-compile.md)）。


增量/dev 构建跨 crate 只内联 `#[inline]` 和小函数，release 才按完整准则（[内联策略](../ir/optimization-pipeline.md#内联策略)）。前者让上游普通函数的修改不传到下游机器码，rustc 增量模式也是这么做的（W08）。

## 预算

- 单图节点上限、内联增长上限、每 pass 燃料：确定性，结果不依赖时间。
- 内存：同时在处理的图数受字节预算限制（[build-pipeline.md](../build/pipeline.md#cpu-与内存预算)）。rustc 并行前端峰值内存最多 +35%。
- jobserver：任务暂停时归还 token、恢复时重新取；暂停任务的个数和字节设上限。提前发布开和关时，诊断、退出码和产物都要一致。

## 参照

Swift 的 SIL 优化器（同样在语义级 IR 上做特化、内联）约占编译时间的三分之一，其余主要是 LLVM（2016 WMO 博客）。kfrc 的 R+S 级以此为参照：R+S 级占比明显更高时，先查是不是在做本该由 L 级做的事。Carbon 在前端按类型化指纹合并实例，压力测试中函数数减少 44%–98%，同条件下 LLVM MergeFunctions 只有 0–4%（[prior-art/generic-sharing.md](../../research/prior-art/generic-sharing.md)）。

## 前端

rustc 前端在 check 构建中占全部工作约 90%，dev 约 50%，release 约 25%；typeck + trait + borrowck 占前端一半以上；增量编辑时依赖图本身占前端 26–33%（E2B，[frontend-perf](../../research/frontend-perf/time-breakdown.md)）。kfrc 的前端收益来自三处，不来自“同样的事做得更快”：

1. MIR 优化不在 rmeta 之前（release 中它占前端 15–22%，且串行在关键路径上）——设计已由 C4 保证；
2. 削减每 crate 的固定开销：metadata 按 `DefKey` 索引、惰性解码、一次构建内共享解码结果（解码占前端 7–9%，Cargo release 中 `impl_trait_header` 被解码 320 万次）；
3. crate 内按 typeck root 并行（[overview](../frontend/overview.md#crate-内的阶段与冻结点)）。

估计单 crate 前端 wall 快 2.7–3.9×；整个 `cargo check` 不到 2.2×，因为 Cargo 的 C 构建脚本在关键路径上（12.95 s，前端 12.23 s）。rustc 的 `-Zthreads=8` 在 `cargo check` 上 wall −15%（rg）/ −6%（Cargo），CPU +60% / +30%。

## 指标与闸门

前端分项：各阶段占比、接口就绪时间、1/4/8 worker 的扩展曲线、metadata 解码次数。

两个新串行点要单独计：下游特化等待上游 R 图的时间；二进制 crate 末段的忙碌 CPU 数（C6 集中生成）。各级耗时按与 LLVM 相同的 pass 族名分项（[llvm-mir](../../research/llvm-mir/README.md) 的总表），便于逐族对比。

分 4/8/16 核、深依赖图和宽依赖图分别测（headstart 在 4 核上收益明显变小）；rustc 的 `-Zthreads=8` 只作参考臂。

每次真实 crate 构建（M5 起）输出：

| 指标 | 期望 |
|---|---|
| 每个 R 图的优化次数 | 恰好 1 |
| 每个答案键的特化/机器码生成次数 | 恰好 1（预生成未用的单独计数） |
| 实例数 / 答案键数 | 去重比，越高越好；与 `-Zmono-dedup` 的类数对比 |
| 每 pass 访问节点数、改动比例 | 无变化比例逐步下降 |
| 关键路径构成 | 接口 / R 图 / 特化 / L 级 / 链接 各段 |
| 每阶段 `instructions:u` 与 wall | 在 E2B 上与 rustc 同场景对比 |

新优化的采纳规则沿用前期经验：看代价落在关键路径的哪一段、看函数数和实例数的变化，不看中间 IR 减少了多少；用 A/D/B（D = 运行但丢弃结果）区分分析成本和收益。

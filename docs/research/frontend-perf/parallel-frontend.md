# 并行前端：rustc `-Zthreads` 的现状与扩展性

## 1. 现状（源码 + 官方资料）

| 事实 | 位置 / 来源 | 性质 |
|---|---|---|
| 默认 `-Zthreads=1`（“behave like a sequential compiler for now”） | [options.rs#L2710-L2717](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/options.rs#L2710-L2717) | 源码 |
| `par_for_each_in` 等在非线程安全模式下退化为串行循环；并行时递归二分 | [sync/parallel.rs#L136-L237](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_data_structures/src/sync/parallel.rs#L136-L237) | 源码 |
| 并行的部分：typeck/常量求值循环、借用检查循环、`check_mod_*`、晚期 lint、wfcheck、privacy、metadata 预取、单态化收集 | [hir_analysis/lib.rs#L231](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/lib.rs#L231)、[passes.rs#L1097-L1136](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L1097-L1136) | 源码 |
| 串行的部分：解析、展开 + 名字解析、AST 降低、coherence 循环、`encode_crate_root` | [passes.rs#L785-L811](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L785-L811)、[ast_lowering/lib.rs#L523-L525](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/lib.rs#L523-L525) | 源码 |
| 同一查询被别的线程计算时等待；环由 deadlock handler 线程收集活动 job 后打断 | [plumbing.rs#L380-L397](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_query_system/src/query/plumbing.rs#L380-L397)、[interface/util.rs#L176-L270](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/util.rs#L176-L270) | 源码 |
| 8 线程最多 −50% 墙钟；单线程模式慢 0–2%；内存最多 +35%；cargo 例子前端 10.2 → 5.9 s | [Inside Rust 2023-11](https://blog.rust-lang.org/2023/11/09/parallel-rustc/) | 官方 |
| 超过 4 线程出现锁争用 | [rustc-dev-guide](https://rustc-dev-guide.rust-lang.org/parallel-rustc.html)（自标过时） | 官方 |
| 2026 目标：测试集、Cargo 启用、perf 基准、稳定化报告；“并行名字解析和宏展开”列为未来工作；2026-04 状态仅 “Continued” | [goals 2026](https://goals.rust-lang.org/2026/parallel-front-end.html)、[2026-04 更新](https://blog.rust-lang.org/2026/05/18/project-goals-2026-04/) | 官方 |

## 2. 实测：`cargo check` 的 `-Zthreads` 扫描

E2B 8 vCPU，官方 1.94.1，`-j8`，无 self-profile，顺序 1,8,2,4,4,2,8,1，每档 2 次（`scripts/threads-table.sh`）。

Cargo：

| -Zthreads | 墙钟均值 s | 相对 1 | user+sys 均值 s | 相对 1 | instructions:u 均值 G | 相对 1 | 峰值 RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 26.22 | 0% | 116.54 | 0% | 489.52 | 0% | 939 |
| 2 | 27.14 | 3.5% | 141.15 | 21.1% | 496.35 | 1.4% | 990 |
| 4 | 25.77 | -1.7% | 146.07 | 25.3% | 496.52 | 1.4% | 1039 |
| 8 | 24.6 | -6.2% | 151.19 | 29.7% | 496.88 | 1.5% | 1080 |

ripgrep：

| -Zthreads | 墙钟均值 s | 相对 1 | user+sys 均值 s | 相对 1 | instructions:u 均值 G | 相对 1 | 峰值 RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 3.31 | 0% | 10.09 | 0% | 38.69 | 0% | 259 |
| 2 | 3.32 | 0.3% | 13.58 | 34.5% | 39.49 | 2.1% | 322 |
| 4 | 2.91 | -12.1% | 14.41 | 42.8% | 39.54 | 2.2% | 326 |
| 8 | 2.81 | -15.1% | 16.17 | 60.2% | 39.62 | 2.4% | 335 |

峰值 RSS 是单个进程的最大值。

## 3. 解读

1. **整构建收益远小于“−50%”。** 8 线程下 Cargo −6.2%、rg −15.1%，而 CPU 多 30%/60%，指令多 1.5–2.4%：多出的 CPU 主要是空转与同步，不是多做的语义工作（推断）。
2. **原因是 crate 级并行已占满 8 核，关键路径还有非前端段。** Cargo check 关键路径 28.4 s 中构建脚本运行 12.95 s（[time-breakdown.md](time-breakdown.md)），`-Zthreads` 只能缩短 cargo 库自身前端 9 s 那段。前期 release 实验在关键路径的前端段上测得 36.2 → 21.1 s（−42%，`-Zthreads=4`，前期 next-directions §4.2），与官方单 crate 数字同量级。
3. **2 线程反而变慢**（Cargo +3.5%）：进入线程安全模式后锁、分片和 job 等待的固定成本先到，并行收益后到。

## 4. 对 kfrc 的上限估算（推断）

按 check 场景的前端占比，可按 typeck root 并行的部分（typeck、trait、借用检查、MIR 构建、常量求值、lint）约占 65%（rg 64.9%、Cargo 65.5%），其余（解析展开、名字解析降低、metadata、其他）约 35% 串行：

| 串行比例 | 4 worker | 8 worker | 无限 |
|---:|---:|---:|---:|
| 35%（rustc 现状的划分） | 1.95× | 2.32× | 2.86× |
| 25%（再把解析按文件并行、metadata 惰性） | 2.29× | 2.91× | 4× |

这是单 crate 内的上限；整构建还要扣掉 crate 间已有的并行（上节）。所以库内并行的价值集中在**关键路径上的大 crate**（cargo 库、regex-automata），对宽依赖图的整构建只有个位数到十几个百分点。

## 5. kfrc 的做法

| 做法 | 理由 |
|---|---|
| 名字解析结束后冻结语义快照，函数体任务只读快照 + 写自己的局部表 | 不需要 rustc 的查询等待与 deadlock handler（K2 阶段不变量、Sorbet 冻结全局表，[prior-art-layout.md](prior-art-layout.md)） |
| typeck root 为任务单位，固定 worker 数、确定性划分 | tsgo 4 checker 约 3×、允许重复工作（[prior-art-lazy-parallel.md](prior-art-lazy-parallel.md)）；输出顺序不依赖完成顺序（[design/build/pipeline.md](../../design/build/pipeline.md#确定性)） |
| 推导期类型进 body 局部表，发布时才进全局驻留 | rustc 把每个 `mk_*` 都送进全局分片表；全局 trait 缓存是单锁（[why-slow-infra.md](why-slow-infra.md)） |
| worker 从 Cargo jobserver 取 token，crate 内外共用一份预算 | 8 核满载时库内并行只增加 CPU（本页 §2、前期 P2b 的 CPU 地板） |
| 指标：每 crate 的 1/4/8 worker 墙钟、CPU、峰值内存 | 与本页同口径对比；rustc `-Zthreads=8` 只作参考臂（[design/goals/compile-time.md](../../design/goals/compile-time.md#指标与闸门)） |

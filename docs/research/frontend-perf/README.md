# 前端优化：kfrc 自写前端怎样才能比 rustc 快

范围：解析、宏展开、名称解析、typeck、trait 求解、借用检查、metadata 读写。基准是 rustc 1.94.1（`e408947b`）编译 ripgrep 15.2.0 与 Cargo 0.95.0。2026-11-08 调研。

标注：**实测** = 本目录脚本输出或前期材料原文；**源码** = 在 `e408947b` 核实的行；**推断** = 由前两者推出。源码链接固定到该 commit。

## 结论

1. **前端是 check 的全部、dev 的一半、release 的四分之一，但在关键路径上占比更高。** check 前端占全部工作 89–92%，dev 约 50%，release 23–25%（实测）。rg 的关键路径以库前端链为主（check 3.37/3.58 s）。rustc 的库在 rmeta 之前单线程，release 的前端里还有 15–22% 是本该放到接口之后的 MIR 优化。
2. **前端内部，typeck + trait + 借用检查约占一半**（check 52–53%）。解析/展开/名字解析/降低约占 17–18%，metadata 读写约 9%，MIR 构建 7%。增量编辑时情况完全不同：真正重查的 typeck/借用检查 ≤6%，依赖图 26–33%，不增量的展开 + 解析 + 降低 16–29%（实测）。
3. **rustc 慢在架构，不在某个热点。** 原因包括：展开、解析、降低全部串行；每个函数体建 AST/HIR/THIR/MIR 四种形态，借用检查还深拷贝一次 MIR；凡输出 metadata 就要稳定哈希全部 HIR（SVH）；每个下游进程重新解码 std 的同一批条目；增量依赖图的固定成本与 crate 大小成正比（源码 + 实测）。
4. **常被寄予厚望的两项其实很小。** 跨 crate 共享 trait 缓存，上限是 trait 时间的 12.5%，约占前端 1%（实测）。借用检查的 bitset 数据流只占借用检查的 4.5%，贵的是 MIR 类型检查与 region 求解（实测，nightly）。
5. **rustc 自己的并行前端整构建收益有限。** `cargo check` 8 线程墙钟 Cargo −6.2%、rg −15.1%，CPU 多 30–60%（实测）。原因是 crate 级并行已经占满 8 核，Cargo 的关键路径上还有 29–46% 是 C 构建脚本。
6. **先例给出的量级**：紧凑布局每阶段 20–40%（Zig）；惰性导入 −12…−20% 墙钟（Go）；共享内存并行检查 4 worker 约 3×（tsgo）；树与旁表合一后分析阶段 2.5–3 倍（Kotlin K2）。
7. **kfrc 单 crate 前端的上限估计（推断）**：墙钟约快 2.7–3.9×，CPU 省 15–26%。整构建冷编译受 crate 图和构建脚本限制，Cargo check 前端无限快也不到 2.2×。大倍数只能来自增量（去掉 O(crate) 固定成本）和缓存。
8. **复用 next-solver 要带闸门。** rustc 1.94.1 全局开启新求解器后，rg check 的 rustc 指令 ×1.148（实测）。

## 总表

| 问题 | 回答 | 文件 |
|---|---|---|
| 时间花在哪 | 四种场景 × 14 类前端分项；增量的固定成本 | [time-breakdown.md](time-breakdown.md) |
| 关键路径 | rmeta 前串行；签名就绪在 rmeta 的 10–17%；P1/P2a/P2b 实验 | [critical-path.md](critical-path.md) |
| 为什么慢（一） | 查询调度、依赖图与指纹、驻留与锁、metadata 编解码与 SVH | [why-slow-infra.md](why-slow-infra.md) |
| 为什么慢（二） | 展开与解析、四种 IR、trait 缓存与新求解器、借用检查拆分 | [why-slow-passes.md](why-slow-passes.md) |
| `-Zthreads` | 现状、实测扫描、Amdahl 上限 | [parallel-frontend.md](parallel-frontend.md) |
| 先例（一） | Zig、Carbon、Kotlin K2、Sorbet、Jai | [prior-art-layout.md](prior-art-layout.md) |
| 先例（二） | Go、Clang/Swift、rust-analyzer、tsgo | [prior-art-lazy-parallel.md](prior-art-lazy-parallel.md) |
| 设计建议 | 减少工作 / 并行 / 缓存 / 数据布局，各自上限 | [recommendations.md](recommendations.md) |
| 对照现有设计 | 13 条缺口：问题 / 证据 / 改法 / 目标文件 | [design-gaps.md](design-gaps.md) |

## 关键数字

| 量 | rg | Cargo | 来源 |
|---|---:|---:|---|
| 前端占全部工作：check / dev / release | 91.9% / 49.7% / 25.4% | 88.8% / 49.7% / 23.1% | `phase-matrix.sh` |
| typeck + trait + 借用检查占前端（check） | 52.7% | 52.4% | 同上 |
| MIR 优化占前端（release） | 14.9% | 21.7% | 同上 |
| 增量编辑：依赖图开销占前端 | 33.3% | 25.9% | 同上 |
| 关键路径上的前端（check） | 3.37 / 3.58 s | 12.23 / 28.4 s | `cargo-critical-path.sh` |
| `-Zthreads=8` 整构建墙钟 / CPU（check） | −15.1% / +60.2% | −6.2% / +29.7% | `threads-table.sh` |
| 跨 crate 重复的 trait 查询时间 | 12.5% | — | `trait-goal-dup.sh`（本机） |
| 新求解器全局开启，rustc 指令 | ×1.1479 | — | `next-solver-ab.sh`（本机） |
| 签名就绪 / rmeta 写出 | 10–17%（各库） | 10%（cargo 库） | 前期 P2b M0 |

## 优先建议（详见 [design-gaps.md](design-gaps.md)）

| 条目 | 一句话 | 目标文件 |
|---|---|---|
| G2 | 定义冻结点：签名、impl 索引、coherence 完成后才开始函数体 | `design/frontend/overview.md` |
| G1 | 补库内按 typeck root 并行的任务 | `plan/m6-quality-performance.md` |
| G5 | metadata 按 `DefKey` 索引、惰性解码、构建内共享 | `plan/m4-cargo-sysroot.md` |
| G4 | 接口指纹不含私有函数体和 span | `design/build/cache.md` |
| G9 | 借用检查用 region 旁表，不复制 Body | `design/frontend/body.md` |
| G10 | next-solver 复用加性能闸门 | `plan/m1/t12-trait-experiment.md` |

## 脚本（[scripts/](scripts/)）

| 脚本 | 作用 |
|---|---|
| `e2b-driver.sh`、`e2b-frontend-profile.sh` | E2B 上跑 check/dev/增量的 self-profile 与 `-Zthreads` 扫描 |
| `phase-breakdown.sh`、`phase-matrix.sh`、`top-labels.sh`（`phase-lib.jq` 定义分类） | self-profile 汇总、分项矩阵、最大 label |
| `cargo-critical-path.sh` | 由 Cargo `--timings` 回溯关键路径并拆分前端/codegen |
| `scenario-table.sh`、`threads-table.sh` | 场景总量、线程扫描表 |
| `trait-goal-dup.sh` | 跨 crate 重复的 canonical trait goal |
| `local-check-profile.sh` | 本机 nightly 借用检查子事件拆分 |
| `next-solver-ab.sh`、`ab-summary.sh`、`time-rustc-wrapper.sh` | 新旧求解器 ABBA（逐 rustc 进程计指令） |

## 数据来源

| 数据 | 平台与版本 |
|---|---|
| check / dev / 增量 self-profile、`-Zthreads` 扫描 | E2B 模板 `mirx-rust1941-linux`（8 vCPU / 16 GiB），官方 1.94.1，2026-11-08 |
| release self-profile 与 timings | 同模板另一 sandbox，2026-10-10（前期 rs-prof-profile 运行） |
| trait 重复、新求解器 A/B | 本机 macOS，Homebrew rustc 1.94.1（同 commit）；只用计数与比例 |
| 借用检查拆分 | 本机 nightly 1.101.0（`12433c307` 2026-10-24） |
| 前期材料 | rust-build-profile-20261008：next-directions-20261010 §4–§5、pass-cost-retrospective、pipelined-p1-20261010、p2a-staged-metadata-20261010、rustc-10x-rewrite-analysis-20261020；rustc-pipeline：p2b-m0、p2b-m2-m3 |

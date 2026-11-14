# rustc + cargo 的时间轴：优化发生在哪里

对象：cargo release 构建（opt-level=3、16 CGU、ThinLocal、`-Zthreads` 默认 1）。每段标注**串行/并行**和是否在 **crate 间关键路径**上。“关键路径”指下游 crate 开工或最终链接要等的那条链。

## 一个库 crate 的时间轴（源码核实）

| # | 阶段 | 其中的优化 | 并行度 | 关键路径 |
|---|---|---|---|---|
| 1 | 解析、宏展开、名称解析、AST→HIR | — | 串行（`-Zthreads` 也不并行宏展开） | 是（rmeta 之前） |
| 2 | typeck、borrowck（`analysis`，[passes.rs#L1144](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L1144)） | — | `par_hir_body_owners`，只有 `-Zthreads>1` 才并行 | 是 |
| 3 | MIR 构建与 **MIR 优化**（`optimized_mir`：内联、GVN、SROA、DestProp、JumpThreading 等） | 多态体，每个定义一次 | 被第 4、5 步按需拉起；`prefetch_mir` 只在 `threads != 1` 时并行（[encoder.rs#L2462](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/encoder.rs#L2462)） | 是 |
| 4 | **单态化收集 + CGU 划分**（`collect_and_partition_mono_items`） | `mono_reachable` 剪枝、LocalCopy/内部化决策 | 根之间可并行（需 `-Zthreads`） | **是**：库的 metadata 要 `exported_generic_symbols`，它调用收集与划分（[symbol_export.rs#L245](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/back/symbol_export.rs#L245)；rlib 总要算泛型导出表：[context.rs#L2277](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L2277)） |
| 5 | **写 rmeta**，发 artifact 通知（[fs.rs#L97](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/fs.rs#L97)）；先于 `codegen_crate`（[passes.rs#L1253](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L1253)） | 要求所有泛型/可跨 crate 内联项的 `optimized_mir`（[encoder.rs#L1099](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/encoder.rs#L1099)） | 串行 | 下游库从这里开工 |
| 6 | CGU → LLVM IR（`codegen_crate`，[base.rs#L683](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/base.rs#L683)） | — | **主线程串行**，逐个喂给 LLVM；`-Zthreads>1` 时首批并行（#L770，源码注释称之为“staircase”） | 否（库）；是（二进制） |
| 7 | 每个 CGU 的 **pre-link** 化简 | [pass-inventory-o3.md](pass-inventory-o3.md) | 每 CGU 一个 worker，受 jobserver token 限制；与第 6 步重叠 | 否（库，除非下游是二进制） |
| 8 | **ThinLTO 屏障** + 全局导入分析 | 导入决策、内部化 | 等所有 pre-link 结束；协调线程串行 | 同上 |
| 9 | 每个模块 **post-link**（重解析 bitcode、导入、化简 + 优化半段） | [thinlto-postlink.md](thinlto-postlink.md) | 并行 | 同上 |
| 10 | **后端** | [codegen-backend.md](codegen-backend.md) | 与第 9 步同一 worker，并行 | 同上 |
| 11 | 写 rlib；二进制 crate 链接 | 链接器 `--gc-sections` 丢弃死代码 | 串行 | 二进制：是 |

## cargo 层（实测 + 前期材料）

- **rmeta 流水线**：库依赖库时 `--extern` 指向 `.rmeta`，二进制依赖库时指向 `.rlib`（实测 `cargo-flags.sh`）。所以二进制 crate 要等所有依赖的第 7–11 步做完才开工。
- **jobserver**：rustc 主线程持有隐含 token，LLVM worker 每多一个就向 cargo 的 jobserver 多要一个（[write.rs#L1434](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/back/write.rs#L1434) 起的注释）。关键 crate 与非关键 crate 平等抢 token。
- **`-Zthreads`**：并行第 2–4 步和第 6 步首批；E2B 上在 Z 臂之上 rg 再 −6.2%、cargo 再 −9.8% 墙钟，产物逐字节相同（前期 next-directions §4.1）。

## 关键路径实测（前期 E2B，8 vCPU，原文）

| 项目 | 关键路径构成 | 说明 |
|---|---|---|
| rg A | regex-syntax → regex-automata → bstr → globset → ignore 的前端链 8.4 s（期间其他 crate 把机器占到 6.4 个 CPU）+ ignore 的 codegen 1.8 s + 最终 rg crate 5.7 s（平均只用 3.8 个 CPU） | next-directions §4.2 |
| cargo A | C 构建脚本 45.6 s + 等 token 16.6 s + 前端段 36.2 s（cargo 库约 31 s，期间只有 1.46 个 CPU 在忙）+ cargo 库 codegen 34.4 s + cargo bin 7.3 s | 同上 |
| CPU 构成 | LLVM 65%–70%（pre-link 26%、post-link 20%、codegen 19%，rg A），前端 23%–28%，IR 生成 5% | next-directions §5 |

## 放错位置的优化

| # | 现象 | 为什么位置不对 | 证据 |
|---|---|---|---|
| P1 | MIR 优化在 rmeta 之前 | 代价串行、在关键路径上；收益是 rmeta 之后的并行 LLVM 工作 | `--mirx=all`：MIR 侧 +1.48 s（84% 在 rmeta 前），LLVM 侧 −1.41 s，墙钟慢约 3%（前期 pass-cost-retrospective） |
| P2 | 库的单态化收集与 CGU 划分在 rmeta 之前 | 只为导出 `#[inline(never)]` 泛型实例的符号表 | 跳过（R 臂）：rg 墙钟 −0.9 个百分点，但指令 +1.8 个百分点（下游自己实例化），cargo 墙钟反而 +1.4 个百分点（next-directions §4.3） |
| P3 | 多态体只在 MIR 优化，单态化后不再有实例级简化 | 闭包、`F`、trait 调用要到 LLVM 内联后才化简，每个实例重做 | evidence.md §2；InstCombine 每函数约 8 次 |
| P4 | LocalCopy：`#[inline]`、小叶子、闭包、drop glue 复制到每个 CGU | 每份副本在各自 CGU 的 pre-link 里被单独化简 | rg A 同 crate 副本占 pre-link 输入 14.5%、进入 codegen 4.7%；跨 crate 同一实例占 pre-link 输入 17.4%、codegen 5.4%（next-directions §3） |
| P5 | post-link 对导入体重新化简 | 已化简的体在每个导入方再化简一次后丢弃 | 占 post-link 40%（[thinlto-postlink.md](thinlto-postlink.md)） |
| P6 | IR 生成在主线程串行 | 喂料阶段限制 LLVM worker | rg A IR 生成 5.7 s（5%）；源码注释“staircase effect” |
| P7 | ThinLTO 屏障 | 最大的 CGU 决定所有 post-link 的开始 | cargo 主库 post-LTO 最长 CGU 约 14.68 s，最短 5.74 s（前期 thinlto-report） |
| P8 | 二进制 crate 等全部 rlib；跨 crate LTO 把全部后端推到二进制 | 末端 crate 平均只用 3.8 个 CPU | P1 原型（bin 在 rmeta 齐备后启动）rg 墙钟 −8.4%/−9.8%/−9.6%（前期 pipelined-p1） |

前四项是“前端为后端准备得不够、又放在串行段”，后四项是“后端按 CGU 批处理带来的重复与屏障”。kfrc 的对应安排见 [timeline-kfrc.md](timeline-kfrc.md)。

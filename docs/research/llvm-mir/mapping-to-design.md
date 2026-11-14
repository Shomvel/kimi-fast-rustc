# LLVM pass → design 承接表

逐个 pass（或紧密的一组）写明 kfrc 在哪一级、用哪个机制承接。状态：**承接**（设计文件已写明）、**部分**（只覆盖 Rust 常见形状）、**不需要**（kfrc 的表示里这份工作不产生）、**尚缺**（设计没写，是否要做未决）、**推迟**（设计已明确推迟）。类别沿用 evidence.md：A 值在内存、B 薄包装、C enum/niche 与比较、D 事实、M 机器级、G 全局/链接期。次数是 ThinPre-O3 / ThinPost-O3 的静态出现次数（[pass-inventory-o3.md](pass-inventory-o3.md)）。

## 中端（opt 流水线）

| LLVM pass | 类 | 次数 | kfrc 机制 | 级别 | 状态 |
|---|---|---|---|---|---|
| `sroa` `mem2reg` | A | 3+1 / 3+1 | 聚合、enum、宽指针是 SSA 值；引用全部消失的 `StackSlot` 退回 SSA；S 级以“被删或被内联调用的实参槽位”为种子 | 构图、R、S | 承接 |
| `early-cse` `instcombine`（代数部分） | — | 2,5 / 1,8 | 构建期窥孔：常量折叠、代数化简、GVN 哈希查找；变化驱动 worklist | R、S、L | 承接 |
| `instcombine` `aggressive-instcombine`（niche/判别值部分） | C | 同上 | 变体操作 + 编码推迟到 L 级；`Discriminant` 规范化；derive `eq` 折叠成变体相等 | R | 不需要（不产生 niche 算术） |
| `simplifycfg` | B、C | 6 / 7 | 不可达控制 DCE；常量 Phi → 条件；`VariantSwitch` 比较链合并；L 级冷块放末尾、相同 panic 尾块合并 | R、L | 部分：`switch-to-lookup`（只在 post-link 末尾启用）与公共指令提升/下沉未写明 |
| `jump-threading` | C | 2 / 2 | 按前驱拆分：`VariantSwitch`/`If` 的输入是 `Phi` 且部分前驱是 `MakeVariant`/常量 | R | 部分：只覆盖变体与常量；一般谓词蕴含的线程化未写明 |
| `correlated-propagation` `constraint-elimination` | C | 2,1 / 2,1 | `Refine` 节点与类型上的范围事实；`Check` 消除；len 相关区间推理 | R | 承接（范围由 Rust 事实给出） |
| `sccp` `ipsccp` | C、G | 1,1 / 1,1 | 构建期常量折叠 + worklist（悲观） | R | 部分：乐观的条件常量传播（穿过循环 Phi）与过程间常量传播未写明（R1、R3） |
| `gvn`（纯值） | — | 1 / 1 | GVN 哈希，构建时 hash-consing | R、S、L | 承接 |
| `gvn`（load 消除、PRE） `licm`（load） | A、M | 1,3 / 1,4 | 分区内 load/store 转发；L 级“穿过分区 Phi 的 load 转发与 PRE” | R、L | 承接 |
| `dse` `memcpyopt` `mldst-motion` `move-auto-init` | A | 各 1 | move 是值转发；分区内死 store 删除；写回已知值的 store 删除 | R | 承接 |
| `inline` `always-inline` | B | 1+1 / 1+1 | 非泛型及实参转发的泛型在 R 级内联；闭包、trait 方法在 S 级；内联拼接已优化图、接缝作种子；跨 crate 用 build 级共享 S 图；dev 只内联 `#[inline]` 和小函数 | R、S | 承接 |
| `function-attrs` `rpo-function-attrs` | B | 2 / 2+1 | `CallSummary`：unwind、发散、内存效果、捕获；SCC 从 ⊥ 迭代 | 摘要 | 承接（在未优化图上，精度待 A/D/B） |
| `argpromotion` `deadargelim` | B | 1,1 / 1,1 | 无 | — | 尚缺（R2）：只读引用参数改按值、删死参数都要改内部实例的签名 |
| `called-value-propagation`、间接调用去虚 | B | 1 / 1 | `CallVirtual` 查答案表变直接 `Call`；整程序 vtable 并集列为后续扩展 | S | 部分 |
| `globalopt` | G | 2 / 2 | `Const` 分区上常量偏移的 load 折叠 | R | 部分：可变 static 从未写入等情形未写明（Rust 少见） |
| `globaldce` `elim-avail-extern` | G | 1 / 2,1 | 链接前在实例图上决定可达性（C6）；没有导入副本 | 链接前 | 承接 / 不需要 |
| `mergefunc` `constmerge` | G | 0 / 1,1 | 答案键去重（含 shim）；只读数据按内容哈希去重（rustc 精读 layout-abi-mono T19） | S、数据 | 承接 |
| `licm`（纯值） `loop-sink` | M | 3 / 4 | placement（GCM）把纯节点放到最浅的循环层 | L | 承接（SoN 固有）；`loop-sink` 对应“冷区域只做最小工作” |
| `indvars` | M | 1 / 1 | L 级归纳变量化简 | L | 承接 |
| `loop-rotate` `simple-loop-unswitch` `loop-deletion` | M | 1,1,1 / 2,1,2 | 无 | — | 尚缺（R3） |
| `loop-idiom` | M | 1 / 1 | `Repeat` 按阈值展开或调用 memset；一般循环识别为 memset/memcpy 未写明 | L | 部分（R3） |
| `loop-unroll(-full)` `loop-vectorize` `slp-vectorizer` `vector-combine` | M | 1,0,0,1 / 2,1,1,2 | 展开、向量化在 T38 之后单独验收 | L | 推迟 |
| `reassociate` `div-rem-pairs` `float2int` | M | — | L 级常量除法/取模（乘法逆元、`x % c == 0`）；重结合未写明 | L | 部分 |
| `tailcallelim` | M | 1 / 2 | 无 | — | 尚缺（影响小，Rust 尾递归少见；推断） |
| `speculative-execution` `chr` `callsite-splitting` | M | — | L 级 if-conversion（`Select` → `cmov`） | L | 部分 |
| `lower-expect`、assume、`infer-alignment` `alignment-from-assumptions` | D | — | 事实挂在类型和 `Refine` 上，不发 assume；冷区域结构识别 | R | 不需要 |
| `coro-*` `openmp-opt*` `libcalls-shrinkwrap` `pgo-icall-prom` | — | — | Rust 的协程由前端降成状态机；无 OpenMP/PGO | — | 不需要 |

## 分析

| LLVM 分析 | rg A 计时占比 | kfrc |
|---|---:|---|
| MemorySSA、AA（BasicAA、TBAA、ScopedNoAlias、GlobalsAA） | 1.3%、1.4% | 内存分区一直保留到 Machine IR 之前（memory-partitions.md），不做别名查询 |
| BPI/BFI | 1.2% | 结构性冷标记；不用 profile（E1 否定 PGO 分层） |
| 支配树、LoopInfo、SCEV | 1.9%（支配树/循环） | placement 与 L 级循环需要，只在 L 级算一次 |

占比来自 `census-passes.sh` 的 rg A 输出（[measurements.md](measurements.md)）。后端各项的承接见 [codegen-backend.md](codegen-backend.md#kfrc-的承接面)。

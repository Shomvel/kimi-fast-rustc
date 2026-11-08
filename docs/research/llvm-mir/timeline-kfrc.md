# kfrc 的时间轴：每类优化搬到了哪里

依据 design 的 levels.md、optimization-pipeline.md、compile-time.md、generics-specialization.md 和 design/build/pipeline.md。rustc 侧见 [timeline-rustc.md](timeline-rustc.md)。本文件是对设计的解读，标“推断”的是设计文件没有写明、由本文推出的结论。

## 时间轴

| # | 阶段 | 单位 | 并行 | 关键路径 |
|---|---|---|---|---|
| 1 | 解析、展开、名称解析 | 模块 | 模块间 | 是 |
| 2 | 定义与**接口发布**（不含任何函数体图） | crate | — | 下游类型检查从这里开工（C4） |
| 3 | 函数体检查 → CheckedBody | typeck root | 体之间 | 只有“接口体”（opaque、const、async、static）在接口之前 |
| 4 | 构建 R 图（构图时窥孔 + GVN 哈希）→ 登记 mentioned → **R 级优化** → 逐体发布 | 定义 | 体之间 | 下游**特化**等它，下游前端不等 |
| 5 | 实例可达 → 孔位答案 → 答案键查重 | 实例 | 实例之间 | — |
| 6 | 调用摘要（未优化 S 图上的廉价遍历，SCC 一起算） | 实例 / SCC | SCC 之间 | 调用者的内联决策等它 |
| 7 | **S 级**：特化 + 种子 worklist 优化 + 内联拼接 | 答案键 | 图之间 | — |
| 8 | **L 级**：布局降低、机器级窥孔、循环、if-conversion → placement → Machine IR → 线性扫描 → 对象片段 | 答案键 | 图之间 | — |
| 9 | 链接：只发射可达实例（C6）；库在空闲 token 时预生成 | 二进制 | — | 是 |

与 rustc 的根本区别：没有 rmeta 这一个“前端全部做完”的屏障，没有 CGU 这一层批处理，也没有 ThinLTO 的全局屏障。并行单位从“crate 内 16 个 CGU”变成“图”。

## 每类优化搬到了哪里

| LLVM 中的位置 | 优化 | kfrc 的位置 | 为什么更好 | 代价或风险 |
|---|---|---|---|---|
| pre-link 开头“前端输出清理”（SROA、EarlyCSE、SimplifyCFG） | 把每局部一个 alloca 的直译 IR 清理掉 | 第 4 步构图时：聚合是值、构建期窥孔 | 这份工作根本不产生（未优化 IR 的 23.3% 是 alloca/lifetime/memcpy） | 构图器要懂 Rust 语义（values-and-types.md） |
| rmeta 之前的 MIR 优化（串行，关键路径） | 多态体化简、MIR 内联 | 第 4 步，**接口发布之后**，体之间并行 | 不再推迟下游前端（P1） | 下游特化要等上游 R 图：首版跨进程是“R 图集合”一道屏障（prior-art ct-parallel-pipeline），等于把 rmeta 里“带优化 MIR”的那部分等待留了下来，只是不再挡下游前端（推断） |
| rmeta 之前的单态化收集与 CGU 划分 | 实例集合、LocalCopy/内部化 | 第 5 步，按需、逐实例；没有 CGU | 不挡接口（P2）；没有 LocalCopy（P4） | 任务粒度细，调度与对象片段数增加 |
| pre-link CGSCC 内联管线（每实例、每 CGU） | 内联、函数化简、属性推断 | 第 6–7 步：摘要先算，S 级只处理孔位替换和内联接缝 | 工作量正比于 `T` 影响到的区域（C3）；不随 CGU 复制 | `size_estimate` 来自未优化 S 图，可能高估被调者、少内联（推断，需 A/D/B） |
| FunctionAttrs（化简后、CGSCC 内） | nounwind、只读、不捕获 | 第 6 步，优化之前 | 调用图深度不再变成串行等待 | 精度低于“化简后再推断”，optimization-pipeline.md 已要求用 A/D/B 量 |
| ThinLTO 导入 + post-link 再化简 | 跨 CGU 内联 | 第 7 步：内联源是 build 级缓存的已优化 S 图，拼接后只跑接缝 | 去掉 post-link 的 40%（P5）；也去掉屏障（P7）和 bitcode 往返 | 作为内联源的 S 图要缓存，占内存和缓存空间（cache.md） |
| post-link 优化半段（向量化、展开、LICM、LoopSink） | 循环与向量 | 第 8 步 L 级；展开、向量化在 T38 后单独验收 | 只在单态、布局已知时做，每答案键一次 | 验收前的运行时缺口没有数据（对照：只去掉跨 CGU 内联的 O 臂已慢 +1.50%…+27.83%，前期 E2B） |
| post-link 的 IPO（IPSCCP、GlobalOpt、DeadArgElim、ArgPromotion、GlobalDCE、MergeFunctions） | 过程间常量、改签名、删死代码、合并函数 | GlobalDCE→C6 可达性；MergeFunctions→答案键去重；其余**未写明** | 去重与可达性在链接前的实例图上完成，覆盖面大于 LLVM 的模块内 | 见 [recommendations.md](recommendations.md) R1 |
| 后端（每模块，在 post-link 之后） | 指令选择、寄存器分配、调度、布局 | 第 8 步，每答案键一次、跨 crate 共享（C2） | 跨 crate 的同一实例只生成一次（rg Z 臂跨 crate 重复占 codegen 输入 5.4%） | 逐函数生成失去模块级的东西（常量合并、函数排序），推断影响小 |
| 链接器 `--gc-sections` | 删死代码（已编译代码的 22–31%） | 第 9 步之前，按实例图可达性 | 不再为死代码付后端时间 | 后端工作集中到二进制 crate，见下 |

## 新的串行点（推断，需要在 M5 实测）

| 位置 | 为什么会串行 | 现有对照数据 | 设计里的缓解 |
|---|---|---|---|
| 二进制 crate 的末端生成（C6） | 可达性只有在二进制 crate 才完整；库预生成之外的部分都在这里 | rg 最终 crate 5.7 s、平均 3.8 个 CPU；cargo 开跨 crate LTO 时库只出 bitcode，后端全部落到二进制（实测 `cargo-flags.sh`） | 库预生成（compile-time.md）；需要“预生成命中率”指标 |
| 跨进程“R 图集合”屏障 | 下游特化等上游整个 crate 的 R 级优化 | P2b 在 8 vCPU 上只兑现 −3.3%：提前开工后 CPU 争用吃掉收益 | 逐体发布；按下游请求优先优化被请求的体（P2a/P2b 的“请求优先”） |
| 调用摘要的 SCC | 一个大 SCC 内的摘要要一起算 | 无 | 摘要在未优化图上，遍历便宜 |
| 单图内部串行 | 一个巨型函数（serde 反序列化器）就是一个任务 | cargo 单次 SLP 264 ms、GVN 165.5 ms 的病态函数（前期 trace-hotspots） | 节点上限与燃料；冷区域只做最小工作（C5） |

## 净效果的判断

- 已测的杠杆在 8 vCPU 上基本是**降 CPU 总量才降墙钟**：rg 构建 CPU/8 约 13 s，P1 之后调度空间只剩约 3 s（前期 p2b-m2-m3）。所以 kfrc 时间轴的主要收益来自 C1/C2/C6 去掉的工作，而不是把同样的工作提前。
- 位置调整（接口先发、无 CGU 屏障）的价值在核数多、依赖链深的项目上才显出来；评估时按 prior-art ct-parallel-pipeline 的建议在 4/8/16 核各测。

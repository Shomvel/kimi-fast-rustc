# 对 kfrc 的结论：必须做的优化与 ≤1.0 的可行性

依据：[attribution-results.md](attribution-results.md)（本轮实测）、[hotspots-ripgrep.md](hotspots-ripgrep.md)、[hotspots-cargo.md](hotspots-cargo.md)、[prior-data.md](prior-data.md)。目标定义见 [design/goals/runtime.md](../../design/goals/runtime.md)，计划见 [plan/m6-quality-performance.md](../../plan/m6-quality-performance.md#t38)。

## 后端必须做到的优化（按影响排序）

| # | 要求 | 不做的代价（实测上界） | 对设计的影响 |
|---|---|---|---|
| 1 | **显式 SIMD：`core::arch` intrinsic 逐个降成内联指令，运行时分派结果与 rustc 一致**（AVX2 → SSSE3/SSE2 → 退路） | 全无：rg 单文件 3.442–10.137×、目录搜索 2.146–2.929×；只缺 AVX2：1.257–2.770× | runtime.md 已把它提前；至少覆盖 memchr 的 14 个、aho-corasick 的 26 个、hashbrown 的 9 个 intrinsic，`is_x86_feature_detected!` 只能报告后端已实现的特性 |
| 2 | **intrinsic 永不成为调用**：即使内联预算不够、跨编译单位、调用者没有同样的 `target_feature`，也要内联或直接按 `Vector` 节点降低 | 内联失败时 dir-regex cycles 1.392，其中 `_mm256_movemask_epi8` 作为函数占 21.9% | 与 #1 一起做：`core::arch` 函数不走普通内联决策 |
| 3 | **跨 crate、跨编译单位的小函数内联**（闭包、迭代器、集合、`Mutex::lock`、serde 访问器） | I 臂：Cargo lockfile 1.207（cycles 1.301），rg dir-files 1.128（1.130）；L 臂同量级；前期 O 臂 rg 至 1.278（cycles 1.448） | runtime.md 的“S 级内联已优化的图”必须能拿到上游 crate 的函数体；内联覆盖面比单次内联的质量更重要 |
| 4 | **字节循环的向量化**：至少“逐字节比较 + 计数/查找”这类归约循环 | rg lines/ctx/json 1.152–1.494（cycles 1.057–1.181）、official 1.097，其余 workload ≤1.014 | 不需要通用循环向量化；一个针对 `count_byte_by_byte` 形状的 SLP/循环规则即可。可以排在 #1–#3 之后，但在 ripgrep 验收前必须有 |
| 5 | **寄存器分配接近 LLVM basic**（线性扫描 + 区间拆分 + 远者优先 spill + 循环外拆分） | rg ≤1.060（cycles ≤1.066），Cargo ≤1.019 | ct-backend 选的方案（[prior-art/ct-backend.md](../prior-art/ct-backend.md)）足够；热点是状态机循环 |
| 6 | 循环展开、跳转表、GVN load/PRE、LICM 提升 | 各自 ±3% 以内，cycles 落在噪声里 | 可以 T38 之后再做；runtime.md 中“向量化 **高** 风险”可下调为“只需 #4” |

叠加估计：C 臂（V+U+R+J，即“首版后端缺 #4–#6”）rg 搜索类 1.001–1.060、输出类 1.172–1.489，Cargo 1.006–1.050。所以做到 #1–#3 后，剩下的缺口约 1–6%，要靠 #4 和中层优势补回。

## 每个 workload 的可行性

| workload 组 | 决定性因素 | 只做到 #1–#3 的预期 | ≤1.0 可行性 |
|---|---|---|---|
| rg official（验收） | 内核 78%、启动、glibc malloc；用户态热点分散 | 墙钟差别淹没在噪声里（S0 墙钟 1.013，spread 308.1%） | **形式上容易“通过”，但不能证明什么**；见 [workloads.md](workloads.md) |
| rg 单文件字面量（lit、lit-i、lit40、suffix、unicode） | 显式 SIMD 占 57.1–91.5% 用户态 cycles | 剩 R（≤1.052）与少量内联 | **可行，前提是 #1/#2 完全对齐**；否则 1.5–10× |
| rg 稠密 DFA（dfa） | 查表循环，对所有臂 1.000–1.003 | ≈1.0 | **高** |
| rg lazy DFA（word500） | 状态机 + 寄存器压力 | R 1.060（cycles 1.035） | **中**：需要分配器在状态机循环上不多 spill |
| rg 输出（lines、ctx、json） | memchr 计数首尾循环 | 无 #4 时 1.152–1.494 | **中**：做了 #4 才可能 |
| rg 目录（dir-files、dir-regex、dir-mt） | 内核 63–78%、malloc、路径处理、ignore 的 glob DFA | 剩 R ≤1.042 | **中–高**：墙钟被内核稀释 |
| Cargo 启动类（metadata-nodeps 验收、noop-build、package-list） | C 库（libcrypto）13.5–33.0%、内核 30–35% | C 臂 1.006–1.016 | **高**（比值被 C 库与内核拉向 1.0） |
| Cargo 解析类（metadata-full、tree、lockfile） | 内联覆盖面、分配、哈希 | C 臂 1.029–1.050 | **中**：要靠 #3 完全到位，再用堆分配消除、move 转发抵消 2–5% |

**总体判断：** “runtime ≤1.0” 在 ripgrep 上取决于显式 SIMD 与 intrinsic 内联两件事，做不到就是数量级的差距（与 Cranelift 的 4–15× 同一来源）；做到了，剩余缺口与 Cargo 一样落在 1–6%，主要来自寄存器分配和展开，需要中层优化补回。没有单个 workload 显示“必须有通用自动向量化”。

## 对计划文档的建议（不在本轮修改）

| 位置 | 建议 |
|---|---|
| [design/goals/runtime.md](../../design/goals/runtime.md) “必须自己做的部分” | 新增一行“intrinsic 不得成为调用”；向量化风险从“高”改为“只需字节归约循环（#4）”；寄存器分配风险“中”，上界 ≤6% |
| [plan/m0-baseline.md](../../plan/m0-baseline.md) T00 | 修正“Cargo MSRV 1.94，1.94.1 满足”：`cargo` 包是 1.96，libsqlite3-sys 0.38.1 用 `cfg_select!`，1.94.1 编不过 0.99.0；要么换 Cargo 0.95.0，要么换参考 rustc |
| 验收协议 | 主指标改为用户态 `instructions:u`/`cycles:u` + 完整 workload 集；两个验收 workload 只做冒烟 |
| M0 | 采用 [m0-experiment.md](m0-experiment.md) 的归因实验，作为 T38 排序依据 |

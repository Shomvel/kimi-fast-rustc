# ripgrep 与 Cargo 的运行时画像

kfrc 的目标之一是 runtime 比值 `kfrc / (rustc+LLVM) ≤ 1.0`（[design/goals/runtime.md](../../design/goals/runtime.md)）。本目录汇总前期 runtime 数据，补测两个项目的热点与“去掉一项 LLVM 优化”的归因实验（2026-11-08，HK E2B，rustc 1.94.1 / LLVM 21.1.8），给出 kfrc 后端必须做到的优化和 M0 实验设计。

## 结论

1. **ripgrep 的 runtime 由显式 SIMD 决定。** 单文件搜索 57.1–91.5% 的用户态 cycles 在 memchr/memmem 的 AVX2 路径或 Teddy 里。去掉全部显式 SIMD，单文件 workload 指令数 3.442–10.137×；只去掉 AVX2 分支也要 1.257–2.770×。
2. **intrinsic 必须内联成指令。** LLVM 内联预算降为 0 或关掉跨 CGU 内联时，`_mm256_movemask_epi8` 成了真实函数调用，rg dir-regex 的 cycles 到 1.392（I）/ 1.381（L）。这与前期 Cranelift SHA 慢 57.82× 是同一机制。
3. **Cargo 没有单一热点，靠内联覆盖面。** 内联是唯一的大项（resolver workload 指令 1.207×、cycles 1.301×），多出来的是闭包、BTreeMap 迭代器、`Mutex::lock` 等小泛型函数；显式 SIMD ≤1.035×（cycles）。
4. **自动向量化只在一处显著：** memchr 计数函数的逐字节首尾循环，影响 rg 带行号的输出（ctx/json 1.489/1.494×、lines 1.152×、official 1.097×），其余 workload ≤1.014×。不需要通用向量化器。
5. **寄存器分配、展开、跳转表、GVN load/PRE、LICM 提升都是小项：** basic 分配器 rg ≤1.060、Cargo ≤1.019；其余各项 ±3% 以内。V+U+R+J 叠加：rg 搜索类 1.001–1.060，Cargo 1.006–1.050。
6. **现有两个验收 workload 测不出编译器质量。** rg official 内核占 78%，去掉全部显式 SIMD 指令数 3.117×，墙钟中位数只差 1.3%（spread 308.1%）；Cargo metadata-nodeps 用户态 33.0% 在 libcrypto。≤1.0 应在完整 workload 集上、以用户态计数判定。
7. **可行性：** 做到“显式 SIMD 对齐 + intrinsic 永不调用 + 跨 crate 内联”后，剩余缺口约 1–6%，需要中层优化（堆分配消除、move 转发）补回；做不到前两项，ripgrep 会是 2–10× 的差距。

另有两条与计划相关的发现：Cargo 0.99.0 用 rustc 1.94.1 编不过（`cargo` 包要求 1.96，libsqlite3-sys 0.38.1 用了 `cfg_select!`），本轮被测 Cargo 改用 0.95.0；rg 的目录 workload 内核占 63–78%，不是约 95%。

## 总表

| 实验 / 因素 | ripgrep 影响 | Cargo 影响 | 机器 | 来源 |
|---|---|---|---|---|
| 去掉全部显式 SIMD（S0） | 单文件 3.442–10.137×，目录搜索 2.146–2.929×（指令） | ≤1.033× | E2B 8 vCPU | [attribution-results.md](attribution-results.md) |
| 去掉 AVX2 分支（S2） | 1.257–2.770×（单文件） | ≤1.005× | 同上 | 同上 |
| LLVM 内联阈值 0（I） | ≤1.245×；cycles 至 1.392 | 1.021–1.207×；cycles 至 1.301 | 同上 | 同上 |
| `-Clto=off`（L，本轮） | 1.003–1.301×；cycles 至 1.381 | 1.015–1.170×；cycles 至 1.268 | 同上 | 同上 |
| `-Clto=off`（O 臂，建在 B 上） | 指令 +1.50%…+27.83%，cycles 0.972…1.448 | +5.79%…+8.71%，cycles 1.036…1.190 | E2B stage1 | thinlto-postlink 笔记（2026-10-09/10）§4 |
| 关自动向量化（V） | lines/ctx/json 1.152–1.494×，official 1.097×，其余 ≤1.014× | ≤1.004× | E2B | [attribution-results.md](attribution-results.md) |
| basic 寄存器分配（R） | ≤1.060×（cycles ≤1.066） | ≤1.019× | E2B | 同上 |
| 关展开（U）/跳转表（J）/GVN load+PRE（G）/LICM 提升（H） | 0.971–1.012× | 0.999–1.024× | E2B | 同上 |
| V+U+R+J（C） | 1.001–1.489× | 1.006–1.050× | E2B | 同上 |
| 整条流水线 O1 | 1.003–1.651× | 1.024–1.183× | E2B | 同上 |
| PGO 冷函数 optnone（B2/B1） | 同模式 1.018–1.036；held-out `-F -f` 2.037 | — | macOS aarch64 | rustc 10× 重写分析（2026-10-20）§5 |
| Cranelift 替换 LLVM（release） | 搜索 4.06–14.63× | metadata-full 4.07×、tree 3.31× | E2B nightly | Rust LLVM 与 Cranelift 编译对比（2026-10-09） |
| `-Zshare-generics=y` | 搜索 +3.1–7.5%，help +10.67% | +4.7–6.2% | E2B stage1 | monoexp 第二轮笔记（2026-10-09） |
| LLVM candidate（ThinLTO、CGU1） | 0.931（rg official） | 0.988（metadata-nodeps） | yicloud-dev | kfrc VALIDATION.md（commit 138723c） |

本轮比值都是 `instructions:u` 中位数比（噪声 ≤0.5%），注明 cycles 的除外。完整前期数据见 [prior-data.md](prior-data.md)。

## 文件

| 文件 | 内容 |
|---|---|
| [prior-data.md](prior-data.md) | 前期 runtime 数据：机器、workload、各实验影响、出处 |
| [workloads.md](workloads.md) | 本轮 22 个 workload、内核占比、验收 workload 的问题 |
| [hotspots-ripgrep.md](hotspots-ripgrep.md) | rg 各搜索模式的热路径、SIMD 分派、intrinsic 清单 |
| [hotspots-cargo.md](hotspots-cargo.md) | Cargo 的热路径（TOML、resolver、启动、文件系统） |
| [attribution.md](attribution.md) | 归因实验的臂、flag 核验、指标与局限 |
| [attribution-results.md](attribution-results.md) | 归因结果表（rg、Cargo 的指令与 cycles） |
| [llvm-dependence.md](llvm-dependence.md) | 热点 × 优化依赖矩阵，实测与推断 |
| [kfrc-implications.md](kfrc-implications.md) | kfrc 必须做的优化排序、各 workload 的 ≤1.0 可行性、对计划的建议 |
| [m0-experiment.md](m0-experiment.md) | M0 归因实验设计：workload、臂、指标、判定标准 |
| [reproduce.md](reproduce.md) | 脚本顺序、环境、构建记录、指纹、踩过的坑 |
| [scripts/](scripts/) | 全部 bash 脚本 |

相关：[prior-art/ct-backend.md](../prior-art/ct-backend.md)（后端算法选择）、[llvm-mir/thinlto-postlink.md](../llvm-mir/thinlto-postlink.md)（ThinLocal 的编译与运行时代价）、[evidence.md](../evidence.md)（前期编译侧剖析）、[plan/m0-baseline.md](../../plan/m0-baseline.md)。

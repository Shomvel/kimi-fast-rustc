# 热点对 LLVM 优化的依赖

对每个热点给出它依赖哪些优化，以及去掉该优化时的 runtime 上界。“实测”取自 [attribution-results.md](attribution-results.md)（指令比；括号内为 cycles 比，只在该 workload 的 A 臂 spread ≤5% 时给出），“推断”是没有单独实验支撑的判断。

## 依赖矩阵

| 热点（代表 workload） | 显式 SIMD | 自动向量化 | 展开 | 内联 | 寄存器分配 | 跳转表 | LICM/GVN |
|---|---|---|---|---|---|---|---|
| memmem 前过滤（lit、suffix、unicode） | **S2 1.497–2.118，S0 3.442–5.008** | 1.000–1.001 | 1.000–1.002 | I ≤1.076，L ≤1.087 | ≤1.029 | ≤1.002 | ≤1.005 |
| Teddy（lit-i、lit40） | **S2 1.825–2.770（lit40 1.518），S0 4.434–10.137（lit40 3.562）** | 1.000 | 1.000 | I ≤1.019；**L 1.205–1.301（lit40 1.160）** | 1.028–1.052（lit40 1.032） | 1.000 | G/H 0.973–1.005 |
| lazy DFA（word500） | S0 1.139（1.123），走的是行边界 memrchr | 1.000 | 1.000 | I 1.097（1.036），L 1.156（1.094） | **1.060（1.035）** | 1.000 | ≤1.008 |
| 稠密 DFA（dfa） | 1.000–1.003 | 1.000 | 1.000 | ≤1.003 | 1.001 | 1.000 | 1.000 |
| 行号计数（lines、ctx、json） | S2 1.257–2.135，S0 3.587–9.699 | **1.152–1.494** | 0.971–1.000 | ≤1.120 | ≤1.042 | 0.987–0.997 | ≤1.008 |
| 目录遍历 + ignore（dir-files） | 0.998–1.005 | 1.001（0.992） | 1.006（1.015） | **I 1.128（1.130），L 1.097（1.119）** | 1.013（1.027） | 1.000 | ≤1.012 |
| 目录搜索（dir-regex、dir-mt） | S2 1.193–1.306，S0 2.146–2.929（2.608、1.785） | 1.001 | ≤1.004 | **I 1.162–1.217（1.232–1.392），L 1.161–1.190（1.256–1.381）** | 1.017–1.042（1.023–1.066） | ≤1.002 | ≤1.006 |
| 启动与排版（rg help、cargo build-help） | 1.000 | ≤1.014 | ≤1.003 | I 1.044/1.084，L 1.088/1.070 | ≤1.025 | 1.000 | ≤1.007 |
| Cargo 解析（metadata-full、tree、lockfile） | S0 1.009–1.033 | ≤1.004 | **1.005–1.024** | **I 1.108–1.207（1.230、1.301），L 1.085–1.170（1.160、1.268）** | 1.018–1.019（1.015、1.019） | ≤1.004 | ≤1.005 |
| Cargo 启动（metadata-nodeps、noop-build、package-list） | ≤1.016 | ≤1.002 | ≤1.005 | I 1.021–1.042（1.055、1.082），L 1.015–1.034（1.053、1.096） | ≤1.008 | ≤1.001 | ≤1.001 |

## 逐项结论

| 优化 | 实测上界 | 机制 | 推断 |
|---|---|---|---|
| 显式 SIMD（`core::arch`，运行时分派） | rg 单文件 S0 3.442–10.137×；目录搜索 2.146–2.929×；Cargo ≤1.033× | memchr/memmem 的 AVX2 字节对、Teddy 的 `pshufb`；S2 切到 SSE2/SSSE3 也要 1.257–2.770× | 必须与 rustc 走同一分派分支，并把每个 intrinsic 生成为内联的指令 |
| 内联 | rg cycles 最高 1.392（dir-regex），Cargo 最高 1.301（lockfile） | rg：`_mm256_movemask_epi8` 等 intrinsic 不内联，成了函数调用（占 dir-regex 21.9%）；Cargo：闭包、BTreeMap 迭代器、`Mutex::lock` 等小泛型函数 | 去掉 intrinsic 的那部分后，rg 剩余的内联依赖接近 Cargo 量级（dir-files 的 I 1.128 里 intrinsic 只占 ≤1.0%） |
| 跨 CGU / 跨 crate 内联（ThinLocal） | 与 I 臂同量级：rg 1.003–1.301，Cargo 1.015–1.170 | 同上；前期 O 臂 rg 到 1.278、cycles 1.448 | kfrc 的内联必须跨 crate、跨“编译单位”，否则等同 L 臂 |
| 自动向量化 | 只在 memchr `count_byte_by_byte` 上显著：ctx/json 1.489/1.494，lines 1.152，official 1.097 | 逐字节计数循环（显式 SIMD 主循环的首尾）在 `target_feature(avx2)` 函数里被 LLVM 向量化 | 一个能处理“字节比较 + 计数归约”的循环向量化即可覆盖；Cargo 完全不依赖 |
| 寄存器分配质量 | basic 分配器：rg ≤1.060（cycles ≤1.066），Cargo ≤1.019 | 状态机循环（lazy DFA、Teddy 确认）对 spill 敏感；稠密 DFA 不敏感 | 线性扫描 + 拆分 + 远者优先 spill 应接近 basic，预期缺口 ≤6% |
| 循环展开 | rg ≤1.006（lines 0.971，展开反而多指令），Cargo ≤1.024 | 热循环在源码里已手工展开（memchr、regex-automata） | 可以晚做 |
| 跳转表 / switch 降低 | 0.987–1.004 | 热点里没有密集 switch | 可以晚做；体积和启动路径可能仍要 |
| GVN load 消除 + PRE | 0.973–1.012 | EarlyCSE、InstCombine 仍在，替代了大部分 | 只关了子功能，不能代表整个 GVN |
| LICM 标量提升 | 0.973–1.007 | — | 外提（hoisting）本身没关；SoN 的全局调度天然外提纯运算 |
| 整体流水线 O1 | rg 1.003–1.651，Cargo 1.024–1.183 | — | “简单优化器”的参考，与 C 臂（1.001–1.489）同量级 |

## 与 Cranelift 的对照

前期 Cranelift release（[prior-data.md](prior-data.md)）在 rg 64 MiB 搜索上 9.73–14.63×、Cargo metadata-full 4.07×。本页 S0（rg 单文件 3.442–10.137×）加 I/L（rg cycles 至 1.392）能解释 rg 的大部分差距，SHA 消融（57.82× → 4.39×）是同一机制的直接证据；Cargo 的 4.07× 远超本页任一单臂，说明 Cranelift 在通用标量代码上还缺多项叠加（推断，前期未拆分）。

## 没有覆盖的依赖

| 优化 | 为什么没量 | 已知信息 |
|---|---|---|
| SROA、move/memcpy 消除、边界检查消除、enum 判断折叠 | LLVM 没有单独关闭它们的稳定开关，关掉等于拆掉 InstCombine/SROA | kfrc 打算由语义直接得到（[design/goals/runtime.md](../../design/goals/runtime.md) “Rust 语义直接带来的部分”） |
| 堆分配消除 | 同上 | Cargo 解析类 workload 的 allocator 占 17.6–20.0%，是 kfrc 可能反超的地方 |
| 过程间常量（IPSCCP）、GlobalOpt | 需要“按 pass 名跳过”的插件 | [llvm-mir/recommendations.md](../llvm-mir/recommendations.md) R1 建议先量；本轮未做 |
| 冷热分层 | 另需 PGO 训练 | 前期 E1：profile 没覆盖的 `-F -f` 路径慢 2.037× |
| 预编译 std 内部的非泛型代码 | 本轮 flag 只作用于 Cargo 编译的 crate | 若 kfrc 自编 std（M4），这部分也要计入，可用 `-Zbuild-std` 臂补测 |

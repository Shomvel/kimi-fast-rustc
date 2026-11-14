# e-graph 与重写系统

## 1. 是什么

| 系统 | 来源 | 年份 |
|---|---|---|
| Cranelift aegraph mid-end | RFC [cranelift-egraph.md](https://github.com/bytecodealliance/rfcs/blob/main/accepted/cranelift-egraph.md)（原型 [wasmtime#4249](https://github.com/bytecodealliance/wasmtime/pull/4249)）；Fallin, [ægraphs 幻灯片](https://cfallin.org/pubs/egraphs2023_aegraphs_slides.pdf)（EGRAPHS '23）；Fallin, [The acyclic e-graph: Cranelift's mid-end optimizer](https://cfallin.org/blog/2026/04/09/aegraph/) | 2022 / 2023 / 2026-04 |
| ISLE | RFC [cranelift-isel-isle-peepmatic.md](https://github.com/bytecodealliance/rfcs/blob/main/accepted/cranelift-isel-isle-peepmatic.md)（2021-08）；Fallin, [ISLE: Term-Rewriting Made Practical](https://cfallin.org/blog/2023/01/20/cranelift-isle/) | 2021 / 2023-01 |
| ISLE 规则验证 | VanHattum 等, [Crocus](https://cfallin.org/pubs/asplos2024_veri_isle.pdf)（ASPLOS '24）；[Arrival](https://dl.acm.org/doi/10.1145/3764383) | 2024 / 2025 |
| egg / egglog | [egg](https://dl.acm.org/doi/10.1145/3815481)（CACM 研究亮点）；Zhang 等, [Better Together (egglog)](https://arxiv.org/abs/2304.04332)（PLDI '23） | 2021 / 2023 |
| eggcc | Flatt 等, [Efficient Extraction for Effectful E-graphs](https://rtjoa.com/papers/oopsla26-eggcc.pdf)（OOPSLA '26） | 2026 |

**aegraph 的机制（Fallin 2026）：** 有副作用的指令留在块里构成“副作用骨架”，纯节点浮动；按支配树前序构建时做浅哈希合并（GVN）；节点**创建时立即重写**，结果用 union 节点挂在原值上，节点创建后不再修改，所以图无环，一遍完成；elaboration 从骨架根按需放置（DCE 免费），用随支配树作用域的哈希表做 GVN，同时做 LICM 和再物化；抽取用拓扑序自底向上 DP，不计共享。RHS 的递归重写“在层数和每轮总调用数上都有上限”（文中未给具体数值）；`subsume` 标记的结果删掉 e-class 其他成员。规则集刻意不含结合律、交换律等非化简恒等式。

## 2. 关键证据

| 量 | 数字 | 来源 |
|---|---|---|
| RFC 原型 vs 旧 mid-end（GVN/LICM/preopt/别名分析） | 编译时间 SpiderMonkey +1%、bz2 −15%；运行时间 −13% / −3% | RFC |
| egg 原型往返开销 | 无重写时 23%（最初 >2×）；被替换的旧流水线只占约 10% 编译时间 | Fallin 2026 |
| aegraph vs 同规则的经典流水线 | 生成代码快约 2%（重复经典流水线 2–3 次追不上）；编译时间多 7–8%（2023 年持平，差距来自规则增长） | Fallin 2026 |
| 有 union vs 只取第一个重写结果（eager pick） | 运行时间约 0.1%，编译时间约 0.005%，“基本是噪声” | Fallin 2026 |
| 平均 e-class 大小 | 1.13 个 e-node | Fallin 2026 |
| 一遍重写错过的等价 | 约 400 万值节点中 2 次 | Fallin 2026 |
| ISLE 规模 | 4 个目标的 lowering + mid-end 重写，2.7 万行 DSL；overlap 检查器发现若干永远不触发的规则 | ISLE 博客 |
| eggcc（Bril，96 个基准） | 抽取外平均 7.925 s + 抽取 1.536 s；LLVM -O3 平均 0.834 s；代码质量与 LLVM 相近 | OOPSLA '26 §8.1 |
| egg 的生产使用 | Cube（SQL 重写）、Intel 硬件工具；Fallin 在 HN 说先用 egg 原型，因性能和工程问题换成自研 aegraph | egg CACM；[HN](https://news.ycombinator.com/item?id=35612391) |
| egglog 在生产编译器中 | 未找到原始来源（只找到研究与教学用例） | — |

Fallin 的结论：SoN + CFG 骨架“works great”；多表示（union）“may not (yet?) be pulling its weight”。

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| equality saturation（egg/egglog/eggcc） | **不采纳（维持现设计）** | eggcc 的编译时间约为 LLVM -O3 的 11 倍；与 C3“工作量与变化量成正比”冲突 |
| aegraph 的 union / 多版本 e-class | **不采纳** | 在生产数据上收益约 0.1%，平均 e-class 1.13；kfrc 的 R/S 规则同样以化简为主 |
| 创建时立即重写 + 规则只做化简 + 递归有上限 | **采纳** | 就是 Click 的构建期窥孔；Cranelift 证明一遍重写只错过 400 万分之 2。kfrc 应把“规则必须单向化简”写成规则准入条件 |
| elaboration：按需放置 + 作用域 GVN + LICM + 再物化 | **采纳到 placement** | L 级的 placement 一次完成 GVN、LICM、DCE；再物化（常量、地址）减少寄存器压力 |
| `subsume` | **采纳** | 规则可以声明“结果严格更好”，直接替换不保留旧节点 |
| 声明式规则 DSL（ISLE 式），编译成 Rust `match` | **改编** | R/S 级重写、L 级窥孔、Machine IR 指令选择用同一种规则格式；先做 overlap/优先级检查，后做 SMT 验证。直接依赖 `cranelift-isle` 是否违反“不用 Cranelift”红线需要单独决定；默认自写简化版 |
| 规则验证（Crocus/Arrival） | **改编，M6 之后** | 对 L 级整数窥孔和地址模式规则做 SMT 检查；Crocus 复现了一个 x86-64 地址模式 CVE |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) | 共同机制 | “构建期窥孔”补充三条准则：规则必须按固定代价单向化简（不加交换律/结合律）；RHS 递归重写有层数和次数上限；可标记 `subsume`。引用 Cranelift 数据：一遍重写在约 400 万节点中只错过 2 次 |
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) | 不做 | “equality saturation”一条改写为：不做 equality saturation，也不保留多版本 e-class（aegraph union）；理由列 eggcc 约 11× LLVM -O3 编译时间、Cranelift union 收益约 0.1% |
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) | L 级 | “placement → Machine IR”改为“elaboration 式 placement”：从固定节点按需放置浮动节点，同时做作用域 GVN、LICM、常量/地址再物化 |
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) | 新增“规则表”一节 | R/S/L 规则和 Machine IR 选择共用一种声明式规则格式，生成 Rust `match`；构建时检查同优先级重叠和永不触发的规则 |
| [design/README.md](../../design/README.md) | 任务 | T38 增加“规则表 overlap 检查”与“L 级整数规则的 SMT 验证（可选）” |

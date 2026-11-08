# 先例三：经验证编译器——CompCert、CakeML

核实日期 2026-11-08。

## CompCert

| 方面 | 内容 | 状态 |
|---|---|---|
| 来源 | Leroy, [Formal Verification of a Realistic Compiler](https://xavierleroy.org/publi/compcert-CACM.pdf)（CACM 2009）；Leroy, [A Formally Verified Compiler Back-end](https://arxiv.org/pdf/0902.2137)（JAR 2009）；[v3.18 文档](https://compcert.org/doc/) | 已核实 |
| 多级 IR | CompCert C（`Csem.v`，另有参考解释器 `Cexec.v`）→ Clight → Csharpminor → Cminor → CminorSel → RTL → LTL → Linear → Mach → Asm；20 个 pass，`Compiler.v` 串起来 | 已核实 |
| 每级都要语义 | CACM §2.4：“所有中间语言都必须有恰当的形式语义” | 已核实 |
| simulation | JAR 定义后向模拟（目标行为蕴含源行为）与前向模拟（反之）；前向“更容易证但信息更少”；目标**确定**时前向推出后向；通用图式有 lock-step、star、plus 等（JAR §3.7 图 4） | 已核实 |
| UB | 未定义的 C 行为统一变成“going wrong”；定理只对 `B ∉ Wrong` 成立 | 已核实 |
| 值 | `Vundef` 表示任意位串；`Val.lessdef`：`lessdef v v`、`lessdef Vundef v`——级间值关系是“更有定义” | 已核实（[Values.v](https://compcert.org/doc/html/compcert.common.Values.html)） |
| 内存 | 权限 Cur/Max（Freeable/Writable/Readable/Nonempty）；`Mem.extends`（块变大、`Vundef` 换成更有定义的值）用于 Allocation、Tailcall、CSE、Constprop；`Mem.inject f`（块映射到不重叠子块）用于 Cminorgen、SimplLocals、Inlining | 已核实（[Memory.v](https://compcert.org/doc/html/compcert.common.Memory.html)） |
| 翻译验证 | 图着色寄存器分配、CFG 线性化事后验证；[Allocation](https://compcert.org/doc/html/compcert.backend.Allocation.html) 标注“validation a posteriori”；CACM 称惰性代码移动和调度也用经验证的验证器 | 已核实 |
| 工作量 | CACM：约 42,000 行 Coq，约 3 人年；编译器 14%、语义 10%、证明 76%；每个 pass 1,500–3,000 行；**每级 IR 的语义 300–600 行**（Clight 1,100）；共享基础 10,000 行。JAR 后端：约 37,000 行 Coq + 1,000 行 Caml，约 2 人年 | 已核实 |

## CakeML

| 方面 | 内容 | 状态 |
|---|---|---|
| 来源 | Owens 等, [Functional Big-Step Semantics](https://kar.kent.ac.uk/53533/1/paper.pdf)（ESOP '16）；Tan 等, [The Verified CakeML Compiler Backend](https://kar.kent.ac.uk/71304/1/paper.pdf)（JFP 2019） | 已核实 |
| 语义形式 | 函数式大步语义“本质上是纯函数风格写的解释器，带一个时钟保证全函数”；时钟设得很大时，**证明用的语义可以直接执行测试程序**；时钟也用来证明发散被保持；I/O 与非确定由状态中的 oracle 建模 | 已核实 |
| 多级 IR | 第 2 版 12 种中间语言，按值分三组：带闭包的抽象值、无闭包的抽象值、机器字与内存；每种有 `evaluate` 解释器（时钟作燃料）和观察语义 `semantics` | 已核实 |
| 证明 | 前向模拟，存在量化额外时钟；“因为 IL 完全确定”所以足够 | 已核实 |
| 规模 | 约 10 万行 HOL4 证明脚本；人年数未找到原始来源 | 部分核实 |

## 要点

两者的共同点：每一级 IR 都有自己的语义，相邻级之间只证明一种关系（CompCert 的 simulation、CakeML 的带时钟前向模拟），而**语义本身都是可执行或接近可执行的**（`Cexec.v`、`evaluate`）。代价集中在证明，不在语义。

## 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| 每级一套语义、相邻级之间有关系 | **采纳（可执行版本）** | 这是 R/S/L 三级要回答的问题；CompCert 的经验是每级语义 300–600 行 Coq，写语义本身不贵，贵的是证明（76%） |
| 机器检查证明 | **不采纳** | 3 人年 / 10 万行 HOL4 量级；kfrc 的目标是快速编译器，不是认证编译器 |
| 可执行语义 = 解释器 | **采纳 CakeML 的路线** | 同一份定义既是规范又能跑测试；kfrc 用 Rust 写解释器，以燃料保证终止 |
| 值关系 `lessdef` | **改编** | kfrc 的 poison 相当于 `Vundef` 在 refinement 中的地位：poison 可被任何值 refine |
| `Mem.extends`/`Mem.inject` | **改编** | S→L 不改内存（三级同一字节内存），所以只需值关系 `encode`；L→Machine 的栈帧布局才需要 inject 式的块映射（推断） |
| 前向模拟 + 确定性 | **采纳思路** | kfrc 解释器对非确定点取确定选择，“同输入同输出”的差分就是前向模拟的可执行近似 |
| 寄存器分配事后验证 | **已采纳**（[ct-testing.md](../prior-art/ct-testing.md)） | 与本文的 placement checker 同类 |

# 与完整计划的关系

PoC 先做中后端，完整计划的 M1–M4 后做前端与 sysroot。PoC 写出的大部分代码直接进入产品。

## 代码去向

| PoC 部分 | 去向 | 对应任务 |
|---|---|---|
| `kfrc-son`：图、节点、verifier、规则 DSL、R/S/L 优化、特化、摘要 | 直接进入产品 | T07、T09、T17、T25、T38 |
| `kfrc-x86`：L 级降低、Machine IR、线性扫描、编码、对象片段 | 直接进入产品 | T08、T17、T18、T19 |
| 小解释器（整数、聚合、enum） | 进入产品，后续扩展到完整语义 | T07、T17 |
| build 级 R 图与机器码缓存 | 进入产品，键的来源换成自有前端的依赖清单 | T39 |
| `TypeOracle` 接口 | 进入产品；PoC 的 tcx 实现丢弃，换成自有前端实现 | T06、T12、T15、T21 |
| MIR → Body 转换器 | 丢弃；它暴露的契约缺口回改 Body 契约 | T06 |
| codegen backend 外壳 | 丢弃（总览固定决策在产品中不变） | — |

## PoC 能提前回答的问题

| 问题 | 完整计划中原本何时知道 |
|---|---|
| R/S/L 设计在真实代码上是否成立，哪些节点、规则不够 | M3–M5 |
| 中后端编译工作能省多少，花在哪一级 | M6 |
| 不做向量化时，运行时间差多少，差在哪 | M6 |
| 特化身份键的去重效果，嵌入实例的比例 | M3–M5 |
| Body 契约能否表达真实 MIR | M1–M3 |

## PoC 不回答的问题

| 问题 | 由谁回答 |
|---|---|
| 自有前端的正确性与速度（[frontend-perf](../../research/frontend-perf/README.md) 估计单 crate 前端快 2.7–3.9×） | M1–M3 |
| 原生 sysroot 的工作量（hello world 一档 32–48 人周） | M3–M4 |
| next-solver 复用的性能（1.15× 指令数） | T12 |
| 接口先发布与逐体等待的流水线收益 | T40 |
| unwind、SIMD、浮点、`dyn` 的完整支持 | M3–M6 |

## 对完整计划顺序的影响

如果 PoC 的主张 2–4 成立，建议完整计划改成“中后端先行”：M1 的纵向链直接用 PoC 的 `kfrc-son`/`kfrc-x86`，自有前端按 `TypeOracle` 与 Body 契约接入；若不成立，PoC 的归因决定是调整设计还是停止。这一判断在 P4 报告中给出，不预先改动 M0–M6。

## 契约缺口清单

PoC 过程中发现的 Body 契约、`TypeOracle`、答案种类的缺口记在这里，P4 结束时批量回改 design。

| 编号 | 发现于 | 缺口 | 处理 |
|---|---|---|---|
| G1 | P0 标量链 | `Assert` 的失败路径要调用 panic lang item，并传 `&'static Location`；原 Body 没有数据对象与重定位 | 2026-11-09 已加入 `DataObject`、`DataRelocation` 和符号地址常量；Location 复用 rustc CTFE 的原始 allocation；除零/越界/溢出 panic 的消息和位置已通过 E2B 差分 |
| G2 | P0 标量链 | `#[track_caller]` 被调者的 ABI 比 MIR 实参多一个隐式 `Location` | 转换器已按真实 FnAbi 归一化参数并维护 MIR local 映射，调用时补位置，进入 tracked 函数后转发调用方位置；源位置不进入 SoN 算术优化 |
| G3 | P0 标量链 | match guard 的绑定是 `&T`（共享借用），纯标量程序也会出现引用 | 2026-11-09 已实现 StackSlot、Load/Store 和统一效果 token；guard 与共享/可变引用通过 E2B 差分。Local/Const 的精细分区优化仍未完成 |
| G4 | P0 标量链 | `-Coverflow-checks=on`（cargo 默认 debug）产生 `AddWithOverflow` 等返回 `(T, bool)` 的运算 | 2026-11-09 已实现聚合结果与各位宽溢出检测；开启 overflow checks 的内存负载和溢出 panic 通过 E2B 差分 |
| G5 | P1 真实 MIR | rustc mono collector 会剪掉编译期已知分支；逐块转换会引用未采集的函数（如关闭 UB checks 时的 precondition_check） | 转换器使用 rustc 的 `mono_reachable_as_bitset` / `mono_successors`；dead 块保留编号但不转换内容。带未定义 extern 符号的恒 false 分支已通过链接回归 |
| G6 | P3 semver | 同一个 Rust 类型在不同调用中的 Cast ABI 不能用类型级缓存表示；部分 lane、padding、扩展和寄存器分组也不能退化成普通整数 | 已在每次调用的 `PassMode::Cast(CastAbi)` 记录真实 lanes 和扩展；候选 06 的双向 C ABI 机器对照通过，包含尾字节与寄存器耗尽。浮点、向量 Cast 和 C IndirectOnStack 仍明确拒绝 |

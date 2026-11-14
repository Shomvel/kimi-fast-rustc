# T13 实验：move、借用与 drop — 细化

`M1 · B · 依赖：T02, T03, T06` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t13) · rustc 笔记：[ownership](../../research/rustc/ownership/README.md)、[borrowck](../../research/rustc/borrowck/README.md)

loan 何时失效完全由它的 region 包含哪些程序点决定，所以“借用实验”离不开最小 NLL。本任务拆成两个子项，T13.a 先做（liveness 依赖初始化数据流）。

## T13.a move、初始化与 drop 显式化

可直接借用的算法，规模约 4–5k 行 rustc 代码，只需六个类型查询：`needs_drop`、是否有 `Drop` impl、Box/union/`ManuallyDrop` 判定、ADT 字段与变体、数组长度、是否 `Copy`。

- move path 前缀树与 lookup；`check_movable_place`（E0507/E0508/E0509）。
- 前向位集数据流框架；MaybeInit / MaybeUninit / EverInit（E0381、E0382、E0384）。
- drop 显式化：Dead / Static / Conditional / struct 与元组的 Open；drop flag 是普通 `bool` 局部。
- 作用域出口：`let`、`if`、`loop`、`return`；一个带副作用 `Drop` 的类型。

推迟：精确 enum drop、数组/slice/Box/union 的 open drop、match guard/or-pattern/fake 借用、`super let`、edition 2024 rescope。

## T13.b loan 与最小 NLL

Body 上需要的事实：引用类型的 region 槽；`Ref` 带 `BorrowKind`（Shared/Mut/两阶段/Fake）和自己的 region；adjustment 中的 `Deref`、`Borrow(kind, two_phase)`；结构化 Place；`StorageDead`；`Drop` 点；签名生命周期（T05 产出）。

算法（都不依赖 TyCtxt）：

- universal region 表：`'static`、签名参数、`fn_body`，及已知关系。
- 约束生成：赋值子类型（带方差）、重借用约束、调用点 late-bound 实例化。
- use-liveness（“出现即 live”）；SCC + 后序并集；universal region 检查。
- `BorrowSet`、`Borrows`（含 out-of-scope 预计算，整局部赋值和 `StorageDead` 使 loan 失效）、`places_conflict`、访问矩阵、两阶段 `is_active`（需支配树）。

推迟（遇到即报未支持）：闭包、`impl Trait`、`for<'a>`、泛型 `T: 'a`、投影的隐含界、真 dropck、Polonius。

## 输入闭包

结构体字段读取、可变借用与重借用、分支部分 move、提前返回、一次有副作用的析构。代表性用例取自 [borrowck/test-cases.md](../../research/rustc/borrowck/test-cases.md) 的“首轮子集”和 [ownership/test-cases.md](../../research/rustc/ownership/test-cases.md)。

oracle 是 NLL（1.94 默认关闭 Polonius；NLL problem case #3 被拒绝）。比较“接受/拒绝 + 错误码 + 主 span”，不移植 rustc 约 14.5k 行的诊断代码。

## 依赖 T15 之前的类型

实验需要 struct 和 `&`/`&mut`，早于 M2 的 T15。本任务只为实验实现最小 struct 与引用类型，不对外开放；生产化在 T16/T22。

## 估算（推断）

T13.a 约 2–3k 行，T13.b 约 3–4k 行。

## 完成标志

两个子项各自的正反例与 oracle 一致；析构 CFG 可 dump 检查；依赖成本报告列出实际用到的 rustc 函数与行数。真实运行的析构次数和顺序由 T22/T27/T26 验收。

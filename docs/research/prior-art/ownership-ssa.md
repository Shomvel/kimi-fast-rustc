# 所有权感知的 SSA：Swift OSSA、Mojo、Hylo

## 1. 是什么

| 系统 | 来源 | 年份 |
|---|---|---|
| Swift OSSA | [docs/SIL/Ownership.md](https://github.com/swiftlang/swift/blob/main/docs/SIL/Ownership.md)、[docs/SIL/SIL.md](https://github.com/swiftlang/swift/blob/main/docs/SIL/SIL.md)（Borrow Scopes 一节）；Gottesman, [SIL Ownership Model](https://gottesmm.github.io/drafts/sil-ownership-model/) | 2017 起；持续更新 |
| Swift 词法生命周期、copy propagation | [Swift 5.7 发布说明](https://www.swift.org/blog/swift-5.7-released/)；[6.2 编译时间退化讨论](https://forums.swift.org/t/poor-scaling-with-optimized-builds-involving-ossa-copy-propagation-in-the-6-2-compiler/80436) | 2022 / 2025 |
| Mojo | [Ownership](https://docs.modular.com/mojo/manual/values/ownership)、[Value destruction](https://docs.modular.com/mojo/manual/lifecycle/death/)、[Lifetimes/origins](https://docs.modular.com/mojo/manual/values/lifetimes) | 2023 起 |
| Hylo（原 Val） | Racordon 等, [Implementation Strategies for Mutable Value Semantics](https://www.jot.fm/issues/issue_2022_02/article2.pdf)（JOT 2022）；Racordon, Borrow checking Hylo（IWACO '23）；[Hylo IR lowering 文档](https://docs.hylo-lang.org/hylo-ir/concepts/lowering) | 2022–2023 |

**OSSA 要点：** `[ossa]` 函数里每条 def-use 边带所有权含义。值的所有权种类：Owned（每条路径恰好被消费一次）、Guaranteed（作用域内借用，依附于外层值）、None（平凡类型、所有地址）、Unowned。`begin_borrow`/`load_borrow` 开启显式借用作用域，`end_borrow` 必须**联合后支配**作用域内所有使用。`struct_extract` 等转发指令在借用内形成隐式作用域；owned 值不能部分消费（用 `destructure_struct` 整体拆）。内部指针（`ref_element_addr` 等）的所有传递使用都算作对基对象的活跃要求。verifier 在 OSSA 降低前检查：漏消费 = 泄漏，消费两次 = use-after-free。只到达 `unreachable` 的 dead-end 块可以不消费。

**Hylo IR：** 类型化 SSA，`access [let, inout, sink]` 请求能力，`end_access` 结束；“access reification”在用法确定后按 `let < set < inout < sink` 选最弱能力；借用检查在 IR 上做。

**Mojo：** ASAP 析构——值在最后一次使用后立即析构（每个子表达式之后），字段独立析构；`@explicit_destroy` 可选择手工时机；通配 origin 的指针活跃期间关闭 ASAP。

## 2. 关键证据

| 事实 | 数字或原文 | 来源 |
|---|---|---|
| copy propagation 自 5.7 起开启，“消除虚假拷贝的重要优化” | — | 6.2 讨论帖 |
| 6.2 开启 OSSA modules 后暴露“非法且可观察地缩短生命周期”的 bug；修复需要把活跃区间延伸到 dead-end 块 | 2048 个属性的 init：6.0 4.6 s → 6.2 2629 s → 缓解后 11 s；1024 个：1 s → 226 s → 2.4 s | 同上 |
| 根因 | 生命周期在 SILGen 后补全，但后续 pass 让部分生命周期重新不完整；“OSSA 生命周期在整条流水线上完整后才能收回编译时间” | 同上（Nate Chandler） |
| 词法生命周期 | 缩短生命周期不能越过“deinit barrier”（同步点、weak/unowned load、指针访问等）；内联时为实参加 lexical borrow | Ownership.md；5.7 发布说明 |
| SIL 优化器的编译时间占比 | “通常约占编译时间的三分之一”，其余大部分是 LLVM | [Whole-Module Optimization](https://www.swift.org/blog/whole-module-optimizations/)（2016） |

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| 每个需要 drop 的 SSA 值在每条路径上恰好被消费一次（move、`DropValue`、`forget`），由 verifier 检查 | **采纳** | kfrc 的 move 是值转发、drop 是节点，正好可以做 OSSA 式线性检查；能直接抓住优化器复制或丢失 drop 的错误 |
| dead-end 块豁免 | **改编** | Swift 的教训是“豁免 + 后续 pass 破坏完整性”导致活跃性反复重算。kfrc 只豁免以 `Unreachable`/`Terminate`/`Resume` 结尾的冷区域，且规则只在 verifier 里算，优化不依赖它 |
| `begin_borrow`/`end_borrow` 联合后支配 | **采纳到 `Reborrow`/`EndBorrow`** | 内联产生的分区必须在每个出口（正常、unwind）关闭；从该分区派生的地址不能在 `EndBorrow` 之后使用 |
| copy propagation、缩短生命周期 | **不采纳** | Rust 没有隐式拷贝；drop 时机是语言语义（作用域结束、逆序），不能像 Mojo ASAP 或 Swift eager-move 那样提前 |
| deinit barrier 的概念 | **采纳** | `DropValue`/`DropInPlace` 解析到用户 `Drop` 或未知 drop glue 时，是 `Unknown` 上的效果节点，不能跨其他 `Unknown` 效果重排 |
| 所有权种类写进值类型（Owned/Guaranteed） | **不采纳** | kfrc 的借用由分区表达，所有权由 Body 的 drop 显式化决定；再加一套种类会与分区重复 |
| Hylo 的 access reification（先请求能力集合，后选最弱） | **不采纳** | Rust 的借用种类在 Body 上已确定 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md) | drop | 新增规则：“线性消费”——`NeedsDrop` 可能为真的值在到达 `Return` 的每条路径上恰好被一个消费者使用（作为 move 实参、`DropValue`、`mem::forget`、写入内存）；到达 `Unreachable`/`Terminate` 的路径豁免 |
| [ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md) | drop | 写明 drop 不提前：`DropValue` 的位置由 Body 决定；只有 `NeedsDrop=false` 时删除；解析到用户 `Drop` 的 drop 不跨 `Unknown` 效果移动 |
| [design/README.md](../../design/README.md) | 架构与契约 · design/ir/verifier.md | 新检查：`OwnershipLinear`（上一条）、`BorrowScopeClosed`（`EndBorrow` 联合后支配 `Reborrow` 的所有使用，含 unwind 出口）、`UseAfterEndBorrow`（派生地址在 `EndBorrow` 后被使用） |
| [memory-partitions.md](../../design/ir/memory-partitions.md) | 规则 5（内联） | 补“每个出口”的含义：所有正常和 unwind 出口联合后支配被调者体内的分区使用 |

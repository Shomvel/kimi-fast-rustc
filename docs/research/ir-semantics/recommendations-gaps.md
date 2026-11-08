# 对设计的建议（二）：语义空白与流程

健全性问题见 [recommendations-soundness.md](recommendations-soundness.md)。本文件是不直接造成误编译、但让语义说不清或让检查无从下手的空白。

## G1 新建语义文件

- **问题：** 设计文件描述了节点、事实和规则，但没有一处定义“程序的行为是什么”：可观察行为、UB 列表、非确定点、级间关系都散在各处或缺失。
- **证据：** [findings.md](../walkthroughs/findings.md) 的 14 条健全性发现里，11 条写语义时就会被问到或被检查模式直接抓到（[issue-origins.md](issue-origins.md)）；CompCert 原则：“所有中间语言都必须有形式语义”。
- **改法：** 新建 `design/ir/semantics.md`（约 300 行），内容：可观察行为与 refinement 定义；值域（[values.md](values.md)）；UB 点与 poison 来源；节点四类；效果 token 与浮动 load 义务；分区两类；非确定点（地址、NaN、分配省略）；级间关系（[refinement.md](refinement.md)）。其他设计文件引用它，不再各自描述语义。
- **目标文件：** `design/ir/semantics.md`（新）、`design/README.md` 的文件表。

## G2 分配不是可观察事件

- **问题：** `Alloc`/`Dealloc` 配对删除（F35）没有写出语义依据；用户自定义 `#[global_allocator]` 时是否仍可删除未说明。
- **证据：** `GlobalAlloc` 文档：分配函数可以非确定地跳过分配器、可以合并；“分配是否发生不属于程序行为”，即使分配器有副作用。
- **改法：** `semantics.md` 的可观察行为排除分配器调用；[nodes.md](../../design/ir/nodes.md#地址与内存) 的 `Alloc` 行引用它；与 rustc 一致地对任何全局分配器成立。
- **目标文件：** `design/ir/nodes.md`、`design/ir/semantics.md`。

## G3 R 的语义以答案为参数

- **问题：** “R 级不出现布局常量”容易被读成“R 级语义与布局无关”；但 `Store`、`TraitCall`、`DropValue` 的含义都依赖答案。
- **证据：** [refinement.md](refinement.md#r-的语义以答案表为参数)。
- **改法：** [levels.md](../../design/ir/levels.md#r-级rust-语义多态) 写明 ⟦R⟧ 是答案表的函数；[generics-specialization.md](../../design/ir/generics-specialization.md#特化) 写明 `specialize` 的正确性是“代入定理”（结果与 ⟦R⟧_A 相等），R 级 pass 的义务对所有答案成立；R 解释器的“读过的孔位”记录作为答案键的动态审计。
- **目标文件：** `design/ir/levels.md`、`design/ir/generics-specialization.md`。

## G4 规则表带语义标志

- **问题：** [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#共同机制) 的规则表有“模式、结果、前提、宽度、overflow 模式”，没有判断规则是否会投机、是否引入 UB 点。
- **改法：** 每条规则加 `poison_producing`、`introduces_ub_point`、`speculates`、`float_bits` 四个标志；后三者为真时 SMT 查询附加相应前提。标量语义只写一次，同时生成解释器的求值函数与 SMT 编码，≤8 位穷举对拍二者（[uses-and-cost.md](uses-and-cost.md#规则验证)）。
- **目标文件：** `design/ir/optimization-pipeline.md`#共同机制。

## G5 移位与其他整数模式

- **问题：** 整数运算模式表只有加法与浮点转整数；Rust 的 `<<`/`>>` 对移位量取模（MIR `Shl` 用 `rem_euclid`），`unchecked_shl` 越界是 UB；有符号除法的 `MIN / -1` 未列。
- **证据：** [syntax.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/syntax.rs#L1619) 的 `BinOp` 注释；[operator.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src/interpret/operator.rs#L112) 的移位处理。
- **改法：** 表中补 `Shl.mask`（Rust 语义）、`Shl.unchecked`（poison）、`Div`/`Rem`（立即 UB：除零与 `MIN/-1`，由 `Check` 守卫）。
- **目标文件：** `design/ir/values-and-types.md`#整数运算模式。

## G6 `VariantField` 与部分 load 的语义

- **问题：** `VariantField` 在变体不符时是 UB 还是 poison 没写；`FieldLoad`/`VariantOfAt` 只解码部分字节，与整值 `Load` 的 UB 集合不同。
- **改法：** `VariantField` 变体不符得 poison（于是它是纯节点，`UnprovenVariant` 保证它不被用错）；`FieldLoad` 只校验被读字段。整值 `Load` + 投影 → `FieldLoad` 合法（UB 变少），反方向需要证明其余字节有效。
- **目标文件：** `design/ir/nodes.md`#纯值、#地址与内存。

## G7 unwind 的值与可观察性

- **问题：** `Landing` 产出的 `Exception` 值、`Terminate` 与 abort 的可观察差别未定义。
- **改法：** `Exception` 是不透明值，只能被 `Resume` 消费、经 Phi 合并；可观察行为里 panic 的终止方式（unwind 到 `main` 外、abort）按 `PanicStrategy` 答案区分，panic 消息与 `Location` 是外部调用的实参（F52：内容对优化器不透明，对可观察行为透明）。
- **目标文件：** `design/ir/ownership-drop-unwind.md`#unwind、`design/ir/semantics.md`。

## G8 解释器进验证流程

- **问题：** 总览红线 3 规定内部 evaluator 只用于定位；但现在没有任何计划任务产出三级解释器。
- **改法：** T07（verifier）同时产出检查模式的接口；T17 交付 R/S/L 解释器首版（[uses-and-cost.md](uses-and-cost.md#首版范围与-m2-的-t17-同期)）；T03 的差分 runner 增加“逐 pass 解释器对拍 + bisect”通道，不替代真实执行。
- **目标文件：** `plan/m1/t07-son-verifier.md`、`plan/m2-types-memory-abi.md`#t17、`plan/m0/t03-differential-runner.md`。

## G9 CTFE 与解释器共享核心

- **问题：** 设计规定 CTFE 解释 Body，但没有说与 IR 解释器的关系，两套值/内存实现会各自漂移。
- **改法：** CTFE 与 IR 解释器共用 `Value`、`AbstractByte`、分配与 provenance、`encode/decode`；差异经类似 rustc `Machine` 的 trait 注入（CTFE：禁止堆分配逃逸、裸指针比较受限；oracle：分区检查）。
- **目标文件：** `design/README.md`#与-rustc-精读笔记的术语差异（现在唯一写“常量求值解释 Body”的地方）、`design/ir/semantics.md`。

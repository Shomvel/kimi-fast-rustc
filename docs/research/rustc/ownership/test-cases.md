# 代表性测试用例（tests/ui）

全部位于 `tests/ui/` 下，链接固定到 `e408947b`。“类型”：run = `run-pass`/`run-fail` 运行对拍；err = 期望编译错误；pass = `check-pass`。挑选原则：每条覆盖一个前面笔记中的具体规则，可直接作为 kfrc 差分用例或拒绝分类用例。

## move path 与非法 move

| 用例 | 类型 | 考察规则 |
|---|---|---|
| [borrowck/borrowck-move-out-of-struct-with-dtor.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-move-out-of-struct-with-dtor.rs) | err E0509 | 带 `Drop` 的结构体不能在 match/let/参数模式中移出字段 |
| [moves/move-out-of-slice-1.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/moves/move-out-of-slice-1.rs) | err E0508 | slice 上的常量下标不能 move |
| [borrowck/borrowck-move-out-from-array-no-overlap.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-move-out-from-array-no-overlap.rs) | pass | 数组正/反向常量下标与子切片不重叠时可分别 move |
| [borrowck/borrowck-move-out-from-array-use.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-move-out-from-array-use.rs) | err | 重叠时报 use after move（子切片元素逐个检查） |
| [borrowck/borrowck-union-move.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-union-move.rs) | err | move 一个 union 字段等于 move 整个 union |
| [borrowck/borrowck-move-from-unsafe-ptr.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-move-from-unsafe-ptr.rs) | err E0507 | 经裸指针 deref 不能 move |
| [moves/moves-based-on-type-tuple.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/moves/moves-based-on-type-tuple.rs) | err E0382 | 部分 move 后使用整体 |

## 初始化与重新初始化

| 用例 | 类型 | 考察规则 |
|---|---|---|
| [borrowck/borrowck-partial-reinit-1.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-partial-reinit-1.rs) | err E0382 | 带 Drop 的结构体整体 move 后给字段赋值被拒 |
| [nll/issue-21232-partial-init-and-use.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/nll/issue-21232-partial-init-and-use.rs) | err | `let s: S; s.x = ..` 部分初始化一律拒绝（`check_parent_of_field`） |
| [borrowck/borrowck-init-in-fru.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-init-in-fru.rs) | err E0381 | FRU `..origin` 读未初始化变量 |
| [borrowck/borrowck-reinit.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-reinit.rs) | err E0382 | 重新赋值恢复初始化，诊断只指向最后一次 move |
| [borrowck/borrowck-and-init.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-and-init.rs) / [borrowck-or-init.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-or-init.rs) | err E0381 | 短路右侧中的赋值只在一条路径上初始化 |
| [moves/issue-72649-uninit-in-loop.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/moves/issue-72649-uninit-in-loop.rs) | err | 循环内声明的变量每轮重新开始（StorageDead 作为 move） |
| [borrowck/borrowck-uninit-in-assignop.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/borrowck/borrowck-uninit-in-assignop.rs) | err E0381 | 复合赋值要求 LHS 已初始化 |
| [nll/maybe-initialized-drop-with-fragment.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/nll/maybe-initialized-drop-with-fragment.rs) | err E0506 | 部分 move 后剩余字段仍要 drop，借用因此存活（初始化数据流 × NLL 交界） |

## drop 显式化与析构顺序

| 用例 | 类型 | 考察规则 |
|---|---|---|
| [drop/dynamic-drop.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/dynamic-drop.rs) | run, unwind | 核心回归：在第 N 次分配/析构处注入 panic，逐个 N 检查无重复释放、无泄漏；覆盖条件 init/drop、字段赋值、slice 模式、if-let guard、`panic_after_*`（#47949 已知泄漏用 `alloc_leaked` 标出） |
| [drop/conditional-drop-10734.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/conditional-drop-10734.rs) | run | 条件 move 后的 drop flag |
| [drop/issue-90752.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/issue-90752.rs) | run | 经 `*(&mut foo) = ..` 写入后，精确 enum drop 不能漏 drop |
| [drop/struct-field-drop-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/struct-field-drop-order.rs) | run | 字段按声明顺序 drop |
| [mir/mir_drop_panics.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/mir/mir_drop_panics.rs) | run-fail | 一个局部的 drop panic 后，其余局部仍在 unwind 中 drop |
| [drop/terminate-in-initializer.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/terminate-in-initializer.rs) | run | `let _x: Box<_> = return/break/continue/panic!()` 不 drop 未初始化绑定 |
| [drop/issue-23338-ensure-param-drop-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/issue-23338-ensure-param-drop-order.rs) | run | 参数晚于体内临时值和局部 drop |
| [drop/nested-return-drop-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/nested-return-drop-order.rs) | run | 结构体字面量求值到一半 `return`：已求值字段的临时值在 return 路径上 drop |
| [mir/mir_early_return_scope.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/mir/mir_early_return_scope.rs) | run | `let conn = &*match ..` 被延长的临时值在后续 `return` 路径上 drop |
| [mir/mir_dynamic_drops_1.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/mir/mir_dynamic_drops_1.rs)（_2/_3） | run-fail | panic 前后每个值恰好 drop 一次（`Droppable` 拒绝二次 drop） |

## 作用域与临时值

| 用例 | 类型 | 考察规则 |
|---|---|---|
| [drop/tail-expr-drop-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/tail-expr-drop-order.rs) | run, 2024 | 块尾临时值先于块内局部 drop；跨 edition 宏 |
| [drop/tail-expr-drop-order-negative.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/tail-expr-drop-order-negative.rs) | 2021 pass / 2024 err E0716 | 同一代码在两个 edition 结果相反 |
| [drop/drop_order_if_let_rescope.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/drop_order_if_let_rescope.rs) | run, 2024 | `if let` scrutinee 临时值在 else 前 drop |
| [drop/drop-order-comparisons.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/drop-order-comparisons.rs) | run, 2021/2024 | `if let`、let 链、let-else、块尾在两个 edition 下的完整事件序列 |
| [drop/for-expr-temporary-drop-scope.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/for-expr-temporary-drop-scope.rs) | run | `for` 头部临时值活到循环结束（`DropTemps`） |
| [drop/super-let-tail-expr-drop-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/super-let-tail-expr-drop-order.rs) | run | `super let` 与块尾规则的组合 |
| [mir/mir_let_chains_drop_order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/mir/mir_let_chains_drop_order.rs) | run, unwind, 2024 | let 链中途失败/panic 时 drop 已建绑定（else DropTree） |

## match

| 用例 | 类型 | 考察规则 |
|---|---|---|
| [nll/match-cfg-fake-edges.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/nll/match-cfg-fake-edges.rs) | err | false edge：borrowck 不能利用决策树形状 |
| [moves/move-in-guard-1.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/moves/move-in-guard-1.rs) | err E0382 | 两个 guard 都 move 同一变量（false edge 让第二个可见） |
| [nll/match-guards-always-borrow.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/nll/match-guards-always-borrow.rs) | err E0507 | guard 内按引用可见的 binding 不能被 move；arm 体内可以修改 |
| [nll/match-guards-partially-borrow.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/nll/match-guards-partially-borrow.rs) | err E0510 + ok | guard 中修改不改变匹配结果的 place 允许；改变被测 place 报 E0510（Shallow fake borrow 的范围） |
| [drop/or-pattern-drop-order.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/or-pattern-drop-order.rs) | run | or-pattern 的 drop 顺序由第一个分支的 binding 顺序决定（#142163） |
| [mir/mir_match_arm_guard.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/mir/mir_match_arm_guard.rs) | run | guard 失败后落到下一 arm |
| [drop/if-let-guards.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/drop/if-let-guards.rs) | run | `if let` guard 的临时值/绑定在 arm 绑定前 drop（`MatchGuard` scope） |

## 用法建议

- run 类用例都依赖 `Drop` 副作用和 `RefCell`/`Cell`/`catch_unwind`，需要 M2 的内存与 M3 的 trait；首轮（T13）应从中抽出不依赖 std 的最小骨架（用全局计数器 + `extern "C"` 打印）。
- dynamic-drop 是 unwind 正确性的金标准，按函数拆分逐个接入；`alloc_leaked` 标出的位置 rustc 本身会泄漏，kfrc 对拍应以 rustc 的行为为准。
- err 类用例只比较“拒绝/接受 + 错误码”，不比较诊断文本。

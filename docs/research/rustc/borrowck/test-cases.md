# 代表性边界用例

路径相对 `tests/ui/`（rustc 1.94.1 源码树，[树链接](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui)）。“结果”列取自用例里的 `//~ ERROR` 注释和 `//@ check-pass`/`run-pass` 头。带 `revisions: nll polonius legacy` 的用例，以 `[nll]` 修订版为 stable 行为。建议 kfrc 差分时比较“接受/拒绝 + 错误码 + 主 span 行号”。

## 1. NLL 作用域与 liveness

| 用例 | 结果 | 考察的规则 |
|---|---|---|
| `nll/region-ends-after-if-condition.rs` | `foo1` 通过，`foo2` E0502 | loan 只活到引用最后一次使用；分支外再用才冲突 |
| `nll/loan_ends_mid_block_pair.rs` | `nll_fail` 中第一次写 E0506，`nll_ok` 通过 | loan 在块中间结束；只有后续还用 `c` 时写 `data.0` 才冲突 |
| `nll/reference-carried-through-struct-field.rs` | E0503 | region 经 struct 字段传递，`Wrap` 活着就锁住 `x` |
| `nll/process_or_insert_default.rs` | run-pass | NLL problem case #2：`match map.get_mut()` 的 `None` 分支里可以再借 `map` |
| `nll/get_default.rs` | `[nll]` 在 `ok` 中也报 E0502 | problem case #3：条件返回借用。NLL 约束位置无关 ⇒ 拒绝；Polonius 接受 |
| `nll/polonius/nll-problem-case-3-issue-21906.rs` | `[nll] known-bug` | 同上，stable 拒绝的已知形状清单（目录下同系列共 10 个） |
| `nll/polonius/assignment-kills-loans.rs` | check-pass（只跑 polonius 修订版，注释称 NLL 同样接受） | `list = n`、`*list = n` 等整体赋值杀死旧 loan（`kill_borrows_on_place`） |
| `nll/polonius/call-kills-loans.rs` | check-pass（同上） | `Call` 目的地写入同样杀死 loan |
| `nll/drop-may-dangle.rs` | check-pass | `#[may_dangle]` ⇒ drop 不要求 region live |
| `nll/drop-no-may-dangle.rs` | 两处 E0506 | 普通 `Drop` impl ⇒ region 活到 drop 点 |
| `nll/maybe-initialized-drop.rs` / `-uninitialized.rs` | E0506 / check-pass | `mem::drop(wrap)` 后 `wrap` 未初始化，析构不再使 loan 存活 |

## 2. 路径与冲突

| 用例 | 结果 | 考察的规则 |
|---|---|---|
| `borrowck/borrowck-reborrow-from-mut.rs` | 11 个错误（E0499/E0502/E0596）；不同字段、imm/imm 通过 | 字段不相交、父路径 `*foo` 与子字段冲突（`places_conflict`）；经 `&` 不能可变借用 |
| `borrowck/borrowck-anon-fields-struct.rs` | `same_variant` 报错 | 元组结构体位置字段的不相交 |
| `borrowck/borrowck-local-borrow-outlives-fn.rs` | E0515 | 返回参数局部的引用：出口处 loan 仍在作用域 |
| `nll/borrowed-universal-error.rs` | E0515 | 返回引用临时值 |
| `nll/match-guards-partially-borrow.rs` | 多处 ERROR | match 的 fake borrow：guard 里不能改变被匹配值决定分支的部分 |

## 3. Region 关系、重借用、`'static`、省略

| 用例 | 结果 | 考察的规则 |
|---|---|---|
| `nll/where_clauses_in_functions.rs` | `bar` 报 “lifetime may not live long enough” | 被调者 where 子句 `'a: 'b` 在调用点变成约束 |
| `nll/closure-requirements/region-lbr1-does-outlive-lbr2-because-implied-bound.rs` | check-pass | `&'a &'b u32` 的隐含界 `'b: 'a` |
| `borrowck/borrowck-reborrow-from-shorter-lived-andmut.rs` | “lifetime may not live long enough” | 经 `&mut` 的重借用要求外层引用也活够（`add_reborrow_constraint`） |
| `nll/user-annotations/cast_static_lifetime.rs` | E0597 | `as &'static u32` 用 `Locations::All` |
| `lifetimes/lifetime-elision-return-type-requires-explicit-lifetime.rs` | 多处 E0106 | 返回类型省略失败的各种情形，含类型别名隐藏的 `'static` |

## 4. 两阶段借用

| 用例 | 结果 | 考察的规则 |
|---|---|---|
| `borrowck/two-phase-baseline.rs` | run-pass | `v.push(v.len())` |
| `borrowck/two-phase-cannot-nest-mut-self-calls.rs` | E0502 | 预留期间允许读，不允许内层 `&mut` |
| `borrowck/two-phase-sneaky.rs` | E0499 | 外层经 `IndexMut`，不是两阶段 |
| `borrowck/two-phase-across-loop.rs` | E0499 | loan 带回循环头后，预留与自己冲突 |
| `borrowck/two-phase-reservation-sharing-interference-2.rs` | 两处报错，`reservation_allowed` 通过 | 预留可与已有共享借用共存；激活时冲突（#56254） |
| `borrowck/two-phase-nonrecv-autoref.rs` | 若干 ERROR | 只有接收者 autoref/运算符/实参重借用是两阶段；unsize、索引、显式 `&mut` 不是 |

## 5. 闭包

| 用例 | 结果 | 考察的规则 |
|---|---|---|
| `nll/closure-requirements/escape-upvar-ref.rs` | E0597，stderr 显示 `where '?3: '?2` | 闭包上传 external region 关系，父函数据此报错 |
| `nll/closure-requirements/propagate-approximated-ref.rs` | “lifetime may not live long enough” | 闭包内 late-bound 关系近似成 external 关系后上传 |
| `borrowck/borrowck-closures-unique.rs` | 多处 ERROR | `ClosureCapture`（unique immutable）冲突规则 |
| `closures/2229_closure_analysis/run_pass/capture-disjoint-field-struct.rs` | run-pass（edition 2021） | 只捕获 `p.x`，外部可 `&mut p.y` |
| `closures/2229_closure_analysis/run_pass/mut_ref.rs` | run-pass | 经多层 `&mut` 的写只要求 unique 借用 |

## 6. opaque

| 用例 | 结果 | 考察的规则 |
|---|---|---|
| `impl-trait/must_outlive_least_region_or_bound.rs` | “captures lifetime that does not appear in bounds” 等 | member constraint 找不到 choice region |
| `nll/polonius/location-insensitive-scopes-issue-116657.rs` | `[nll]` 报 hidden type captures lifetime | member constraint 未生效时的 loan 作用域 |

## 7. 建议的首轮子集（[T13](../../../plan/m1-vertical-chain.md#t13)）

只用单 crate、无泛型、无闭包、无 opaque 的用例：第 1 节前 4 行与 `get_default.rs`（验证“拒绝”与 NLL 一致）、第 2 节前 3 行、第 3 节的重借用一行、第 4 节前 3 行。按 kfrc 当前语法能力改写成只含整数字段的版本时，原用例和改写版都要记录 rustc 结果，改写版不计入原用例覆盖。

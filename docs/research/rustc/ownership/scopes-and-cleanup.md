# 作用域栈、退出 drop 与 cleanup/unwind

源码：`compiler/rustc_mir_build/src/builder/scope.rs`（模块注释 L1–79 是最好的总述），以及 `builder/mod.rs`、`builder/block.rs`。dev-guide：[mir/construction.md](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/src/doc/rustc-dev-guide/src/mir/construction.md)。

## 数据结构

- [`Scopes`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L108)：`scopes: Vec<Scope>`（词法栈）、`breakable_scopes`（loop/带标签块/函数体，记录 break 目的地与 break/continue 两棵 DropTree）、`if_then_scope`（当前 if 条件/guard 的 else DropTree）、全函数共享的 `unwind_drops` 与 `coroutine_drops`。
- [`Scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L128)：`region_scope`（来自 region scope tree，见 [temporaries.md](temporaries.md)）、`drops: Vec<DropData>`（**按调度顺序的栈，退出时逆序执行**）、`moved_locals`、`cached_unwind_block`。
- [`DropData`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L153) = `{source_info, local, kind}`，[`DropKind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L166)：`Value`（Drop 终结符）、`Storage`（StorageDead 语句）、`ForLint`（edition 迁移 lint 提示）。drop 的对象**只能是 local**，不是任意 place。
- [`DropTree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L232)：节点 `{data, next}`，根是“全部 drop 做完后去的地方”；[`add_drop`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L311) 以 `(next, local)` 去重，所以共享后缀的退出路径共用同一串 drop 块；`entry_points` 记录“哪个块从哪个节点进入”。

## 调度 drop

[`schedule_drop(span, region_scope, local, kind)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1405)：`Value` 且类型不 `needs_drop` 直接忽略；从栈顶往下找目标 scope 压入 `DropData`，沿途使 unwind 缓存失效（只在 `Value` 或协程时，[L1478](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1478)）。

调度时机决定 unwind 时谁被 drop：
- 临时值：`Storage` 在求值前调度，`Value` 在 `expr_into_dest` 返回**之后**调度（[as_temp.rs L123/L131](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/as_temp.rs#L123)）。
- `let x = e;`：`StorageLive` → 把 `e` 直接写进 `x` → 再调度 `x` 的 `Value`（[`expr_into_pattern`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/matches/mod.rs#L572)）。`let x;` 立即调度，靠 drop 显式化处理未初始化。
- 参数：在 `Arguments` scope 调度（[mod.rs L1077](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/mod.rs#L1077)），该 scope 包住函数体，所以参数晚于体内一切局部和临时值 drop。

## 正常退出（fallthrough）

[`in_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L684) → `pop_scope` → [`leave_top_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1222) → [`build_scope_drops`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1817)：逆序遍历 `drops`，`Value` 生成 `Drop{target: next, unwind: Continue}` 并把该块登记为 unwind 树的入口（指向“剩下的 drop”节点），`Storage` 追加 `StorageDead`。`moved_locals` 中的 local 在正常路径不生成 Drop（[L1894](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1894)），由 [`record_operands_moved`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1573) 在 Call 的参数被整体 move 时登记（为避免 #64391 的伪借用错误）。

块中多个 `let` 各推一个 `Remainder` scope，块末逆序弹出（[block.rs L345](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/block.rs#L345)），因此局部变量逆声明顺序 drop。

## break / continue / return / if-else

- 函数体包在 `in_breakable_scope(None, RETURN_PLACE, …)` 里（[mod.rs L537](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/mod.rs#L537)），`return` 就是 break 到第 0 个 breakable scope。
- [`break_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L748)：先把 break 值写入目的地，再把**目标 scope 以内**所有 scope 的 drops 依次加入目标的 break（或 continue）DropTree，登记入口，块暂以 `UnwindResume` 占位终结。
- [`in_breakable_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L539) 结束时 [`build_exit_tree`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1987) 一次性物化：`ExitScopes` 把占位改为 Goto；continue 树的根就是 loop 头块；退出树里的每个 `Value` drop 再挂到 unwind 树上。
- [`break_for_else`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1063) 与 [`in_if_then_scope`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L655)：`if` 条件、let 链、match guard 的每个失败出口都把已建 binding/临时值的 drop 收进 else 树，按“哪一个 `let` 失败”drop 不同集合。

## unwind（diverge）路径

- 可 unwind 的终结符（Call、Assert、Drop、FalseUnwind、InlineAsm）先以 `UnwindAction::Continue` 生成，再 [`diverge_from(block)`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1644)：取 [`diverge_cleanup_target`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1611)（自栈底向上复用 `cached_unwind_block`，只把 `Value` drop 放进 unwind 树——非协程不在 unwind 路径发 StorageDead，见 [`needs_cleanup`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L269) 注释），登记入口。
- 整个函数降低完后 [`build_drop_trees`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L2059) 用 `Unwind` builder 物化：块是 cleanup 块，根是 `UnwindResume`；树内 Drop 的 unwind 是 `Terminate(InCleanup)`；[`link_entry_point`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L2169) 把入口终结符的 unwind 改为 `Cleanup(bb)`。
- `loop` 头是 `FalseUnwind`（[into.rs L229](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/expr/into.rs#L229)），让 borrowck 即使循环体内没有调用也看见 unwind 路径。
- [`assign_blocks`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L344)：连续 `StorageDead` 共享块，`Value` 后继必须新块。

## CFG 不变量与 panic 策略

- [`UnwindAction`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/mir/syntax.rs#L1036)：`Continue / Unreachable / Terminate(Abi|InCleanup) / Cleanup(bb)`。
- validator [`check_edge`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/validate.rs#L139)、[`check_unwind_edge`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/validate.rs#L251)：普通块只能经 unwind 边进 cleanup 块；cleanup 块之间只走普通边；cleanup 块里不能有 `Cleanup`/`Continue`，`Terminate(InCleanup)` 只能在 cleanup 块；[`check_cleanup_control_flow`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/validate.rs#L173) 要求 cleanup 区域成树（funclet 约束）。
- **panic=abort 不改变 MIR 构建**：`rustc_mir_build` 不读 panic 策略，borrowck 看到的 CFG 含全部 cleanup 边。drop 显式化之后 [`AbortUnwindingCalls`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/abort_unwinding_calls.rs#L24) 用 [`fn_can_unwind`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/layout.rs#L1233)（abort 下非 foreign 函数一律不 unwind，[L1244](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/layout.rs#L1244)）把不会 unwind 的终结符改成 `UnwindAction::Unreachable`（[L122](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/abort_unwinding_calls.rs#L122)）并删死块；不能 unwind 的函数里 `UnwindResume` 变 `UnwindTerminate(Abi)`。`RemoveNoopLandingPads` 只在 unwind 策略下运行。

## tricky 边界

- 守卫可能多次降低（or-pattern），scope 不再是 SEME；[`clear_match_arm_and_guard_scopes`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/builder/scope.rs#L1787) 在每次之间清空 arm/guard scope 的调度。
- break 值先写入目的地，再执行内层 drop；若内层 drop panic，已写入目的地的值不会被 drop——这是已知泄漏 #47949（`tests/ui/drop/dynamic-drop.rs` 中 `alloc_leaked` 标出的用例）。对拍要避开或按同样行为期望。
- `let _x: Box<_> = return;`：`return` 在 `_x` 的 `Value` drop 被调度之前就把当时的 drop 集合收进 break 树，因此不会 drop 未初始化的 `_x`（`tests/ui/drop/terminate-in-initializer.rs`）。调度时机本身就是正确性的一部分。

## 对 kfrc 的含义

- Body builder 内需要一份“作用域栈 + 三类 DropTree（break/continue、else、unwind）”，作用域对象由 region scope tree 给出；T09 的 SoN builder 不再决定 drop。
- 需要在 Body 上区分 `Drop(local)` 终结符（可 unwind、带两个后继）、`StorageDead(local)` 语句、cleanup 块标记、`UnwindResume`/`Terminate`。
- abort-only 首轮也应生成 cleanup 边供检查，然后在 Body→SoN 前按 panic 策略裁掉，这与 rustc 的阶段划分一致，也满足 T27 的“abort 限制显式化”。

# 调用、实参检查与闭包签名推导

源码：`compiler/rustc_hir_typeck/src/{callee.rs, fn_ctxt/checks.rs, fn_ctxt/_impl.rs, closure.rs, upvar.rs}`。链接指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. 被调者

- 路径表达式 `check_expr_path` → `instantiate_value_path`（[_impl.rs#L910](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L910)）：泛型参数中未显式给出的建新变量，类型为 `FnDef(def_id, args)`（零大小 fn item 类型，不是 fn 指针）；`node_args` 记录实参；where 子句注册为 obligation。
- `check_expr_call`（[callee.rs#L65](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/callee.rs#L65)）：被调者无期望检查，然后 autoderef，每步 `try_overloaded_call_step`（[#L197](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/callee.rs#L197)）：
  - `FnDef`/`FnPtr` → `CallStep::Builtin`，把 deref 步骤写成被调者的 adjustment。
  - kind 尚未推出的本地闭包 → `CallStep::DeferredClosure`：用闭包签名检查实参，登记 `DeferredCallResolution`，等 upvar 分析定出 `Fn/FnMut/FnOnce` 后再补 autoref 和 callee（[#L990](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/callee.rs#L990)）。
  - 其他尝试 `Fn`/`FnMut`/`FnOnce` 的 `call*` 方法 → `CallStep::Overloaded`。
- `confirm_builtin_call`（[#L511](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/callee.rs#L511)）：取 `fn_sig(def_id).instantiate(args)`，late-bound region 换新变量，规范化，然后 `check_argument_types`。

## 2. `check_argument_types`

[checks.rs#L192](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L192)：

1. 每个形参类型注册 WF obligation。
2. **由期望返回类型推出期望实参类型**：若调用表达式有硬期望，在 `fudge_inference_if_ok` 里执行 `expected_output :> formal_output` 并 select，得到“期望实参类型”（[#L251](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L251)）。这只是提示：快照回滚，结果中的新变量被换成无关变量。源码 FIXME(#149379) 指出这样推出的提示可能不 WF。
3. 实参个数不符时进入错误恢复（`arg_matrix`），不影响正常路径。
4. 每个实参 `demand_compatible`（[#L366](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L366)）：用 `rvalue_hint(期望实参类型)` 检查实参 → `coerce(实参, 期望类型或形参类型, AllowTwoPhase::Yes)` → 再 `eq(形参类型, coerce 目标)`。
5. **先查非闭包实参，再查闭包实参**；两轮之间 `select_obligations_where_possible`（[#L434-L439](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L434)）。目的是让闭包在检查时能从已解出的泛型参数和 `F: Fn(..)` 约束拿到签名。`async` 块不算闭包。
6. 实参从左到右检查，泛型参数由第一个能定型的实参决定（`test(&mut 7, &7)` 失败的原因）。

方法调用走同一函数，只是形参去掉 `self`（[expr.rs#L1494](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L1494)）。

## 3. 闭包签名推导

`check_expr_closure`（[closure.rs#L48](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/closure.rs#L48)）：

- **期望签名来源** `deduce_closure_signature`（[#L302](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/closure.rs#L302)），按期望类型（`to_option`）分类：
  - `TyVar(v)`：扫描以 `v` 为 self 的待证 obligation（`obligations_for_self_ty`），从 `Fn*` trait 和其 `Output` 投影推出签名与 kind（`deduce_closure_signature_from_predicates`，[#L347](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/closure.rs#L347)，会 elaborate 父 trait）。这就是第 2 节“先 select 再查闭包”的原因。
  - `dyn Fn(..)`：从投影约束和 principal trait 推出。
  - `impl Fn(..)`（opaque）：从 item bounds 推出。
  - `fn(..)` 指针：签名直接取用，kind 定为 `Fn`。
- **合并用户注解** `sig_of_closure_with_expectation`（[#L720](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/closure.rs#L720)）：参数个数不符则另行报错；否则在快照中把用户写的签名与期望签名逐个 `eq`（`merge_supplied_sig_with_expectation`，[#L819](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/closure.rs#L819)），失败则退回只用用户注解（未注解的参数/返回是新变量）。
- **闭包类型** `ty::Closure(def_id, ClosureArgs { parent_args, closure_kind_ty, closure_sig_as_fn_ptr_ty, tupled_upvars_ty })`：签名的参数被打包成一个元组（`rust-call` ABI）；`closure_kind_ty` 来自期望或新变量；`tupled_upvars_ty` 是新变量。
- 闭包体与外层函数共用同一个 infcx 和 `TypeckResults`，在 `check_expr_closure` 中**立即**检查（有自己的 `ret_coercion`）。

## 4. 捕获与 kind：fallback 之后

- `closure_analyze`（[upvar.rs#L77](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L77)）在 fallback、cast 检查之后运行：用 `ExprUseVisitor` 按 place 粒度收集捕获（精确捕获按闭包 span 的 edition 开关，2021 起启用，`enable_precise_capture`，[upvar.rs#L2594](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L2594)），定出每个捕获的 by-value/by-ref 模式与闭包 kind（被 move 出 → `FnOnce`，被修改 → `FnMut`，否则 `Fn`），把 `tupled_upvars_ty` 与实际捕获类型关联（[#L512](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L512)），然后处理延迟的闭包调用（[#L530](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/upvar.rs#L530)）。
- 结果写入 `closure_min_captures`、`closure_fake_reads`、`closure_kind_origins`。闭包 → fn 指针 coercion 要求无捕获；此时捕获分析还没做（`tupled_upvars_ty` 仍是变量），所以 `sig_for_closure_coercion` 改查名称解析给出的 `upvars_mentioned`（[coercion.rs#L1148](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L1148)）：提到任何外层变量即失败。

## 对 kfrc 的含义

- 标量 M1 的调用只有“非泛型 `FnDef` + 位置实参 coerce 到形参”这一条路径；但要保留 `FnDef` 作为 callee 的类型（零大小、按 `InstanceKey` 解析），而不是直接把函数名当 fn 指针，否则 M3 的泛型实参记录（`node_args`）和 fn item → fn ptr coercion 都没有挂载点。
- “先非闭包后闭包 + 中间 select”和“由期望返回类型推实参提示（fudge）”都依赖 snapshot 与 trait 求解器，属于 T21；首轮只需在 API 上留出“期望实参类型”这一输入。
- 闭包需要与外层共用推导上下文，且捕获分析排在 fallback 之后；kfrc 的 typeck 任务单位因此是“函数 + 其全部闭包”，与 rustc 的 typeck root 一致。

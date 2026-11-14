# 推导上下文：变量、合一、快照、关系

源码根：`compiler/rustc_infer/src/infer/`（类型关系的通用部分在 `compiler/rustc_type_ir/src/`）。链接全部指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. `InferCtxt` 与 `InferCtxtInner`

- **`InferCtxt`**（[mod.rs#L231](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L231)）：`tcx`、`typing_mode`、`considering_regions`、`in_hir_typeck`、`inner: RefCell<InferCtxtInner>`、selection/evaluation cache、`tainted_by_errors`、`universe`。一个 typeck root（函数及其所有闭包）共用一个。
- **`InferCtxtInner`**（[mod.rs#L89](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L89)）：`undo_log`、`projection_cache`、`type_variable_storage`、`int/float/const_unification_storage`、`region_constraint_storage`、`region_obligations`、`opaque_type_storage`。所有可回滚状态都在这里，靠一条共享 undo log 回滚。
- HIR typeck 用 `tcx.infer_ctxt().ignoring_regions().in_hir_typeck()` 构造（[typeck_root_ctxt.rs#L80](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/typeck_root_ctxt.rs#L80)）。`TypeckRootCtxt` 另外持有 `fulfillment_cx`（待证 obligation）和若干 deferred 列表（cast、call resolution、sized、asm、repeat）。

## 2. 四类推导变量

| 变量 | 创建 | 值域 | 存储 |
|---|---|---|---|
| `TyVar(TyVid)` | `next_ty_var`（[mod.rs#L775](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L775)） | 任意类型；`TypeVariableValue::{Known{value}, Unknown{universe}}`（[type_variable.rs#L108](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/type_variable.rs#L108)） | `TypeVariableStorage`（[#L59](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/type_variable.rs#L59)）：origin 表 + `eq_relations` + `sub_unification_table` |
| `IntVar(IntVid)` | `next_int_var`（[mod.rs#L818](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L818)） | `IntVarValue::{Unknown, IntType, UintType}`（[ty_kind.rs#L505](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/ty_kind.rs#L505)） | `UnificationTableStorage<IntVid>` |
| `FloatVar(FloatVid)` | `next_float_var`（[#L824](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L824)） | `FloatVarValue::{Unknown, Known}`（[ty_kind.rs#L525](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/ty_kind.rs#L525)） | `UnificationTableStorage<FloatVid>` |
| region var | `next_region_var`（[#L833](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L833)） | 见 §6 | `RegionConstraintStorage` |

- 都是 `ena` 风格的 union-find：`eq_relations` 的 key 是 `TyVidEqKey`，值是 `TypeVariableValue`；`order_roots` 让小编号做根（[type_variable.rs#L328](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/type_variable.rs#L328)）。两个都已知的类型变量不会被合并（`unify_values` 遇到 Known/Known 直接 `bug!`，[#L363](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/type_variable.rs#L363)）：调用方先 `shallow_resolve`，再结构化地关联两个已知类型。
- `TyVar` 不会直接指向另一个 `TyVar`：`instantiate`（[#L183](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/type_variable.rs#L183)）断言值不是 ty var；变量间相等走 `equate`（union）。但 `TyVar` 可以被实例化成 `IntVar`，所以 `shallow_resolve`（[mod.rs#L1086](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L1086)）对 ty var 递归一层再解 int/float var。
- `sub_unification_table` 只记“两个变量经子类型相关”，不区分方向；用于诊断、never fallback 的图和新求解器（[#L59-L84](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/type_variable.rs#L59)）。
- 整型变量只能与 `Int`/`Uint`/另一个 `IntVar` 合一，其他组合是 `TypeError::Sorts`（`super_combine_tys`，[combine.rs#L45](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/relate/combine.rs#L45)）。`IntVar` 不区分有/无符号，`bool`、`char` 都不行。
- `unresolved_variables`（[mod.rs#L657](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L657)）按编号返回未解的 ty/int/float 变量，fallback 用它。
- `resolve_vars_if_possible`（[#L1206](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L1206)）是幂等 fold，不改状态；`fully_resolve`（[#L1245](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L1245)）遇到未解变量返回错误。

## 3. 快照与回滚

- **undo log**：`InferCtxtUndoLogs { logs, num_open_snapshots }`（[undo_log.rs#L99](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/undo_log.rs#L99)）。只有存在打开的快照时才 `push`（[#L116](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/undo_log.rs#L116)），所以快照外的操作零开销。条目种类见 `UndoLog`（[#L20](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/undo_log.rs#L20)）：类型变量、int/float/const 表、region 约束与 region 表、projection cache、opaque、region obligation。
- **`CombinedSnapshot`**（[snapshot/mod.rs#L14](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/mod.rs#L14)）= undo log 长度 + region 快照 + 当前 universe。`rollback_to` 逆序弹出并 `reverse`，最外层快照结束时断言 log 为空（[undo_log.rs#L139](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/undo_log.rs#L139)）；`commit` 只减计数，最外层时清空 log（[#L159](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/undo_log.rs#L159)）。快照严格嵌套（栈式）。
- **`commit_if_ok(f)`**（[snapshot/mod.rs#L81](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/mod.rs#L81)）：`Ok` 保留，`Err` 回滚。**`probe(f)`**（[#L101](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/mod.rs#L101)）：总是回滚。
- **`fudge_inference_if_ok`**（[fudge.rs#L89](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/snapshot/fudge.rs#L89)）：在 probe 里算出结果，回滚后把结果中“快照内新建的变量”替换成新的、无约束的变量。用于“期望参数类型”这类只作提示的推导（见 [calls-and-closures.md](calls-and-closures.md)）。
- **回滚范围之外的东西**：`TypeckResults`（node 类型、adjustment、method callee）不在 undo log 里。rustc 的约定是只在成功路径写表，例如 `FnCtxt::coerce` 先 `commit_if_ok(|_| coerce.coerce(..))`，成功后才 `apply_adjustments`（[coercion.rs#L962](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L962)）；dev-guide 明确禁止在 `probe` 里调 `FnCtxt::coerce`、`CoerceMany`。fulfillment 队列也不随快照回滚，候选 obligation 以 `InferOk { value, obligations }` 返回、成功后才注册。

## 4. 类型关系：sub / eq / sup / lub

- 入口 `At::{sub, sup, eq, lub}`（[at.rs#L126-L272](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/at.rs#L126)），返回 `InferResult<T> = Result<InferOk<T>, TypeError>`：成功也可能附带新的 obligation。
- **`TypeRelating`**（[type_relating.rs#L15](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/type_relating.rs#L15)）实现 sub/sup/eq，状态是 `ambient_variance`。`tys`（[#L120](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/type_relating.rs#L120)）：
  - 两边都是未解 `TyVar`：协变/逆变时**不立即合并**，登记 `PredicateKind::Subtype` obligation；不变时 `equate`。
  - 一边是 `TyVar`：`instantiate_ty_var`（[generalize.rs#L54](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/generalize.rs#L54)）先按 variance **泛化**另一边（把其中的 region 和需协变处理的类型换成新变量，做 occurs check 与 universe 检查），再实例化并关联泛化结果与原类型。
  - 其他交给 `super_combine_tys` → `structurally_relate_tys`（[relate.rs#L333](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/relate.rs#L333)）。
- **variance**：`Variance::xform`（[lib.rs#L268](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/lib.rs#L268)）组合外层与位置 variance；结果为 `Bivariant` 时直接跳过（[type_relating.rs#L102](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/type_relating.rs#L102)）。函数签名：参数逆变、返回协变（[relate.rs#L149](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/relate.rs#L149)）；ADT 参数按 `variances_of` 查表。
- 引用和裸指针的规则见 [references.md](references.md)。
- **`LatticeOp`**（[lattice.rs#L49](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/lattice.rs#L49)）实现 LUB/GLB：一边是 `TyVar` 时新建 `v`，登记 `a <: v`、`b <: v`，先关联非变量一边（`relate_bound`，[#L245](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/lattice.rs#L245)），顺序影响 coercion 结果；遇到高阶 binder 退化为不变（[#L212](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/lattice.rs#L212)）。
- `subtype_predicate`（[mod.rs#L719](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L719)）是 fulfillment 处理 `Subtype`/`Coerce` 谓词的入口：两边仍都是变量时 `sub_unify` 后返回“卡住”，否则做一次真正的 sub。`Coerce` 谓词在这里**退化为子类型**，不产生 adjustment（注释说明 coercion 的 adjustment 只能由 `FnCtxt::coerce` 记录）。

## 5. 错误与 universe

- `tainted_by_errors` 一旦设置，fallback 把所有未解变量设为 `{type error}`，抑制级联错误（[fallback.rs#L122](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fallback.rs#L122)）。`ty::Error` 与任何类型关联都成功（`super_combine_tys` 首分支）。
- universe 只在进入 `for<'a>` binder 时增加；函数自身的泛型参数在根 universe（[mod.rs#L307](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L307)）。

## 6. region 在 HIR typeck 中的处理（已核实：延后到 borrowck）

- `ignoring_regions()` 把 `considering_regions` 设为 false（[mod.rs#L537](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L537)）；fulfillment 遇到 `RegionOutlives`/`TypeOutlives` 直接视为成功、不登记（[fulfill.rs#L461](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/fulfill.rs#L461)）。
- typeck 仍然**创建** region 变量（`&expr` 用 `BorrowRegion`，autoref 用 `Autoref`，reborrow 用 `Coercion`），类型关系仍往 `RegionConstraintCollector` 写子 region 约束（[type_relating.rs#L216](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/type_relating.rs#L216)），但 `rustc_hir_typeck` 从不求解它们（全目录无 `resolve_regions` 调用）；它们只在高阶 leak check 中起作用。
- writeback 把所有非 bound region 擦成 `ReErased`（[writeback.rs#L996](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L996)）。MIR borrowck 重新编号 region、重新做 MIR 类型检查产出 outlives 约束（见 [../borrowck/flow.md](../borrowck/flow.md)）。
- `in_hir_typeck` 的注释（[mod.rs#L242-L261](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/mod.rs#L242)）说明不变式：HIR typeck 不得依赖 region 身份，否则 MIR typeck 会出现不一致。

## 对 kfrc 的含义

- 推导状态集中在一个 per-body 上下文；变量表 + 一条 undo log 即可覆盖 ty/int/float 三类变量的回滚。region 变量在 kfrc typeck 里可以只是占位（或直接用 `Erased`），因为 rustc typeck 不消费它们。
- side table（节点类型、adjustment）与可回滚状态分离，写表只在提交点发生；kfrc 要在 API 上保证这一点，而不是给 side table 也加 undo。
- `Subtype(?a, ?b)` 这类“两个变量之间的子类型”在标量版不会出现（没有子类型），但 never 和引用加入后会出现；数据结构要能挂起这类约束，而不是假定一律能立即合一。

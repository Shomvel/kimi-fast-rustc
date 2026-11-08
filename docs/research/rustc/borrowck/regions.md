# Region 变量、outlives 约束与求解

源码根：`compiler/rustc_borrowck/src/`，链接固定到 `e408947b`。liveness（region 何时 live）见 [liveness.md](liveness.md)，生命周期参数的来源见 [lifetimes-origin.md](lifetimes-origin.md)。

## 1. Region 变量

- **编号顺序**（`UniversalRegions::build`，[universal_regions.rs#L456](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/universal_regions.rs#L456)）：`'static`（Global）→ external（闭包所在父函数的 region）→ local（本函数 early-bound + 签名里的 late-bound）→ `fr_fn_body`。分类 `RegionClassification`（[#L225](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/universal_regions.rs#L225)）按 vid 区间判断。
- **existential**：`renumber_mir`（[renumber.rs#L14](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/renumber.rs#L14)）给 body 中每处 region（局部类型、`Rvalue::Ref` 的 region、调用的泛型实参、常量）新建变量。HIR typeck 求出的 region 不被使用（[coercion.rs#L404](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L404) 的注释：“We don't care about free lifetimes during HIR typeck”）。
- **placeholder**：高阶关系 `for<'a>` 进入时创建，带 universe（`MirTypeckRegionConstraints::placeholder_region`，[type_check/mod.rs#L289](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L289)）。
- 定义信息 `RegionDefinition { origin, universe, external_name }`（[region_infer/mod.rs#L124](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L124)）。

## 2. 约束从哪来（MIR typeck）

约束 `OutlivesConstraint { sup, sub, locations, span, category, from_closure }`（[constraints/mod.rs#L65](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/constraints/mod.rs#L65)），意为 `sup: sub`；`push` 丢弃 `'a: 'a`。

| 来源 | 位置 | 规则 |
|---|---|---|
| 赋值 | `visit_statement`（[type_check/mod.rs#L586](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L586)） | `rv_ty <: place_ty`，`Locations::Single(loc)` |
| 子类型关系 | `NllTypeRelating::regions`（[relate_tys.rs#L419](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/relate_tys.rs#L419)） | 协变 `a: b`，逆变 `b: a`，不变两条 |
| 方差 | [rustc_type_ir relate.rs#L439](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/relate.rs#L439)；ADT 用 `variances_of`（[relate_tys.rs#L311](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/relate_tys.rs#L311)） | `&'r T` 对 `T` 协变，`&'r mut T` 对 `T` 不变；对 `'r` 都协变 |
| 借用/重借用 | `add_reborrow_constraint`（[#L2319](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L2319)） | 见 [two-phase-and-reborrow.md](two-phase-and-reborrow.md) |
| 调用 | `visit_terminator`（[#L767](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L767)） | 被调签名的 late-bound region 每次调用实例化为新变量，并在调用点 live（[#L836](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L836)）；先证明未规范化签名 WF；实参 `<:` 形参（`check_call_inputs` [#L1981](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L1981)） |
| 被调的 where 子句 | `FnDef` 的 `predicates_of`（[#L1805](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L1805)） | 例如被调要求 `'a: 'b` |
| 签名 | `equate_inputs_and_outputs`（[input_output.rs#L131](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/input_output.rs#L131)） | 参数局部、返回槽与签名类型相等，`Locations::All` |
| 用户标注 | `Locations` 文档（[#L312](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L312)） | `let x: &'static T` 等，`Locations::All` |
| `T: 'a` | `TypeTest`（[region_infer/mod.rs#L186](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L186)） | 能拆成 region 约束的就拆；剩下的（参数、投影）在求解后检查 |
| 闭包 | 子 body 的 `ClosureRegionRequirements` | 见 [closures.md](closures.md) |
| drop | dropck 返回的约束 | 见 [liveness.md](liveness.md) |

## 3. 约束图与 SCC

- 正向图：`sup → sub`（[constraints/graph.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/constraints/graph.rs)）。`region_graph` 给 `'static` 加隐式边到所有 region（[graph.rs#L189](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/constraints/graph.rs#L189)）：任何 `R: 'static` 都使 R 包含一切。
- Tarjan SCC；`all_sccs()` 按依赖序（后序）给出（[rustc_data_structures scc/mod.rs#L147](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_data_structures/src/graph/scc/mod.rs#L147)）。
- 代表元 `Representative`：FreeRegion < Placeholder < Existential，同类取最小 vid（[region_infer/mod.rs#L53](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L53)）。
- **placeholder 处理**（1.94 新位置）：`compute_sccs_applying_placeholder_outlives_constraints`（[handle_placeholders.rs#L265](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/handle_placeholders.rs#L265)）。SCC 注解 `RegionTracker` 记录能命名的最大 universe 和能到达的 placeholder；若 SCC 到达自己命名不了的 placeholder，加 `rep: 'static`（`OutlivesUnnameablePlaceholder`）并重算 SCC。无 placeholder 时直接跳过。源码自称这是等新 trait 求解器的权宜做法（[#L262](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/handle_placeholders.rs#L262)）。

## 4. Region 值与传播

- 元素 `RegionElement`：`Location` | `RootUniversalRegion`（即 `end('a)`）| `PlaceholderRegion`（[values.rs#L25](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/values.rs#L25)）。`RegionValues` 按 SCC 存：点用稀疏区间矩阵，universal/placeholder 用位集（[#L247](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/values.rs#L247)）。
- 初值（`new` [region_infer/mod.rs#L292](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L292) + `init_free_and_bound_regions` [#L401](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L401)）：existential = liveness 点；universal = 全部 CFG 点 + `end(self)`；placeholder = 自身。
- `propagate_constraints`（[#L551](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L551)）：按后序遍历 SCC，`value[A] ∪= value[B]`（对每个后继 B）。一遍完成，无不动点循环，不看 `Locations`。

## 5. 求解后的检查（`solve`，[#L493](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L493)）

1. `check_type_tests`（[#L589](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L589)）：`VerifyBound` 求值；闭包里失败的可经 `try_promote_type_test`（[#L667](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L667)）上传给父函数。
2. `check_universal_regions`（[#L1038](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L1038)）→ `check_universal_region`（[#L1184](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L1184)）：对每个 universal `'a` 值里的每个 `end('b)`，要求已知 `'a: 'b`。已知关系来自 `UniversalRegionRelations`（[free_region_relations.rs#L182](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/free_region_relations.rs#L182)）：where 子句、隐含界、`'static: 'r`、`'r: 'fn_body`、自反，取传递闭包。不成立时：闭包里尝试上传（[closures.md](closures.md)），否则报 “lifetime may not live long enough”。非代表元只检查与代表元的关系，减少重复。
3. `check_bound_universal_region`（[#L1363](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/mod.rs#L1363)）：placeholder 的 SCC 只能包含它自己。

## 6. tricky 边界

- 任何流入 universal region 的 existential region 都包含全部 CFG 点；借这个 region 的 loan 在本函数内永不失效。“返回局部变量的引用”因此在出口处表现为 loan 冲突（E0515），不是 region 错误（见 [loans.md](loans.md)）。
- 遇到 `ReError` 时清空 outlives 约束和 type test，避免级联错误（[type_check/mod.rs#L181](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L181)）。
- 签名等式用 `Locations::All`：返回槽在入口处是死的，若只在入口相等，`_0 = &y` 不会被约束（[#L312](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/type_check/mod.rs#L312) 的例子）。

## 7. 对 kfrc 数据模型的含义

- Typed Body 的类型里每个引用位置要有可编号的 **region 槽**；typeck 阶段留空，借用检查时统一分配 `RegionVid`（universal 在前、按上面的固定顺序）。
- 自有类型模型需要 `relate(a, b, variance)`：引用、裸指针、元组、数组、ADT（按参数方差）、fn 指针。ADT 方差需要单独的方差推导（[rustc_hir_analysis/src/variance/](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/variance/mod.rs)），这在 T15/T16 的清单里还没有。
- 求解器本身（约束集、SCC、区间集合、后序并、universal 检查）不依赖 TyCtxt，可以直接移植。首轮不支持 `for<'a>` 高阶签名时报错拒绝，可推迟 universe/placeholder。

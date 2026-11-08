# `impl Trait`（opaque）与 member constraints

链接固定到 `e408947b`。这部分不在首轮闭包内，记录它是因为它改变了借用检查的**输出**和**调度**。

## 1. 为什么借用检查要管 opaque

- 返回位置 `impl Trait`（RPIT）、TAIT 的具体类型（hidden type）里的 **region** 要由借用检查确定。HIR typeck 只知道类型骨架，region 都是推导变量。
- 因此 `mir_borrowck` 的返回值是 `FxIndexMap<LocalDefId, DefinitionSiteHiddenType>`（[lib.rs#L117](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L117)），`type_of(opaque)` 在 `DefiningScopeKind::MirBorrowck` 下调用 `tcx.mir_borrowck(owner)`（[rustc_hir_analysis/src/collect/type_of/opaque.rs#L271](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_analysis/src/collect/type_of/opaque.rs#L271)）。
- 结论：调用一个返回 `impl Trait` 的函数的代码，在类型层面依赖**被调函数的借用检查结果**。借用检查不再是“叶子分析”。

## 2. 流程（`handle_opaque_type_uses`，[root_cx.rs#L85](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/root_cx.rs#L85)）

对同一 typeck root 下的所有 body：

1. `clone_and_resolve_opaque_types`：取 MIR typeck 期间登记的 opaque 使用 `(OpaqueTypeKey, ProvisionalHiddenType)`。
2. `compute_definition_site_hidden_types`（[opaque_types/mod.rs#L178](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/opaque_types/mod.rs#L178)）：在一份**独立的** `RegionCtxt` 上（不改动正式求解用的约束图）：
   - `collect_defining_uses`：opaque 的实参必须是互不相同的 universal region/参数才算“定义性使用”；`arg_regions = ['static] ++ 被捕获的 lifetime 实参`。
   - `apply_member_constraints`（[member_constraints.rs#L14](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/opaque_types/member_constraints.rs#L14)）：按 SCC 后序边传播边处理；hidden type 中的每个 region 是 member，`arg_regions` 是 choice。`apply_member_constraint`（[#L59](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/opaque_types/member_constraints.rs#L59)）：去掉小于 member 当前下界的选项、去掉不被 member 的所有上界 outlive 的选项，再在全序子集中取最小者，把它并入 member 的值（相当于加 `'member: 'choice`）。member 落在非 root universe 时不处理。
   - `compute_definition_site_hidden_types_from_defining_uses`：把 hidden type 中每个 region 映射回某个 `arg_region`；映射不上 ⇒ “hidden type captures lifetime” 类错误（延迟到没有 region 错误时才报）。
3. `apply_definition_site_hidden_types`（[#L484](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/region_infer/opaque_types/mod.rs#L484)）：把定义处的 hidden type 实例化后与每个使用处相等，产生新的 outlives 约束加入正式约束集，然后才做正式的 `compute_regions`。

## 3. 与闭包的相互依赖

- 父函数推 hidden type 需要子 body（例如 `async` 块）的闭包要求；子 body 的非定义性使用又依赖父函数推出的 hidden type。`root_cx` 先用“不考虑 opaque 的闭包要求”（`compute_closure_requirements_modulo_opaques`，[nll.rs#L85](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/nll.rs#L85)）给父函数用，处理完 opaque 后再算一次最终要求（[root_cx.rs#L127](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/root_cx.rs#L127) 的注释）。
- 不含 opaque 的 body 在第一遍就完成检查（性能优化）。

## 4. 对 kfrc 的含义

- 首轮到 M3 前期：拒绝返回位置 `impl Trait` 中捕获 lifetime 的函数（或全部 RPIT），报“不支持”；不要让它们静默通过。
- 支持 RPIT 时，任务图要加一条边：“调用者的 typeck/布局” 依赖 “被调者 typeck root 的借用检查”。这影响 [build-pipeline](../../../design/build/pipeline.md) 的并行度和缓存失效范围：改动一个函数体可能改变其 opaque 的 hidden type，从而让调用者失效。
- std 和真实 crate 普遍使用 `impl Iterator` 等 RPIT（推断：ripgrep/Cargo 依赖链中必然出现，未逐一统计），所以 member constraints 至迟在 [M4](../../../plan/m4-cargo-sysroot.md) 前要移植；它依赖完整的 universal region 关系表和 SCC 值，放在 region 求解器稳定之后做。

# Writeback 与 TypeckResults：typeck 交给下游的东西

源码：`compiler/rustc_hir_typeck/src/writeback.rs`、`compiler/rustc_middle/src/ty/{typeck_results.rs, adjustment.rs}`、消费方 `compiler/rustc_mir_build/src/thir/cx/`。链接指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. `TypeckResults` 字段（按下游用途）

`TypeckResults`（[typeck_results.rs#L31](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L31)）是一个 owner 内以 `ItemLocalId` 为键的 side table 集合，不改 HIR。

| 字段 | 内容 | 下游用途 |
|---|---|---|
| `node_types`（[#L48](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L48)） | 每个表达式、模式、局部的**未调整**类型 | THIR 节点类型 |
| `adjustments`（[#L71](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L71)） | 表达式的 `Vec<Adjustment { kind, target }>`，按顺序作用 | THIR 插入 Deref/Borrow/PointerCoercion/NeverToAny |
| `type_dependent_defs`（[#L37](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L37)） | 方法调用、重载运算符/索引/解引用、`<T>::f` 路径解析出的 `DefId` | 生成调用；有记录即视为重载 |
| `node_args`（[#L54](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L54)） | 泛型项引用处的实参 | 实例化、单态化 |
| `field_indices`（[#L43](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L43)） | 字段访问/结构体表达式/模式的字段下标 | Place 投影 |
| `pat_binding_modes`（[#L74](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L74)） | 每个绑定的实际 `BindingMode(ByRef, Mutability)` | move/copy 还是借用 |
| `pat_adjustments`（[#L102](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L102)） | match ergonomics 剥掉的引用类型序列 | THIR 模式插隐式 deref |
| `skipped_ref_pats`、`rust_2024_migration_desugared_pats` | 见 [patterns.md](patterns.md) | 诊断、edition 2024 报错 |
| `coercion_casts`（[#L156](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L156)） | 由 coercion 完成的 `as` | THIR 把 cast 变成 `Use` |
| `liberated_fn_sigs`（[#L146](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/typeck_results.rs#L146)） | 函数/闭包体内视角的签名 | MIR 参数与返回类型 |
| `fru_field_types` | `S { ..base }` 中各字段类型 | 结构体更新 |
| `user_provided_types`/`user_provided_sigs` | 用户写的类型注解（canonical） | MIR `AscribeUserType`，borrowck 检查注解中的 region |
| `closure_min_captures`、`closure_fake_reads`、`closure_kind_origins` | 捕获分析结果 | 闭包降低、borrowck |
| `tainted_by_errors` | 有错误时置位 | `thir_body` 直接返回错误，不建 MIR（[thir/cx/mod.rs#L25](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/mod.rs#L25)） |

`Adjust` 变体与 `AutoBorrow`/`PointerCoercion` 的定义见 [references.md §3](references.md)；`Adjustment` 只存 `kind` 和 `target`，源类型由前一步的 target（或节点类型）给出（[adjustment.rs#L83](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/adjustment.rs#L83)）。

## 2. typeck 期间如何写表

- `write_ty`（[_impl.rs#L141](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L141)）：值可含推导变量；重复写同一节点时保留错误类型，否则延迟 bug。
- `apply_adjustments`（[_impl.rs#L252](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L252)）：空列表不写；已有条目时只处理两种组合——旧条目是单个 `NeverToAny` 时改其 target；旧条目是重借用 `[Deref, Borrow(Ref)]`、新条目以 `Deref` 开头时**整体替换**（重借用后再解引用等于无效）；其他组合发延迟 bug（编译必失败）并替换（[#L311-L352](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/_impl.rs#L311)）。adjustment 列表里含可变 autoref 时，顺带 `convert_place_derefs_to_mutable`。LUB coercion 回头给先前分支加 adjustment 时受这条规则约束（推断：dev-guide 所说“传递 coercion 在 stable 上 ICE”与此有关）。
- `write_method_call_and_enforce_effects`：写 `type_dependent_defs` + `node_args`。

## 3. Writeback 流程

`resolve_type_vars_in_body`（[writeback.rs#L41](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L41)）新建一个空 `TypeckResults`，遍历 body 把 fcx 表中的条目**解析后搬过去**：

1. `visit_expr`（[#L260](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L260)）：`visit_node_id`（[#L657](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L657)）搬 `type_dependent_defs`、adjustment、节点类型、`node_args`；递归子表达式后执行 `fix_scalar_builtin_expr`（[#L151](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L151)）和 `fix_index_builtin_expr`（[#L220](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L220)）：两侧（或操作数）是标量时删除运算符的方法记录和按引用比较时加的 autoref；数组/slice 用 `usize` 索引时删除 `Index` 方法记录。
2. `visit_pat`：绑定模式、`pat_adjustments`、`skipped_ref_pats`、迁移信息、节点类型。
3. 之后批量处理闭包捕获、liberated sig、FRU 类型、opaque hidden type、coercion casts、用户类型注解、transmute 等。
4. 每个值经 `Resolver`（[#L893](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L893)）fold：仍含非 region 推导变量 → 报 E0282 并换成 `{type error}`；所有非 bound region → `ReErased`（[#L980](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L980)）；新求解器下还会深度规范化。`write_ty_to_typeck_results` 断言结果无推导变量、无 placeholder、无自由 region（[#L138](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/writeback.rs#L138)）。

## 4. 下游如何“执行”这些决定

- `mirror_expr_inner`（[thir/cx/expr.rs#L51](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L51)）：先按未调整类型构造节点，再对 `expr_adjustments` 逐个 `apply_adjustment`（[#L89](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L89)），每个 adjustment 包一层新节点。
- `NeverToAny` 在 target 仍是 `!` 时被省略；`Deref(Some)` 展开为 “借用 → 调 `deref` → 解引用结果”；`Pointer(_)` 变成 `PointerCoercion`。
- 二元运算：`type_dependent_defs` 中有记录 → 重载调用，否则内建运算。cast：`coercion_casts` 中有 → `Use`，否则 `Cast`（MIR 再用 `mir_cast_kind` 分类，见 [casts.md](casts.md)）。

## 对 kfrc 的含义：Typed Body 需要记录的 typeck 结果

按 T06 → T21 分级（✓ = M1 标量即需要）：

| 结果 | 级别 | 说明 |
|---|---|---|
| 每个表达式/模式/局部的最终类型 | ✓ | 已解析、无变量；区分“错误类型”与正常类型 |
| per-expr adjustment 列表 | ✓（只有 `NeverToAny`） | M2 起加 Deref/Borrow(two-phase)/Pointer；顺序有意义 |
| 运算符：内建 or 重载 callee | ✓（只有内建） | 不要“先记重载再撤销” |
| cast 种类、是否 coercion cast | ✓（数值类） | 见 casts.md |
| 调用目标 `DefKey` + 泛型实参 | ✓（无泛型） | M3 起实参来自推导 |
| 绑定模式、模式 deref 序列 | M2 | match ergonomics |
| 字段下标 | M2 | Place 投影 |
| 方法 callee（含接收者 adjustment） | M2/M3 | |
| 闭包签名、捕获、kind | M3 | 排在 fallback 之后 |
| 用户类型注解（含 region） | T22 | borrowck 需要检查注解里的生命周期 |

- **typeck 结果必须先完整算出（包括 fallback 与 writeback 式的“全部解析、无变量”检查），再构建 Body**。原因：LUB coercion 会回改早先表达式的 adjustment；fallback 在全函数末尾才决定 `{integer}` 的类型；运算符是否内建要到 writeback 才最终确定。这与 [frontend.md](../../architecture/frontend.md) 中 “TypedBody 每个表达式/Place 有类型” 的保证一致，但意味着 typeck 输出应是“side table + 后续 Body builder”，而不是边 typeck 边产出 CFG。
- 错误时 rustc 不建 MIR；kfrc 的 TypedBody 产物在 `tainted` 时也应不进入 SoN（[红线](../../README.md)：错误恢复值不进入 codegen）。

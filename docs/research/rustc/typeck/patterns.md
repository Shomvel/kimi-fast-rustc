# 模式类型检查与 match ergonomics

源码：`compiler/rustc_hir_typeck/src/pat.rs`；edition 2024 报错位于 `compiler/rustc_mir_build/src/thir/pattern/migration.rs`。链接指向 commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`。

## 1. 入口与状态

- `check_pat_top(pat, expected, ..)`（[pat.rs#L380](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L380)）：初始 `PatInfo { binding_mode: ByRef::No, max_ref_mutbl: MutblCap::Mut, .. }`（[#L92](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L92)）。`TopInfo` 只用于诊断。
- 调用点：`let`（初始化表达式类型为 expected）、函数/闭包参数、match 各 arm（scrutinee 类型）、`if let`/`while let`。模式通过约束 expected 参与推导（`let (a, b) = x;` 会把 `x` 的变量定为元组）。多数构造用 `demand_eqtype_pat`，即**等同**而非子类型（理由见 `check_pat` 中的 note_1 注释，[#L436](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L436)）；只有字面量和常量用子类型（`check_pat_lit` 末尾 `demand_suptype`，[#L938](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L938)）。
- `check_pat`（[#L406](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L406)）：先解析路径类模式的 `Res`，算 `AdjustMode`，调 `check_pat_inner`，`write_ty(pat.hir_id, ty)`。

## 2. 默认绑定模式（RFC 2005）

- **何时剥引用**：`calc_adjust_mode`（[#L768](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L768)）。
  - `Peel`：元组、范围、slice、never、结构体/元组结构体/路径（路径解析为常量时除外）、非引用类型的字面量。
  - `Pass`：绑定、`_`、`&`/`&mut` 模式、或模式、类型为引用的字面量（如 `"abc"`，所以 `match &s { "abc" => .. }` 中 `s: &str` 报类型不符，`tests/ui/rfcs/rfc-2005-default-binding-mode/lit.rs`）、常量路径。
- **剥一层**：`check_pat_inner` 中 expected 为 `&T`/`&mut T` 且模式可剥时（[#L517](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L517)）：向 `pat_adjustments[pat]` 追加 `PatAdjustment { kind: BuiltinDeref, source: &T }`，用 `adjust_pat_info` 更新绑定模式，然后以 `T` 递归同一模式。剥之前先 `try_structurally_resolve_type`，expected 若仍是变量则不剥。
- **绑定模式转移** `adjust_pat_info`（[#L680](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L680)）：`move` 遇 `&` → `ref`，遇 `&mut` → `ref mut`；`ref mut` 遇 `&` → `ref`；`ref` 永远保持 `ref`。变为 `ref` 后 `max_ref_mutbl = Not`。
- **绑定** `check_pat_ident`（[#L1148](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L1148)）：
  - 无修饰的 `x`：采用默认模式。
  - `mut x` 在默认模式为 `ref` 时：≤2021 **重置为按值**（`BindingMode(ByRef::No, Mut)`）；同时登记迁移信息。
  - 显式 `ref x`/`ref mut x` 在默认模式为 `ref` 时：使用显式模式；同时登记迁移信息。
  - 结果写入 `pat_binding_modes`；绑定类型 = `ByRef::Yes(m)` 时 `&m expected`，否则 `expected`，与局部变量类型 `demand_eqtype`。或模式中同名绑定须类型一致。
- **显式 `&`/`&mut` 模式** `check_pat_ref`（[#L2696](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L2696)）：
  - 默认模式已是 `ref`（处在“继承引用”之下）时，稳定版规则 `InheritedRefMatchRule::EatBoth { consider_inherited_ref: false }`（[#L342](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L342)）：把默认模式重置为按值，并登记迁移信息；然后 `&` 模式与**底层**类型匹配。
  - 匹配：expected 是同可变性的引用则剥一层；否则建 `&m ?I` 与 expected 等同（`&` 不能匹配 `&mut`，除非开启 RFC 3627 实验特性）。
  - `EatOuter`/`EatInner`/`consider_inherited_ref: true` 都只在 `ref_pat_eat_one_layer_2024*` 实验特性下启用。

## 3. edition 2024 的变化（已核实）

- typeck 本身对 2021/2024 用同一套规则推类型；差别在 `add_rust_2024_migration_desugared_pat`（[#L3229](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L3229)）记入 `rust_2024_migration_desugared_pats` 的三类情况：默认模式非 move 时出现 **`mut` 绑定修饰**、**显式 `ref`/`ref mut`**、**`&`/`&mut` 模式**。
- THIR 模式降低时统一发出（[migration.rs#L50](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/pattern/migration.rs#L50)）：任一相关 span 属于 edition ≥ 2024 时是**硬错误**；否则是 `RUST_2024_INCOMPATIBLE_PAT`（默认 Allow 的 future-incompat lint）。错误信息形如“cannot explicitly dereference / borrow / mutably bind by value within an implicitly-borrowing pattern”（`tests/ui/pattern/rfc-3627-match-ergonomics-2024/min_match_ergonomics_fail.rs`）。
- 因此 2024 不是“新规则”，而是把旧版里语义可疑的写法（`mut` 重置模式、在继承引用下写 `&`）禁止，为以后改用 RFC 3627 规则留空间。

## 4. 其他模式

- 元组/结构体/元组结构体：字段数检查，按字段类型递归；结构体字段索引写 `field_indices`。
- slice/数组：`check_pat_slice`（[#L3001](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L3001)），数组长度与固定元素数比对；`..` 绑定得到子数组或子 slice。
- 字面量：类型由 `check_pat_expr_unadjusted` 给出（整数字面量同样可能是 `{integer}`），再 `expected :> lit_ty`；`b"..."` 在 expected 为 `&[u8]` 时改为 slice 类型。
- 范围：`check_pat_range`（[#L1017](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/pat.rs#L1017)）两端各自与 expected 等同，类型须为数值或 `char`。
- `box`、`deref!` 模式与 `deref_patterns` 是不稳定特性。

## 对 kfrc 的含义

- 标量 M1 的模式只有绑定、`_`、整数/bool 字面量、可能的元组；不需要 match ergonomics，但 `pat_binding_modes` 与 `pat_adjustments` 两张表的位置要在 Typed Body 的设计里预留：M2 加入引用后，Body 构建要根据它们插入隐式 deref 并决定绑定是 move/copy 还是借用。
- 首选按 edition 2024 实现：迁移表中的三类写法在 2024 直接报错，kfrc 可以把它们当“拒绝”处理，从而不必实现“`mut` 重置绑定模式”这一旧语义；若要支持 2021 crate，则需完整实现 §2 的旧规则（只发 lint）。
- 绑定与 expected 用**等同**而不是子类型，模式不产生 coercion；kfrc 不需要在模式检查中调用 coercion 模块。

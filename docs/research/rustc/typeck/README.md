# 类型检查与推导（rustc_hir_typeck / rustc_infer）

基于官方 rustc 1.94.1（commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`）。链接全部固定到该 commit，行号经过核对；标“推断”的内容未在源码中直接验证。trait 求解本身见 [../traits/](../traits/README.md)，这里只写 typeck 与求解器的接口。

## 文件

| 文件 | 内容 |
|---|---|
| [infer-ctxt.md](infer-ctxt.md) | `InferCtxt`、ty/int/float 变量、union-find、undo log 快照（`probe`/`commit_if_ok`/fudge）、sub/eq/lub 与 variance、region 在 typeck 中被忽略并擦除 |
| [expectation-and-fallback.md](expectation-and-fallback.md) | typeck root 的阶段顺序、`Expectation`、`check_expr` 各分支、obligation 注册/select 时机、整数/浮点 fallback、never fallback（edition 2024） |
| [coercion.md](coercion.md) | `Coerce::coerce` 判定顺序、reborrow/`&mut→&`/deref/unsize/never/fn item/闭包 coercion、`CoerceMany` 与 LUB |
| [references.md](references.md) | `&T`/`&mut T` 的 variance、显式借用、autoref/reborrow/两阶段借用作为 adjustment |
| [autoderef-and-methods.md](autoderef-and-methods.md) | `Autoderef`、方法 probe（步骤 × by-value/`&`/`&mut` × inherent/trait）、confirm、运算符的内建/重载判定、索引 |
| [calls-and-closures.md](calls-and-closures.md) | 被调者与 `FnDef`、`check_argument_types`（期望实参、先非闭包后闭包）、闭包签名推导、捕获分析时机 |
| [casts.md](casts.md) | `as` 的延迟检查、先试 coercion、`CastTy` 规则表、MIR cast 分类 |
| [patterns.md](patterns.md) | 默认绑定模式、引用剥离、`&` 模式、edition 2024 的硬错误 |
| [writeback.md](writeback.md) | `TypeckResults` 字段、写表规则、writeback 解析与擦除、THIR 如何执行 adjustment；kfrc Typed Body 应记录的结果 |
| [test-cases.md](test-cases.md) | 代表性 `tests/ui` 用例及其考察的规则 |

## 概览：一个函数怎样被检查

```text
typeck(def) ──闭包转发到外层──▶ typeck_with_inspect(root)
  InferCtxt(ignoring_regions, in_hir_typeck) + FulfillmentContext(旧求解器)
  check_fn: 参数模式 → 函数体（Expectation 向下、类型向上；coercion 写 adjustment；obligation 入队）
  ─▶ type_inference_fallback（{integer}→i32, {float}→f64, diverging→()/!）
  ─▶ check_casts ─▶ closure_analyze（捕获、闭包 kind）─▶ 歧义报告
  ─▶ writeback：解析全部变量、擦除 region、修正内建运算 ─▶ TypeckResults（side table）
THIR: 节点 + 逐个 apply adjustment ─▶ MIR build ─▶ borrowck（重新推 region）
```

要点：

1. **推导是“收集 + 末尾定型”。** 表达式检查时类型可以是变量；fallback、cast 判定、闭包捕获都在全函数检查完之后；直到 writeback 才保证无变量。
2. **Expectation 是提示，coercion/eq 才是约束。** 同一位置的约束种类（coerce / 子类型 / 等同）由调用方决定，见 [expectation-and-fallback.md §2](expectation-and-fallback.md)。
3. **所有隐式操作都是 per-expr adjustment 列表**（`NeverToAny`、`Deref`、`Borrow`、`Pointer`），由下游逐个执行，typeck 后不再推断。
4. **region 不在 typeck 求解。** 只创建变量、写 adjustment 时不带 region，writeback 全部擦除；borrowck 在 MIR 上重做。
5. **side table 不随快照回滚**，只在成功路径写入。

## 本轮发现（会改变 kfrc 计划的）

1. **i32 是必需类型。** rustc 对无约束整数字面量 fallback 到 `i32`（[fallback.rs#L123](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fallback.rs#L123)）。kfrc 现有 `i64/u64/bool/()` 无法正确表达 `let x = 1;` 的语义（溢出点不同、`3_000_000_000` 应被拒绝）。T06 必须加入 `i32`（以及 `!`），或对需要 fallback 的字面量明确报未支持。
2. **Typed Body 不能边 typeck 边生成。** LUB coercion 会回改先前分支的 adjustment（[coercion.rs#L1315](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L1315)），fallback 和 cast 判定在函数末尾，运算符是否内建到 writeback 才定。T06 应拆成“typeck → side table（≈TypeckResults）→ Body builder（≈THIR+MIR build）”两步。
3. **阶段顺序是语义。** `cap as u8` 中 `cap` 为 `i32`、`256 as u8` 中字面量为 `u8`、`let _: u8 = 0 + 0` 能通过，都取决于“cast 在 fallback 后”“字面量看 castable 期望”“内建运算结果取 lhs 类型”。kfrc 的 M1 回归要包含这些（见 [test-cases.md](test-cases.md)）。
4. **拒绝分类包含默认 Deny 的 lint。** `overflowing_literals`（[rustc_lint/types.rs#L69](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_lint/src/types.rs#L69)）、`arithmetic_overflow`、`unconditional_panic`、`dependency_on_unit_never_type_fallback` 等在 1.94.1 都是 Deny。差分测试按“rustc 是否接受”分类时，这些 lint 必须实现或从对拍集中剔除。
5. **edition 有两种粒度。** never fallback 看 crate 的 `sess.edition()`（[fn_ctxt/mod.rs#L504](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/mod.rs#L504)）；模式 2024 报错和闭包精确捕获看 **span** 的 edition（宏展开来源）。kfrc 的 span/hygiene 信息（T04/T23）要带 edition。
6. **HIR typeck 用的是旧 trait 求解器。** 1.94.1 默认只在 coherence 用新求解器（[config.rs#L993](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L993)）。typeck 里大量 `if next_trait_solver()` 分支；若 trait 模块移植新求解器，需与对照 rustc 的默认行为核对推导差异（unsize、规范化时机、`structurally_resolve`）。
7. **typeck 需要求解器的三种接口**，不只是“注册 + select”：①fulfillment 队列（`register`/`select_where_possible`/最终报歧义）；②局部选择循环（`coerce_unsized` 当场证明 `CoerceUnsized`/`Unsize`）；③探测式评估（`predicate_may_hold`：autoderef 判 `Deref`、方法候选筛选）。另需“按 self 类型列出挂起 obligation”供闭包签名推导。
8. **即使无 trait，标量版也需要挂起约束。** `-x`（`x: {integer}` 后定为 `u64`）要报 E0600（推断：rustc 靠挂起的 `Neg` obligation），never 引入后还有 `Subtype(?a, ?b)`/`Coerce(?a, ?b)`；M1 的推导上下文就要有“待证谓词队列”，T12 再接到 trait 求解器。

## kfrc 最小闭包与移植建议

### T06（M1 标量）需要的 typeck 结果

见 [writeback.md 末节](writeback.md)。M1 最少：每个表达式/模式/局部的最终类型；per-expr adjustment（仅 `NeverToAny`）；运算符“内建”标记；数值 cast 种类；调用目标 `DefKey`；`tainted` 标志。表结构按 M2/M3 预留绑定模式、模式 deref、字段下标、方法 callee、闭包信息。

### 推导变量与快照（为扩展预留）

- 类型表示：`TyKind::{Int(w), Uint(w), Bool, Tuple([]), Never, Infer(InferVar), Error, ...}`，`InferVar::{Ty(TyVid), Int(IntVid), Float(FloatVid)}`。首轮就保留 `Ty` 变量（`let x;`、`loop {}`/`return` 的发散目标、无期望的分支初值都需要）与 `Never`。
- 推导期类型是 **body 局部**的：可以引用全局驻留的 `TypeId`，但 `InferVar` 不得出现在 TypedBody、缓存键或跨任务产物中（[identities.md](../../../design/ids.md) 要求数字 ID 不作持久键）；“发布前无变量”的检查对应 writeback 的断言。
- 存储：三张 union-find（值 = Known/Unknown；int 值域 = 有/无符号宽度）+ 一条 undo log；快照 = log 长度，严格嵌套；提供 `commit_if_ok`、`probe`。首轮即使不回滚也实现，coercion 尝试、方法 probe、fudge 都建立在它上面，补做的代价是改所有调用点。
- 关系 API 从一开始带 variance：`relate(a, b, Variance) -> Result<Goals, TypeError>`；M1 只用 `Invariant`。coercion API 返回 `(adjustments, target, goals)`，由调用方在成功后写表。
- region：类型里留 region 槽，typeck 一律填 `Erased`（或单一占位变量），不进相等判断。

### 可以推迟的

region 变量与约束（rustc typeck 也不求解）；方法 probe/confirm 与 autoderef（需 `Deref`，T21）；unsize、`CoerceUnsized`、fn item/闭包 → fn 指针（M2/M3）；match ergonomics（M2，按 edition 2024 实现可省掉旧的 `mut` 重置语义）；闭包推导与捕获（M3）；opaque/`impl Trait`、新求解器分支、`pin_ergonomics` 等不稳定特性；全部诊断建议代码（`method/suggest.rs`、`fn_ctxt/suggestions.rs`、`errors.rs`、`adjust_fulfillment_errors.rs`、`demand.rs`、`arg_matrix.rs`、`prelude_edition_lints.rs` 共 12,812 行，占 `rustc_hir_typeck` 46,327 行的 28%）。

### 与 TyCtxt/query 的耦合点

typeck 读取的外部信息（kfrc 需要由自有语义快照提供）：`fn_sig`、`type_of`、`predicates_of`、`variances_of`、`lang_items`（`Deref`、`Neg`、`Add`…、`CoerceUnsized`、`Unsize`）、`inherent_impls`/`incoherent_impls`、关联项表、`upvars_mentioned`（名称解析产物）、`recursion_limit`、crate/span edition、feature 开关、lint 级别。还有两个查询型调用：`method_autoderef_steps`（canonical 输入，[probe.rs#L575](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/probe.rs#L575)）和 `typeck` 对闭包的转发。kfrc 不需要 query 系统，直接在 body 任务内计算；只需保证这些输入来自不可变快照。

### 工作量与风险（规模为实测行数，比例为推断）

- `rustc_hir_typeck` 46,327 行，去掉诊断约 33.5k；推导核心（`rustc_infer` 的 `mod.rs`/变量/快照/relate/resolve 加 `rustc_type_ir` relate、autoderef）约 6.1k 行。
- T06 标量版：§“需要的结果” + 字面量/运算符/if/match/loop/return/let/cast 规则 + 整数 fallback + never 规则，按本目录各节逐条移植，规模小（推断：千行级）。主要风险是 i32 缺失和“边检查边建 Body”的现有结构。
- T15/M2（引用、指针、聚合）：coercion 的 ref/raw/unsize 分支、autoderef、模式默认绑定模式、字段与索引；风险在 adjustment 组合规则和两阶段标记与 borrowck 的契约（T16）。
- T21/M3（泛型/trait/方法/闭包）：probe/confirm、闭包推导与捕获、fudge、与 trait 求解器的三种接口；这是最大块（推断：数千到上万行）。风险是求解器版本差异与“推导顺序敏感”的用例（`order-dependent-cast-inference.rs`、`coerce-reborrow-multi-arg-fail.rs`），以及 rustc 自己标为 FIXME 的行为（#149379 fudge、传递 coercion ICE）需要按 rustc 原样复现而不是“修正”。

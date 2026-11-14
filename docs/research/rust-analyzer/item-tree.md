# item tree 与“接口先发布”：签名与函数体分离

对应设计：[design/frontend/overview.md](../../design/frontend/overview.md)、[design/goals/compile-time.md](../../design/goals/compile-time.md)（C4）、[design/build/cache.md](../../design/build/cache.md)。

## RA 的四级切分

```text
文件文本 ─→ 语法树 + AstIdMap ─→ ItemTree（每 HirFileId 一份）─→ DefMap（每 crate / 每块）
                                     │
                                     └─→ 每项签名 FunctionSignature::of(FunctionId)
                                                       └─→ Body::of(DefWithBodyId) ─→ infer
```

- **ItemTree 只有项。** 头注释（[item_tree.rs L1–L31](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/item_tree.rs#L1)）说明它的主要目的就是增量的“失效屏障”：在函数体内打字通常不改变该文件的 ItemTree，于是名称解析和项数据都不重算。它按文件建、与 crate 无关、cfg 未求值。
- **比签名还瘦。** [`Function`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/item_tree.rs#L568) 只有 `name` 和 `visibility`；`Struct` 多一个值命名空间构造器的可见性；`Impl` 只有 `is_trait_impl`。名称解析需要的只有这些，所以改参数类型也不会让 `DefMap` 重算。
- **签名单独一层。** [`FunctionSignature`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/signatures.rs#L613)（名字、泛型、参数/返回类型、ABI、flags）按需从该项的语法节点降低，不读函数体。
- **函数体与位置分开。** `Body::with_source_map` 同时产出 `Body` 和 `BodySourceMap`，消费者只依赖 [`Body::of`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/expr_store/body.rs#L137)；签名同理（[`ExpressionStore` / `ExpressionStoreSourceMap`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/expr_store.rs#L160)）。只移动位置时 `Body` 不变，Salsa 的 backdating 让下游全部命中。函数体查询带 `lru = 512`（[L89](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/expr_store/body.rs#L89)），内存里最多留 512 个。
- **有测试守着。** [`nameres/tests/incremental.rs`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/nameres/tests/incremental.rs#L129) 用“哪些查询被执行”的事件日志断言：函数体内、宏调用内、属性内打字都不重算 `DefMap`，ItemTree 未变时不重新解析。

## 与 kfrc 接口的对照

| | RA ItemTree / 签名 | kfrc“接口先发布”（C4） |
|---|---|---|
| 粒度 | 每文件 / 每项 | 每定义产物，按 crate 发布 metadata |
| 内容 | 语法级、未解析；签名层已降低但未类型检查 | 已解析、已类型化：定义身份、类型约束、可见性、layout/impl/常量依赖 |
| 目的 | 进程内失效屏障 | 跨任务、跨进程的发布屏障 + 缓存键 |
| 屏障被打穿的地方 | RPIT 要推导函数体；`const _: () = { impl … }` 里的 impl 要读体（[method_resolution.rs L629](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-ty/src/method_resolution.rs#L629)） | “接口体”：`impl Trait` 返回、async、const/const fn、数组长度、static |

RA 证明“签名不读函数体”可以作为一条结构性不变量守住，并且例外可以枚举。kfrc 的接口体清单与 RA 的例外基本对应，但 RA 有一处是近似：只看 `const _` 块里的 impl，别的函数体里的 impl 不进全局表。

## 体内的项

**rustc 中函数体里的 impl 全局可见**（参与 coherence 和方法解析），`#[macro_export]` 的 `macro_rules` 写在函数体里也导出到 crate 根（推断，T05 用例核实）。因此 kfrc 的“接口”不能只是“不读函数体”，而应是：

- 接口 = 所有项（**含函数体内嵌套的项**）的签名与可见性，**不含任何表达式**。
- 收集接口要扫描函数体里的项，但不降低表达式。RA 的 [`block_item_tree_query`](https://github.com/rust-lang/rust-analyzer/blob/03ae6be57aa063bb373f69eead3b8e15168a45d2/crates/hir-def/src/item_tree.rs#L192) 正是这个数据结构：块内项单独成树。
- 接口指纹覆盖“体内项树”的哈希，而不是体的文本哈希：给函数体加一条语句不改变接口，加一个 `impl` 才改变。

## 建议做法

1. **分三层发布，而不是一层。** 名字表（模块树、名字、可见性、再导出）→ 签名（泛型、类型、where 子句、impl 头）→ 函数体。下游 crate 的名称解析只等第一层（推断：rustc 的 rmeta 把它们合在一起，收益需在 4/8/16 核上测）。
2. **[body.md](../../design/frontend/body.md) 的 `span_map` 移出 `Body`。** 参照 `Body`/`BodySourceMap` 分离：CheckedBody 的哈希不含位置；`line!()`/`column!()`/`file!()` 的值仍是语义输入（[cache.md](../../design/build/cache.md#源位置隔离) 已有此边界）。
3. **接口收集扫描体内项。** 在 [overview.md](../../design/frontend/overview.md) 的 ResolvedInterface 一行写明“含函数体内嵌套项，不含表达式”，并在失效测试里加“函数体内新增 impl”。
4. **失效测试按 RA 的方式写。** 断言“哪些任务被重跑”，而不只是最终产物相同；RA 的 `typing_inside_*_should_not_invalidate_*` 用例可以逐条改写成 kfrc 的编辑场景。
5. **不借：** ItemTree 的“与 crate 无关、cfg 未求值”（kfrc 不缓存 AST，见 [cache.md](../../design/build/cache.md#存什么)）；签名每次从语法树重新降低（依赖内存中常驻语法树）。

# W08：增量缓存的编辑场景

检验 [design/build/cache.md](../../design/build/cache.md) 和 [design/build/cache.md](../../design/build/cache.md) 的键设计。最重要的问题是有没有漏掉依赖，因为漏掉会导致错误命中；其次是有没有过度失效。命令：`RUSTC_NIGHTLY=<nightly> scripts/w08.sh <workdir>`。

## 1. 程序

源码在 [scripts/w08/](scripts/w08)。上游 `lib.rs` 把每类函数放进一个模块：rustc 增量编译时，一个模块就是一个 CGU，方便按 CGU 观察。下游 `bin.rs` 的 `main` 调用全部函数并打印。脚本用 `sed` 对 `lib.rs` 做一次编辑，生成一个场景。

| 模块 | 项 | 场景 |
|---|---|---|
| `generic` | `generic_sum<T: Copy + Into<u64>>(&[T]) -> u64` | E4：`.sum()` 的结果乘 2 |
| `inline` | `#[inline] scale(u32) -> u32` | E5：`wrapping_mul(3)` 改成 `5` |
| `plain` | 非泛型 `pub fn plain(u32) -> u32` | E3：`x - 10` 改成 `x - 9`；E6：返回类型改成 `u64` |
| `panics` | `checked_div`（含 `panic!`）；`line_here() = Location::caller().line()` | E2 下行号变化 |
| `point` | `struct Point { pub x, pub y }`、`Point::new`、`norm1(&Point)` | E7：加私有字段 `id: u64` |
| `shape` | `trait Shape`，`impl Shape for u8 { area = 1 }` | E8：新增 `impl Shape for i32 { area = 2 }` |
| （全文件） | — | E1：改首行注释文字；E2：文件最前面加一行；E9：两个 crate 都用 `-Cpanic=abort` |

`bin` 中的 `Shape::area(&200)` 只靠 impl 集合推断字面量类型：只有 u8 的 impl 时它是 u8；加了 i32 的 impl 后变成歧义，回退到 i32。

## 2. rustc 现在怎么做（实测）

nightly 1.101.0（2026-10-24），用 `-Cincremental` 和 `-Zhuman-readable-cgu-names`。每个场景都从同一份基线增量目录的副本开始。**判定方法**：rustc 复用 CGU 时，会把上一会话的 work product 硬链接到新会话，所以 inode 不变即为复用（`redone` = inode 变了；名字后面的 `=` 表示重新生成了但字节相同）。输出列依次为 `generic_sum scale plain checked_div norm1 area line`。`rel` 配置（`-Copt-level=3 -Clto=off`）：

```text
  base             out=6 36 2 8 16 1 27
  E1-comment       out=6 36 2 8 16 1 27 | lib CGU 4 reused 4 redone 0 (same bytes 0): | bin CGU 12 reused 12 redone 0 (same bytes 0):
  E2-lineshift     out=6 36 2 8 16 1 28 | lib CGU 4 reused 2 redone 2 (same bytes 1): w08lib-panics w08lib-plain= | bin CGU 12 reused 9 redone 3 (same bytes 3): core-in-w08bin-iter-adapters-map.volatile= w08bin= w08lib-in-w08bin-generic.volatile=
  E3-plain-body    out=6 36 3 8 16 1 27 | lib CGU 4 reused 3 redone 1 (same bytes 0): w08lib-plain | bin CGU 12 reused 12 redone 0 (same bytes 0):
  E4-generic-body  out=12 36 2 8 16 1 27 | lib CGU 4 reused 4 redone 0 (same bytes 0): | bin CGU 12 reused 11 redone 1 (same bytes 0): w08lib-in-w08bin-generic.volatile
  E5-inline-body   out=6 60 2 8 16 1 27 | lib CGU 4 reused 4 redone 0 (same bytes 0): | bin CGU 12 reused 11 redone 1 (same bytes 0): w08bin
  E6-signature     out=6 36 2 8 16 1 27 | lib CGU 4 reused 3 redone 1 (same bytes 0): w08lib-plain | bin CGU 12 reused 11 redone 1 (same bytes 0): w08bin
  E7-add-field     out=6 36 2 8 16 1 27 | lib CGU 4 reused 3 redone 1 (same bytes 0): w08lib-point | bin CGU 12 reused 11 redone 1 (same bytes 0): w08bin
  E8-new-impl      out=6 36 2 8 16 2 27 | lib CGU 4 reused 3 redone 1 (same bytes 0): w08lib-shape | bin CGU 12 reused 11 redone 1 (same bytes 0): w08bin
  E9-panic-abort   out=6 36 2 8 16 1 27 | lib CGU 4 reused 0 redone 4 (same bytes 0): (all) | bin CGU 12 reused 0 redone 12 (same bytes 0): (all)
```

`dev` 配置（`-Copt-level=0 -Cdebuginfo=2`）与 rel 只在 E2 上有明显差别，因为行表变了：

```text
  E2-lineshift     out=6 36 2 8 16 1 28 | lib CGU 7 reused 3 redone 4 (same bytes 0): w08lib-panics w08lib-plain w08lib-point w08lib-shape | bin CGU 25 reused 20 redone 5 (same bytes 3): core-in-w08bin-iter-adapters-map.volatile= core-in-w08bin-iter-traits-accum.volatile= core-in-w08bin-slice-iter.volatile= w08lib-in-w08bin-generic.volatile w08lib-in-w08bin-inline
```

O3 汇编（主机 aarch64；关闭跨 crate 内联推断，与 `-Cincremental` 下一致）：

```text
  base          __RNvNtCsguSlGpsZlcB_6w08lib5point5norm1:: ldp w8, w9, [x0] ; add w0, w9, w8 ; ret
  E7-add-field  __RNvNtCsguSlGpsZlcB_6w08lib5point5norm1:: ldp w8, w9, [x0, #8] ; add w0, w9, w8 ; ret
  base          __RNvNtCsguSlGpsZlcB_6w08lib6panics9line_here:: mov w0, #27 ; ret
  E2-lineshift  __RNvNtCsguSlGpsZlcB_6w08lib6panics9line_here:: mov w0, #28 ; ret
```

要点如下。
- E1：rustc 增量编译**一个 CGU 都不重做**。design/build/cache.md 引用的 28.7% 是在非增量 release 下测得的。
- E2：rel 下重做的 5 个 CGU 中有 4 个字节相同，属于纯粹的过度失效。
- E7：`norm1` 的源码没变，但字段偏移变了。
- E8：bin 的源码没变，它点名的项也都没变，输出却从 1 变成 2。
- `line_here` 的行号被常量折叠进了代码。
- rustc 在增量模式下故意关闭跨 crate 内联推断（`compiler/rustc_mir_transform/src/cross_crate_inline.rs`：“the additional inlining that inference permits also creates more work for small edits”），所以 E3 时 bin 不重做。

## 3. kfrc 编译计划：各产物的命中与失效（推断）

产物和键见 design/build/cache.md：CheckedBody = 语义哈希 + 依赖接口哈希；R 图 = CheckedBody + 被内联者的 R 图 + 优化配置；摘要 = 答案键；机器码 = 答案键 + 被内联者的答案键 + L 级配置 + target；Location 表和行表单独生成。下表只列失效项，“—” 表示全部命中。**加粗**的是按现有的键会错误命中的地方。

| 场景 | CheckedBody | R 图 | 摘要 | 机器码片段 | Location 表 / 行表 |
|---|---|---|---|---|---|
| E1 注释 | — | — | — | — | — |
| E2 行号移动 | —（span 在旁表） | — | — | —（前提：Location 不透明，W08-4） | lib 两张表重写 |
| E3 非泛型体 | `plain` | `plain` | `plain` | `plain`；如果 bin 的 `main` 内联了 `plain`，`main` 也失效（W08-7） | — |
| E4 泛型体 | `generic_sum` | `generic_sum` | `generic_sum<u8>` | bin 的 `generic_sum<u8>`，以及内联它的 `main` | — |
| E5 `#[inline]` 体 | `scale` | `scale` | `scale` | bin 的 `main`（lib 不为 `scale` 发射代码，见 size.md） | — |
| E6 签名 | `plain`；bin `main`（接口哈希变了） | `plain`、`main` | `plain` | `plain`、`main` | — |
| E7 加字段 | `Point::new`；`norm1`、`main` 重新检查，结果相同 | `Point::new` | `Point::new` | `Point::new`、`main`；**`norm1` 的 R 图和答案键都没变，按现有的键会命中旧偏移**（W08-1） | — |
| E8 新 impl | lib `shape`；**bin `main` 应该失效，但它依赖的接口没有变**（W08-2） | `main` | `main` | `main` | — |
| E9 panic=abort | 按 body.md，Body 与 panic 策略无关，本可全部命中；但键里有全部 cfg，所以全部失效（W08-6） | 全部 | 全部 | 全部 | — |

对比 rustc：E2 上 kfrc 能省下 rustc 重做的那 5 个 CGU，其中 4 个本来就字节相同；E9 上 CheckedBody 有可能全部命中，而 rustc 整个会话作废。E3 正相反：如果 kfrc 对非 `#[inline]` 函数也做跨 crate 内联，失效范围会比 rustc 更大。

## 4. 代价核算

- **重算量**：E1、E2 只重算两张表；E3–E6 每个场景失效 1–2 个定义，外加内联了它们的调用者。这些都是推断，要等 T39 的失效矩阵实测。
- **缓存字节**：本例没有测量。S 图不存储；库里的非泛型函数只存 R 图，不预先生成机器码（预生成单独计数，见 compile-time.md）。
- **解释性**：每次命中都要能列出匹配了哪些依赖（design/build/cache.md）。W08-1/2/3 说明，“依赖”必须包括布局查询、impl 集合查询和摘要查询，而不只是“名字引用到的项”。

## 5. 设计细化

| # | 问题 | 证据 | 建议改法 | 文件 |
|---|---|---|---|---|
| W08-1 | **错误命中**：非泛型实例在 L 级要用具体类型的布局，但这不是孔位，所以不进答案键。E7 时 `norm1` 的 R 图不变、答案为空，会命中带旧偏移的机器码 | `[x0]` → `[x0, #8]`；rustc 重做了 `w08lib-point` | L 级的每次布局查询（不管类型是不是具体的）都记入实例的答案向量；或者把机器码键改为“答案键 + 布局指纹列表” | design/build/cache.md；generics-specialization.md 孔位 |
| W08-2 | **错误命中**：CheckedBody 的键只有“依赖接口哈希”，没有覆盖 trait 选择读过的 impl 集合（包括“只有一个候选”“没有候选”这类否定事实） | E8 时 bin 输出 1 → 2；rustc 重做了 `w08bin` | Body 的依赖清单为每个做过选择的 trait 记录 impl 集合指纹（按 crate 分），方法探测要记录固有 impl 集合；不透明类型的隐藏类型、auto trait 泄漏同理 | design/build/cache.md；design/build/cache.md 键必须包含 |
| W08-3 | **错误命中**：design/build/cache.md 写“被调者改了但没被内联，调用者不失效”。但调用者的优化用到了被调者的摘要（`may_unwind`、`captures`、`memory`） | W05-8：`grow_one` 的 `captures=∅` 决定了循环的形状 | 机器码键加入每个被查询过的摘要的哈希；摘要不变时仍然命中（early cutoff） | design/build/cache.md；optimization-pipeline.md |
| W08-4 | **错误命中**：如果优化器把 `Location` 的内容折叠进代码，“代码身份不含源位置”就不成立了 | rustc 把 `line_here` 折叠成 `mov w0, #27/#28` | 写明 `Location` 条目对 R/S/L 级都不透明：不对它做 load 转发和常量折叠；`line!()` 是字面量，正常进入语义哈希 | values-and-types.md 不进图的东西；design/build/cache.md 源位置隔离 |
| W08-5 | **错误命中风险**：R 图、摘要、机器码键里的“优化配置”和“L 级配置”没有列出具体字段。panic 策略在 SoN 之前就裁剪了 R 图；rustc 允许 unwind 编译的 lib 链接进 abort 的 bin | E9；body.md 的“按 panic 策略裁剪”一节 | 逐项列出：panic 策略（定义方和消费方各一份）、定义方 crate 的 overflow-checks 和 debug-assertions、target features、unwind 表、frame pointer、重定位模型；Location 表还要加上 `--remap-path-prefix` | design/build/cache.md |
| W08-6 | **过度失效**：CheckedBody 的键包含全部 cfg，panic=abort 会让所有 Body 失效 | E9；body.md 规定 Body 与 panic 策略无关 | 只记录真正求值过的 cfg 谓词（例如 `cfg(panic = "unwind")`） | design/build/cache.md；design/build/cache.md |
| W08-7 | **过度失效（权衡）**：所有 R 图都在 metadata 里，非 `#[inline]` 的函数也可能被跨 crate 内联，上游改函数体就会让下游调用者失效 | E3 时 rustc 的 bin 不重做；rustc 在增量模式下特意关闭了跨 crate 内联推断 | 内联策略的输入加上“构建模式”：增量 dev 构建时只跨 crate 内联 `#[inline]` 函数和极小的叶函数；用失效矩阵测量这样做的代价 | optimization-pipeline.md 内联策略；compile-time.md |
| W08-8 | **口径**：early cutoff 的收益是和非增量 release 比出来的；rustc 增量编译下注释修改已经不重做任何 CGU | E1：`redone 0` | 改写 early cutoff 那一段：真正的收益在 E2（rustc 有 4/5 的重做字节相同）和 E9 | design/build/cache.md 键只含读过的东西 |

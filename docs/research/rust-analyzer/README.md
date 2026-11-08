# rust-analyzer 与可独立使用的 rustc 组件：怎么参考


**决策结果（2026-11-08）：** D1 依赖 4 个叶子算法 crate（钉 `=0.148.0`）；D2 复用 next-solver；D3 宏生成项用 rustc 形状；D4 不允许以 RA 源码作为移植来源。已写入 [总览](../../README.md#固定的技术决策) 和对应任务。

基于 rust-analyzer master [`03ae6be5`](https://github.com/rust-lang/rust-analyzer/tree/03ae6be57aa063bb373f69eead3b8e15168a45d2)（2026-11-08）与官方 rustc 1.94.1（[`e408947b`](https://github.com/rust-lang/rust/tree/e408947bfd200af42db322daf0fadfe7e26d3bd1)），2026-11-08 阅读。RA 链接固定到 `03ae6be5`，rustc 链接固定到 `e408947b`。未加标注的内容在源码中核实；“实测”指查询 crates.io、对发布包做源码 diff、`cargo check`；标“推断”的没有直接验证。目标是参考设计和算法，**不复制 RA 代码**；语义以 rustc 为准（固定决策“自有实现，参考 rustc”）。

## 结论

1. **RA 是同一语言的另一个前端，不是 rustc 的替身。** 它为 IDE 的容错和懒惰设计：名称解析不报歧义和重复定义，未解析的属性宏当作不存在，别的函数体里的 impl 不可见，内建 derive 不真正展开，环和类型错误降成回退值，不做借用检查，trait 一律走 next-solver。kfrc 要完整、精确的拒绝，**任何语义规则都不以 RA 为准**。
2. **RA 值得借的是增量友好的数据切分：** item tree 只有项（连签名都不含），签名、函数体、source map 各自一层；展开身份由调用位置决定；span 相对锚点项；durability 分级。它们分别对应 kfrc 的接口先发布、DefKey/并行展开定序、源位置隔离、缓存分级，但都要从“会话内驻留身份”改成持久键。
3. **可独立的 rustc crate 分两类。** 叶子算法（`rustc_lexer`、`rustc_parse_format`、`rustc_pattern_analysis`、`rustc_abi` 的 `LayoutCalculator`）通过 `&str` 或 trait 接 kfrc 自有类型，依赖它们不改变 kfrc 的数据模型。类型核心（`rustc_type_ir` + `rustc_next_trait_solver`）要求 kfrc 的类型就是 `TyKind<I>`，等于把类型表示交给外部。
4. **`ra-ap-*` 0.148.0 与 rustc 1.94.1 逐字节相同**（7 个 crate 的 `src/` diff 全为 0），并能用 rustc 1.94.1 stable 通过 `cargo check`（实测）。钉死这一版即可，不必跟随每周发布。
5. **复用 next-solver 的真实成本有了数据：** RA 的 `hir-ty/src/next_solver/` 共 19,838 行，其中 8,182 行是 rustc_infer 的移植；RA 自己也锁在 0.166.0（最新 0.184.0）。

## 组件 × 三档

| 组件 / 阶段 | 档 | 理由 | 详情 |
|---|---|---|---|
| 词法：`rustc_lexer` | **直接依赖（待决策 D1）** | 1,654 行纯函数、stable、闭包 4 个 crate；自写只会在原始字符串、Unicode、frontmatter 上制造分歧。cooking 仍自写 | [syntax.md](syntax.md) |
| 语法分析 | 参考设计、自写 | 语义与错误码以 `rustc_parse` 为准；借 RA 的“只出事件”和 fragment 入口；不用 rowan | [syntax.md](syntax.md) |
| 宏展开与 hygiene | 参考设计、自写 | 规则按 `rustc_expand`；借按调用位置驻留的展开身份、相对锚点 span、预存两种规范化的卫生表 | [macros-hygiene.md](macros-hygiene.md) |
| 格式串：`rustc_parse_format` | **直接依赖（待决策 D1）** | 1,563 行、stable；`format_args!` 降低必须与冻结 core 一致 | [macros-hygiene.md](macros-hygiene.md) |
| 名称解析与 import 不动点 | 不参考（以 rustc 为准） | RA 单调、先到先得、无 finalize，8192 轮后静默退出 | [resolve-defmap.md](resolve-defmap.md) |
| item tree / 签名与函数体分离 | 参考设计、自写 | 借四级切分和“签名不读函数体”不变量；接口要包含函数体内的项 | [item-tree.md](item-tree.md) |
| 类型推导 | 不参考（以 rustc 为准） | RA 的推导跟随 rustc 阶段顺序，但容错、只有 46 种诊断、用 next-solver | [types-traits.md](types-traits.md) |
| trait 求解：`rustc_type_ir` + `rustc_next_trait_solver` | 参考设计、自写（复用待 T14 决策 D2） | 维持 T12；RA 的 `next_solver/` 是实现 `Interner` 唯一的大规模先例 | [types-traits.md](types-traits.md) |
| 模式穷尽检查：`rustc_pattern_analysis` | **直接依赖（待决策 D1）** | `PatCx` 6 个关联类型 + 约 12 个方法，RA 用 518 行接上；算法本体约 3k 行且难 | [patterns-and-layout.md](patterns-and-layout.md) |
| 布局计算：`rustc_abi` | **直接依赖（待决策 D1）** | 只用 `LayoutCalculator`；计划本就要求与 rustc 位级一致 | [patterns-and-layout.md](patterns-and-layout.md) |
| 过程宏进程外执行 | 参考设计、自写 | RA 的 proc-macro-srv 是 `rustc_private`、随工具链发布；借进程池、版本协商、span 回问、依赖记录 | [proc-macro-srv.md](proc-macro-srv.md) |
| Salsa / base-db | 参考设计、自写 | 借 durability 与“读即记边”；内存驻留、不落盘、按需拉取与 kfrc 不同 | [salsa-cache.md](salsa-cache.md) |
| `rustc_index` | 参考设计、自写 | `IndexVec`/位集几百行；依赖它会带入过程宏 crate | [components.md](components.md) |
| `cfg` | 参考设计、自写 | 1,141 行，只借 `CfgExpr` 形状 | [components.md](components.md) |
| `project-model` | 不参考 | kfrc 被 Cargo 当作 rustc 调用，不读 `cargo metadata` | [components.md](components.md) |
| rowan / `syntax`、`hir-ty` 的 MIR 与解释器 | 不参考 | 无损树为编辑服务；RA 的常量求值不是 CTFE | [components.md](components.md) |

“直接依赖”的 4 个 crate 只在用户拍板后进入产品；未拍板前，它们作为 dev-dependency 做差分 oracle，不与固定决策冲突。风险与来源记录见 [risks.md](risks.md)。

## 需要用户决策

| # | 决策 | 与“自有实现”的冲突点 | 建议 |
|---|---|---|---|
| D1 | 产品是否依赖 4 个叶子算法 crate | 算法代码不是自己写的；数据模型仍自有（`rustc_abi` 例外：`LayoutData` 会成为 L 级读的结构，除非边界转换） | 依赖，钉 `=0.148.0`，按 crate 记录来源；不依赖则逐行移植并保留 crate 作差分 |
| D2 | trait 求解是否复用 next-solver | 类型表示变成 `rustc_type_ir::TyKind<KfrcInterner>`；求解器代码外部；升级成本见 RA | 维持 T12：自写最小版 + 钉 `=0.148.0` 的 1–2 天 spike；T14 拿三项数据决定 |
| D3 | 宏生成项的 DefKey：rustc 形状（展开树先序分配 disambiguator）还是带 `ExpnKey`（RA 式，无屏障） | 后者与 rustc 的 `DefPathHash` 不再一一对应 | 由 T02 冻结；倾向带 `ExpnKey` |
| D4 | 是否把 RA 源码列为可移植来源 | RA 部分代码派生自旧版 rustc，复制会引入双重来源 | 不列；只作“仅设计参考”记录 |

## 对 design / plan 的修改建议

本目录不改这些文件，只列建议：

1. [design/frontend/overview.md](../../design/frontend/overview.md)：ResolvedInterface 写明“含函数体内嵌套的项（impl、trait、`macro_rules`），不含任何表达式”；接口指纹覆盖体内项树而不是体文本（[item-tree.md](item-tree.md#体内的项)）。可选：接口分“名字表 → 签名”两层发布。
2. [design/frontend/body.md](../../design/frontend/body.md)：`span_map` 移出 `Body`，做成旁表，CheckedBody 哈希不含位置（RA `Body`/`BodySourceMap`）。
3. [design/ids.md](../../design/ids.md) 与 [plan/m1/t05-resolve.md](../../plan/m1/t05-resolve.md#defkey-与-hygiene)：补展开身份 `ExpnKey`（按调用位置，不按创建顺序）和 D3 的两个选项（[resolve-defmap.md](resolve-defmap.md#身份方案)）。
4. [design/build/cache.md](../../design/build/cache.md)：durability 三级（sysroot / registry / 工作区）与“每产物记录最低级别”；语义表只能经由记录读集的接口访问（[salsa-cache.md](salsa-cache.md)）；失效测试断言“哪些任务重跑”，并加“函数体内新增 impl”。
5. [plan/m1/t12-trait-experiment.md](../../plan/m1/t12-trait-experiment.md)：spike 钉 `=0.148.0`；读 RA `next_solver/interner.rs` 定位必需方法；mini core 借 RA minicore 的标志裁剪机制，内容用 rustc `#![no_core]` 验证；注明 RA 结果不是 oracle。
6. plan 新增“模式穷尽与可反驳性检查”子项（当前无任务归属）：T06 之后、Body 构建之前，M1 的 `match` 已需要 E0004；同时冻结 THIR 级模式 IR（[patterns-and-layout.md](patterns-and-layout.md#建议)）。
7. [plan/m1/t04-syntax.md](../../plan/m1/t04-syntax.md)：若 D1 通过，写明“词法用 `rustc_lexer`，cooking 自写”；RA `parser/test_data`（526 个输入）作负例语料，期望值由 rustc 生成。
8. [plan/m4-cargo-sysroot.md#t30](../../plan/m4-cargo-sysroot.md#t30)：协议要素补“版本协商、span 子请求、响应携带 `tracked_env_vars`/`tracked_paths`、进程池计入 jobserver 预算”；注明 RA 也未在工具链外兼容 bridge，方案 A 风险更高。
9. [plan/m0/t00-freeze-inputs.md](../../plan/m0/t00-freeze-inputs.md)：输入清单加 `ra-ap-*` 精确版本与“src/ diff 为 0”证据、RA commit；来源记录增加“依赖 crate”和“仅设计参考”两类；红线 7 扩到依赖 rustc crate。

## 文件

| 文件 | 内容 |
|---|---|
| [components.md](components.md) | RA crate 与 `ra-ap-rustc_*` 清点：规模、依赖、stable、许可、一致程度；0.148.0 = 1.94.1 的实测 |
| [syntax.md](syntax.md) | `rustc_lexer`、RA parser、rowan |
| [macros-hygiene.md](macros-hygiene.md) | 展开身份、span 锚点、卫生表、`macro_rules` 引擎 |
| [resolve-defmap.md](resolve-defmap.md) | DefMap 与 rustc 的差异；DefKey 身份方案 |
| [item-tree.md](item-tree.md) | item tree 与接口先发布；体内的项 |
| [types-traits.md](types-traits.md) | RA 的类型推导；next-solver 复用成本；T12 spike 做法 |
| [patterns-and-layout.md](patterns-and-layout.md) | `rustc_pattern_analysis`、`rustc_abi` |
| [proc-macro-srv.md](proc-macro-srv.md) | 过程宏服务端与协议 |
| [salsa-cache.md](salsa-cache.md) | Salsa durability 与 kfrc 缓存 |
| [risks.md](risks.md) | 依赖、照搬设计、许可与来源 |

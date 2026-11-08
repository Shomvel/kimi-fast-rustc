# 先例一：Rust 自己的语义——MiniRust、Miri、Reference

核实日期 2026-11-08。“已核实”指读过原始来源；“推断”是本调研的判断。

## MiniRust

来源：[minirust/minirust](https://github.com/minirust/minirust)（读的是 2026-11-06 的 `063d784`）。

| 方面 | 内容 | 状态 |
|---|---|---|
| 定位 | README：“理想化的、类 MIR 的语言”“Rust 的核心语言”“理想化的 Miri”；与 Miri 的已知差异“一般是 Miri 的 bug”（[miri#2159](https://github.com/rust-lang/miri/issues/2159)） | 已核实 |
| 写法 | 规范**写成解释器**：“the spec itself is code”，用类 Rust 的伪代码 specr lang；`List`/`Map`/`Set` 是抽象集合；规范里的 `panic!` 算规范 bug | 已核实 |
| 目录 | `spec/prelude`、`spec/mem/{pointer,interface,basic,intptrcast,concurrent}` 与 `spec/mem/tree_borrows/`、`spec/lang/{values,types,representation,syntax,well-formed,machine}` 与 `spec/lang/step/*` | 已核实 |
| UB 与非确定 | `throw_ub!` 经 `Result` 返回；`Nondet<T>` 的 `pick` 是恶魔选择（所有选择都要对），`predict` 是天使选择（无可选即 UB） | 已核实 |
| 值 | `Value = Int(数学整数) \| Bool \| Ptr(Pointer<Prov>) \| Tuple(List<Value>) \| Variant{discriminant, data} \| Union(List<List<AbstractByte>>)` | 已核实 |
| 表示关系 | `representation.md` 定义 `decode(bytes) -> Option<Value>`、`encode(Value) -> bytes`（互逆）与 `check_value`；同一文件定义“validity invariant” | 已核实 |
| 内存可插拔 | 语言对 `trait Memory`（`allocate/deallocate/store/load/retag_ptr`，无类型、按字节）泛型；实现有 `basic`（不管别名）和 Tree Borrows；**没有** Stacked Borrows 模块 | 已核实 |
| 抽象字节 | `AbstractByte<Prov> = Uninit \| Init(u8, Option<ProvenanceFrag<Prov>>)` | 已核实 |
| 可执行 | `specr-transpile` 把规范转成 Rust（`tooling/minirust-rs`）；`minimize` 链接 rustc，经 stable MIR 把 MIR 翻成 MiniRust；`tests/pass` 91 个、`tests/ub` 47 个 | 已核实 |
| 覆盖缺口 | README：“极不完整”，**没有浮点**；`minimize` 拒绝 `TailCall`、`InlineAsm`、`UnwindTerminate`、协程终结指令；处理 `Drop`、`Assert`、`SwitchInt`、`Call`、`UnwindResume` | 已核实 |
| 与 Reference 的差别 | 故意比 Reference 少一些 UB：悬空/空指针上的 `addr_of!(*ptr)` 有定义；指向无效值的引用不总是 UB | 已核实 |
| 单态 | 程序里没有泛型参数，`minimize` 输入是单态化后的 MIR | 推断（由“经 stable MIR 翻译”与值/类型定义推出） |

## Miri

| 方面 | 内容 | 状态 |
|---|---|---|
| 论文 | Jung, Kimock, Poveda, Sánchez Muñoz, Scherer, Wang, [Miri: Practical Undefined Behavior Detection for Rust](https://research.ralfj.de/papers/2026-popl-miri.pdf)（POPL '26，[DOI](https://doi.org/10.1145/3776690)） | 已核实 |
| 主张 | 第一个能找出确定性 Rust 程序中**全部 de-facto UB**（rustc 当前利用的 UB）的工具；跟踪 provenance、校验类型不变式、检测数据竞争、探索弱内存、模拟 OS；10 万+ crate 上跑通 70%+ 的测试；在 std CI 中运行 | 已核实 |
| 架构 | 解释**关掉全部 MIR 优化**的 MIR；论文脚注：核心解释器与 rustc 的 CTFE 共享。共享点是 [`trait Machine`](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src/interpret/machine.rs#L99)：关联类型 `Provenance`、`MemoryKind`、`AllocExtra` 等，钩子 `find_mir_or_eval_fn`、`call_intrinsic`、`generate_nan`、`atomic_*`、`ptr_from_addr_cast` | 已核实 |
| 指针 | `Pointer = {addr, prov}`，`Provenance = Known(AllocId) \| Wildcard`（int→ptr） | 已核实 |
| 与 MiniRust | Miri 论文正文没有提到 MiniRust；关系只见于 MiniRust 一侧的说明 | 已核实 |

rustc CTFE 一侧的结构见 [rustc/layout-abi-mono/const-eval.md](../rustc/layout-abi-mono/const-eval.md)：值是 `Scalar | ScalarPair | Uninit`，**形状跟随 `backend_repr`**，所以 CTFE 用与 codegen 相同的布局；解释的是 `mir_for_ctfe`（优化前的 MIR）。

## Rust Reference 与相关文档

| 文档 | 对 kfrc 语义有用的原文要点 | 状态 |
|---|---|---|
| [Behavior considered undefined](https://doc.rust-lang.org/reference/behavior-considered-undefined.html) | 列表“不完整”，“没有形式模型”；“产生”一个值 = 赋给或读出 place、传给或从函数/原始运算返回；产生无效值是立即 UB；整数、浮点、裸指针必须已初始化，读未初始化内存只在 union 和 padding 中允许；引用须对齐、非空、不悬空，“指向有效值”这一点“仍有争议” | 已核实 |
| [UCG 术语表：validity/safety invariant](https://github.com/rust-lang/unsafe-code-guidelines/blob/master/reference/src/glossary.md#validity-and-safety-invariant) | validity 在每次有类型的拷贝/访问时必须成立（MIR 赋值、函数实参与返回值），编译器知道并利用它；safety 只关系库的健全性 | 已核实 |
| [RFC 3514 浮点语义](https://github.com/rust-lang/rfcs/blob/master/text/3514-float-semantics.md)、[`f32` NaN bit patterns](https://doc.rust-lang.org/std/primitive.f32.html#nan-bit-patterns) | IEEE 754；NaN 结果符号非确定，quiet 位与 payload 在 preferred / 传播 / 目标特定中非确定选择；执行时选定，之后位串稳定；CTFE 可以与运行时不同；x86_64 没有目标特定 payload | 已核实 |
| [`GlobalAlloc`](https://doc.rust-lang.org/std/alloc/trait.GlobalAlloc.html) | 分配函数可以非确定地跳过分配器、可以合并；“分配是否发生不属于程序行为” | 已核实 |
| RustBelt（Jung 等，[POPL '18](https://plv.mpi-sws.org/rustbelt/popl18/paper.pdf)） | 第一个对实际 Rust 子集的机器检查安全性证明；不是编译器语义 | 已核实 |

## 对 kfrc 的结论

| 问题 | 结论 | 理由 |
|---|---|---|
| Body 层的参考语义 | **采纳 MiniRust 的结构，不直接运行 MiniRust** | Body 与 MIR 同形，MiniRust 的 `Value`/`AbstractByte`/`encode`/`decode`/`Memory` 可以一一照搬；但 MiniRust 没有浮点、unwind 不全，`minimize` 的输入是 rustc 的 MIR，不是 kfrc 的 Body（推断） |
| 源程序 UB 的判定 | **采纳 Miri** | 唯一能跑真实 crate 的 de-facto UB 判定器；kfrc 的生成器和 reducer 已计划用它（[ct-testing.md](../prior-art/ct-testing.md)） |
| 解释器核心与 CTFE 共享 | **采纳 rustc 的做法** | rustc 用 `Machine` trait 让 CTFE 与 Miri 共享引擎；kfrc 的 CTFE 与 IR 解释器共享值、内存、`encode/decode` |
| 别名模型可插拔 | **不在 kfrc 解释器里做** | 见 [memory.md](memory.md)：kfrc 只检查自己声称的分区事实 |
| “引用指向有效值” | **不依赖** | Reference 称仍有争议，MiniRust 不总当 UB；投机 load 必须对无效内容给 poison 而不是 UB |

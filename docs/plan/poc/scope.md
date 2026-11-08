# 范围

原则：理念只在中后端，所以中后端按设计完整实现；前端与标准库借用 rustc；凡是不影响六项主张的能力一律不做，遇到即明确报错（整个 crate 交给 LLVM 并标记，不做函数级静默回退）。

## 做什么、不做什么

| 维度 | 做 | 不做 |
|---|---|---|
| 前端 | rustc 前端，取 `-Zmir-opt-level=0` 的 MIR（drop 显式化之后） | 自有解析、宏、typeck、trait、借用检查 |
| 类型 | 整数（≤ 64 位）、bool、char、struct/tuple/闭包环境、enum（含 niche：`Option<&T>`、`Option<Box<T>>`、`Result`）、引用、`&[T]`/`&str`、`Box`、经泛型 std 代码的 `Vec`/`String` | f32/f64、i128、SIMD、原子、union |
| 语义 | 泛型与静态分派、闭包与迭代器、`?`、`match`、drop、`panic=abort` | unwind、`dyn`（扩展目标，见下）、内联汇编、TLS、协程 |
| IR | R/S/L 三级、[semantics](../../design/ir/semantics.md) 的首版范围；分区 `Unknown` + `Local` + `Const`，`Frozen`/`Unique` 作为单独开关 | 流敏感逃逸、内联 `Reborrow` |
| 优化 | 规则 DSL 窥孔、GVN、聚合与变体折叠、按前驱拆分、R/S 级内联、槽位提升、分区内 load 转发、检查消除、DCE、elaboration 式 placement（含 LICM）、switch 降低、常量除法 | 循环展开、向量化、IPSCCP、改签名 |
| 特化 | 孔位表、特化身份键、递归去重、嵌入实例、mentioned items、调用摘要 | 跨进程按函数体等待 |
| 后端 | x86-64 SysV；Rust ABI 的 Direct/Pair/Indirect（PassMode 由 rustc 给出）；线性扫描；整数与指针指令；只读数据与重定位 | SSE/AVX、调试信息、展开表 |
| 缓存 | 内容寻址的 R 图缓存与机器码缓存、源位置隔离 | 跨机器共享、回收 |
| 验证 | crate 测试；R→S 相等检查；逐 pass 小解释器（整数、聚合、enum） | 完整三级语义解释器 |

## 工作负载

| 负载 | 用途 | 为什么选它 |
|---|---|---|
| W01–W10 程序 | P0–P2 的回归与展示 | 已有 rustc 对照数据 |
| semver | P3 的第一个真实 crate | Cargo 的依赖；`?` 和错误类型密集，覆盖 W01 的错误路径；规模小 |
| regex-syntax | P4 的主负载 | ripgrep 的依赖；AST/HIR 是大 enum 加递归 `Box`，`Vec`/`String`/`match` 密集；几乎无浮点和 SIMD；有完整测试 |
| regex-syntax 解析负载 | 运行时间 | 解析一组真实正则（取 ripgrep/regex 自带的 benchmark 语料） |

选择依据：这类代码正是 LLVM 花最多 pass 重建结构的地方（[evidence](../../research/evidence.md)），也最少依赖 PoC 不做的能力。

## 扩展目标（时间允许时）

- `dyn Trait`：vtable 孔位、`CallVirtual`、去虚化（[W07](../../research/walkthroughs/w07-dyn-trait.md)）。regex-syntax 的错误类型路径会碰到 `dyn Error`；不做时这些函数所在的 crate 交给 LLVM。
- `Frozen`/`Unique` 分区的运行时收益对比（[W05](../../research/walkthroughs/w05-mut-alias-vec-push.md)）。
- toml 解析器（Cargo 的真实负载），先核实浮点用量。

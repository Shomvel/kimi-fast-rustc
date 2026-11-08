# 测试与正确性：随机生成、规则验证、checker、bisect

## 1. 是什么

| 项目 | 年份 | 来源 | 要点 |
|---|---|---|---|
| Csmith | 2011 | Yang, Chen, Eide, Regehr, [Finding and Understanding Bugs in C Compilers](https://users.cs.utah.edu/~regehr/papers/pldi11-preprint.pdf)（PLDI '11） | 生成无 UB 的随机 C 程序，多编译器/多优化级差分 |
| YARPGen / v2 | 2020 / 2023 | Livinskii, Babokin, Regehr, [YARPGen](https://users.cs.utah.edu/~regehr/yarpgen-oopsla20.pdf)（OOPSLA '20）；[Fuzzing Loop Optimizations](https://users.cs.utah.edu/~regehr/pldi23.pdf)（PLDI '23） | 生成时静态避开 UB；“generation policy”定向触发某类优化；v2 专攻循环优化 |
| RustSmith | 2023 | Sharma, Yu, Donaldson, [RustSmith](https://www.doc.ic.ac.uk/~afd/homepages/papers/pdfs/2023/ISSTA-tool.pdf)（ISSTA '23 工具演示） | 生成满足类型/借用检查的 Rust 源码，跨 rustc 版本和 mrustc 差分 |
| Rustlantis | 2024 | Wang, Jung, [Rustlantis](https://research.ralfj.de/papers/2024-oopsla-rustlantis.pdf)（OOPSLA '24）；[源码](https://github.com/cbeuw/rustlantis) | 直接生成 custom MIR（`#![feature(custom_mir)]`），绕过类型/借用检查；静态跟踪程序状态、对编译器隐藏状态、加诱饵块；四个后端对拍：MIR opt+LLVM、仅 LLVM、Cranelift、Miri |
| Miri | 2026 | Jung 等, [Miri: Practical Undefined Behavior Detection for Rust](https://research.ralfj.de/papers/2026-popl-miri.pdf)（POPL '26） | Rust 抽象机的直接实现；对确定性程序发现全部 de-facto UB；Rustlantis 用它判定生成器自身是否产出 UB |
| Alive | 2015 | Lopes 等, [Provably Correct Peephole Optimizations with Alive](https://users.cs.utah.edu/~regehr/papers/pldi15.pdf)（PLDI '15） | 窥孔规则写成 DSL，SMT 证明（含 undef/poison/UB 语义），再生成 C++ |
| Alive2 / alive-tv | 2021 | Lopes 等, [Alive2: Bounded Translation Validation for LLVM](https://users.cs.utah.edu/~regehr/alive2-pldi21.pdf)（PLDI '21）；[README](https://github.com/AliveToolkit/alive2) | 对每个 pass 前后的 IR 做有界 refinement 检查（循环按界展开）；`opt`/clang 插件和独立 `alive-tv` |
| Crocus | 2024 | VanHattum 等, [Lightweight, Modular Verification for Wasm-to-Native Instruction Selection](https://cfallin.org/pubs/asplos2024_veri_isle.pdf)（ASPLOS '24） | ISLE 规则旁写 SMT 注解，按位宽单态化逐条验证；2024-10 进入 wasmtime 主线（[#9178](https://github.com/bytecodealliance/wasmtime/pull/9178)） |
| Arrival | 2025 | McLoughlin 等, [Scaling Instruction-Selection Verification against Authoritative ISA Semantics](https://cs.wellesley.edu/~avh/scaling-isa-preprint.pdf)（OOPSLA '25） | 自动串联规则链、建模 trap/副作用、从 Arm ASL 自动导出机器指令规格；2026-06 替换主线验证器（[#13550](https://github.com/bytecodealliance/wasmtime/pull/13550)），2026-07 进 CI（[#13929](https://github.com/bytecodealliance/wasmtime/pull/13929)）并覆盖中端规则（[#13935](https://github.com/bytecodealliance/wasmtime/pull/13935)） |
| 翻译验证 | 1998 / 2000 | Pnueli, Siegel, Singerman, [Translation Validation](https://link.springer.com/chapter/10.1007/BFb0054170)（TACAS '98）；Necula, [Translation Validation for an Optimizing Compiler](https://people.eecs.berkeley.edu/~necula/Papers/tv_pldi00.pdf)（PLDI '00） | 不证明编译器，只验证每次编译的结果；Necula 在 gcc 每个 pass 前后比较 RTL，不改优化器 |
| CompCert 分配验证 | 2010 | Rideau, Leroy, [Validating Register Allocation and Spilling](https://xavierleroy.org/publi/validation-regalloc.pdf)（CC '10）；[Allocation 模块](https://compcert.org/doc/html/compcert.backend.Allocation.html) | 分配器不经证明，事后用“伪寄存器 = 位置”等式的反向数据流验证 |
| regalloc2 checker | 2021 / 2022 | Fallin, [Correctness in Register Allocation](https://cfallin.org/blog/2021/03/15/cranelift-isel-3/)、[A New Register Allocator](https://cfallin.org/blog/2022/06/09/cranelift-regalloc2/)；[checker.rs](https://github.com/bytecodealliance/regalloc2/blob/main/src/checker.rs) | 抽象解释：每个位置映射到“此处等于哪些 vreg”的集合（格 = 𝒫(V)，交为 meet）；检查每个操作数和约束；Cranelift 设置 `regalloc_checker` 可开 |
| Cranelift 模糊测试 | 2022– | [wasmtime fuzz README](https://github.com/bytecodealliance/wasmtime/blob/main/fuzz/README.md)；[cranelift-control](https://docs.wasmtime.dev/api/cranelift_control/index.html)；[2023 年回顾](https://bytecodealliance.org/articles/wasmtime-and-cranelift-in-2023) | `cranelift-fuzzgen`：随机 CLIF 编译执行 vs 解释器；`cranelift-icache`：变异后增量结果 vs 从头编译；`differential`：对 wasmi/V8；OSS-Fuzz 持续运行；chaos mode 在合法决策点随机扰动，fuel 二分定位 |
| opt-bisect | 2016 / 2026 | LLVM [OptBisect](https://llvm.org/docs/OptBisect.html)（[f0f2792](https://github.com/llvm/llvm-project/commit/f0f279291c7ca1a0b2c125f53cd08deafcc9e44f)，2016-04）；rustc `-Zmir-opt-bisect-limit`（[#152474](https://github.com/rust-lang/rust/pull/152474)，2026-02） | 全局递增 pass 序号，超过上限的可跳过 pass 不运行；二分出第一个出错的 pass 执行 |

## 2. 关键证据

| 数字 | 来源 |
|---|---|
| Csmith 3 年报告 325+ 个未知 bug；所有被测编译器都有静默错编译，只有开发中的 CompCert 找不到 wrong-code | Csmith 摘要、正文 |
| YARPGen：GCC 83、LLVM 62、ICC 76，共 220+ | YARPGen 摘要、§1 |
| YARPGen v2：122 个（GCC 66、LLVM 28、DPC++ 16、ISPC 12），另有 Alive2 自身 2 个 | v2 摘要、§1 |
| RustSmith：独立复现 5 个历史 rustc bug，mrustc 2 个新 bug | RustSmith 正文 |
| Rustlantis：10 CPU 年，22 个未知 bug（rustc 8、LLVM 12、Cranelift 2），16 个是错编译；生成程序数千万个，Miri 未报 UB | Rustlantis §4.1、图 10 |
| Miri：10 万+ 个库上跑通 70%+ 的测试 | Miri 摘要 |
| Alive：翻译 334 条 InstCombine，8 条错误；MulDivRem 44 条中 6 条错 | Alive 摘要、正文 |
| Alive2：跑 LLVM 单元测试，报告 47 个新 bug（发表时修 28 个），推动 8 处 LangRef 修订 | Alive2 摘要、§1 |
| Necula：多数优化误报率很低，个别优化约 10% 的函数误报 | Necula 引言 |
| Crocus：96 条规则 × 388 个类型实例；复现 3 个已知 bug（含 9.9/10 的 x86-64 地址模式沙箱逃逸 [GHSA-ff4p-7xrq-q5r8](https://github.com/bytecodealliance/wasmtime/security/advisories/GHSA-ff4p-7xrq-q5r8)），2 个新 bug；10 条含乘除/popcnt 的规则在宽位宽超时 | Crocus 摘要、§4.1 |
| Arrival：10,807 条路径证明无错编译，97.8% 在 11 分钟内验证，2.6× 更少手写规格，1 个新 bug | Arrival §1 |
| 主线：中端 4115 个规则单态化约 2 分钟；验证器发现 6 条缺 `ty_int` 守卫的中端规则（[#14631](https://github.com/bytecodealliance/wasmtime/pull/14631)）；CI 用提交入库的 SMT 查询缓存，未命中即失败 | #13935、#14631、#13929 |
| rustc 删除 `-Zfuel`：全局可变状态，不被增量编译跟踪 | [#115293](https://github.com/rust-lang/rust/pull/115293)（2024-11） |

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| (a) 规则形式化验证 | **改编** | 纯整数/位运算规则（构建期窥孔、R/S 级标量折叠、L 级窥孔与地址模式）与 Alive/Crocus 的对象同构，SMT 位向量可直接表达。规则应以可导出的数据形式存在（模式 + 结果 + 前提 + 宽度/overflow 模式），而不是只有手写 Rust 匹配代码。聚合/变体/`Refine` 规则在 SMT 里不自然，改用小宽度穷举 + 参考解释器。乘除宽位宽会超时（Crocus），按 Cranelift 做法标 `slow`、缓存查询结果 |
| 整图翻译验证（Alive2 式） | **不采纳** | 需要为 SoN 三个级别写完整 SMT 语义（分区、效果、unwind），成本接近再写一个编译器；以每 pass 差分 + verifier 代替 |
| (b) MIR 级 fuzz | **改编** | Rustlantis 输出 custom MIR 文本，且 harness 依赖 std（`DefaultHasher`、`LazyLock`、`println!`）。kfrc 需要一个 custom MIR → CheckedBody 的导入器（跳过 typeck/borrowck，单列环境），被测函数由 kfrc 生成，哈希/打印 driver 由 rustc 编译后链接。先用 Miri 验证无 UB，再 rustc（`-Zmir-opt-level=0` 与默认两档）vs kfrc 对拍。Rustlantis 程序覆盖引用/裸指针/enum/检查算术，正对 R/S 级分区与变体折叠 |
| 自写生成器的约束 | **采纳** | YARPGen 的静态避开 UB 和 generation policy：按 R/S/L 规则表定向生成（`VariantSwitch` 已知变体、`Check` 消除、分区转发）；Rustlantis 的“隐藏状态 + 诱饵块”防止前端直接常量折叠掉被测路径 |
| Miri 作 oracle | **采纳** | 生成程序和最小化后的用例都先过 Miri；Miri 报 UB 时丢弃结果、修生成器/reducer。分区（`Frozen`/`Unique`）上线时只用 Miri 无 UB 的程序对拍 |
| (c) 分配/placement checker | **采纳** | regalloc2/CompCert 式事后验证与 kfrc 的线性扫描正交：分配器保持启发式，checker 独立、简单、可复查。placement 同理：放置后的块逐条检查支配和每个分区 token 的顺序 |
| (d) 每 pass bisect | **采纳并补缓存约束** | runtime.md 已要求每 pass 可关；再加全局 pass 执行序号与上限（LLVM/rustc 式）和规则级 fuel（chaos mode 式）。rustc 删 `-Zfuel` 的原因提示：bisect 设置必须进入 R 级缓存键或强制旁路缓存，否则发布的 R 图不可复现 |
| chaos mode | **改编（后期）** | placement 尽早/尽晚、分配拆分、冷块布局等合法选择点接受随机种子，固定种子可复现；M5 后做 |
| 增量差分（`cranelift-icache`） | **采纳** | 改一个定义后的增量结果必须与从头编译的对象一致，直接检验 R 级缓存与 S 级重算 |

## 4. 具体建议

| 目标文件#节 | 改法 |
|---|---|
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md)#共同机制 | 加“规则表”：纯值规则以数据形式登记（模式、结果、前提、适用级别、宽度、overflow 模式），由同一份数据生成匹配代码和 SMT-LIB 查询；加“可二分”：全局 pass 执行序号 + `--opt-bisect-limit`，每条规则改写消耗 fuel，超限即停 |
| [plan/README.md](../../plan/README.md#验收)#随机程序生成 | 分两路：(1) Rustlantis custom MIR → CheckedBody 导入器，driver 由 rustc 编译；(2) 自写生成器按规则表的 generation policy 定向。两路都先过 Miri，Miri 报 UB 即作废；最小化后再过一次 Miri |
| plan 验收#步骤 第 4 步 | 失败后先用 bisect 定位到 pass/规则序号，序号写入证据包 |
| [plan/README.md](../../plan/README.md#验收)#优化 | 增加“规则级”证据：整数规则的 SMT 证明或 ≤8 位穷举；超时规则标 `slow` 并单列；SMT 查询结果缓存入库，CI 只读命中 |
| [design/ir/verifier.md](../../design/ir/verifier.md#其他层的-verifier)“分配后”行 | 增加符号 checker：每个物理位置映射到 vreg 集合，检查每个操作数位置、固定寄存器约束、spill/reload/并行复制；验证配置默认开 |
| [design/ir/verifier.md](../../design/ir/verifier.md)#检查清单 | 增加 `placed` 级别：放置后每个值使用被支配、每个分区 token 的读写顺序与图一致、may-trap 不早于守卫；负例加“placement 把 load 提到同分区 store 之前” |
| [memory-partitions.md](../../design/ir/memory-partitions.md)#首版与分阶段 M6 | “用 Miri 对拍”具体化：Rustlantis 程序中含 `&mut`/`&T` 参数和内联的子集，Miri 无 UB 后 rustc vs kfrc（分区开/关）三方对拍 |
| [design/build/cache.md](../../design/build/cache.md#失效测试) | 加 icache 式差分：随机修改一个定义，增量产物 = 从头编译产物；bisect/fuel 设置不同的构建不得共享 R 级缓存条目 |
| [runtime.md](../../design/goals/runtime.md)#验证 | 指向上面的 bisect 与规则表；L 级窥孔/地址模式规则在引入时就带 SMT 证明或穷举证据 |

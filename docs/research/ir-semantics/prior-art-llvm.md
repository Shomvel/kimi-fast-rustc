# 先例二：LLVM 的 poison 语义、Alive2、Vellvm

核实日期 2026-11-08。LangRef 现在的源文件是 `llvm/docs/LangRef.md`。

## LangRef 的值语义

| 条目 | 原文要点 | 状态 |
|---|---|---|
| [poison](https://llvm.org/docs/LangRef.html#poison-values) | 多数指令传播 poison；`select` 例外，`freeze` 截断。poison 作为 load/store 地址、除法/取余的除数、**`br` 条件**、被调者、带 `noundef` 的实参/返回值时立即 UB | 已核实 |
| [undef](https://llvm.org/docs/LangRef.html#undefined-values) | 能用 poison 时优先用 poison | 已核实 |
| [`select`](https://llvm.org/docs/LangRef.html#select-instruction) | 条件为 poison → 结果 poison；未被选中的一臂是 poison 不影响结果 | 已核实 |
| [`freeze`](https://llvm.org/docs/LangRef.html#freeze-instruction) | 返回“任意但固定”的值，逐元素；冻结的 poison 指针不可解引用 | 已核实 |
| [`noundef`](https://llvm.org/docs/LangRef.html#parameter-attributes) | 值的任何位是 undef/poison 即 UB；padding 除外 | 已核实 |
| [`add nsw/nuw`](https://llvm.org/docs/LangRef.html#add-instruction) | 溢出产生 **poison**，不是立即 UB | 已核实 |
| [`dereferenceable(n)`](https://llvm.org/docs/LangRef.html#parameter-attributes) | 可以“投机读取而不必担心陷入”；蕴含 `noundef` | 已核实 |
| [`load`](https://llvm.org/docs/LangRef.html#load-instruction) | 地址不良定义即 UB，对齐声明过大即 UB；`!noundef` 元数据使不良定义的结果成为 UB | 已核实 |
| Byte 类型 | LangRef 新增“Byte Type”，表示可装指针、未初始化或 poison 的原始内存 | 已核实 |

rustc 的对应：MIR `AddUnchecked` 是立即 UB（[CTFE](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src/interpret/operator.rs#L101) 报错），codegen 降为 `add nsw`（[builder.rs#L480](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/builder.rs#L480)）；load 的 `!noundef` 只在标量不允许未初始化时发（[builder.rs#L699](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/builder.rs#L699)）。

## 论文

| 工作 | 要点 | 状态 |
|---|---|---|
| Lee 等，[Taming Undefined Behavior in LLVM](https://web.ist.utl.pt/nuno.lopes/pubs/undef-pldi17.pdf)（PLDI '17） | 循环 unswitching 与 GVN 对“按 poison 分支”需要矛盾的语义；提案：去掉 undef 只留 poison，加 `freeze`，除 phi/select/freeze 外运算对 poison 严格，按 poison 分支即 UB；报告无编译时间和性能退化 | 已核实 |
| Lee 等，[Reconciling High-Level Optimizations and Low-Level Code in LLVM](https://web.ist.utl.pt/nuno.lopes/pubs/llvmmem-oopsla18.pdf)（OOPSLA '18） | 逻辑指针（来自分配，带依赖）与物理指针（来自 `inttoptr`）；越界检查推迟；“孪生分配”使地址必须被观察而不能被猜中；给出一个**由 GVN 按指针相等传播引起的安全 Rust 误编译**例子 | 已核实 |
| Lopes 等，[Alive2](https://web.ist.utl.pt/nuno.lopes/pubs/alive2-pldi21.pdf)（PLDI '21） | refinement：目标行为 ⊆ 源行为；poison 可被任何值 refine；UB 终态可被任何东西 refine；有界翻译验证（循环展开）；47 个新 bug、8 处 LangRef 修订 | 已核实 |
| Zhao 等，[Vellvm](https://acg.cis.upenn.edu/papers/popl12_vellvm.pdf)（POPL '12） | Coq 中多种操作语义：非确定小步 LLVM_ND（undef 为全部值的集合）及若干确定化版本；**抽取出解释器**并用 LLVM 测试集检验；案例：经验证的 SoftBound pass | 已核实 |
| Zakowski 等，Modular, Compositional, and Executable Formal Semantics for LLVM IR（ICFP '21，[DOI](https://doi.org/10.1145/3473572)） | 用 interaction tree 写成“解释器塔”：原始 ITree → intrinsic → 全局环境 → 局部环境/栈 → 内存模型 → `pick`（undef）与 UB 事件；命题模型把 pick/UB 送进集合，可执行解释器对 undef 取默认值、遇 UB 失败；抽取的 OCaml 解释器被证明 refine 模型 | 已核实 |

## 对 kfrc 的结论

| 问题 | 结论 | 理由 |
|---|---|---|
| poison/undef/freeze | **采纳 poison，不设 undef，freeze 按需** | PLDI '17 的终点就是这个设计；LLVM 因兼容至今保留 undef |
| `nsw` 的含义 | **采纳“溢出得 poison”** | rustc 自己就是把 MIR 的立即 UB 降为 poison；SoN 的浮动节点需要全函数 |
| UB 点列表 | **改编** | 照 LangRef 的 poison→UB 位置；Rust 的“传参/返回即产生值”对应 LLVM 的 `noundef` 实参/返回值 |
| `select` | **采纳** | 条件 poison → poison；未选臂不影响 |
| 投机 load | **采纳“丢掉 UB 型元数据”** | LLVM 投机 load 只靠 `dereferenceable`，`!noundef` 等元数据不随之上提（推断：依 LangRef 对二者的定义推出） |
| 指针相等替换 | **不采纳** | OOPSLA '18 的 Rust 误编译例；kfrc 的规则表禁止按 `p == q` 把 `q` 换成 `p` |
| refinement 定义 | **采纳 Alive2 的形式** | 可执行检查只需比较可观察行为与 poison/UB 的偏序 |
| Vellvm 的“命题模型 + 可执行解释器”两层 | **改编** | kfrc 只要可执行一层：非确定点（地址、NaN、分配省略）取确定选择，对拍时按集合比较 |
| Alive2 式整图验证 | **不采纳**（与 [ct-testing.md](../prior-art/ct-testing.md) 一致） | 三级、分区、unwind 的 SMT 编码成本过高；用逐 pass 差分代替 |

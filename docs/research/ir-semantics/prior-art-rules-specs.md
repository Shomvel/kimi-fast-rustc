# 先例四：规则验证与规范解释器——Cranelift、WebAssembly、Lean 4

核实日期 2026-11-08。

## Cranelift

| 工作 | 要点 | 状态 |
|---|---|---|
| Crocus（VanHattum 等，[ASPLOS '24](https://cfallin.org/pubs/asplos2024_veri_isle.pdf)） | ISLE term 旁写 `(spec (term args) (provide …) (require …))`，运算符几乎一一对应 SMT-LIB，`widthof` 取位宽；原文：“Cranelift 的 IR **没有语义**”，CLIF 语义只以这些逐 term 规格存在；规格对位宽多态，按类型实例单态化后逐条验证；纯函数、寄存器分配前；覆盖 Wasm 测试集触发规则的 19.8%（50/253），找到 3 个已知 bug（含 9.9/10 CVE）和 2 个新 bug | 已核实 |
| Arrival（McLoughlin 等，[OOPSLA '25](https://cs.wellesley.edu/~avh/scaling-isa-preprint.pdf)） | 输入是厂商验证过的 ASL ISA 规格，自动生成 AArch64 的 ISLE 规格；用少量效果变量（load/store、trap、flags）建模状态，Crocus 只有纯函数；验证规则链和编码；论文范围**不含中端规则** | 已核实 |
| 主线 | 2026-07 起中端规则也进入验证（[#13935](https://github.com/bytecodealliance/wasmtime/pull/13935)），见 [ct-testing.md](../prior-art/ct-testing.md) | 已核实（见该文件） |
| CLIF 解释器 | [cranelift/interpreter](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift/interpreter)：README “Cranelift IR 的解释器”，“许多指令未实现”；[cranelift-fuzzgen](https://github.com/bytecodealliance/wasmtime/blob/main/fuzz/fuzz_targets/cranelift-fuzzgen.rs) 以它为 oracle（燃料 4096），与优化后再解释或本机编译的结果逐位比较；解释器中陷入的输入跳过 | 已核实 |

## WebAssembly

| 工作 | 要点 | 状态 |
|---|---|---|
| 规范解释器 | [spec/interpreter](https://github.com/WebAssembly/spec/blob/main/interpreter/README.md)（OCaml）：“为清晰简单而非速度”，是“钉死精确语义的装置”；解码、验证、运行 `.wast` 脚本 | 已核实 |
| 规范语义 | [执行约定](https://webassembly.github.io/spec/core/exec/conventions.html)：配置 ↪ 配置的归约规则，“每条规则一步”；操作数栈、标签、帧都在指令序列里 | 已核实 |
| 机械化 | Watt, Mechanising and Verifying the WebAssembly Specification（CPP '18）：Isabelle 规范 + 经验证的可执行解释器和类型检查器，证明过程暴露了规范 bug；[WasmCert-Coq](https://github.com/WasmCert/WasmCert-Coq)：抽取的解释器对机械化证明是 sound 的，二进制解析未验证 | CPP '18 仅见检索摘要；WasmCert-Coq 已核实 |
| SpecTec（Youn 等，PLDI '24） | 每个特性要写形式语义、文字规范、参考解释器三份；SpecTec 是“单一事实来源”，生成 LaTeX、文字和一个元解释器（通过全部适用测试）；在 5 个提案里找到 10 处错误。[2025-03 公告](https://webassembly.org/news/2025-03-27-spectec/)：Wasm 3.0 官方规范由 SpecTec 产出 | 已核实 |

## Lean 4

| 工作 | 要点 | 状态 |
|---|---|---|
| IR 解释器 | [ir_interpreter.cpp](https://github.com/leanprover/lean4/blob/master/src/library/ir_interpreter.cpp)：“求值 λRC IR 的简单解释器”；复用编译器 IR 而不是另造字节码，供没有 LLVM JIT 的平台使用；有本机符号时经 dlsym 切到本机代码 | 已核实 |
| LCNF 解释器 | `src/Lean/Compiler/LCNF/` 中没有解释器文件；有 `Check.lean`（类型与结构检查，注释称类型兼容关系不传递，放弃了这项检查；`compiler.checkTypes` 默认关，“不是完整检查”） | 文件列表推断 |
| 4.22 | [发布说明](https://lean-lang.org/doc/reference/latest/releases/v4.22.0/)：新编译器取代旧编译器（#8577）；检查只在 `init`、`saveBase`、`toMono`、`saveMono` 后跑（#8764） | 已核实 |

## 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| 规则旁写 SMT 规格（Crocus） | **改编** | Crocus 承认 CLIF 无语义，只能逐 term 写规格，规格之间的一致性靠人；kfrc 应让规则表的 SMT 编码与解释器出自同一份标量语义定义（[uses-and-cost.md](uses-and-cost.md#规则验证)） |
| 效果变量（Arrival） | **采纳到 L 级规则** | L 级地址模式、带陷入的除法规则需要 trap/内存效果 |
| 解释器作 fuzz oracle（cranelift-fuzzgen） | **采纳** | 与 kfrc 现有 `interpreter.rs` 对拍做法一致；扩展到三级 |
| “单一事实来源”生成规范与解释器（SpecTec） | **改编，不建 DSL** | kfrc 的语义就是 Rust 写的解释器；规则表另成数据。两份东西的一致性用 SMT 编码对解释器的穷举对拍保证（推断） |
| 解释器为清晰不为速度（Wasm 规范解释器） | **采纳** | 参考解释器与 CTFE 的快速路径分开 |
| Lean 只有 IR 层解释器、LCNF 只有检查器 | **说明代价** | Lean 放弃了 LCNF 的类型检查，结果是 base/mono 层没有可执行语义；kfrc 若只在 L 级有解释器，R/S 的 pass 错误要到 L 才暴露，定位成本高 |

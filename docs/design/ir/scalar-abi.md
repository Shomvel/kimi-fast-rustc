# 窄整数的调用传输

本实现的目标固定为 `x86_64-unknown-linux-gnu`、System V、rustc 1.94.1。
Rust 逻辑值、对象中的字节和 ABI scalar 是三个不同的层次：bool 存一个
字节，producer 的函数 ABI 却是带 zeroext 的 i1。Rust signedness 也不能
替代 producer 对该次调用声明的 signext / zeroext / 无扩展属性。

`PassMode::DirectScalar(ScalarAbi)` 保留一个 scalar 的 source width 和
extension；`ScalarPair` 保留两个 scalar，次序对应 oracle 中的物理字段。
Direct/Pair 中没有窄 scalar 时保留既有表示，因为此目标对至少 32 位的
scalar 无需额外扩展。synthetic 的 Direct/Pair 声明继续采用原来的传输
策略；不能由它们的裸 Rust 类型反推出 producer 曾经给过哪些属性。
CastTarget 继续使用独立的 Cast 表示。

对于此目标：

| 边界 | 传输 |
|---|---|
| 带 zeroext / signext 的窄参数 | 明确扩为 32 位 lane，入函数时截回实际存储宽度 |
| 没有扩展属性的窄参数 | 只传递实际宽度，新增的参数寄存器位未规定 |
| 8/16 位结果 | 只要求 AL/AX；不能把参数的 32 位扩展规则套到结果上 |
| 带 zeroext 的 i1 bool 结果 | AL 是有效的 0/1 字节，高于 AL 的位未规定 |
| 收到的 scalar/pair | 按字段的字节宽度截断，再绑定逻辑 scalar 或写入私有存储 |

Cranelift 使用明确的 lane 类型和 extend/reduce 指令表示上述传输，不再
用其会扩到整机器字的 Uext/Sext attribute 表示已知的窄 ABI。原 x86 后端
仍使用全字临时值，但出边界时按 producer 的属性选择扩展种类；多定义
一些未规定的高位是允许的。解释器继续建模逻辑值及其字节，不把额外
返回寄存器位当作 Rust 语义。

函数 ABI 的完整相等和哈希包含 scalar 属性。`is_direct` / `is_pair` 仅
用于判断传输形状，不能用来替代 ABI 相等检查；克隆、callee interning、
函数指针和内联 splice 仍复制完整 ABI。scalar width 必须与其存储及 pair
字段匹配，非法宽度和 bool 的非法扩展组合会被拒绝。这里没有新增
noalias、noundef 或跨未知调用的内存优化推断。

`KFRC_SCALAR_ABI=legacy` 在新的 rustc 进程中恢复旧 producer 表示，用于
原版对象与成本对照。它是验证入口，不是可移植 ABI 的选择器。

验证入口为 `crates/rustc_codegen_kfrc/tests/scalar-abi/run.sh`。真实 LLVM
caller/callee、函数指针、汇编对规定扩展位的检查和未规定高位填脏覆盖
两个调用方向；ScalarPair、栈参数、transparent signed wrapper 和邻接
字节存储分别检查。该 fixture 的通过不能替代完整原始 gate、四个真实
crate 的 corpus/checksum 对照或独立的编译与运行成本测量。

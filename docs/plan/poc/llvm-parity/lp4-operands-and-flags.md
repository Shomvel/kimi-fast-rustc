# LP4 指令选择：操作数、地址模式、标志位、宽度

2026-11-09：最终 `8b2113` 对象的精确归因、分配器契约及下一独立候选见
[体积优先实施方案](lp4-size-implementation.md)。以下旧计数只保留为原计划背景，以新方案为当前基线。

[goals/runtime.md](../../../design/goals/runtime.md) 把“地址模式、`lea`、`cmov`、
`setcc`、`test`”列为 L 级窥孔加 Machine IR 选择表。目前机器指令只有 `Reg`／`Stack`／
`Imm(i32)` 三种操作数，没有 LEA，没有通用内存操作数，比较一律先 `setcc` 物化。

## 1. 内存操作数与地址模式

现状：地址先在寄存器里算好（如 `mov r11, 0x10; add r10, r11`），再 `LoadIndirect`／`StoreIndirect`。

目标：

- 按 [backend.md](../../../design/backend.md) 增加 `Mem { base, index: Option<(Reg, Scale)>, disp: i32 }` 操作数；`Stack` 作为它的特例。
- 选择时把 `PtrOffset` 链和 `base + idx*{1,2,4,8} + disp` 折进操作数；常量偏移累加进 disp。
- 地址计算只被访存使用时，不单独物化。
- load 只有一个使用者、且中间没有别名写时，允许折进算术或比较指令的源操作数（`add rax, [rdi+8]`、`cmp byte ptr [rsi+rcx], 0x2e`）。

## 2. LEA 与立即数

- `a + b + k`、`a + b*s`、三操作数加法用 `lea`，避免先拷贝再加。
- 立即数在 i32 范围内直接作为操作数；超过范围的才用 `movabs`。
- 64 位常量在同一函数中多次使用时复用同一个寄存器（交给 LP2 的重物化代价判断）。

## 3. 比较与分支融合

现状：`Compare` → `setcc` → `movzx` → `test x, 0` → `jne` → `jmp`。`from_str` 中 cmp 160、
sete 136、test 150、jne 150，基本是一一对应。

目标：

- `If(Compare(a, b))` 且比较结果只被这个 If 使用时，直接发 `cmp a, b; jcc`。
- `And`／`Sub` 的结果再与 0 比较时，复用它设置的标志位。
- `test r, r` 代替 `cmp r, 0`。
- 两个后继中有一个是下一块时，只发一条 `jcc`（fallthrough 已有，确认与融合后的序列配合）。
- 标志位跨越会改写标志的指令时，重新计算，不保存到寄存器。

## 4. 宽度

现状：所有值 64 位，窄整数在每次运算后用 shl/sar 或 and 掩码规范化。

目标：

- 按 [backend.md](../../../design/backend.md) 给 `MachineInstr` 加 `Width { B8, B16, B32, B64 }`。
- 32 位运算利用 x86-64 的隐式零扩展；只在比较、除法、移位、扩展和跨 ABI 边界时才规范化高位。
- 窄 load 用 `movzx`／`movsx` 直接到 32/64 位。

## LLVM 参考

- [X86ISelDAGToDAG.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86ISelDAGToDAG.cpp)：`matchAddress`、`selectAddr`、`selectLEAAddr`；
- [CodeGenPrepare.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/CodeGenPrepare.cpp)：`optimizeMemoryInst`，把地址计算下沉到使用点以便折进操作数；
- [X86InstrInfo.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86InstrInfo.cpp)：`foldMemoryOperandImpl`、`optimizeCompareInstr`（复用算术指令的标志位）；
- [X86ISelLowering.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86ISelLowering.cpp)：`LowerBRCOND`、`EmitCmp`、`LowerSETCC`；
- [X86FixupLEAs.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86FixupLEAs.cpp)、[X86OptimizeLEAs.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86OptimizeLEAs.cpp)：LEA 与 add 的取舍、LEA 复用；
- [X86FixupSetCC.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86FixupSetCC.cpp)：setcc 后的零扩展。

LLVM 的 SelectionDAG 太重，不照搬。kfrc 在 SoN 上已有 GVN 和窥孔，选择器可以按
[codegen-backend.md](../../../research/llvm-mir/codegen-backend.md) 的结论写成树模式匹配。

## 验收

- 编码器单测：每种新操作数（各种 base/index/scale/disp 组合，包括 r12/r13 特例、rip 相对寻址）编码后用 objdump 解码比对。
- 形状测试：小 Rust 函数（数组下标、结构体字段、比较分支、u8/u32 运算）的 kfrc 指令序列与 LLVM 同一函数对照，记录指令数比值。
- 差分与 semver 按 LP9。

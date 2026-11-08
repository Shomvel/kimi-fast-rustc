# 机器与 ABI 契约（draft-0）

## 现状

- 所有值 64 位整数；`MachineInstr` 操作数是 `Reg`、`Stack(StackSlot)` 或 `Imm(i32)`（[x86.rs:343](../../../../src/x86.rs#L343)）。
- 线性扫描只分配 10 个寄存器：`rcx rsi rdi r8 r9 rbx r12–r15`（[native.rs:25](../../../../src/native.rs#L25)）。`r10/r11` 固定作 scratch，`rax/rdx` 留给返回值和除法。
- 参数：前 6 个走 System V 整数寄存器 `rdi rsi rdx rcx r8 r9`，其余走栈（`LoadArg/StoreArg { index }`）。被后续指令覆盖的参数寄存器先存到“argument home”栈槽。
- `MachineInstr::Load/Store` 只访问 spill 栈槽，没有通用地址。
- `MachineFunction.symbols: Vec<String>` 按序号对应 `Call { target: u32 }`。

## FnAbi

```rust
pub struct FnAbi {
    pub conv: CallConv,            // Rust | C (SysV x86_64)
    pub args: Vec<ArgAbi>,
    pub ret: ArgAbi,
    pub c_variadic: bool,
}
pub struct ArgAbi { pub layout: LayoutId, pub mode: PassMode }
pub enum PassMode {
    Ignore,                        // ZST、uninhabited 返回
    Direct(RegClass),              // 一个寄存器：Int 或 Sse
    Pair(RegClass, RegClass),      // ScalarPair，如 &[T]、(i64, bool)
    Cast(CastTarget),              // 按 SysV 分类拆成 ≤2 个 eightbyte
    Indirect { on_stack: bool },   // 指针传递（sret/大聚合），或 byval 栈拷贝
}
pub enum RegClass { Int, Sse }
```

命名与语义刻意对齐 rustc 的 `PassMode`，方便对照 rustc 的分类结果做差分。具体分类规则（Rust ABI 何时 Indirect、SysV 的 INTEGER/SSE/MEMORY 合并）以 [rustc/layout-abi-mono](../../rustc/layout-abi-mono/README.md) 的笔记为准。

M1 只实现 `Direct(Int)` 和 `Ignore`；其余变体在类型上存在，lowering 遇到即返回 `UnsupportedFeature`。

### 第二层：CallLayout

rustc 的 `FnAbi` 只到“IR 层参数列表”，具体用哪个寄存器、哪个栈槽由 LLVM 决定。kfrc 没有 LLVM，需要自己补一层：

```rust
pub fn assign_locations(abi: &FnAbi) -> CallLayout;
pub struct CallLayout {
    pub args: Vec<SmallVec<[ArgLoc; 2]>>,   // 每个 ArgAbi 拆成 0–2 个位置
    pub ret: SmallVec<[ArgLoc; 2]>,
    pub sret: Option<Reg>,                  // Indirect 返回：rdi 传入，rax 返回
    pub stack_bytes: u32,                   // 调用方栈参数区，16 字节对齐
}
pub enum ArgLoc { Gpr(Reg), Xmm(XmmReg), Stack { offset: u32 } }
```

现有 `StoreArg/LoadArg { index }` 是 `ArgLoc::Stack` 的特例。SysV 寄存器计数（6 个 GPR、8 个 XMM，聚合放不下就整体走栈）在这一层实现，见 [sysv-x86_64.md](../../rustc/layout-abi-mono/sysv-x86_64.md)。

### Rust ABI 的尺寸规则（x86_64）

x86_64 上 Rust ABI 没有平台特有规则，完全由布局决定（细节见 [fn-abi.md](../../rustc/layout-abi-mono/fn-abi.md)）：

| 情况 | 传递方式 |
|---|---|
| 返回值 > 16 字节 | sret（`Indirect`） |
| `BackendRepr::Memory` 且 ≤ 8 字节 | 当作一个整数 `Cast` |
| `BackendRepr::Memory` 且 > 8 字节 | 按指针（`Indirect { on_stack: false }`），不是 byval |
| `ScalarPair` | 拆成两个参数（`Pair`） |

所以 `#[repr(C)] struct F(f64)` 和无 repr 的同名结构体走不同寄存器——ABI 正确性依赖 [布局与 rustc 一致](../../validation/layout-abi-parity.md)。

### 隐式参数

`#[track_caller]` 函数和所有 panic lang item 多一个尾随参数 `&'static Location`。Location 常量内含指向文件名 `&str` 的指针，因此第一个可能 panic 的程序（溢出/越界检查）就需要隐式参数和带重定位的只读数据。

## Machine IR 扩展

| 需求 | 改动 |
|---|---|
| 宽度 | `MachineInstr` 带 `Width { B8, B16, B32, B64 }`；编码器按宽度选 opcode/前缀 |
| 通用内存 | 操作数新增 `Mem { base: Reg, index: Option<(Reg, Scale)>, disp: i32 }`；`Stack` 变成 `Mem { base: Rsp, … }` 的特例 |
| 寄存器类别 | 虚拟寄存器带 `RegClass`；线性扫描按类别分池（GPR 14 个可分配、XMM 16 个） |
| 固定寄存器 | 现有 `LiveInterval::fixed` 继续使用：参数、返回、`div` 的 rax/rdx、移位的 cl |
| 调用 clobber | 由 `FnAbi.conv` 给出 caller-saved 集合；跨调用活跃的值分到 callee-saved 或 spill |
| 符号 | `Call { target: SymbolId }`，`SymbolId` 来自 `InstanceKey` 的确定性符号名 |
| 释放 rax/rdx | 返回值和除法改用固定区间而不是整体保留，GPR 可分配数从 10 增到 14（性能项，非 M1 必需） |

## ObjectFragment

```rust
pub struct ObjectFragment {
    pub instance: InstanceKey,
    pub sections: Vec<SectionPiece>,   // .text / .rodata / .data / .bss / .tdata
    pub symbols: Vec<SymbolDef>,       // 名称、section、偏移、大小、可见性、绑定
    pub relocations: Vec<Reloc>,       // offset、符号、类型（PC32/PLT32/64/GOTPCREL/TPOFF…）、addend
    pub unwind: Option<UnwindInfo>,    // M3 起
}
```

合并按 `InstanceKey` 排序，保证 jobs 数和完成顺序不影响对象字节。现有 [elf_reloc.rs](../../../../src/elf_reloc.rs) 支持 PC32/PLT32，M2 扩展数据重定位，M3 扩展 TLS 与 `.eh_frame`。

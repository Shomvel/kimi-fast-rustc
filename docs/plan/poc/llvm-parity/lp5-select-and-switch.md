# LP5 条件选择（cmov）与 switch 降低

## 1. `Select` 与 cmov

现状：

- SoN 没有 `Select` 节点；
- L 层的 `select_scalar` 用 `IntCast`／`Negate`／`BitNot`／`And`／`Or` 掩码算术实现，一次选择约 6 条指令；
- 简单的 `if c { a } else { b }` 仍是两块加一个 Phi。

目标：

- 增加纯值节点 `Select(cond, a, b)`，S 层和 L 层通用，解释器和 verifier 同步支持。
- **if-conversion**（[optimization-pipeline.md](../../../design/ir/optimization-pipeline.md) 的 L 级“`Select` → `cmov`”）：
  两臂都是不会 trap、无效果、体积小的纯计算，且合流处只有少量 Phi 时，把菱形改成 `Select`。
- x86 降低：`cmp`／`test` 设置标志位后接 `cmovcc`；被选的两个值都已在寄存器或可作为内存操作数。
- 不转换的情况：
  - 某一臂含 load（可能读非法地址）、除法或调用；
  - 循环中条件可预测、某一臂代价高（参考 LLVM 的 X86CmovConversion，把这类 cmov 改回分支）。
- `select_scalar` 改为生成 `Select`；LP1 后剩余的少量选择也走 cmov。

poison 语义：`Select` 只传播被选中一臂的 poison，与 LLVM `select` 一致；
在解释器里按此实现并加回归。

## 2. switch 降低

现状：`VariantSwitch` 和整数 match 降成比较链。[optimization-pipeline.md](../../../design/ir/optimization-pipeline.md) 已列“跳转表、位测试、查找表、比较树按阈值选择”，尚未实现。

目标（按 case 数和密度选择）：

| 形态 | 条件（起点，按 LP0 数据调整） |
|---|---|
| 比较链 | ≤ 3 个 case |
| 二分比较树 | case 稀疏 |
| 跳转表 | case ≥ 4 且密度 ≥ 40%；表放 `.rodata`，带重定位 |
| 位测试 | 多个 case 跳到同一目标，值域 ≤ 64 |
| 查找表 | 每个 case 只产生常量结果（含 LP1 中真正需要序号的映射）；表放 `.rodata`，按内容去重 |

- 默认分支可证不可达（`Unreachable`）时，省略范围检查。
- 跳转表需要 object 写出支持 `.rodata` 中的代码地址重定位，由 D 负责 `object_file.rs`。

## LLVM 参考

- [X86ISelLowering.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86ISelLowering.cpp) `LowerSELECT`；
- [X86CmovConversion.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Target/X86/X86CmovConversion.cpp)：何时把 cmov 改回分支；
- [EarlyIfConversion.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/EarlyIfConversion.cpp)、[SelectOptimize.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SelectOptimize.cpp)；
- [SimplifyCFG.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Utils/SimplifyCFG.cpp)：双入口 Phi 折叠成 select、`switch-to-lookup` 查找表；
- [SwitchLoweringUtils.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/SwitchLoweringUtils.cpp)：跳转表、位测试、聚类阈值。

## 验收

- 解释器：`Select` 的 poison 只来自被选中一臂；有 trap 风险的菱形不被转换（负例）。
- 形状：`min`／`max`／`clamp`／`bool as u8`／小型 match 的 kfrc 指令序列与 LLVM 对照。
- switch：稠密、稀疏、负值、u8 全覆盖、带不可达默认分支的 match，分别走三臂差分；跳转表 ELF 重定位用 readelf 核对。

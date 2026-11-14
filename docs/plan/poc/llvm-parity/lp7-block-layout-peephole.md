# LP7 块布局、冷路径与机器级清理

## 现状

- 块按逆后序发射，只做了“跳到下一块的 jmp 删除”（`lower/fallthrough.rs`）。
- 每个 If 固定发 `jne` 加 `jmp`。
- `from_str` 有 142 条无条件跳转，LLVM 是 20 条。
- panic、溢出等冷路径与热路径交错。

## 目标

1. **块布局**：
   - 热路径连续，条件跳转尽量指向冷块；
   - 冷块（panic、`Unreachable` 前的路径、`#[cold]` 调用）放函数末尾；
   - 没有 profile，用静态规则：panic 路径冷，循环回边热，`Result::Err` 路径偏冷（依据 W01）。
2. **分支折叠**：
   - 只含 `jmp` 的空块直接改写目标；
   - 两个后继相同的 If 删除；
   - 相同的 panic 尾块合并（design 的“冷区域布局”项）。
3. **尾复制**（可选）：小的合流块复制到前驱，去掉一次 jmp，受体积预算约束。
4. **机器级拷贝传播与死代码**：LP2 之后若仍有 `mov a, b; mov c, a` 链，做一次拷贝传播并删除无用定义。
5. **短跳转与对齐**：已有短跳转编码，确认在新布局下迭代收敛。循环头对齐按 LP0 数据决定是否做（体积与 runtime 的取舍）。

## LLVM 参考

- [MachineBlockPlacement.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/MachineBlockPlacement.cpp)：链式布局、冷块后置；
- [BranchFolding.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/BranchFolding.cpp)：空块、尾合并；
- [TailDuplicator.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/TailDuplicator.cpp)；
- [MachineCopyPropagation.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/MachineCopyPropagation.cpp)、[PeepholeOptimizer.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/PeepholeOptimizer.cpp)；
- 静态分支概率：[BranchProbabilityInfo.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/BranchProbabilityInfo.cpp)（cold call、unreachable、loop back edge 的启发式）。

## 验收

- 形状：W01 的 `?` 错误路径中，热路径无向前的无条件跳转；panic 尾块在函数末尾且合并。
- semver：无条件跳转数、函数体积前后对照；runtime 不得回退。

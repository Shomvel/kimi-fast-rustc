# 各层表示

## Typed Body

参考 rustc MIR 的职责，但是自有结构。

- **内容：** 基本块、局部变量（LocalId）、`Place` 及 field/index/deref 投影、copy/move 操作数、赋值、借用、调用、分支、return、drop、assert、失败/清理边、作用域退出；每个操作带源位置和类型。
- **局部身份：** 与语法名分离，遮蔽产生新 LocalId；投影是结构化的，不是字符串。
- **控制流：** 分支、短路、match、循环、提前返回、失败路径都用同一套 CFG 表达。已知的 if 错编译（分支内修改的变量没有合并）在这一层根治，见 [回归样例](../validation/regression-if-expression.md)。
- **用途：** 初始化、move、借用约束、drop 显式化和诊断。

## SoN

表达可执行计算：值、控制、内存和其他可观察效果。

- 参数、常量、计算值都带类型：整数宽度、符号、溢出模式、浮点行为、指针。后端不猜类型。
- 调用引用稳定的实例/符号，并带已知或保守的效果描述。
- 值 Phi 与内存/效果合并都和前驱一一对应。循环图封闭前不能交给优化器或缓存。

### 效果模型

首版是**一条保守的内存/效果序列**：未知调用、读写内存、volatile、atomic、fence、检查失败和可能 unwind 的操作都在链上，不当作纯值。返回、检查失败、unwind 和不可返回调用是可观察控制。

以后要区分读/写集合、别名类别或线程同步时，作为单独的优化阶段验收，并定义“未知状态”与合并规则。

## Machine IR

指令约束、寄存器类别、调用约定、栈帧、机器控制流。placement、liveness、Phi 消除、两地址约束和 spill 都需要——不做 ILP 调度不会省掉这些。

## ObjectFragment

已编码的代码/数据、符号、重定位和必要的展开信息。合并顺序稳定，按显式符号身份进行；不依赖 worker 完成顺序、HashMap 迭代顺序或 arena 编号。

字段级定义见 [contracts/body.md](contracts/body.md)（Typed Body）、[contracts/son.md](contracts/son.md)（SoN）、[contracts/machine-abi.md](contracts/machine-abi.md)（Machine IR 与 ABI）。

## 每层 verifier 检查什么

验证配置下每个变换的入口和出口都跑 verifier。verifier 通过不等于算法正确，最终以真实执行对拍为准。

| 层 | 检查 |
|---|---|
| Typed Body | 每个 Place 的类型和投影/布局、局部初始化、终结指令、退出路径；所有权结果和 drop 能对应回源位置 |
| SoN | NodeId 有效、数据类型、控制支配、Phi 与前驱对应、循环入口/回边、内存状态汇合、调用/检查/异常效果连续。拒绝：内存节点缺效果边、may-trap 节点移出其控制条件、一个分支的定义被另一个分支使用。区分合法循环和误构造的数据环 |
| 放置后的块 | 每条控制路径保留效果；节点移动满足支配和效果约束 |
| Machine IR | 操作数宽度/类别、固定寄存器、调用 clobber、控制和 flags 依赖、定义/活跃区间、spill 地址、分支目标 |
| 分配后 | 所有虚拟值有位置、栈对齐、callee-save、边上并行复制正确 |
| 对象 | section/symbol/relocation 一致、目标特征匹配、链接并真实执行成功 |

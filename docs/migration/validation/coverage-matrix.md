# 覆盖矩阵

每个阶段列出三类集合：已实现、明确拒绝、尚未覆盖。不要求 M1 就做完整张表，但交付的能力要有对应的组合验证。

| 主题 | 组合维度 | 断言 |
|---|---|---|
| 控制与状态 | if/match/短路 × 赋值 × loop/break/continue/return | 所有路径的值和副作用一致；未执行的分支没有效果 |
| 整数 | 各宽度/符号 × cast × 边界值 × overflow 模式 | 截断、扩展、移位、除法和错误行为一致 |
| 内存 | 别名 × load/store × 调用 × 循环 | 读写顺序、对齐、有效性和可见结果一致 |
| 类型与布局 | struct/enum/tuple/slice × 泛型 × `repr` | 大小/对齐、字段投影、判别值、跨函数传值正确 |
| ABI | 整数/浮点/聚合 × 寄存器/栈 × 直接/间接调用 | C driver 双向传参返回、caller/callee 保存、栈对齐 |
| 所有权 | move/drop × 分支 × 提前退出 × panic | 无效程序拒绝；析构次数、顺序和 cleanup 一致 |
| 泛型与 trait | 约束/关联类型 × 实例化 × 递归 × 动态派发 | 实例身份、vtable/drop glue 和约束行为正确 |
| 并发效果 | atomics/volatile/fence × 调用/优化 | 依据语言内存模型的合法观测，不只用单线程模拟 |
| 构建 | host/target × proc macro/build script × cache × jobs | 产物来源无混淆，失效与取消正确 |

## 优化

每个 pass 启用和禁用都跑真实执行。浮点规则不默认 reassociate；整数规则按宽度和 overflow 模式验证；可能 trap 的除法不随意消除；pass 不跨检查、volatile 或未知调用移动内存访问。

# 验证

每次交付都要回答：**什么源程序**经过**什么完整路径**，用的是**哪个源码/二进制**，**和谁比较**，**实际看到了什么**。

| 文件 | 内容 |
|---|---|
| [regression-if-expression.md](regression-if-expression.md) | 第一个永久回归：有状态 if 表达式错编译 |
| [differential-testing.md](differential-testing.md) | 与 stock rustc 的差分执行协议、随机程序生成约束 |
| [coverage-matrix.md](coverage-matrix.md) | 各主题必须覆盖的组合维度 |
| [pipeline-faults.md](pipeline-faults.md) | 调度、缓存、取消的故障测试 |
| [performance.md](performance.md) | 编译时间、运行时间、体积的公平比较 |
| [evidence.md](evidence.md) | 证据包格式与执行环境 |

## 验证层次

| 层次 | 对象 | 通过条件 |
|---|---|---|
| 输入与语义 | AST、展开/解析、类型/trait、borrow/move/drop | 有效输入接受，无效输入按类别拒绝（不要求诊断文字相同） |
| 中间表示 | Typed Body、SoN、Machine IR | [verifier](../architecture/representations.md#每层-verifier-检查什么) 的不变量成立 |
| 迁移闭包 | 一组相互依赖的语言功能 | 源码经新链运行，与 rustc 输出一致 |
| 工具链 | 多 crate、ABI、sysroot、Cargo、host 工具 | 依赖身份、行为、产物来源、失败恢复可验证 |
| 产品 | ripgrep/Cargo 的冻结 workload | 先功能通过，再比较编译时间、运行时间、体积、资源 |

模块测试通过 ≠ 语言兼容；微基准加速 ≠ 整项目更快；编译成功 ≠ 运行正确。

## 发现错误时

1. 首次暴露一类正确性错误后，停止重复跑同类压测，冻结失败输入和源码。
2. 静态排查同类实现和相邻风险（解析、Body、SoN、机器码各层）。
3. 批量补回归并修复，再恢复差分。

以下情况阻止阶段晋级：新的错编译、无效 Rust 被接受、旧缓存错误命中、产物来源不明、测试被静默跳过。

迁移闭包过大时，缩小范围并明确拒绝剩余能力。回退保留已验证模块、证据和可复现的失败记录。

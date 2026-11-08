# 测试、脚本、示例与文档

## 测试

单元测试大多写在源文件内部，integration tests 在 [tests/](../../../tests)。迁移源码时同时登记内部测试——只搬 `tests/*.rs` 会漏掉大量编码、缓存、调度验证。下表列代表文件，不是可删除其余测试的清单。

| 测试组（代表文件） | 去向 | 必须保留的验收 |
|---|---|---|
| 语法/类型：[integer_syntax](../../../tests/integer_syntax.rs)、[casts](../../../tests/casts.rs)、[const_items](../../../tests/const_items.rs)、[const_expressions](../../../tests/const_expressions.rs)、[function_visibility](../../../tests/function_visibility.rs) | 改为新入口的接受/拒绝对拍；legacy 结果单列 | edition、宽度、常量错误、作用域、可见性；补目标宽度和错误边界 |
| 有状态控制流：[shadowing](../../../tests/shadowing.rs)、[loop_values](../../../tests/loop_values.rs)、[statement_if](../../../tests/statement_if.rs)、[match_ranges](../../../tests/match_ranges.rs)、[early_return](../../../tests/early_return.rs) | 新 Body → SoN 的永久语义回归；补赋值/调用/drop 组合 | 与 stock rustc 对拍并真实执行；false/零次迭代等未执行分支也要覆盖 |
| 调用/ABI：[function_calls](../../../tests/function_calls.rs)、[forward_calls](../../../tests/forward_calls.rs)、[stack_arg_encoding](../../../tests/stack_arg_encoding.rs)、[multi_function](../../../tests/multi_function.rs) | 统一到新实例、符号与 ABI 测试 | 参数/返回、递归、callee-save、跨 crate/FFI |
| 图/优化：[bool_optimizer_regression](../../../tests/bool_optimizer_regression.rs)、[div_differential](../../../tests/div_differential.rs)、[loop_invariant](../../../tests/loop_invariant.rs)、[control_optimization](../../../tests/control_optimization.rs) | 按新类型/效果前提逐 pass 迁移；保留最小反例与图 dump | 启用/禁用优化都正确；trap、overflow、内存效果不被纯值简化抹掉 |
| 编码/寄存器：[immediate_encoding](../../../tests/immediate_encoding.rs)、[short_jumps](../../../tests/short_jumps.rs)、[phi_moves](../../../tests/phi_moves.rs)、[regalloc_preference](../../../tests/regalloc_preference.rs) | 保留精确编码/约束测试，适配 Machine IR | 精确字节 + 真实执行；新寄存器类别、spill、固定寄存器、flags 组合 |
| 产物/并行：[direct_control_object](../../../tests/direct_control_object.rs)、[function_parallel](../../../tests/function_parallel.rs)、[native_pipeline](../../../tests/native_pipeline.rs)、[repro](../../../tests/repro.rs) | 扩展到新任务/对象发布和缓存 namespace | jobs=1/多 jobs、失败/取消/重复输入、原子发布、确定性、失效 |
| 模型与 CLI：[machine_differential](../../../tests/machine_differential.rs)、[cli_help](../../../tests/cli_help.rs) | 模型保留定位用途；CLI 区分新旧入口和能力标签 | 模型一致不算最终正确性；未知特性报错 |
| 编译耗时：[function_graph_performance](../../../tests/function_graph_performance.rs) 及源内局部计时 | 保留为可解释的微基准 | 与整项目测量分开 |

## 脚本与示例

- [scripts/native_microbench.py](../../../scripts/native_microbench.py)：保留“当前子集微基准”标签；新链覆盖同语义输入后，加上 frontend/driver 选择、版本、缓存模式记录；不覆盖历史结果。
- [scripts/bench-yicloud.sh](../../../scripts/bench-yicloud.sh)：保持 LLVM reference 说明；正式 native 测量用新协议。历史主机/工具路径不是新环境的默认值。
- [examples/](../../../examples)：`.kfr` 和 `fn main() -> i64` 等约定保留标识；新的稳定 Rust 例子必须能被 oracle 编译。不通过修改样例语义掩盖不兼容。

## 文档

- [README.md](../../../README.md)：每个阶段只公布已验证能力；新入口稳定后再切默认。
- [docs/ARCHITECTURE.md](../../ARCHITECTURE.md)、[docs/VALIDATION.md](../../VALIDATION.md)：保留当前事实和历史证据；新架构实施后按版本补充。
- [docs/research/](../../research)：保留来源和适用范围。
- 本迁移目录的状态随实施证据更新。

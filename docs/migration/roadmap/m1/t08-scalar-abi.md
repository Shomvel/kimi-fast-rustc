# T08 标量 ABI 与机器接口 — 细化

`M1 · D · 依赖：T02, T03` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t08) · 契约见 [machine-abi.md](../../architecture/contracts/machine-abi.md)

## 步骤

1. **FnAbi 计算**：`src/machine/abi.rs`，输入函数签名（M1 只有 `i8–i64/u8–u64/bool/()`）和 `CallConv`，输出 `FnAbi`。bool 与窄整数在 SysV 下 `Direct(Int)`，调用方负责零/符号扩展到 32 位（需对照 rustc 的 `ArgAttribute::ZExt/SExt` 行为核实）。
2. **lowering 入口改签名**：`native::compile_graph(&Graph, name)` → `machine::lower(&FunctionGraph, &FnAbi) -> Result<MachineFunction, UnsupportedFeature>`。参数位置不再由 `Parameter { index }` 隐式推断，而由 `FnAbi.args[i].mode` 决定。
3. **宽度**：`MachineInstr` 增加 `Width`；M1 只需要正确实现 64 位与对窄类型的扩展/截断（`movzx/movsx`、`setcc` 后零扩展已存在）。
4. **位置分配**：实现 `assign_locations(FnAbi) -> CallLayout`（见 [machine-abi.md](../../architecture/contracts/machine-abi.md#第二层calllayout)），现有 `StoreArg/LoadArg` 改为消费 `ArgLoc::Stack`。`PassMode` 的完整枚举现在就定义好，只实现 `Ignore` 与 `Direct(Int)`。
5. **符号**：`Call { target: SymbolId }`，`ObjectFragment.symbols` 由 `InstanceKey` 生成；`native_program.rs` 改为合并 `ObjectFragment`。
6. **C driver 对拍**：`tests/fixtures/abi/scalar/` 下，C 侧以 `int64_t f(int64_t, …)` 等签名调用 kfrc 生成的函数，覆盖 0–8 个参数（跨过 6 个寄存器）。

## 测试

| 维度 | 取值 |
|---|---|
| 参数个数 | 0、1、6、7、8 |
| 类型 | `i64 u64 i32 u32 i8 u8 bool` 混合 |
| 调用形状 | 叶函数、递归（`fib`）、互相递归、调用后仍活跃的值 ≥ 11 个（逼出 spill 和 callee-save） |
| 方向 | C → kfrc、kfrc → C（`extern "C"` 声明的外部函数） |

每个用例：stock rustc 编出的对象与 kfrc 对象分别与同一个 C driver 链接，真实执行输出逐字节比较。

## 完成标志

- 上表全部组合真实执行一致。
- 未实现的 `PassMode`（Pair/Cast/Indirect）与浮点参数在 lowering 时报 `UnsupportedFeature`。
- 仍是线性扫描，没有引入调度器。

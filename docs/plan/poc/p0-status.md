# P0 进度

状态日期 2026-11-08。阶段定义见 [phases.md](phases.md)，代码在 `crates/`。

## 已有

| 部分 | 位置 | 状态 |
|---|---|---|
| Body 契约 | `crates/kfrc-body` | MIR 形状的 Typed Body（语句、投影、常量入池）、`TypeOracle` 与记录的 rustc 布局、Body verifier |
| R 级 SoN | `crates/kfrc-son` | 16 字节节点记录 + 列存 + 转发表；Body → R 图用 Braun 等人的 SSA 构造（所有局部和效果 token 走同一条读写路径）；结构/类型/支配 verifier；参考解释器 |
| x86 | `crates/kfrc-x86` | 图调度（逆后序、纯节点最早放置）、`VirtualInstr` 降低（窄整数规范化）、数据流活跃区间、复用现有线性扫描与编码器、`object` crate 写 ELF |
| 外壳 | `crates/rustc_codegen_kfrc` | rustc 1.94.1 的 `CodegenBackend`；MIR → Body 转换器；不支持的构造直接报错并给出函数与构造，不回退 LLVM |

## 支持的输入

整数（≤64 位）、`bool`、`char`、单元类型；算术（回绕）、位运算、移位（按位宽取模）、比较、`as` 整数转换；`if`/`match`（含区间、或模式）/`while`/`loop`/带值 `break`/带标签跳转/提前返回；直接调用、递归、互递归、>6 个参数。要求 `-Cpanic=abort -Coverflow-checks=off`，目标 x86_64-unknown-linux-gnu。

## 验证（HK 主机，rustc 1.94.1）

| 测试 | 命令 | 结果 |
|---|---|---|
| 固定输入表：T01 各形状、控制流、调用、各位宽 | `tests/p0-scalar/run.sh` | 565 行与 LLVM 构建逐字节一致；目标文件由 kfrc 写出 |
| 生成式差分：随机标量程序 | `tests/p0-scalar/generated/run.sh` | 种子 1–30 × 每程序 40 函数 × 6 组输入一致 |
| 单元测试 | `cargo test -p kfrc-son`、根 crate `cargo test` | 通过 |

差分发现并修复的错误（都在复用的旧后端里，不在新代码的语义层）：调用前先装参数寄存器再保存调用者保存寄存器；可变移位用 `rcx` 却没有固定区间；驱逐溢出的区间复用了在其起点之前仍被占用的溢出槽；调用区栈槽与溢出槽重叠。

## 离 P0 出口还差

- `Assert` 与 panic 路径（G1、G2）：除法、取余、越界检查都依赖它。
- 可执行文件的 `main` 入口垫片与分配器垫片；cargo 按包选后端（`codegen-backend` profile 选项）的核实。
- W01–W10 的纯标量部分（本轮用自写的标量负载代替）。
- 规则 DSL 首版与构建期窥孔。

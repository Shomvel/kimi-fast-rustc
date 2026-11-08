# T09 Typed Body → SoN 的统一控制降低 — 细化

`M1 · C · 依赖：T06, T07, T08` · 概要见 [m1-vertical-chain.md](../m1-vertical-chain.md#t09)

## 要解决的问题

T01 显示现在的前端在 `parse_if_expression`、`parse_match_expression` 中不合并可变变量，而 `parse_if_statement`、`parse_loop` 各有一套合并逻辑。新 builder 只有一套：变量是 Body 里的 `LocalId`，SSA 由 builder 按 CFG 统一构造，语法结构不再参与 Phi 决策。

## 输入

SoN builder 只接收检查、drop 显式化、清理之后的 Body（[body.md](../../design/frontend/body.md#阶段与清理)）：`FalseEdge` 已变 `Goto`，`FakeRead`/Fake 借用/`StorageDead` 已删除，drop flag 是普通 `bool` 局部。`Call`、`Assert`、`Drop` 降为带异常控制出口的效果节点；`panic=abort` 下 unwind 出口在进入 builder 前已裁掉（对应 rustc 的 `AbortUnwindingCalls`）。溢出/越界失败分支调用 panic lang item，带隐式 `&'static Location` 参数（[machine-abi.md](../../design/backend.md#隐式参数)）。

## 算法

采用 Braun et al. 2013《Simple and Efficient Construction of Static Single Assignment Form》的按块构造：

- `write_local(block, local, value)` / `read_local(block, local)`。
- 读一个未在本块定义的 local：块已 sealed 且单前驱 → 递归读前驱；多前驱 → 建 Phi 并对每个前驱递归读；未 sealed（循环 header）→ 建不完整 Phi，封闭时补齐。
- 去除平凡 Phi（所有输入相同或为自身）。
- 封闭时机：Body 的 CFG 已完整，块的所有前驱处理完即可 seal；循环 header 在回边块处理完后 seal。

因为 Body 已有完整 CFG（break/continue/return/match 臂都是边），这个算法自然覆盖所有控制结构；多回边、不可达块（无前驱，不建图节点）也统一处理。

## 交付物

- `src/son/build.rs`：`build_son(&CheckedBody) -> Result<FunctionGraph, LoweringError>`，产出 R 级图（[levels.md](../../design/ir/levels.md)），结束时调用 full verifier。M1 没有泛型，`specialize` 是恒等。
- Body 到 SoN 的映射表（`BlockId → Region`、`LocalId@Block → NodeId`），dump 时一起输出，便于定位。

## 测试

1. T01 全部形状（含“碰巧正确”的支配违规）：图通过 full verifier，T03 runner 真实执行与 rustc 一致。
2. 组合生成：`{if 表达式, if 语句, match 表达式, match guard, while, for, loop 表达式}` × `{在第一个分支赋值, 在第二个分支赋值, 两个都赋值, 都不赋值}` × 嵌套深度 1–3 × `{break, continue, return 提前退出}`。每个生成的程序由 stock rustc 编译出参考值。
3. 不可达代码：`return` 之后的语句、`loop {}` 后的代码，图中不出现节点。

## 完成标志

- 以上全部通过真实执行对拍；图层通过但未执行的不算。
- 一个测试可同时导出 Body、SoN、机器码三份 dump。

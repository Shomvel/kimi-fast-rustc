# T01 从干净源码复现已知错编译 — 细化

`M0 · E · 依赖：T00` · 概要见 [m0-baseline.md](../m0-baseline.md#t01) · 问题说明见 [regression-if-expression.md](t01-reproduce-miscompile.md)

## 根因（已读代码确认）

`Parser` 用一张 `variables: HashMap<String, Binding>` 表示当前 SSA 值（[frontend.rs:636](../../../src/frontend.rs)）。各控制结构对这张表的处理不一致：

| 入口 | 位置 | 分支前快照 | 分支间恢复 | 合并可变变量 |
|---|---|---|---|---|
| `parse_if_statement` | frontend.rs:2016 | 有 `outer_variables` | 有 | 有（为每个 `mutable` 绑定建 Phi） |
| `parse_loop`（while/for/loop） | frontend.rs:2153 | 有 | — | 有（header Phi + break 快照） |
| `parse_if_expression` | frontend.rs:2818 | **无** | **无** | **无**，只合并结果值 |
| `parse_match_expression` | frontend.rs:2447 | **无** | **无** | **无** |
| `parse_short_circuit` | frontend.rs:2781 | 无 | 无 | 无；目前操作数里不能出现 `loop`/块，所以暂不可触发 |

结果有两种：

1. **前一个分支的赋值泄漏到后一个分支**（真分支先解析，假分支读到真分支里的新值）。
2. **合并点之后使用了只在某个分支里定义的值**——违反支配关系，运行时读到寄存器残值。

## 同类实例探测（2026-11-08，machine_eval，非真实执行）

用 `frontend::parse → placement::place_for_native → native::compile_graph → machine_eval::evaluate` 探测：

| 程序形状 | 期望 | 观察 | 说明 |
|---|---|---|---|
| `let y = if f { loop { x = 2; break 10; } } else { 20 }; x + y` | f=false → 21 | **20** | 已报告的问题 |
| 同上，赋值在 else 分支 | f=true → 11 | 11 | 图中 `x + y` 使用了 else 分支内的 Phi（支配违规），答案碰巧正确 |
| `match v { 0 => loop { x = 2; break 10; }, _ => 20 }` | v=1 → 21 | 21 | 同样是支配违规，碰巧正确 |
| if 表达式嵌套在 if 表达式分支中并赋值 | f=false → 4 | **3** | 新发现的错编译 |
| `match` guard 内 `loop` 赋值 | 14 / 21 | 14 / 21 | 碰巧正确；guard 路径需在图层确认 |
| `f && loop { … }` | — | 解析报错 | `loop` 不能作操作数，属覆盖缺口 |

“碰巧正确”的例子必须靠图层的支配检查发现，执行对拍发现不了——这是 [T07](../m1/t07-son-verifier.md) 要加支配检查的直接理由。

## 交付物

1. `tests/fixtures/regress/if_expr_state/`：上表每个形状一份 `.rs`，加上 `expected.txt`（stock rustc 实际运行结果）。
2. 冻结构建的 kfrc（T00 的 commit，clean 工作区）产出的 ELF、链接命令、真实 x86_64 执行输出。
3. 风险清单：表达式级 if/match、match guard、以及将来允许 `loop`/块作操作数后的 `&&`/`||`。
4. 在 known-fail 登记表中记录每个形状（T03 runner 读取），修复在 [T09](../m1/t09-body-to-son.md) 完成。

## 完成标志

- 两个错编译形状在干净构建 + 真实执行下复现（或记录不复现的差异）。
- 每个形状都有图 dump，标出违反支配关系的使用点。

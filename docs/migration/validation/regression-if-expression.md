# 永久回归：有状态 if 表达式

## 程序

合法 Rust，`check(false)` 应为 21，`check(true)` 应为 12：

```rust
pub fn check(flag: bool) -> i64 {
    let mut x = 1;
    let y = if flag {
        loop { x = 2; break 10; }
    } else { 20 };
    x + y
}
```

## 已观察到的结果

2026-11-08 的审查在 Linux x86_64 上执行 kfrc 直接生成的 ELF：

| | `check(false)` | `check(true)` |
|---|---|---|
| stock rustc | 21 | 12 |
| kfrc | **20** | 12 |

原因：[frontend.rs](../../../src/frontend.rs) 的 `parse_if_expression` 只合并表达式的值，没有合并分支内修改的可变绑定。[machine_eval.rs](../../../src/machine_eval.rs) 也显示出同样差异，但证据以真实执行为准。

## 来源限制

这是已有审查的观察，不是本计划 PR 新跑的结果：

- 审查复用了工作区里已有的 debug 二进制和 rlib；当时工作树有五个未提交文件，所以**不能**说成“clean HEAD 构建已复现”。
- HEAD 为 `44c9d80b02ab140d7edf5348b6946d8122d842b6`，所检查的 `parse_if_expression` 与 HEAD 一致。
- 编译器二进制 SHA-256：`9f9e730d3ddc9fcc233297399cab3bd5e813c325f3216513a9b97c0913ed3c0d`
- 执行对象 SHA-256：`c7f4a5eaeeef8e97d3aa57e3520d709b818db340815c98bd098fabe9127775cb`

## 同类实例（2026-11-08 计划细化时探测）

用现有库函数 `frontend::parse → placement::place_for_native → native::compile_graph → machine_eval::evaluate` 探测（内部模型，不是真实执行）：

- **另一个错编译：** if 表达式嵌套在 if 表达式分支里并在 `loop` 中赋值，`f=false` 期望 4，得到 3。
- **潜在错编译：** 赋值放在 else 分支，或放在 `match` 表达式的某个臂里，结果碰巧正确；但图中合并点之后的 `x + y` 直接使用了只在该分支内定义的 Phi（例如 `n23 = Add(n18, n22)`，`n18` 位于 else 分支内部的 Region），属于支配违规，结果取决于寄存器分配。

根因是 `parse_if_expression` 和 `parse_match_expression` 不快照、不恢复、不合并 `Parser.variables`，详见 [T01 细化](../roadmap/m0/t01-reproduce-miscompile.md)。“碰巧正确”的形状靠执行对拍发现不了，需要 SoN verifier 的支配检查（[son-verifier.md](../architecture/contracts/son-verifier.md)）。

## 后续

- [T01](../roadmap/m0-baseline.md#t01)：在冻结源码上重新构建并复现，把源码、driver、参考/候选命令和原始输出存为公开回归。
- [T09](../roadmap/m1-vertical-chain.md#t09)：在 Body → SoN 统一控制降低中根治。修复时一并覆盖 if/match/短路/loop 中的绑定、控制和效果合并，包括嵌套、提前退出和多次赋值，不只特判这个程序。

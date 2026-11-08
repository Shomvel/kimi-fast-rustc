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

## 后续

- [T01](../roadmap/m0-baseline.md#t01)：在冻结源码上重新构建并复现，把源码、driver、参考/候选命令和原始输出存为公开回归。
- [T09](../roadmap/m1-vertical-chain.md#t09)：在 Body → SoN 统一控制降低中根治。修复时一并覆盖 if/match/短路/loop 中的绑定、控制和效果合并，包括嵌套、提前退出和多次赋值，不只特判这个程序。

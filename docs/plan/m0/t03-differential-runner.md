# T03 源级差分与证据 runner — 细化

`M0 · E · 依赖：T00, T01` · 概要见 [m0-baseline.md](../m0-baseline.md#t03) · 协议见 [differential-testing.md](../README.md#验收)

## 现状

现有测试几乎都走 `frontend::parse → placement::place_for_native → native::compile_graph → machine_eval::evaluate`（例如 `tests/early_return.rs`），即用内部模型执行机器码，不跑真实 ELF。`native_driver.rs` 能调用 `as`/链接器并执行产物，但没有与 rustc 对拍的入口。

## 设计

fixture 目录结构：

```text
tests/fixtures/<group>/<case>/
  main.rs          # 合法 Rust，stock rustc 能编译
  case.toml        # 种类、输入、期望
```

`case.toml`：

```toml
kind = "run"              # run | reject | known-fail
entry = "check"           # 被测函数；C driver 调用它
args = [[0], [1]]         # 每组参数一次执行
reject_class = ""         # reject 时：type | borrow | move | uninit | unsupported …
known_fail = "T09"        # known-fail 时：修复它的任务
```

runner（规划 `tools/kfrc-diff/`，独立 binary，不放进编译器）：

1. 用 stock rustc 编译 `main.rs` + 一个自动生成的 Rust `main`，打印每组参数的返回值 → 参考输出。`reject` 用例用默认模式和 `-Znext-solver=globally` 各跑一次，记录两者的错误码集合。
2. 用 kfrc 只编译被测函数为对象；用 C driver（`driver.c`，按 `entry`/`args` 生成）调用并打印 → 候选输出。目标函数必须来自 kfrc 对象，driver 只负责调用和打印。
3. 在真实 x86_64 上执行两边，逐行比较 stdout、退出码。
4. 写证据：`evidence/<run-id>/<case>/{cmd.txt, ref.out, cand.out, cand.o.sha256, kfrc.sha256, diff.txt}`。
5. 汇总按 `pass / fail / known-fail / unsupported / timeout / solver-divergence / lint-dependent` 计数，只有 `pass` 计入成功。

## 完成标志

- T01 的 fixture 全部登记为 `known-fail`，runner 报告它们为 known-fail 而不是 pass。
- 故意改错一个 `expected`（或注入错误的候选对象）时 runner 报 fail。
- 超时、kfrc 报 unsupported 都不计成功。
- 换一台干净机器按 T00 manifest 重放，结果一致。

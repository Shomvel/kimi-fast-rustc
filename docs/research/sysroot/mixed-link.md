# 替代方案：混链官方 sysroot、只原生编 core、全原生

计划的红线（[m4-cargo-sysroot](../../plan/m4-cargo-sysroot.md) 出口与 [cargo-sysroot](../../design/build/cargo-sysroot.md)）：混合 bootstrap 必须单列，不等于 full-native；M4 出口至少要达到“目标 Rust 闭包全 native”。下面三条路线都按这条红线评估。

## 混链需要满足什么

kfrc 编出的对象和 rustc 编出的对象要在一个进程里互相调用，以下几项必须**逐位一致**：

| 项目 | 为什么必须一致 | kfrc 计划里是否本来就要做 |
|---|---|---|
| `repr(Rust)` 布局、niche、enum 判别值编码 | 跨对象传 `Option<&T>`、`fmt::Arguments`、`Vec<T>` | 是：T15 要求与 rustc 位级一致 |
| Rust ABI 的 PassMode（标量对、cast、间接；SIMD 按间接传） | 跨对象调用非 `extern "C"` 函数 | 是：T18 |
| vtable 布局、drop glue | `dyn Trait`、`Box<dyn Any>` 跨对象传 | 是：T25 |
| **符号名**（legacy/v0 mangling 加 crate 哈希，`#[rustc_std_internal_symbol]` 的命名） | 链接器按名字解析 | **否**：kfrc 本来可以用自己的命名 |
| **`TypeId` 的值**（rustc 对类型做稳定哈希，依赖 DefPathHash） | std 的 panic 载荷是 `Box<dyn Any + Send>`，`catch_unwind` 与默认 panic hook 用 `downcast_ref::<&str>()`/`<String>()` 判断类型 | **否** |
| 泛型实例归属（谁生成 `Vec<u8>::push`，`share-generics` 策略） | 两边都生成同名实例，要么重复定义，要么内容不同 | **否** |
| 读 rustc 的 `.rmeta` | 拿到 std 泛型函数体才能在用户 crate 单态化 | **否**：T29 明确“不兼容官方 `.rmeta`” |

最后一行决定了：kfrc 不能直接消费官方 rlib，只能**从 std 源码自己跑前端**拿到泛型体和类型，链接时再用 rustc 编出的非泛型对象。这样一来，前端那部分（[tiers](tiers.md) 第一档的 F1–F6，占大头）一点都没省下。

## 路线 A：官方 rustc 编译的 sysroot + kfrc 用户代码

- **利：** 后端的 unwind/TLS/asm/compiler-builtins/shim 可以推迟，先在真实 std 上测 kfrc 生成的用户代码；能尽早跑 ripgrep 级别的运行时差分。
- **弊：** 前端仍要完整处理 sysroot 源码（见上）。另外新增三项与 full-native 无关、做完就丢的工作：rustc 的符号 mangling 与 crate 哈希、`TypeId` 稳定哈希、泛型实例归属策略。这三项估计 3–5 人周（推断），而且随 rustc 版本变化。
- **风险：** 任何一项不一致都会表现为链接错误，或者更糟的静默错误。例如 `TypeId` 不一致时，`panic!("msg")` 的消息会打印成 `Box<dyn Any>`，`catch_unwind` 后 downcast 失败。差分结果因此不可信，查起来极难。
- **对照红线：** 只能作为单列的“混合 bootstrap”标签，不计入 M4 出口。由于不可复用的成本和静默错误风险，**不建议**作为主线的阶段目标。

## 路线 B：只原生编 core，alloc/std 混链

- **利：** core 没有运行时依赖，先在 core 上验证前端的最难部分（194 个 lang item、const trait、特化、intrinsic）。
- **弊：** 方向相反的 ABI 依赖：rustc 编出的 alloc/std 对象里调用的每个 core 符号、用到的每个 core 类型布局，都要由 kfrc 编出的 core 逐位满足。rustc 的 std 对象里还内联了大量 core 代码副本，同一个函数会有两份实现，一份是 kfrc 的，一份是 rustc 的。rustc 的 std 是对着 rustc 自己的 core metadata（SVH/crate 哈希）编译的，换成别的 core 对象不在任何保证范围内。
- **风险：** 比 A 更高。A 的接缝只在“用户代码 ↔ std”，B 的接缝在 sysroot 内部，调用密度高得多。
- **对照红线：** 同样只能是混合 bootstrap；不能证明 alloc/std 的需求，因为 F 系列需求在 alloc/std 中也大量存在（`default fn` 34/10、`#[rustc_allow_incoherent_impl]` 36/139）。**不建议。**

## 路线 C：全原生（计划主线）

- **利：** 不依赖 rustc 的 ABI 细节（布局/ABI 一致仍然要做，但用途是对拍，不是链接正确性的前提）；产物来源单一，满足 M4 出口；符号命名、`TypeId`、实例归属都由 kfrc 自己定。
- **弊：** 第一档全部需求都在“hello world 能跑”之前（32–48 人周，推断）；core 不能整体通过类型检查时，就产出不了任何 sysroot 产物。
- **降低风险的做法：**
  1. M1–M3 继续用 `#![no_core]` mini core；
  2. T31 先做“core + compiler_builtins + kfrc 自带的极小 `#![no_std]` 运行时”（panic handler、bump 分配器、`_start` 经 libc），以 `panic=abort` 运行 core 测试；
  3. 再加 alloc（测试分配器），最后 std；
  4. unwind 与 asm 子系统并行开发，用 C/汇编夹具单独验收（T27）。

## 唯一推荐的混链：C ABI 部件

compiler-builtins 导出的 169 个函数全部是 `extern "C"`（少数 `unadjusted`），与 libgcc/compiler-rt 的接口相同；`libgcc_s`（unwinder）、glibc 也是 C 库。混链它们只依赖 SysV C ABI（含 i128/f16/f128 的 C 传参规则），不依赖 `repr(Rust)`、mangling 或 `TypeId`。建议：

- 允许在 T31 完成之前，用 libgcc 或 compiler-rt 提供 `__udivti3`、`__addtf3` 等，并标注为“混合 bootstrap（C ABI 部件）”；
- M4 出口要求 compiler_builtins 由 kfrc 原生编译。它是 Rust crate，属于“目标 Rust 闭包”；
- libgcc_s、glibc 按计划作为 C/C++ 依赖单独记账，不影响“目标 Rust 闭包全 native”。

## 结论

| 路线 | 省下的工作 | 新增且不可复用的工作 | 静默错误风险 | 能否作为 M4 出口 |
|---|---|---|---|---|
| A 官方 sysroot 混链 | 后端 runtime 部分可以推迟 | mangling、`TypeId`、实例归属（3–5 人周，推断） | 高 | 否（单列为混合 bootstrap） |
| B 原生 core，alloc/std 混链 | 很少 | 同 A，且接缝在 sysroot 内部 | 很高 | 否 |
| C 全原生 | — | 无 | 低 | 是 |
| C + C ABI 部件过渡 | compiler-builtins 可稍后 | 无 | 低 | 过渡期单列，出口时去掉 |

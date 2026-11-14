# semver 的验收入口与语义边界

日期：2026-11-09。固定负载为 semver **1.0.28**，与既有 Cargo benchmark
的 `Cargo.lock` 一致。crates.io 归档 SHA-256 为
`8a7852d02fc848982e0c167ef163aaff9cd91dc640ba85e263cb1ce46fae51cd`。

最新 candidate19 已在 Linux/E2B 复验通过：semver 原版四套集成测试
**34/34 项在 LLVM/kfrc 两臂全部通过**，可见负载输出完全一致；25 个原始
发布文件构建前后哈希一致；`Version::Debug::fmt` 的 kfrc 对象与最终 driver
符号归属校验通过。测试 harness、std 与部分跨 crate 泛型保留 LLVM，具体
边界如下。此结果不包含性能结论。

## 验收入口

```sh
bash crates/rustc_codegen_kfrc/tests/p3-semver/run.sh <backend.so> <fresh-results>
```

该命令用 Cargo 的按包 `codegen-backend` 配置编译 semver 库，再用 LLVM
编译未修改的上游 integration-test harness 和单独的可见负载。默认 `std`
功能下的四个测试文件合计 **34 项**：`test_autotrait` 1 项、
`test_identifier` 3 项、`test_version` 10 项、`test_version_req` 20 项。
版本中没有源文件单元测试或 `should_panic` 测试。harness 使用
`-Cpanic=abort -Zpanic-abort-tests`，避免把要求 abort 的库与 unwind
配置混用。

验收保留原始 JSON 事件、构建命令、源码和产物哈希；同时要求 semver
rlib 的目标文件带有 kfrc writer 的来源符号。只用 LLVM 编译所有代码并运行
这些测试不构成此次验收。harness、std 和测试中的泛型实例依然由 LLVM
生成，不能据此宣称整套测试程序已由 kfrc 编译。

## 静态排查得到的功能组

下表来自发布归档的源码，属于实现和排查清单，不是已经运行通过的结论。

| 功能组 | semver 中的触发点 | 实现必须保留的语义 |
|---|---|---|
| 大聚合与 ABI | `Version` 的三个整数和两个 identifier、`Comparator`、`Vec<Comparator>` | 真实 rustc field offsets；Direct 小聚合与值地址分开；Pair、byref/sret 调用一致 |
| 小聚合与位重解释 | `Identifier` 是对齐 8 的 `NonNull<u8>` 加零长度尾数组；inline 字符串通过 `[u8;8]` transmute | 8 字节小聚合跨 Direct ABI 时传内容；transmute 保留原始字节，不把对象地址转换成内容 |
| enum 与 niche | `Option<Version>`、`Option<NonZeroUsize>`、`Result<Version,Error>`、`ErrorKind`、`Position` | 原始 discriminant、实际 tag 位宽、niche 范围与 wrapping；不读取非活动 variant 的 typed fields |
| 指针与内存 | `Identifier::new_unchecked`、`clone`、`drop`、varint 头与字符串内容 | copy_nonoverlapping、原始 load/store、指针整数转换、按元素大小的 offset、分配和释放 |
| 位运算 intrinsic | `ptr_to_repr`、`inline_len`、`bytes_for_varint` | rotate_right；little-endian 路径的 leading_zeros；非零输入的 ctlz/cttz 结果位宽 |
| 检查与错误分支 | `numeric_identifier` 的 `checked_mul/checked_add`、错误信息 | checked pair 返回、`?`、`and_then` closure、正确 panic Location 与 abort 行为 |
| slice/str 与迭代器 | `as_bytes().get`、`split_at`、`chars`、`strip_prefix`、`split('.')` | 胖指针 metadata、subslice、UTF-8 iterator、rust-call 参数展开与间接调用 |
| 格式化与常量 | `Display` 的 `write!`、`fmt::Arguments`、错误文本 | 只读常量重定位、函数地址、擦除类型后的 formatter fnptr 与间接调用；不得静默回退 LLVM |

rustc 1.94.1 的 `library/core/src/fmt/rt.rs` 中，`Argument` 使用
`NonNull<()>` 和 formatter 函数指针；这里不把 `write!` 本身误记为必须
支持动态 trait-object 调用。array → slice 的 unsize 与泛型 std 函数仍需
由实际 MIR 边界核对。

semver 的 `Version::fmt`（Debug）实际调用 `DebugStruct::field`，会把
`&u64`、`&Prerelease`、`&BuildMetadata` unsize 为 `&dyn Debug`。
`VersionReq` 与 `Comparator` 的派生 Debug 也产生相同边界。首次 Linux
编译已在 `Version::fmt` 的 `&u64 → &dyn Debug` 转换处停止，因此
formatter 函数指针支持本身不足以覆盖整个库。这里需要真实 vtable 常量、
方法重定位、Pair ABI，以及 LLVM std 回调 kfrc 生成的 Debug 实例。

`dyn-probe.sh` 用独立小库对拍 manual Debug、3 字段和 6 字段派生 Debug、
virtual 调用及捕获闭包。6 字段派生会经过 rustc 的通用 names/values
slice 分支，额外覆盖 `[&dyn Debug; N] → &[&dyn Debug]`。可见 semver
负载也输出 Version/VersionReq 的 Debug，避免仅在失败断言中保留这条链。
candidate 11 的独立探针已经通过，12 行输出与 LLVM 完全一致，并保留了
kfrc object writer 来源证据：
`evidence/poc-20261109/candidate11-results/results/candidate-11/dyn-probe/`。
该独立结果先于 candidate 15 patch 02 的 semver 完整验收。

`test_identifier::test_new` 覆盖空串、1–8 字节 inline、9 字节以上 heap、
多字节 varint 长度，并会构造长字符串。新增可见负载另覆盖
0/1/7/8/9/127/128/1024 长度、clone/drop、版本解析、错误与匹配结果。

## 证据状态

实际 Linux/E2B 构建和运行由本任务的验证 agent 统一串行执行。最早在
candidate 15 patch 02 的原版 34 项集成测试两臂全部通过，可见输出完全
一致。同轮 P1 内存/聚合差分的 1082 行也全部一致。runner 核对发布版四个
`[[test]]` 的完整性、构建前后 25 个源文件的哈希，以及手写
`Version::Debug::fmt` 的 rlib 对象/最终可执行文件符号归属。该实现须定义
在带 `FILE kfrc` 的对象中，并实际产生可见 Debug 输出；这些校验已在此次
实际运行中通过。`VersionReq` 的派生 inline Debug 可能由 LLVM driver 单态化，其
输出不能单独作为 kfrc Debug 实现来源证明。

首次成功的独立证据目录为
[`patch02-call15-results/results/candidate-15/patch-02/semver`](../../../evidence/poc-20261109/patch02-call15-results/results/candidate-15/patch-02/semver/)：

- [`summary.json`](../../../evidence/poc-20261109/patch02-call15-results/results/candidate-15/patch-02/semver/summary.json)
  记录两臂各 34 项测试身份/结果及 Debug 来源，SHA-256 为
  `8a091dcd17d6152fc9dec30767d7d8be13051b82ac5dd5b82c51549fa6d9aad5`。
- [`kfrc/debug-provenance.json`](../../../evidence/poc-20261109/patch02-call15-results/results/candidate-15/patch-02/semver/kfrc/debug-provenance.json)
  记录 `Version::Debug::fmt` 所在 kfrc 对象与 driver 同名定义。
- [`source-verification-after.json`](../../../evidence/poc-20261109/patch02-call15-results/results/candidate-15/patch-02/semver/source-verification-after.json)
  与 before 内容/哈希相同，确认 25 个原始文件及四套测试清单。

候选后端 `.so` SHA-256 为
`c8160ed11d59b123d71afdb750285651ae345ca8777ad0e825dccb783ba4a03f`；
编译器输入归档 SHA-256 为
`fc4437efc03a4e732a1a45eb87fbac14b17b316b1362cc39f6cef522ceaf129b`，
同轮 `source-verification.json` 对 842 文件复核无差异。两臂可见负载各 27 行，
输出文件 SHA-256 同为
`d3da705ee02c167e5831feb21eb71b9b9efd4a8076d0ad309fa7d2baea899354`。

candidate17、18、19 各自完整重跑了 semver，34 项在两臂均全部通过。
最终 candidate19 的
[`semver/summary.json`](../../../evidence/poc-20261109/candidate19-results/results/candidate-19/semver/summary.json)
SHA-256 为
`c3e1fdba91fb68c350f60263d8ed6f53a7b9e9f0eecc592d4b5ace594ca43a07`。
同轮再次通过 25 源文件前后哈希、kfrc `Version::Debug::fmt` 对象和 driver
同符号来源校验，27 行可见输出保持上述同一 SHA-256。
最终后端 SHA-256 为
`b16007417002cdea3a55e465e0d0cb0febddcc7485ebc38e9a86bd95a814ecb4`，
源码归档为
`0357d1d76dff113acd8ff67ab8e817ea64b8152b0f4acaac46f04fd268b0a492`；
[`final-provenance.json`](../../../evidence/poc-20261109/candidate19-results/results/candidate-19/final-provenance.json)
对 867 源文件及后端/工具身份复核一致。

## regex-syntax public API smoke

candidate19 已完成 regex-syntax 0.8.8 默认 `std`/Unicode 功能的 public API
差分：18 个原定 pattern 的成功 HIR/错误输出完全一致，两臂各 30 行，
SHA-256 同为
`f78807c961ef877c20b77dd0cc149d70733ee2cb54d3e00f494460a7e78a5c84`。
regex 库使用 kfrc，driver/std 保留 LLVM。此结果是公共接口负载的通过证明，
不替代内部完整测试。

同一 rlib 的 16 个 kfrc codegen objects 中，8584 个 private allocations
全部为 LOCAL，重复 GLOBAL 定义为 0。
[`data-binding-verification.json`](../../../evidence/poc-20261109/candidate19-results/results/candidate-19/regex-smoke/kfrc/objects/data-binding-verification.json)
保存对象 SHA 和逐符号记录，文件 SHA-256 为
`b5ce5e88eb64c1379b85fda1d6c73290d5225e01ad00e0d1a010ee6a545ca79b`。
该项验证跨 CGU 数据绑定，和实际输出差分分别保存证据。

## regex-syntax 完整内部测试路径

`tests/p3-semver/regex-tests.sh` 固定使用 regex-syntax 0.8.8 原版归档，
SHA-256 为 `7a2d987857b319362043e95f5353c0535c1f58eec5336fdfcf626430af7def58`。
它直接执行 `rustc --test src/lib.rs`，不提取测试、不修改源文件，编译前后
逐文件校验哈希。默认功能闭包从发布版 manifest 读取：`default`、`std`、
`unicode` 及 7 个 Unicode 子功能。测试程序的库代码、内部单元测试和自动
生成的 harness 都经过 kfrc；libtest/std 保留已有 LLVM 机器码。

源码有 158 个词法层面的 `#[test]`，其中包含互斥 cfg。实际 Linux/E2B
LLVM 基线发现并通过 **147/147** 项，failed/ignored/filtered_out 均为 0。
原始事件为
`evidence/poc-20261109/candidate0708-results/results/regex-full/llvm/tests.jsonl`；
同目录保留 `tests.list`。candidate19 的完整 LLVM 基线再次通过 147/147，
failed/ignored/filtered_out 均为 0；kfrc 尚未编译成功，因此内部测试执行数为 0。
脚本用全部测试的实际身份和 JSON 完成事件对拍，不以源码计数代替执行
证明。默认 Unicode 功能下，两处
`#[should_panic]` 的禁用 Unicode 分支不编译；脚本仍统一采用
`-Cpanic=abort -Zpanic-abort-tests`。不运行 doc tests、benchmarks 或
可选 arbitrary feature 的配置。

candidate19 的实际首次编译阻塞为
`core::array::try_from_fn_erased<NeverShortCircuit<hir::literal::Literal>, Drain<...>>`
在 SoN bb12 的 `constructing an uninhabited enum`。完整 kfrc 测试程序尚未
生成，本轮停在该能力边界，没有修改测试源、默认功能或测试集合。
[`regex-full/partial-summary.json`](../../../evidence/poc-20261109/candidate19-results/results/candidate-19/regex-full/partial-summary.json)
明确记录 LLVM 147 通过和 kfrc tests_executed=0，SHA-256 为
`1f24b63b3cc29e6bf96c5fe814138b07e764827161f129433df7914fcbc53b4d`。
[`kfrc/build.log`](../../../evidence/poc-20261109/candidate19-results/results/candidate-19/regex-full/kfrc/build.log)
保留原始完整函数实例与诊断，SHA-256 为
`9f42375abb729759b3d2b2b9ebefd3dae6e7d88e21f4444c0818db9bb84ddcfc`。

官方 rustc 1.94.1 的 `rustc_builtin_macros/src/test_harness.rs` 在 abort
策略下生成 `fn main() -> ()`，调用 `test::test_main_static_abort`。它符合
PoC 现有 unit-main runtime adapter 的入口约束。测试描述对象和
`StaticTestFn` 函数指针仍需通过真实常量池与重定位链，不能替换成手写
测试调度器。

完整测试新增两处小栈线程负载：`ast::tests::no_stack_overflow_on_drop`
和 `hir::tests::no_stack_overflow_on_drop`。它们原样调用
`std::thread::Builder` 的 `spawn/join`，可能通过单态化 std 代码暴露
原子、线程与动态调用边界；这是一组待真实编译核对的风险，尚不是已经
观察到的失败，不提前跳过测试或声称通过。

除线程外，静态检查还得到三组可定位的增量负载：

| 功能组 | 原版源码触发点 | 排查目标 |
|---|---|---|
| 递归 enum 的迭代析构 | `ast/mod.rs:1634` 的 Ast Drop、`:1689` 的 ClassSet Drop；`hir/mod.rs:1913` 的 Hir Drop | Box/Vec 与不同活动 variant、`mem::replace`、`Vec::drain`/`extend`、析构循环的地址与所有权 |
| 泛型排序与查找 | `hir/interval.rs:347`、`hir/literal.rs:1380` 的 sort；`unicode.rs:183/199/477/557/579/614/630` 的 binary search | 比较闭包的 rust-call ABI、切片边界与元素移动；可能单态化的 std 排序实现必须遵守同一内存/ABI 契约 |
| Unicode 常量表与复杂 Debug | `unicode_tables/*`；`hir/mod.rs:1974` 的 PropertiesI（超过 5 字段） | 大型只读常量及内部指针重定位；多字段 Debug 使用 `[&dyn Debug]` slice 和 std 反向回调 |

源码本身带 `#![forbid(unsafe_code)]`，这些安全 API 仍会单态化带底层内存
操作的 std 实现。上表用于在真实首错出现后完整排查同类路径，不据此提前
增加未经触发的后端功能或改变上游测试配置。

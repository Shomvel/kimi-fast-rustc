# 布局与 ABI 一致性测试

## 为什么需要单独一类

[差分测试](differential-testing.md) 只比较可观测行为，布局只用有 `repr` 契约的类型。但 kfrc 的 `repr(Rust)` 布局也必须与 rustc 位级一致：

- x86_64 Rust ABI 的传参方式完全由布局决定（见 [machine-abi.md](../architecture/contracts/machine-abi.md#rust-abi-的尺寸规则x86_64)）。
- `size_of`/`align_of` 是程序可观测的。
- M4 混合 bootstrap 期间 kfrc 对象要与 rustc 编译的对象互相调用。

rustc 的布局算法（`rustc_abi::layout`）不依赖 `TyCtxt`，可以几乎原样移植（[layout.md](../rustc/layout-abi-mono/layout.md)），因此“与 rustc 一致”是可达到的目标。

## 方法

期望值来自 nightly rustc 的调试属性（与参考版本同源的 nightly，或用 `RUSTC_BOOTSTRAP=1`），具体属性和输出格式见 [test-cases.md](../rustc/layout-abi-mono/test-cases.md)：

- `#[rustc_layout(debug)]`：size、align、字段偏移、variants、tag 编码、niche、`BackendRepr`。
- `#[rustc_abi(debug)]`：每个参数和返回值的 `PassMode`。

kfrc 提供对应的 dump（`--dump=layout`、`--dump=fn-abi`），runner 比较两边的规范化文本。

## 用例分组

| 组 | 例子 |
|---|---|
| 字段重排 | 混合 `u8/u16/u32/u64` 的结构体、含 ZST 字段、泛型结构体不同实例 |
| enum | 无字段、带字段、niche（`Option<&T>`、`Option<NonZeroU32>`、`Option<bool>`、嵌套 Option）、显式判别值、`repr(u8)` |
| repr | `C`、`transparent`、`packed(N)`、`align(N)`、`u8…i64` |
| ABI | 上表 4 条尺寸规则的边界：8/9/16/17 字节、`(f64, f64)`、`(i64, bool)`、`&[u8]`、`&dyn Trait` |

这类测试与行为差分分开计数；标签为 `layout-parity`/`abi-parity`。

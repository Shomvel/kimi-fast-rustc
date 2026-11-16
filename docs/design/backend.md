# 调用边界与机器层

机器层的结构、导出映射、存储 lane、Cranelift 的版本与补丁、目标文件分别见
[machine/layer.md](machine/layer.md)、[machine/export.md](machine/export.md)、
[machine/storage-lanes.md](machine/storage-lanes.md)、[machine/cranelift.md](machine/cranelift.md)、
[machine/object.md](machine/object.md)。本文只定义调用边界：图里记录什么，机器层据此传递什么。

## FunctionAbi

图为每个函数、每个直接调用目标和每个间接调用记录一份 `kfrc_body::FunctionAbi`：

```rust
pub struct FunctionAbi {
    pub args: Vec<PassMode>,   // 每个 MIR 参数一项，按顺序
    pub ret: PassMode,
    pub c_variadic: bool,
}
pub enum PassMode {
    Ignore,                    // 零大小：不传递任何位
    Direct,                    // 一个整数寄存器
    Pair,                      // 两个寄存器（&[T]、(i64, bool)）
    Vector { bytes: u8 },      // 一个 SIMD 寄存器
    Cast(CastAbi),             // 位转换成至多两个 eightbyte 的整数 lane
    Indirect,                  // 按指针传递
    IndirectOnStack,           // C ABI 的按值内存参数
}
```

命名与语义对齐 rustc 的 `PassMode`，取值直接来自 rustc 对该实例的 `FnAbi` 查询
（`crates/rustc_codegen_kfrc/src/abi.rs`）：同一个聚合在 Rust 函数与 C 函数里可以有不同的传递方式，所以它属于调用，
不属于类型。`Pair` 的两个字段及其偏移记录在类型答案表的 `abi_pairs` 里；`CastAbi` 记录每个整数 lane 的偏移与宽度、
是否必须连续、是否带 `i32` 填充、扩展方式。

`Ignore` 只表示不运输位，不表示源值不存在、已有效或可正常返回。R/S 保留源码签名、Return 值及其契约；
普通 ZST、Never 和 MaybeUninit 分别处理。要求与证明引用在声明的布局边界消解，见
[requirements-and-facts](ir/requirements-and-facts.md)。

## L 级的边界形状

布局降低之后，标量参数、实参、返回值是标量节点；聚合是其存储的地址，类型为 `LayoutValue<T>`。
寄存器的选择在机器层：导出器把 `PassMode` 翻译成签名里的 lane（[machine/export.md](machine/export.md#调用边界)），
聚合的各 lane 来自存储 lane 或内存（[machine/storage-lanes.md](machine/storage-lanes.md)）。

## Rust ABI 的尺寸规则（x86_64）

x86_64 上 Rust ABI 完全由布局决定（细节见 [fn-abi.md](../research/rustc/layout-abi-mono/fn-abi.md)）：

| 情况 | 传递方式 |
|---|---|
| 返回值 > 16 字节 | sret（`Indirect`） |
| `BackendRepr::Memory` 且 ≤ 8 字节 | 当作一个整数 `Cast` |
| `BackendRepr::Memory` 且 > 8 字节 | 按指针（`Indirect`） |
| `ScalarPair` | 拆成两个参数（`Pair`） |

所以 `#[repr(C)] struct F(f64)` 和无 repr 的同名结构体走不同寄存器——ABI 正确性依赖
[布局与 rustc 一致](../research/rustc/layout-abi-mono/test-cases.md)。

## 隐式参数

`#[track_caller]` 函数和所有 panic lang item 多一个尾随参数 `&'static Location`。Location 常量内含指向文件名 `&str`
的指针，因此第一个可能 panic 的程序（溢出/越界检查）就需要隐式参数和带重定位的只读数据。

## 当前范围

浮点值、宽于 64 位的整数、16 字节以外的向量 ABI、`IndirectOnStack`、C 可变参数在机器层报“不支持”，
编译该 crate 失败并指明函数。展开表（`.eh_frame`）尚未生成，产物以 `-Cpanic=abort` 构建。

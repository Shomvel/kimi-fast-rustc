# T02 冻结共享数据与阶段契约 — 细化

`M0 · I · 依赖：T00` · 概要见 [m0-baseline.md](../m0-baseline.md#t02) · 契约草案见 [design/](../../design/README.md)

## 现状里需要被替换的隐式契约

| 现在 | 位置 | 问题 |
|---|---|---|
| 类型只存在于 `Parser.types: HashMap<NodeId, ScalarType>` | frontend.rs | SoN 节点本身没有类型；解析结束后类型丢失，后端把所有值当 i64 |
| 函数身份 = 声明序号，`NodeKind::Call(usize)` | son.rs | 跨模块、泛型实例、缓存键都无法表达 |
| 整个程序共用一个 `Graph`，用 `FunctionMetadata { start, return_node }` 切分 | son.rs:204 | 函数图不能独立并行修改 |
| `Node.memory` 预留但无语义；`native` 遇到即报 `UnsupportedReason::MemoryInputs` | son.rs、native.rs:39 | 内存效果没有定义 |
| `Graph::validate` 只查输入存在与元数 | son.rs:432 | 不查类型、支配、Phi 与 Region 的对应 |
| 变量 = 名字字符串，`HashMap<String, Binding>` | frontend.rs | 遮蔽靠栈维护，各控制结构各自合并（T01 的根因） |

## 交付物

1. [design/](../../design/README.md) 下每个契约一份（ids、frontend/types、frontend/body、ir/graph、ir/verifier、backend）：字段级 Rust 类型草案、不变量、谁产生谁消费。
2. 一个贯穿例子（下方程序）在每层的完整表示：AST 片段、Typed Body、SoN 节点表、Machine IR、对象符号。
3. 未支持的类型/效果的表示方式：类型层用 `TyKind::Unsupported(reason)` 之类的显式变体，lowering 遇到即报 `UnsupportedFeature`。

贯穿例子：

```rust
fn add_if(f: bool, a: i64) -> i64 {
    let mut x = a;
    if f { x = x + 1; }
    x
}
fn main_entry(a: i64) -> i64 { add_if(a > 0, a) }
```

## 完成标志

- 贯穿例子的每个 ID（DefKey、LocalId、BlockId、NodeId、InstanceKey、符号名）能从源码一路追到对象文件。
- A/B/C/D/E 各角色确认自己的输入输出；没有两套 TypeId 或函数编号。
- 契约文件标注版本号，后续修改走版本递增。

# ID 契约（draft-0）

## 新类型

```rust
// 构建图层：跨进程、可持久化
pub struct CrateKey(Fingerprint);          // crate 名 + 版本 + 源指纹 + features + target
pub struct DefKey { krate: CrateKey, path: DefPathHash }   // 规范化定义路径的哈希

// 语义快照层：一次编译会话内有效
pub struct DefId(u32);                     // 会话内稠密编号，映射到 DefKey
pub struct TyId(u32);                      // 驻留在 TypeInterner
pub struct LayoutId(u32);                  // 驻留在 LayoutInterner，键为 (TyId, TargetSpecId)
pub struct BodyId(u32);                    // 一个函数/常量体

// 函数体内
pub struct LocalId(u32);                   // Body 内局部变量，遮蔽产生新的 LocalId
pub struct BlockId(u32);                   // Body 的基本块

// 实例层
pub struct InstanceKey { kind: InstanceKind, def: DefKey, args: GenericArgsHash }  // 持久；kind 区分 Item/ReifyShim/ClosureOnceShim/DropGlue/VTableShim 等，const 参数按 valtree 哈希
pub struct InstanceId(u32);                // 会话内编号

// 图内
pub struct NodeId(u32);                    // 只在一个 FunctionGraph 内有效
```

## 规则

- **持久 vs 会话。** `CrateKey`、`DefKey`、`InstanceKey`、`Fingerprint` 可以写进缓存和 metadata；`DefId`、`TyId`、`LayoutId`、`InstanceId`、`NodeId` 只在会话内有效，持久化时必须转换成持久键。
- **驻留表只追加。** 会话内 `TypeInterner` 只追加；并行任务读快照，新建类型走同步驻留（初版一把锁即可）。
- **DefPathHash** 由规范化路径计算：`crate::module::item` + 消歧器（同名 impl、闭包、宏生成的同名项按创建顺序编号）。与 rustc 一致，DefKey 不含 hygiene；“完成顺序不影响 DefKey”依赖展开顺序确定，并行展开时按展开树中的位置定序（见 [T05](../../roadmap/m1/t05-resolve.md#defkey-与-hygiene)）。
- **符号名** 由 `InstanceKey` 确定性生成（v0 mangling 风格或自有格式），用于对象文件和链接。
- **跨图引用** 一律用 `InstanceId`/`DefId`，不用另一个图的 `NodeId`。

## 与现状的映射

| 现在 | 替换为 |
|---|---|
| `NodeKind::Call(usize)`（声明序号） | `Call { callee: InstanceId }`，符号名来自 `InstanceKey` |
| `FunctionMetadata.name: String` | `InstanceKey` + 确定性符号名 |
| `HashMap<NodeId, ScalarType>`（frontend 内） | 节点自带 `ValueTy`（见 [son.md](son.md)），源类型在 Body 中用 `TyId` |
| `Binding` 按名字查找 | Body 中的 `LocalId` |
